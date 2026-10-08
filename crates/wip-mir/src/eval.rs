//! Running a program's own code while it is compiled: the
//! functions that compute the constants folding could not work out.
//!
//! What runs is the MIR every backend compiles, statement by statement, on
//! a memory laid out as the target's is — the same layouts, the same
//! widths, a pointer being an address — so that `offsetBytes`, a `Vec`'s
//! slots and a table read as they do in the program. The heap is the
//! interpreter's own, and so are the few functions of C the standard
//! library is written over (`memcpy`, `malloc` and the like); any other call
//! into C is refused, since C cannot be run here and need not answer the
//! same twice. A panic, a call that cannot be made, and a computation past
//! its budget are errors at the constant, with the calls that led to them.

use std::rc::Rc;

use rustc_hash::{FxHashMap, FxHashSet};
use wip_hir::{
    BinaryOp, ConstId, ConstValue, FloatTy, FnId, InterfaceId, Program, Ty, TyKind, UnaryOp,
};
use wip_syntax::{Diagnostic, Interner, Span, Symbol, codes};

use crate::*;

/// Where a place in the source is: its file, line and column.
pub type Locate<'a> = &'a dyn Fn(Span) -> (String, u32, u32);

/// How many statements a constant may run, the constants it reads
/// included: about two seconds.
const STEPS: u64 = 100_000_000;
/// How many bytes of memory it may use.
const BYTES: u64 = 512 << 20;
/// The first address of memory: below it, as at run time, is no value's.
const BASE: u64 = 0x1_0000;
/// Where the addresses of functions begin: never memory, only a number a
/// call through a function value is taken back from.
const CODE: u64 = 0x7F00_0000_0000_0000;
/// What a move leaves in a build that checks moves, as the
/// code generator writes it.
const POISON: u64 = 0xA5A5_A5A5_A5A5_A5A5;
/// The words the runtime keeps its counters in.
const RUNTIME_WORDS: u64 = 8;

/// Works out every constant that is to be run, and sets its value, then
/// checks every top-level `assert`; answers what went
/// wrong, each at the constant or the assert it went wrong in.
pub fn evaluate_constants(
    program: &mut Program,
    interner: &mut Interner,
    locate: Locate<'_>,
) -> Vec<Diagnostic> {
    evaluate_within(program, interner, locate, STEPS)
}

/// The same, with a budget of `steps` for each constant.
fn evaluate_within(
    program: &mut Program,
    interner: &mut Interner,
    locate: Locate<'_>,
    steps: u64,
) -> Vec<Diagnostic> {
    let mut ids: Vec<ConstId> = program
        .consts
        .iter()
        .filter(|(_, def)| def.code.is_some() && def.value.is_none())
        .map(|(id, _)| id)
        .collect();
    if ids.is_empty() && program.asserts.is_empty() {
        return Vec::new();
    }
    ids.sort_by_key(|id| id.into_raw());
    let mut diagnostics = Vec::new();
    let mut values: Vec<(ConstId, Built)> = Vec::new();
    {
        let mut machine = Machine::new(program, interner, steps);
        for id in ids {
            // Each constant has a budget of its own.
            machine.steps = 0;
            match machine.constant(id) {
                Ok(address) => {
                    let ty = program.consts[id].ty;
                    match machine.built(address, ty) {
                        Ok(built) => values.push((id, built)),
                        Err(stop) => {
                            let failure = Failure {
                                stop,
                                calls: Vec::new(),
                            };
                            let subject = Subject::Constant(id);
                            if let Some(d) = machine.report(subject, failure, locate) {
                                diagnostics.push(d);
                            }
                        }
                    }
                }
                Err(failure) => {
                    if let Some(d) = machine.report(Subject::Constant(id), failure, locate) {
                        diagnostics.push(d);
                    }
                }
            }
        }
        // Each top-level assert, once every constant it may read is known.
        for id in program.asserts.clone() {
            machine.steps = 0;
            if let Err(stop) = machine.call(Callable::Fn(id), Vec::new(), None) {
                let failure = Failure {
                    stop,
                    calls: machine.stack(),
                };
                machine.calls.clear();
                if let Some(d) = machine.report(Subject::Assert(id), failure, locate) {
                    diagnostics.push(d);
                }
            }
        }
    }
    for (id, built) in values {
        program.consts[id].value = Some(built.into_value(interner));
    }
    diagnostics
}

/// A constant's value as the machine read it back, its strings not yet
/// interned.
enum Built {
    Int(u128),
    Float(f64),
    Bool(bool),
    Str(String),
    Array(Vec<Built>),
    Struct(Vec<Built>),
    Variant { variant: u32, fields: Vec<Built> },
}

impl Built {
    fn into_value(self, interner: &mut Interner) -> ConstValue {
        let all = |parts: Vec<Built>, interner: &mut Interner| {
            parts
                .into_iter()
                .map(|part| part.into_value(interner))
                .collect()
        };
        match self {
            Built::Int(bits) => ConstValue::Int(bits),
            Built::Float(value) => ConstValue::Float(value),
            Built::Bool(value) => ConstValue::Bool(value),
            Built::Str(text) => ConstValue::Str(interner.intern(&text)),
            Built::Array(parts) => ConstValue::Array(all(parts, interner)),
            Built::Struct(parts) => ConstValue::Struct(all(parts, interner)),
            Built::Variant { variant, fields } => ConstValue::Variant {
                variant,
                fields: all(fields, interner),
            },
        }
    }
}

/// What was being run when something went wrong.
#[derive(Clone, Copy)]
enum Subject {
    Constant(ConstId),
    /// A top-level `assert`'s function.
    Assert(FnId),
}

/// Why running stopped.
enum Stop {
    /// A panic, with its message and where it was written.
    Panic { message: String, at: Span },
    /// Something that cannot be done while the program is compiled.
    Cannot(String),
    /// The budget of steps or of memory ran out.
    Budget(&'static str),
    /// A constant it needed had failed, which was reported there.
    Quiet,
}

/// A stop, and the calls it happened in, innermost first: each function's
/// name and the place it had reached.
struct Failure {
    stop: Stop,
    calls: Vec<(String, Span)>,
}

/// What can be called through an address: a function, or the drop
/// function of an `own` of a type.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Callable {
    Fn(FnId),
    Drop(Ty),
    DropInPlace(Ty),
}

/// An argument: a scalar's bits, or an aggregate's address.
#[derive(Clone, Copy)]
enum Arg {
    Scalar(u128),
    Address(u64),
}

/// One call being run: its body, and the address of each local.
struct Frame {
    body: Rc<Body>,
    locals: Vec<u64>,
    /// The locals whose memory the frame took, freed when it returns.
    owned: Vec<(u64, u64)>,
}

struct Machine<'a> {
    program: &'a Program,
    interner: &'a Interner,
    layouts: Layouts,
    memory: Vec<u8>,
    free: FxHashMap<u64, Vec<u64>>,
    bodies: FxHashMap<Callable, Rc<Body>>,
    code: Vec<Callable>,
    code_of: FxHashMap<Callable, u64>,
    strings: FxHashMap<Symbol, u64>,
    tables: FxHashMap<ConstId, u64>,
    vtables: FxHashMap<(InterfaceId, Ty), u64>,
    words: Option<u64>,
    steps: u64,
    /// How many steps a constant may take.
    budget: u64,
    /// The calls being run, outermost first, each with the place it has
    /// reached.
    calls: Vec<(String, Span)>,
    evaluating: Vec<ConstId>,
    failed: FxHashSet<ConstId>,
}

type Run<T> = Result<T, Stop>;

impl<'a> Machine<'a> {
    fn new(program: &'a Program, interner: &'a Interner, budget: u64) -> Self {
        Machine {
            program,
            interner,
            layouts: Layouts::new(),
            memory: vec![0; BASE as usize],
            free: FxHashMap::default(),
            bodies: FxHashMap::default(),
            code: Vec::new(),
            code_of: FxHashMap::default(),
            strings: FxHashMap::default(),
            tables: FxHashMap::default(),
            vtables: FxHashMap::default(),
            words: None,
            steps: 0,
            budget,
            calls: Vec::new(),
            evaluating: Vec::new(),
            failed: FxHashSet::default(),
        }
    }

    // --- Constants -------------------------------------------------------

    /// The address of a constant's value: run once, where it is to be run,
    /// and laid out as a table otherwise.
    fn constant(&mut self, id: ConstId) -> Result<u64, Failure> {
        if let Some(&address) = self.tables.get(&id) {
            return Ok(address);
        }
        let quiet = || Failure {
            stop: Stop::Quiet,
            calls: Vec::new(),
        };
        if self.failed.contains(&id) {
            return Err(quiet());
        }
        let def = &self.program.consts[id];
        let (ty, code) = (def.ty, def.code);
        let Some(code) = code else {
            return self.table(id).map_err(|stop| Failure {
                stop,
                calls: Vec::new(),
            });
        };
        if self.evaluating.contains(&id) {
            let name = self.interner.resolve(def.name);
            return Err(Failure {
                stop: Stop::Cannot(format!(
                    "`{name}` is defined in terms of itself, through what it runs"
                )),
                calls: self.stack(),
            });
        }
        self.evaluating.push(id);
        let depth = self.calls.len();
        let size = self.size(ty).max(1);
        let result = self.alloc(size).and_then(|address| {
            if let Some(bits) = self.call(Callable::Fn(code), Vec::new(), Some(address))? {
                self.store(address, ty, bits)?;
            }
            Ok(address)
        });
        self.evaluating.pop();
        match result {
            Ok(address) => {
                self.calls.truncate(depth);
                self.tables.insert(id, address);
                Ok(address)
            }
            Err(stop) => {
                let calls = self.stack();
                self.calls.truncate(depth);
                self.failed.insert(id);
                Err(Failure { stop, calls })
            }
        }
    }

    /// A constant folding worked out, laid out in memory as the program
    /// keeps it, with its strings where the pool has them.
    fn table(&mut self, id: ConstId) -> Run<u64> {
        let def = &self.program.consts[id];
        let Some(value) = &def.value else {
            return Err(Stop::Quiet);
        };
        let table = table_bytes(
            self.program,
            self.interner,
            &mut self.layouts,
            value,
            def.ty,
            cfg!(target_endian = "little"),
        );
        let address = self.alloc(table.bytes.len().max(1) as u64)?;
        self.write_bytes(address, &table.bytes)?;
        for (offset, sym) in table.strings {
            let text = self.string(sym)?;
            self.write_word(address + u64::from(offset), text)?;
        }
        // A file's bytes, embedded.
        for (offset, bytes) in table.blobs {
            let blob = self.alloc(bytes.len().max(1) as u64)?;
            self.write_bytes(blob, &bytes)?;
            self.write_word(address + u64::from(offset), blob)?;
        }
        self.tables.insert(id, address);
        Ok(address)
    }

    /// The error a failure is, at the constant or the assert it happened
    /// in; none where the failure was another constant's, already
    /// reported.
    fn report(&self, subject: Subject, failure: Failure, locate: Locate<'_>) -> Option<Diagnostic> {
        let (span, working, cannot) = match subject {
            Subject::Constant(id) => {
                let def = &self.program.consts[id];
                let name = self.interner.resolve(def.name);
                (
                    def.span,
                    format!("working out `{name}`"),
                    format!("`{name}` cannot be worked out while the program is compiled"),
                )
            }
            Subject::Assert(id) => (
                self.program.fns[id].span,
                "checking this assert".to_string(),
                "this assert cannot be checked while the program is compiled".to_string(),
            ),
        };
        let calls = |mut diagnostic: Diagnostic| {
            for (function, at) in &failure.calls {
                let (file, line, _) = locate(*at);
                diagnostic = diagnostic.with_note(format!("in {function} ({file}:{line})"));
            }
            diagnostic
        };
        let diagnostic = match &failure.stop {
            Stop::Quiet => return None,
            // The assert's own condition, not a panic in what it calls: the
            // message it would print at run time.
            Stop::Panic { message, .. }
                if matches!(subject, Subject::Assert(_)) && failure.calls.len() == 1 =>
            {
                Diagnostic::error(
                    codes::ASSERTION_FAILED,
                    message.clone(),
                    span,
                    "does not hold while the program is compiled",
                )
                .with_note(
                    "a top-level assert is checked while the program is compiled, for the target it is compiled for",
                )
            }
            Stop::Panic { message, at } => {
                let (file, line, column) = locate(*at);
                let diagnostic = Diagnostic::error(
                    codes::CONSTANT_PANICKED,
                    format!("{working} panicked: {message}"),
                    span,
                    "panicked while the program was compiled",
                )
                .with_note(format!("at {file}:{line}:{column}"));
                calls(diagnostic)
            }
            Stop::Cannot(why) => calls(
                Diagnostic::error(codes::NOT_RUN_AT_COMPILE_TIME, cannot, span, why.clone())
                    .with_note(
                        "what runs while the program is compiled is the program's own code; C, the time, files and threads are not there until the program runs",
                    ),
            ),
            Stop::Budget(what) => calls(
                Diagnostic::error(
                    codes::EVALUATION_TOO_LONG,
                    format!("{working} took too {what}"),
                    span,
                    "stopped while the program was compiled",
                )
                .with_note(format!(
                    "a constant or an assert may run {} steps and use {} MiB",
                    self.budget,
                    BYTES >> 20
                )),
            ),
        };
        Some(diagnostic)
    }

    /// The calls being run, innermost first.
    fn stack(&self) -> Vec<(String, Span)> {
        self.calls.iter().rev().cloned().collect()
    }

    // --- Reading a value back --------------------------------------------

    /// The value of type `ty` at `address`, as a constant holds it.
    fn built(&mut self, address: u64, ty: Ty) -> Run<Built> {
        let program = self.program;
        Ok(match program.types.kind(ty) {
            TyKind::Int(_) | TyKind::Char => Built::Int(self.load(address, ty)?),
            TyKind::Float(FloatTy::F32) => {
                let bits = self.load(address, ty)? as u32;
                Built::Float(f64::from(f32::from_bits(bits)))
            }
            TyKind::Float(FloatTy::F64) => {
                Built::Float(f64::from_bits(self.load(address, ty)? as u64))
            }
            TyKind::Bool => Built::Bool(self.load(address, ty)? != 0),
            TyKind::Str => {
                let pointer = self.read_word(address)?;
                let length = self.read_word(address + 8)?;
                Built::Str(self.text(pointer, length)?)
            }
            TyKind::Cstring => {
                let pointer = self.read_word(address)?;
                let length = self.strlen(pointer)?;
                Built::Str(self.text(pointer, length)?)
            }
            TyKind::Array(elem, count) => {
                let stride = u64::from(self.layouts.of(program, elem).stride());
                let mut parts = Vec::with_capacity(count as usize);
                for i in 0..count {
                    parts.push(self.built(address + i * stride, elem)?);
                }
                Built::Array(parts)
            }
            TyKind::Struct(..) => {
                let fields = program.field_tys(ty);
                let mut parts = Vec::with_capacity(fields.len());
                for (i, field) in fields.into_iter().enumerate() {
                    let offset = self.layouts.field_offset(program, ty, i as u32);
                    parts.push(self.built(address + u64::from(offset), field)?);
                }
                Built::Struct(parts)
            }
            TyKind::Enum(..) => {
                let variant = self.variant(address, ty)?;
                let tys = program.variant_field_tys(ty);
                let mut fields = Vec::new();
                for (i, field) in tys[variant as usize].iter().enumerate() {
                    let offset = self.layouts.variant_offset(program, ty, variant, i as u32);
                    fields.push(self.built(address + u64::from(offset), *field)?);
                }
                Built::Variant { variant, fields }
            }
            TyKind::Ptr(_) if self.read_word(address)? == 0 => Built::Int(0),
            _ => {
                return Err(Stop::Cannot(format!(
                    "a constant cannot hold {}, which points into memory that is gone once the program is compiled",
                    program.ty_name(ty, self.interner)
                )));
            }
        })
    }

    // --- Calls -----------------------------------------------------------

    fn body(&mut self, callable: Callable) -> Run<Rc<Body>> {
        if let Some(body) = self.bodies.get(&callable) {
            return Ok(body.clone());
        }
        let body = match callable {
            Callable::Fn(id) => {
                function_body(self.program, self.interner, id).ok_or_else(|| {
                    Stop::Cannot(format!(
                        "calls `{}`, which has no body to run",
                        self.program.fn_name(id, self.interner)
                    ))
                })?
            }
            Callable::Drop(ty) => lower_drop_fn(self.program, self.interner, ty),
            Callable::DropInPlace(ty) => lower_drop_in_place_fn(self.program, self.interner, ty),
        };
        let body = Rc::new(body);
        self.bodies.insert(callable, body.clone());
        Ok(body)
    }

    fn name_of(&self, callable: Callable) -> String {
        match callable {
            Callable::Fn(id) => self.program.fn_name(id, self.interner),
            Callable::Drop(ty) | Callable::DropInPlace(ty) => {
                format!("drop<{}>", self.program.ty_name(ty, self.interner))
            }
        }
    }

    /// Runs a call: its arguments, and where an aggregate result goes.
    /// Answers a scalar result's bits.
    fn call(&mut self, callable: Callable, args: Vec<Arg>, ret: Option<u64>) -> Run<Option<u128>> {
        if let Callable::Fn(id) = callable {
            let def = &self.program.fns[id];
            if def.is_extern || def.accesses.is_some() || def.variadic_of.is_some() {
                return self.c_call(id, &args);
            }
        }
        let body = self.body(callable)?;
        let name = self.name_of(callable);
        let at = match callable {
            Callable::Fn(id) => self.program.fns[id].span,
            _ => Span::default(),
        };
        self.calls.push((name, at));
        let mut frame = Frame {
            body: body.clone(),
            locals: Vec::with_capacity(body.locals.len()),
            owned: Vec::new(),
        };
        let mut args = args.into_iter();
        for (i, decl) in body.locals.iter().enumerate() {
            let local = Local(i as u32);
            let aggregate = is_aggregate(self.program, decl.ty);
            let address = match decl.kind {
                LocalKind::Param if aggregate => match args.next() {
                    Some(Arg::Address(address)) => address,
                    _ => unreachable!("an aggregate argument is passed by address"),
                },
                LocalKind::Return if aggregate && ret.is_some() => ret.expect("checked"),
                _ => {
                    let size = self.size(decl.ty).max(1);
                    let address = self.alloc(size)?;
                    frame.owned.push((address, size));
                    address
                }
            };
            frame.locals.push(address);
            if decl.kind == LocalKind::Param && !aggregate {
                match args.next() {
                    Some(Arg::Scalar(bits)) => self.store(address, decl.ty, bits)?,
                    _ => unreachable!("a scalar argument is passed by value"),
                }
            }
            let _ = local;
        }
        let answer = self.run(&frame)?;
        for (address, size) in frame.owned {
            self.release(address, size);
        }
        self.calls.pop();
        Ok(answer)
    }

    /// Runs a body from its first block to a `return`.
    fn run(&mut self, frame: &Frame) -> Run<Option<u128>> {
        let body = frame.body.clone();
        let mut block = BlockId(0);
        loop {
            let this = body.block(block);
            for statement in &this.statements {
                self.step()?;
                self.statement(frame, statement)?;
            }
            self.step()?;
            match &this.terminator {
                Terminator::Goto(target) => block = *target,
                Terminator::Branch {
                    cond,
                    then,
                    otherwise,
                } => {
                    block = match self.scalar(frame, cond)? != 0 {
                        true => *then,
                        false => *otherwise,
                    }
                }
                Terminator::Switch {
                    value,
                    cases,
                    otherwise,
                } => {
                    // Compared whole: a number past what a case holds is
                    // no case, rather than the case its low bits are.
                    let value = self.scalar(frame, value)?;
                    block = cases
                        .iter()
                        .find(|&&(case, _)| u128::from(case) == value)
                        .map_or(*otherwise, |&(_, target)| target);
                }
                Terminator::Return => {
                    return match body.ret {
                        Some(ret) if !is_aggregate(self.program, body.local(ret).ty) => {
                            let ty = body.local(ret).ty;
                            Ok(Some(self.load(frame.locals[ret.0 as usize], ty)?))
                        }
                        _ => Ok(None),
                    };
                }
                Terminator::Panic { note, message, at } => {
                    let mut text = String::new();
                    if let Some(Operand::Copy(place)) = note {
                        let address = self.place(frame, place)?;
                        let pointer = self.read_word(address)?;
                        let length = self.read_word(address + 8)?;
                        text.push_str(&self.text(pointer, length)?);
                    }
                    if let Some(message) = message {
                        text.push_str(self.interner.resolve(*message));
                    }
                    return Err(Stop::Panic {
                        message: text,
                        at: *at,
                    });
                }
                Terminator::Unreachable => {
                    return Err(Stop::Cannot(
                        "reached code that no path was to reach".to_string(),
                    ));
                }
            }
        }
    }

    fn step(&mut self) -> Run<()> {
        self.steps += 1;
        match self.steps > self.budget {
            true => Err(Stop::Budget("long")),
            false => Ok(()),
        }
    }

    fn statement(&mut self, frame: &Frame, statement: &Statement) -> Run<()> {
        let program = self.program;
        match statement {
            Statement::At(span) => {
                if let Some(call) = self.calls.last_mut() {
                    call.1 = *span;
                }
            }
            Statement::Assign(place, rvalue) => {
                let ty = place_ty(program, &frame.body, place);
                if is_aggregate(program, ty) {
                    let Rvalue::Use(Operand::Copy(source)) = rvalue else {
                        unreachable!("an aggregate is assigned by copying a place")
                    };
                    let dest = self.place(frame, place)?;
                    let source = self.place(frame, source)?;
                    let size = self.size(ty);
                    self.copy(dest, source, size)?;
                    return Ok(());
                }
                if self.size(ty) == 0 {
                    return Ok(());
                }
                let value = self.rvalue(frame, rvalue, ty)?;
                let address = self.place(frame, place)?;
                self.store(address, ty, value)?;
            }
            Statement::SetVariant(place, variant) => {
                let ty = place_ty(program, &frame.body, place);
                let address = self.place(frame, place)?;
                match self.layouts.enum_tag(program, ty) {
                    Tag::Byte { size } => {
                        self.write_int(address, u64::from(size), u128::from(*variant))?
                    }
                    Tag::Niche { empty, offset, .. } if *variant == empty => {
                        self.write_word(address + u64::from(offset), 0)?
                    }
                    Tag::Niche { .. } => {}
                }
            }
            Statement::Zero(place) => {
                let ty = place_ty(program, &frame.body, place);
                let size = self.size(ty);
                if size > 0 {
                    let address = self.place(frame, place)?;
                    self.fill(address, size, 0)?;
                }
            }
            Statement::Poison(place) => {
                let ty = place_ty(program, &frame.body, place);
                let size = self.size(ty);
                let address = self.place(frame, place)?;
                let mut offset = 0;
                while offset + 8 <= size {
                    self.write_word(address + offset, POISON)?;
                    offset += 8;
                }
                self.fill(address + offset, size - offset, POISON as u8)?;
            }
            Statement::CheckMoved { place, at } => {
                let ty = place_ty(program, &frame.body, place);
                let size = self.size(ty).min(8);
                if size > 0 {
                    let address = self.place(frame, place)?;
                    let first = self.read_int(address, size)?;
                    let pattern = u128::from(POISON) & mask(size as u32 * 8);
                    if first == pattern {
                        return Err(Stop::Panic {
                            message: "a value was dropped after it was moved away".to_string(),
                            at: *at,
                        });
                    }
                }
            }
            Statement::Call { callee, args, dest } => {
                let values = args
                    .iter()
                    .map(|arg| self.arg(frame, arg))
                    .collect::<Run<Vec<Arg>>>()?;
                let ret_ty = dest
                    .as_ref()
                    .map(|dest| place_ty(program, &frame.body, dest));
                let aggregate = ret_ty.is_some_and(|ty| is_aggregate(program, ty));
                let dest_address = match dest {
                    Some(dest) => Some(self.place(frame, dest)?),
                    None => None,
                };
                let answer = match callee {
                    Callee::StrCmp => Some(self.str_cmp(&values)?),
                    Callee::Fn(id) => {
                        let ret = if aggregate { dest_address } else { None };
                        self.call(Callable::Fn(*id), values, ret)?
                    }
                    Callee::Value(value) => {
                        let address = self.scalar(frame, value)? as u64;
                        let callable = self.callable(address)?;
                        let ret = if aggregate { dest_address } else { None };
                        self.call(callable, values, ret)?
                    }
                };
                if let (Some(bits), Some(address), Some(ty), false) =
                    (answer, dest_address, ret_ty, aggregate)
                {
                    self.store(address, ty, bits)?;
                }
            }
            Statement::Alloc { dest, ty } => {
                let size = self.size(*ty).max(1);
                let pointer = self.alloc(size)?;
                let address = self.place(frame, dest)?;
                self.write_word(address, pointer)?;
            }
            Statement::AllocBuffer { dest, elem, count } => {
                let count = self.scalar(frame, count)? as u64 as i64;
                let stride = i64::from(self.layouts.of(program, *elem).stride());
                let Some(size) = count.checked_mul(stride) else {
                    return Err(Stop::Budget("much memory"));
                };
                let pointer = self.alloc((size as u64).max(1))?;
                let address = self.place(frame, dest)?;
                self.write_word(address, pointer)?;
                self.write_word(address + 8, count as u64)?;
            }
            Statement::Free(pointer) => {
                let _ = self.scalar(frame, pointer)?;
            }
            Statement::DropFn { ty, ptr } => {
                let pointer = self.scalar(frame, ptr)?;
                self.call(Callable::Drop(*ty), vec![Arg::Scalar(pointer)], None)?;
            }
            Statement::DropInPlace { ty, ptr } => {
                let pointer = self.scalar(frame, ptr)?;
                self.call(Callable::DropInPlace(*ty), vec![Arg::Scalar(pointer)], None)?;
            }
            Statement::Arith {
                dest,
                op,
                lhs,
                rhs,
                at,
            } => {
                let ty = place_ty(program, &frame.body, dest);
                let (l, r) = (self.scalar(frame, lhs)?, self.scalar(frame, rhs)?);
                let Some(value) = checked(program, ty, *op, l, r) else {
                    return Err(Stop::Panic {
                        message: overflow_message(*op).to_string(),
                        at: *at,
                    });
                };
                let address = self.place(frame, dest)?;
                self.store(address, ty, value)?;
            }
            // One thread computes a constant, so each is a plain step.
            Statement::Atomic {
                op,
                ty,
                address,
                value,
                expected,
                dest,
            } => {
                let address = self.scalar(frame, address)? as u64;
                let (bits, _) = int_of(program, *ty);
                let before = self.load(address, *ty)?;
                let operand = |machine: &mut Self| {
                    machine.scalar(frame, value.as_ref().expect("an atomic's value"))
                };
                let answer = match op {
                    AtomicOp::Add => {
                        let amount = operand(self)?;
                        self.store(address, *ty, before.wrapping_add(amount) & mask(bits))?;
                        Some(before)
                    }
                    AtomicOp::Subtract => {
                        let amount = operand(self)?;
                        self.store(address, *ty, before.wrapping_sub(amount) & mask(bits))?;
                        Some(before)
                    }
                    AtomicOp::Swap => {
                        let stored = operand(self)?;
                        self.store(address, *ty, stored & mask(bits))?;
                        Some(before)
                    }
                    AtomicOp::CompareSwap => {
                        let wanted =
                            self.scalar(frame, expected.as_ref().expect("an expected value"))?;
                        let stored = operand(self)?;
                        if before == wanted & mask(bits) {
                            self.store(address, *ty, stored & mask(bits))?;
                        }
                        Some(before)
                    }
                    AtomicOp::Load => Some(before),
                    AtomicOp::Store => {
                        let stored = operand(self)?;
                        self.store(address, *ty, stored & mask(bits))?;
                        None
                    }
                };
                if let (Some(dest), Some(answer)) = (dest, answer) {
                    let address = self.place(frame, dest)?;
                    self.store(address, *ty, answer)?;
                }
            }
            Statement::Check { fails, kind, at } => {
                if self.scalar(frame, fails)? == 0 {
                    return Ok(());
                }
                let message = match kind {
                    CheckKind::Bounds { index, length } => {
                        let index = self.scalar(frame, index)? as u64 as i64;
                        let length = self.scalar(frame, length)? as u64 as i64;
                        format!("index {index} is out of bounds for length {length}")
                    }
                    CheckKind::Length { length } => {
                        let length = self.scalar(frame, length)? as u64 as i64;
                        format!("a buffer cannot have length {length}")
                    }
                    CheckKind::Division => "divided by zero".to_string(),
                    CheckKind::Overflow(op) => overflow_message(*op).to_string(),
                };
                return Err(Stop::Panic { message, at: *at });
            }
        }
        Ok(())
    }

    /// An argument: a scalar's bits, or an aggregate's address.
    fn arg(&mut self, frame: &Frame, operand: &Operand) -> Run<Arg> {
        let ty = operand_ty(self.program, &frame.body, operand);
        if is_aggregate(self.program, ty) {
            let Operand::Copy(place) = operand else {
                unreachable!("an aggregate operand is a place")
            };
            return Ok(Arg::Address(self.place(frame, place)?));
        }
        Ok(Arg::Scalar(self.scalar(frame, operand)?))
    }

    /// What a function of C does, where it is one of those the standard
    /// library is written over; any other is refused.
    fn c_call(&mut self, id: FnId, args: &[Arg]) -> Run<Option<u128>> {
        let def = &self.program.fns[id];
        let name = self.interner.resolve(def.symbol.unwrap_or(def.name));
        // What the program calls it, which C may call otherwise.
        let written = self.interner.resolve(def.name);
        let word = |i: usize| match args.get(i) {
            Some(Arg::Scalar(bits)) => *bits as u64,
            Some(Arg::Address(address)) => *address,
            None => 0,
        };
        let answer = match name {
            "memcpy" | "memmove" => {
                self.copy(word(0), word(1), word(2))?;
                u128::from(word(0))
            }
            "memset" => {
                self.fill(word(0), word(2), word(1) as u8)?;
                u128::from(word(0))
            }
            "memcmp" => {
                let (a, b, count) = (word(0), word(1), word(2));
                let order = self.read_bytes(a, count)?.cmp(self.read_bytes(b, count)?);
                (order as i32) as u32 as u128
            }
            "strlen" => u128::from(self.strlen(word(0))?),
            "malloc" => u128::from(self.alloc(word(0).max(1))?),
            "calloc" => {
                let size = word(0).saturating_mul(word(1)).max(1);
                let pointer = self.alloc(size)?;
                self.fill(pointer, size, 0)?;
                u128::from(pointer)
            }
            "realloc" => {
                let (old, size) = (word(0), word(1).max(1));
                let pointer = self.alloc(size)?;
                if old != 0 {
                    let kept = size.min(self.memory.len() as u64 - old);
                    self.copy(pointer, old, kept)?;
                }
                u128::from(pointer)
            }
            "free" if def.accesses.is_none() => 0,
            _ if def.accesses.is_some() => {
                return Err(Stop::Cannot(format!(
                    "reads `{written}`, which is C's, and C is not there while the program is compiled"
                )));
            }
            _ => {
                return Err(Stop::Cannot(format!(
                    "calls `{written}`, which is C's, and C does not run while the program is compiled"
                )));
            }
        };
        Ok(Some(answer))
    }

    /// The runtime's comparison of two strings' bytes: the
    /// order of their shared bytes, then of their lengths.
    fn str_cmp(&mut self, args: &[Arg]) -> Run<u128> {
        let word = |i: usize| match args[i] {
            Arg::Scalar(bits) => bits as u64,
            Arg::Address(address) => address,
        };
        let (a, a_len, b, b_len) = (word(0), word(1), word(2), word(3));
        let a = self.read_bytes(a, a_len)?.to_vec();
        let b = self.read_bytes(b, b_len)?;
        let order = match a.as_slice().cmp(b) {
            std::cmp::Ordering::Less => -1i64,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        Ok(u128::from(order as u64))
    }

    // --- Places and values -----------------------------------------------

    /// The address of a place.
    fn place(&mut self, frame: &Frame, place: &Place) -> Run<u64> {
        let program = self.program;
        let mut address = frame.locals[place.local.0 as usize];
        let mut ty = frame.body.local(place.local).ty;
        for &projection in &place.projections {
            address = match projection {
                Projection::Deref => self.read_word(address)?,
                Projection::Field(index) => {
                    let offset = match program.types.kind(ty) {
                        TyKind::Struct(..) => self.layouts.field_offset(program, ty, index),
                        // A `str`'s or a slice's pointer, then its length.
                        _ => index * 8,
                    };
                    address + u64::from(offset)
                }
                Projection::VariantField { variant, field } => {
                    address + u64::from(self.layouts.variant_offset(program, ty, variant, field))
                }
                Projection::Index(_) | Projection::ConstIndex(_) => {
                    let elem = element_ty(program, ty).expect("elements");
                    // An array's elements are where the array is; any
                    // other's are where its pointer points.
                    let first = match program.types.kind(ty) {
                        TyKind::Array(..) => address,
                        _ => self.read_word(address)?,
                    };
                    let stride = i128::from(self.layouts.of(program, elem).stride());
                    let index = match projection {
                        Projection::Index(local) => {
                            let local_ty = frame.body.local(local).ty;
                            let bits = self.load(frame.locals[local.0 as usize], local_ty)?;
                            signed(program, local_ty, bits)
                        }
                        Projection::ConstIndex(index) => i128::from(index),
                        _ => unreachable!(),
                    };
                    (i128::from(first) + index * stride) as u64
                }
            };
            ty = project_ty(program, ty, projection);
        }
        Ok(address)
    }

    /// The bits of a scalar operand.
    fn scalar(&mut self, frame: &Frame, operand: &Operand) -> Run<u128> {
        match operand {
            Operand::Copy(place) => {
                let ty = place_ty(self.program, &frame.body, place);
                if self.size(ty) == 0 {
                    return Ok(0);
                }
                let address = self.place(frame, place)?;
                self.load(address, ty)
            }
            Operand::Const(constant) => self.constant_bits(constant),
        }
    }

    fn constant_bits(&mut self, constant: &Const) -> Run<u128> {
        Ok(match *constant {
            Const::Int { bits, .. } => bits,
            Const::Float { value, ty } => match self.program.types.kind(ty) {
                TyKind::Float(FloatTy::F32) => u128::from((value as f32).to_bits()),
                _ => u128::from(value.to_bits()),
            },
            Const::Bool(value) => u128::from(value),
            Const::CStr(sym) => u128::from(self.string(sym)?),
            Const::Table { id, .. } => match self.constant(id) {
                Ok(address) => u128::from(address),
                Err(failure) => return Err(failure.stop),
            },
            Const::Fn { id, .. } => u128::from(self.code_address(Callable::Fn(id))),
            Const::DropFn(ty) => u128::from(self.code_address(Callable::Drop(ty))),
            Const::VTable { interface, ty } => u128::from(self.vtable(interface, ty)?),
            Const::RuntimeWords => u128::from(self.runtime_words()?),
            Const::FrameTables => {
                return Err(Stop::Cannot(
                    "reads the tables a panic walks, which the program has only once it runs"
                        .to_string(),
                ));
            }
            Const::SizeOf { of, .. } => u128::from(self.layouts.of(self.program, of).stride()),
            Const::AlignOf { of, .. } => u128::from(self.layouts.of(self.program, of).align),
        })
    }

    fn rvalue(&mut self, frame: &Frame, rvalue: &Rvalue, ty: Ty) -> Run<u128> {
        let program = self.program;
        Ok(match rvalue {
            Rvalue::Use(operand) => self.scalar(frame, operand)?,
            Rvalue::Unary(op, operand) => {
                let value = self.scalar(frame, operand)?;
                let bits = (self.size(ty) * 8) as u32;
                match (op, program.types.kind(ty)) {
                    (UnaryOp::Neg, TyKind::Float(FloatTy::F32)) => value ^ (1 << 31),
                    (UnaryOp::Neg, TyKind::Float(_)) => value ^ (1 << 63),
                    (UnaryOp::Neg, _) => 0u128.wrapping_sub(value) & mask(bits),
                    (UnaryOp::Not, TyKind::Bool) => value ^ 1,
                    (UnaryOp::Not, _) => !value & mask(bits),
                }
            }
            Rvalue::Binary(op, lhs, rhs) => {
                let operand = operand_ty(program, &frame.body, lhs);
                let (l, r) = (self.scalar(frame, lhs)?, self.scalar(frame, rhs)?);
                match binary(program, operand, *op, l, r) {
                    Some(value) => value,
                    None => {
                        return Err(Stop::Cannot(
                            "divides by zero, or overflows a division, where no check stood before it"
                                .to_string(),
                        ));
                    }
                }
            }
            Rvalue::Cast(operand, to) => {
                let from = operand_ty(program, &frame.body, operand);
                let value = self.scalar(frame, operand)?;
                cast(program, value, from, *to)
            }
            Rvalue::AddressOf(place) => u128::from(self.place(frame, place)?),
            Rvalue::Variant(place) => {
                let place_ty = place_ty(program, &frame.body, place);
                let address = self.place(frame, place)?;
                u128::from(self.variant(address, place_ty)?)
            }
            Rvalue::CstrLen(operand) => {
                let pointer = self.scalar(frame, operand)? as u64;
                u128::from(self.strlen(pointer)?)
            }
            // Exact on every machine, so exactly what the program's
            // instruction gives.
            Rvalue::Float(op, operand) => {
                let bits = self.scalar(frame, operand)?;
                match program.types.kind(ty) {
                    TyKind::Float(FloatTy::F32) => {
                        let value = f32::from_bits(bits as u32);
                        let answer = match op {
                            FloatOp::Sqrt => value.sqrt(),
                            FloatOp::Floor => value.floor(),
                            FloatOp::Ceil => value.ceil(),
                            FloatOp::Trunc => value.trunc(),
                        };
                        u128::from(answer.to_bits())
                    }
                    _ => {
                        let value = f64::from_bits(bits as u64);
                        let answer = match op {
                            FloatOp::Sqrt => value.sqrt(),
                            FloatOp::Floor => value.floor(),
                            FloatOp::Ceil => value.ceil(),
                            FloatOp::Trunc => value.trunc(),
                        };
                        u128::from(answer.to_bits())
                    }
                }
            }
            // The same bits, of the same width, read as the other type.
            Rvalue::Bits(operand) => self.scalar(frame, operand)?,
            // One rounding, which IEEE 754 defines: Rust's is correctly
            // rounded wherever the compiler runs.
            Rvalue::MulAdd(first, second, third) => {
                let first = self.scalar(frame, first)?;
                let second = self.scalar(frame, second)?;
                let third = self.scalar(frame, third)?;
                match program.types.kind(ty) {
                    TyKind::Float(FloatTy::F32) => {
                        let of = |bits: u128| f32::from_bits(bits as u32);
                        u128::from(of(first).mul_add(of(second), of(third)).to_bits())
                    }
                    _ => {
                        let of = |bits: u128| f64::from_bits(bits as u64);
                        u128::from(of(first).mul_add(of(second), of(third)).to_bits())
                    }
                }
            }
            // An integer's bits, within its width.
            Rvalue::Integer(op, operand) => {
                let (bits, _) = int_of(program, ty);
                let value = self.scalar(frame, operand)? & mask(bits);
                match op {
                    IntegerOp::CountOnes => u128::from(value.count_ones()),
                    IntegerOp::LeadingZeros => u128::from(value.leading_zeros() - (128 - bits)),
                    IntegerOp::TrailingZeros => u128::from(value.trailing_zeros().min(bits)),
                    IntegerOp::SwapBytes => value.swap_bytes() >> (128 - bits),
                    IntegerOp::ReverseBits => value.reverse_bits() >> (128 - bits),
                }
            }
            Rvalue::Rotate(turn, value, amount) => {
                let (bits, _) = int_of(program, ty);
                let value = self.scalar(frame, value)? & mask(bits);
                // Modulo the width, as a shift's amount is.
                let amount = self.scalar(frame, amount)? as u32 % bits;
                let left = match turn {
                    Turn::Left => amount,
                    Turn::Right => (bits - amount) % bits,
                };
                match left {
                    0 => value,
                    left => ((value << left) | (value >> (bits - left))) & mask(bits),
                }
            }
            // Whether the answer would not fit: what a checked `+` panics
            // on.
            Rvalue::Overflows(op, lhs, rhs) => {
                let operand = operand_ty(program, &frame.body, lhs);
                let (l, r) = (self.scalar(frame, lhs)?, self.scalar(frame, rhs)?);
                u128::from(checked(program, operand, *op, l, r).is_none())
            }
            Rvalue::VTableFn { table, index, .. } => {
                let table = self.scalar(frame, table)? as u64;
                u128::from(self.read_word(table + u64::from(*index) * 8)?)
            }
        })
    }

    /// Which variant the enum at `address` holds.
    fn variant(&mut self, address: u64, ty: Ty) -> Run<u32> {
        Ok(match self.layouts.enum_tag(self.program, ty) {
            Tag::Byte { size } => self.read_int(address, u64::from(size))? as u32,
            Tag::Niche {
                payload,
                empty,
                offset,
            } => match self.read_word(address + u64::from(offset))? {
                0 => empty,
                _ => payload,
            },
        })
    }

    // --- Addresses that are not memory ------------------------------------

    fn code_address(&mut self, callable: Callable) -> u64 {
        if let Some(&address) = self.code_of.get(&callable) {
            return address;
        }
        let address = CODE + self.code.len() as u64 * 16;
        self.code.push(callable);
        self.code_of.insert(callable, address);
        address
    }

    fn callable(&self, address: u64) -> Run<Callable> {
        let index = address.wrapping_sub(CODE) / 16;
        match address >= CODE && address.wrapping_sub(CODE).is_multiple_of(16) {
            true => self.code.get(index as usize).copied(),
            false => None,
        }
        .ok_or_else(|| Stop::Cannot("calls through an address that is no function's".to_string()))
    }

    /// A type's table of methods for an interface.
    fn vtable(&mut self, interface: InterfaceId, ty: Ty) -> Run<u64> {
        if let Some(&address) = self.vtables.get(&(interface, ty)) {
            return Ok(address);
        }
        let methods = self
            .program
            .vtables
            .iter()
            .find(|table| table.interface == interface && table.ty == ty)
            .expect("every table a program needs was made with its instances")
            .methods
            .clone();
        let address = self.alloc((methods.len() as u64 * 8).max(8))?;
        for (slot, method) in methods.into_iter().enumerate() {
            let code = self.code_address(Callable::Fn(method));
            self.write_word(address + slot as u64 * 8, code)?;
        }
        self.vtables.insert((interface, ty), address);
        Ok(address)
    }

    fn runtime_words(&mut self) -> Run<u64> {
        if let Some(address) = self.words {
            return Ok(address);
        }
        let address = self.alloc(RUNTIME_WORDS * 8)?;
        self.words = Some(address);
        Ok(address)
    }

    /// A string's bytes, with a NUL after them, kept once.
    fn string(&mut self, sym: Symbol) -> Run<u64> {
        if let Some(&address) = self.strings.get(&sym) {
            return Ok(address);
        }
        let text = self.interner.resolve(sym).as_bytes().to_vec();
        let address = self.alloc(text.len() as u64 + 1)?;
        self.write_bytes(address, &text)?;
        self.strings.insert(sym, address);
        Ok(address)
    }

    // --- Memory ------------------------------------------------------------

    fn size(&mut self, ty: Ty) -> u64 {
        u64::from(self.layouts.of(self.program, ty).size)
    }

    /// Memory for `size` bytes, zeroed, aligned for anything.
    fn alloc(&mut self, size: u64) -> Run<u64> {
        let size = size.next_multiple_of(16);
        if let Some(address) = self.free.get_mut(&size).and_then(Vec::pop) {
            self.fill(address, size, 0)?;
            return Ok(address);
        }
        let address = self.memory.len() as u64;
        if address + size > BYTES {
            return Err(Stop::Budget("much memory"));
        }
        self.memory.resize((address + size) as usize, 0);
        Ok(address)
    }

    fn release(&mut self, address: u64, size: u64) {
        self.free
            .entry(size.next_multiple_of(16))
            .or_default()
            .push(address);
    }

    fn range(&self, address: u64, len: u64) -> Run<std::ops::Range<usize>> {
        let end = address.checked_add(len);
        match end {
            Some(end) if address >= BASE && end <= self.memory.len() as u64 => {
                Ok(address as usize..end as usize)
            }
            _ if len == 0 => Ok(0..0),
            _ => Err(Stop::Cannot(format!(
                "reads or writes memory at {address:#x}, which is no value's"
            ))),
        }
    }

    fn read_bytes(&self, address: u64, len: u64) -> Run<&[u8]> {
        let range = self.range(address, len)?;
        Ok(&self.memory[range])
    }

    fn write_bytes(&mut self, address: u64, bytes: &[u8]) -> Run<()> {
        let range = self.range(address, bytes.len() as u64)?;
        self.memory[range].copy_from_slice(bytes);
        Ok(())
    }

    fn copy(&mut self, dest: u64, source: u64, len: u64) -> Run<()> {
        let from = self.range(source, len)?;
        let to = self.range(dest, len)?;
        self.memory.copy_within(from, to.start);
        Ok(())
    }

    fn fill(&mut self, address: u64, len: u64, byte: u8) -> Run<()> {
        let range = self.range(address, len)?;
        self.memory[range].fill(byte);
        Ok(())
    }

    fn read_int(&self, address: u64, size: u64) -> Run<u128> {
        let bytes = self.read_bytes(address, size)?;
        let mut word = [0u8; 16];
        word[..size as usize].copy_from_slice(bytes);
        Ok(u128::from_le_bytes(word))
    }

    fn write_int(&mut self, address: u64, size: u64, bits: u128) -> Run<()> {
        let bytes = bits.to_le_bytes();
        self.write_bytes(address, &bytes[..size as usize])
    }

    fn read_word(&self, address: u64) -> Run<u64> {
        Ok(self.read_int(address, 8)? as u64)
    }

    fn write_word(&mut self, address: u64, word: u64) -> Run<()> {
        self.write_int(address, 8, u128::from(word))
    }

    /// A scalar of type `ty` at `address`.
    fn load(&mut self, address: u64, ty: Ty) -> Run<u128> {
        let size = self.size(ty);
        self.read_int(address, size)
    }

    fn store(&mut self, address: u64, ty: Ty, bits: u128) -> Run<()> {
        let size = self.size(ty);
        self.write_int(address, size, bits)
    }

    fn strlen(&self, pointer: u64) -> Run<u64> {
        let start = self.range(pointer, 1)?.start;
        match self.memory[start..].iter().position(|&byte| byte == 0) {
            Some(length) => Ok(length as u64),
            None => Err(Stop::Cannot("reads a C string that has no end".to_string())),
        }
    }

    fn text(&self, pointer: u64, length: u64) -> Run<String> {
        let bytes = self.read_bytes(pointer, length)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

// --- Arithmetic, as the code generator does it -----------------------------

/// The low `bits` bits.
fn mask(bits: u32) -> u128 {
    if bits >= 128 { !0 } else { (1u128 << bits) - 1 }
}

/// A scalar type's width and whether it is signed: an integer's own, and
/// every other scalar's as the unsigned number it is held in.
fn int_of(program: &Program, ty: Ty) -> (u32, bool) {
    match program.types.kind(ty) {
        TyKind::Int(t) => (t.bits(), t.signed()),
        TyKind::Char => (32, false),
        TyKind::Bool => (8, false),
        _ => (64, false),
    }
}

/// The bits of a value of type `ty`, as the signed number they are.
fn signed(program: &Program, ty: Ty, bits: u128) -> i128 {
    let (width, is_signed) = int_of(program, ty);
    if !is_signed || width >= 128 {
        return bits as i128;
    }
    let shift = 128 - width;
    ((bits << shift) as i128) >> shift
}

fn overflow_message(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "the sum does not fit",
        BinaryOp::Sub => "the difference does not fit",
        BinaryOp::Mul => "the product does not fit",
        _ => "the answer does not fit",
    }
}

/// `+`, `-` or `*` that panics where the answer does not fit:
/// `None` then.
fn checked(program: &Program, ty: Ty, op: BinaryOp, l: u128, r: u128) -> Option<u128> {
    let (bits, is_signed) = int_of(program, ty);
    if is_signed {
        let (a, b) = (signed(program, ty, l), signed(program, ty, r));
        let answer = match op {
            BinaryOp::Add => a.checked_add(b)?,
            BinaryOp::Sub => a.checked_sub(b)?,
            BinaryOp::Mul => a.checked_mul(b)?,
            _ => unreachable!("only `+`, `-` and `*` are checked this way"),
        };
        if bits < 128 {
            let (low, high) = (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1);
            if answer < low || answer > high {
                return None;
            }
        }
        return Some(answer as u128 & mask(bits));
    }
    let answer = match op {
        BinaryOp::Add => l.checked_add(r)?,
        BinaryOp::Sub => l.checked_sub(r)?,
        BinaryOp::Mul => l.checked_mul(r)?,
        _ => unreachable!("only `+`, `-` and `*` are checked this way"),
    };
    (answer <= mask(bits)).then_some(answer)
}

/// A binary operator on operands of type `operand`; `None` for a division
/// the machine traps on.
fn binary(program: &Program, operand: Ty, op: BinaryOp, l: u128, r: u128) -> Option<u128> {
    use BinaryOp::*;
    let bool_of = |b: bool| u128::from(b);
    match program.types.kind(operand) {
        TyKind::Float(FloatTy::F32) => {
            let (a, b) = (f32::from_bits(l as u32), f32::from_bits(r as u32));
            let bits = |v: f32| u128::from(v.to_bits());
            return Some(match op {
                Add => bits(a + b),
                Sub => bits(a - b),
                Mul => bits(a * b),
                Div => bits(a / b),
                Eq => bool_of(a == b),
                Ne => bool_of(a != b),
                Lt => bool_of(a < b),
                Le => bool_of(a <= b),
                Gt => bool_of(a > b),
                Ge => bool_of(a >= b),
                _ => unreachable!("the type checker rejects `{op:?}` on floats"),
            });
        }
        TyKind::Float(_) => {
            let (a, b) = (f64::from_bits(l as u64), f64::from_bits(r as u64));
            let bits = |v: f64| u128::from(v.to_bits());
            return Some(match op {
                Add => bits(a + b),
                Sub => bits(a - b),
                Mul => bits(a * b),
                Div => bits(a / b),
                Eq => bool_of(a == b),
                Ne => bool_of(a != b),
                Lt => bool_of(a < b),
                Le => bool_of(a <= b),
                Gt => bool_of(a > b),
                Ge => bool_of(a >= b),
                _ => unreachable!("the type checker rejects `{op:?}` on floats"),
            });
        }
        _ => {}
    }
    let (bits, is_signed) = int_of(program, operand);
    let m = mask(bits);
    let (a, b) = (signed(program, operand, l), signed(program, operand, r));
    Some(match op {
        Add => l.wrapping_add(r) & m,
        Sub => l.wrapping_sub(r) & m,
        Mul => l.wrapping_mul(r) & m,
        Div | Rem if r & m == 0 => return None,
        Div if is_signed => a.checked_div(b)? as u128 & m,
        Div => (l & m) / (r & m),
        Rem if is_signed => a.checked_rem(b)? as u128 & m,
        Rem => (l & m) % (r & m),
        BitAnd => l & r & m,
        BitOr => (l | r) & m,
        BitXor => (l ^ r) & m,
        // The amount is taken modulo the width.
        Shl => (l << (r as u32 % bits)) & m,
        Shr if is_signed => (a >> (r as u32 % bits)) as u128 & m,
        Shr => (l & m) >> (r as u32 % bits),
        Eq => bool_of(l & m == r & m),
        Ne => bool_of(l & m != r & m),
        Lt | Le | Gt | Ge => {
            let order = match is_signed {
                true => a.cmp(&b),
                false => (l & m).cmp(&(r & m)),
            };
            bool_of(match op {
                Lt => order.is_lt(),
                Le => order.is_le(),
                Gt => order.is_gt(),
                _ => order.is_ge(),
            })
        }
        And | Or => unreachable!("`&&` and `||` are branches in MIR"),
    })
}

/// `value as to`: widening by the source's sign,
/// narrowing to the low bits, floats toward zero and saturating.
fn cast(program: &Program, value: u128, from: Ty, to: Ty) -> u128 {
    let float = |ty: Ty| match program.types.kind(ty) {
        TyKind::Float(f) => Some(f),
        _ => None,
    };
    match (float(from), float(to)) {
        (Some(FloatTy::F32), Some(FloatTy::F64)) => {
            u128::from(f64::from(f32::from_bits(value as u32)).to_bits())
        }
        (Some(FloatTy::F64), Some(FloatTy::F32)) => {
            u128::from((f64::from_bits(value as u64) as f32).to_bits())
        }
        (Some(_), Some(_)) => value,
        (Some(f), None) => {
            let v = match f {
                FloatTy::F32 => f64::from(f32::from_bits(value as u32)),
                FloatTy::F64 => f64::from_bits(value as u64),
            };
            let (bits, is_signed) = int_of(program, to);
            let int = match (bits, is_signed) {
                (8, true) => v as i8 as i128 as u128,
                (16, true) => v as i16 as i128 as u128,
                (32, true) => v as i32 as i128 as u128,
                (64, true) => v as i64 as i128 as u128,
                (128, true) => v as i128 as u128,
                (8, false) => u128::from(v as u8),
                (16, false) => u128::from(v as u16),
                (32, false) => u128::from(v as u32),
                (64, false) => u128::from(v as u64),
                _ => v as u128,
            };
            int & mask(bits)
        }
        (None, Some(f)) => {
            let (_, is_signed) = int_of(program, from);
            let v = signed(program, from, value);
            match (f, is_signed) {
                (FloatTy::F32, true) => u128::from((v as f32).to_bits()),
                (FloatTy::F32, false) => u128::from((value as f32).to_bits()),
                (FloatTy::F64, true) => u128::from((v as f64).to_bits()),
                (FloatTy::F64, false) => u128::from((value as f64).to_bits()),
            }
        }
        (None, None) => {
            let (to_bits, _) = int_of(program, to);
            (signed(program, from, value) as u128) & mask(to_bits)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The constants `src` declares, worked out with a budget of `steps`:
    /// what each came to, by name, and the errors.
    fn evaluate(src: &str, steps: u64) -> (Vec<(String, Option<ConstValue>)>, Vec<Diagnostic>) {
        let mut interner = Interner::new();
        let lexed = wip_syntax::lex(src, &mut interner);
        let parsed = wip_syntax::parse(src, &lexed);
        let lowered = wip_hir::lower_file(&parsed.ast, &interner);
        assert!(
            !lowered.diagnostics.iter().any(Diagnostic::is_error),
            "{:#?}",
            lowered.diagnostics
        );
        let mut program = lowered.program;
        let locate = |_: Span| ("main.wip".to_string(), 0, 0);
        let diagnostics = evaluate_within(&mut program, &mut interner, &locate, steps);
        let values = program
            .consts
            .iter()
            .map(|(_, def)| (interner.resolve(def.name).to_string(), def.value.clone()))
            .collect();
        (values, diagnostics)
    }

    /// A loop that never ends runs out of its budget, and is an error at
    /// its constant; one that ends within it is worked out.
    #[test]
    fn a_constant_has_a_budget_of_steps() {
        let src = "fn spin(): i64 = {\n    var n = 0\n    while true {\n        n += 1\n        n -= 1\n    }\n    return n\n}\n\nfn sum(to: i64): i64 = {\n    var total = 0\n    for i in 0..to {\n        total += i\n    }\n    return total\n}\n\n@comptime\nval FOREVER: i64 = spin()\n@comptime\nval SMALL: i64 = sum(10)\n";
        let (values, diagnostics) = evaluate(src, 1_000);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].code, codes::EVALUATION_TOO_LONG);
        let small = values.iter().find(|(name, _)| name == "SMALL");
        assert_eq!(
            small.map(|(_, v)| v.clone()),
            Some(Some(ConstValue::Int(45)))
        );
    }
}
