//! Cranelift lowering: MIR to native object files.
//!
//! This is the debug path, built for the speed of compiling: no
//! optimization, and aggregates (structs, enums, arrays, strings and slices)
//! always in memory. A scalar local whose address is never taken is an SSA
//! variable; every other local has a stack slot. Aggregates are passed by
//! pointer and returned through a pointer the caller supplies. Only scalars
//! cross the C boundary, so none of this leaks into the
//! platform ABI.
//!
//! The bodies come from `wip-mir`, which has already decided
//! every drop and every evaluation order, so this crate translates one
//! statement at a time.

mod cpu;
mod debuginfo;
mod debugtypes;
mod frames;
mod function;
mod module;
mod numeric;
mod place;
mod unwind;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    self, AbiParam, ArgumentPurpose, InstBuilder, MemFlagsData, Signature, SourceLoc, StackSlot,
    StackSlotData, StackSlotKind, TrapCode, UserFuncName, Value, types,
};
use cranelift_codegen::isa::{CallConv, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use la_arena::ArenaMap;
use rustc_hash::FxHashMap;
use wip_hir::{BinaryOp, FloatTy, FnDef, FnId, Program, Ty, TyKind, Types, UnaryOp};
use wip_mir::c_abi::{Answer, Arch, CCall, Pass, Promoted, Reg};
use wip_mir::{
    self as mir, AtomicOp, Const, Layout, Layouts, Local, LocalKind, Operand, Place, Projection,
    Rvalue, Statement, Tag, Terminator,
};
use wip_syntax::{Diagnostic, Interner, Symbol, parallel};

/// The Cranelift integer type of a width in bits.
fn int_type(bits: u32) -> ir::Type {
    match bits {
        8 => types::I8,
        16 => types::I16,
        32 => types::I32,
        64 => types::I64,
        _ => types::I128,
    }
}

/// Compiles a type-checked program to object files, one per module, on up to
/// [`parallel::threads`] threads. The root module's object defines the C entry
/// point `main`, unless what is being written is a library, which has none.
///
/// Every program the checkers accept is compiled; the `Err` case is kept for
/// features the code generator may lag behind in again. Any failure is a
/// compiler bug and panics.
/// Where a span was written, for the message of a panic:
/// the file as diagnostics name it, and the line and column, counted from
/// one.
pub type Locations<'a> = &'a (dyn Fn(wip_syntax::Span) -> (String, u32, u32) + Sync);

/// What the program starts at: its `main`, or the tests `wip test` runs.
/// A library starts at neither.
#[derive(Clone, Copy)]
pub enum Entry<'a> {
    Main(FnId),
    /// Each test, in the order the program declares them.
    Tests(&'a [FnId]),
}

pub use cpu::{Cpu, Level};

/// How a build compiles the program.
#[derive(Debug, Clone, Copy, Default)]
pub struct Settings {
    /// Cranelift's optimisations: a release build's.
    pub optimize: bool,
    /// The processor the program is built for.
    pub cpu: Cpu,
}

/// Compiles the program to one object per module, as `settings` say.
pub fn compile(
    program: &Program,
    interner: &Interner,
    main: Option<Entry<'_>>,
    locations: Locations<'_>,
    settings: Settings,
) -> Result<Vec<Vec<u8>>, Vec<Diagnostic>> {
    // Which calls into C go through a wrapper, decided once for every
    // module.
    let arch = Arch::host();
    let shimmed = mir::c_abi::shimmed(program, arch);
    let c = CallsToC {
        arch,
        shimmed: &shimmed,
    };
    let program_wide = program_wide(program);
    let count = program.modules.len().max(1);
    let mut modules: Vec<Codegen<'_>> = (0..count)
        .map(|index| {
            open_module(
                program,
                interner,
                index as u32,
                locations,
                c,
                &program_wide,
                settings,
            )
        })
        .collect();
    // What is compiled is what the program can reach: from
    // where it starts, and from what C may call, each module compiles what
    // it was asked for and says what that referred to in the others, until
    // nothing new is asked for.
    let mut asked: Vec<Vec<Found>> = vec![Vec::new(); count];
    for (id, def) in program.fns.iter() {
        if def.exports_c && program.is_compiled(id) {
            asked[def.module as usize].push(Found::Function(id));
        }
    }
    match main {
        Some(Entry::Main(main)) => modules[0].define_entry(main),
        Some(Entry::Tests(tests)) => modules[0].define_test_entry(tests),
        None => {}
    }
    let started = std::mem::take(&mut modules[0].found);
    let ask = |asked: &mut Vec<Vec<Found>>, found: Found| match found {
        Found::Function(id) => {
            if program.is_compiled(id) {
                asked[program.fns[id].module as usize].push(found);
            }
        }
        Found::Table(id) => asked[program.consts[id].module as usize].push(found),
    };
    // Most of it can be seen in the calls the program writes, before any
    // is compiled, and is asked for at once, so that the modules are
    // compiled side by side, not one after what another found in it.
    let written = written_calls(program, &asked, &started);
    for found in started.into_iter().chain(written) {
        ask(&mut asked, found);
    }
    loop {
        let mut work: Vec<(&mut Codegen<'_>, Vec<Found>)> = modules
            .iter_mut()
            .zip(asked.iter_mut())
            .filter(|(_, asked)| !asked.is_empty())
            .map(|(module, asked)| (module, std::mem::take(asked)))
            .collect();
        if work.is_empty() {
            break;
        }
        // A thread takes a run of modules, as many as there are roots to
        // start from in them; the answers come back in the modules' order,
        // so what is compiled, and in what order, does not depend on the
        // threads.
        let weights: Vec<usize> = work.iter().map(|(_, asked)| asked.len()).collect();
        let mut shares = Vec::new();
        for range in parallel::split(&weights, parallel::threads())
            .into_iter()
            .rev()
        {
            shares.push(work.split_off(range.start));
        }
        shares.reverse();
        let found = parallel::run(shares, |share| {
            share
                .into_iter()
                .flat_map(|(module, asked)| module.compile_reachable(asked))
                .collect::<Vec<_>>()
        });
        for found in found.into_iter().flatten() {
            ask(&mut asked, found);
        }
    }
    let weights: Vec<usize> = modules
        .iter()
        .map(|module| module.described.len())
        .collect();
    let mut shares = Vec::new();
    for range in parallel::split(&weights, parallel::threads())
        .into_iter()
        .rev()
    {
        shares.push(modules.split_off(range.start));
    }
    shares.reverse();
    let objects = parallel::run(shares, |share| {
        share
            .into_iter()
            .map(|module| finish_module(module, locations))
            .collect::<Vec<_>>()
    });
    Ok(objects.into_iter().flatten().collect())
}

/// The functions reached from `roots` through the calls, function values
/// and closures a body writes. It is a guess at what the code generator
/// will refer to, made to start every module early: the drops of types,
/// the tables of methods and what the compiler writes itself are found
/// only as they are compiled, and what is here and never referred to is
/// compiled for nothing, which costs a little and breaks nothing.
fn written_calls(program: &Program, asked: &[Vec<Found>], started: &[Found]) -> Vec<Found> {
    let mut seen = rustc_hash::FxHashSet::default();
    let mut reached: Vec<FnId> = asked
        .iter()
        .flatten()
        .chain(started)
        .filter_map(|found| match found {
            Found::Function(id) => Some(*id),
            Found::Table(_) => None,
        })
        .filter(|&id| seen.insert(id))
        .collect();
    let roots = reached.len();
    let mut next = 0;
    while next < reached.len() {
        let id = reached[next];
        next += 1;
        let Some(body) = &program.fns[id].body else {
            continue;
        };
        for (_, expr) in body.exprs.iter() {
            match expr.kind {
                wip_hir::ExprKind::Call { callee: id, .. }
                | wip_hir::ExprKind::FnRef { id, .. }
                | wip_hir::ExprKind::Closure { id, .. }
                    if program.is_compiled(id) && seen.insert(id) =>
                {
                    reached.push(id);
                }
                _ => {}
            }
        }
    }
    reached.drain(..roots);
    // An `@inline` function is spliced where it is called:
    // what it calls is reached through it, and it is not referred to.
    reached
        .into_iter()
        .filter(|&id| !program.fns[id].is_inline)
        .map(Found::Function)
        .collect()
}

/// What the program calls rather than a module: the drop of a type,
/// wherever a value of it ends, and the methods in an interface's table.
/// They are exported whether or not they are `pub`, since the module that
/// ends the value is rarely the one that wrote the drop.
fn program_wide(program: &Program) -> rustc_hash::FxHashSet<FnId> {
    let mut program_wide: rustc_hash::FxHashSet<FnId> = program
        .drop_fns
        .values()
        .copied()
        .chain(
            program
                .vtables
                .iter()
                .flat_map(|table| table.methods.iter().copied()),
        )
        .collect();
    // What the prelude declares for the compiler to call is called from
    // whatever module the compiler meets the need in: `String::over`,
    // where text is lent as a `String`, is the prelude's own and not `pub`.
    program_wide.extend(
        wip_hir::KnownFn::ALL
            .iter()
            .filter_map(|&known| program.prelude_items.function(known)),
    );
    // A test is called by the runner `wip test` writes, which belongs
    // to no module: it is the program's, wherever it was declared.
    program_wide.extend(
        program
            .fns
            .iter()
            .filter(|(_, def)| def.is_test)
            .map(|(id, _)| id),
    );
    // An instance is compiled in the module that made it, and its body
    // is its generic function's, which may call what that function's own
    // module keeps to itself: those are the program's too.
    // So is what an `@inline` function calls, since its body is spliced into
    // its callers, in whatever module they are.
    for (_, def) in program.fns.iter() {
        let Some(body) = &def.body else {
            continue;
        };
        if def.instance_of.is_none() && !def.is_inline {
            continue;
        }
        for (_, expr) in body.exprs.iter() {
            match expr.kind {
                wip_hir::ExprKind::Call { callee: id, .. }
                | wip_hir::ExprKind::FnRef { id, .. }
                | wip_hir::ExprKind::Closure { id, .. } => {
                    program_wide.insert(id);
                }
                _ => {}
            }
        }
    }
    program_wide
}

/// One module's object, with nothing in it yet. It comes to define the
/// module's own functions that the program reaches, and to declare what
/// those call in the others.
fn open_module<'p>(
    program: &'p Program,
    interner: &'p Interner,
    this_module: u32,
    locations: Locations<'p>,
    c: CallsToC<'p>,
    program_wide: &'p rustc_hash::FxHashSet<FnId>,
    settings: Settings,
) -> Codegen<'p> {
    let isa = host_isa(settings);
    let call_conv = call_conv(&isa);
    let builder = ObjectBuilder::new(isa, "wip", cranelift_module::default_libcall_names())
        .expect("the host has a supported object format");
    let module = ObjectModule::new(builder);
    let ptr = module.target_config().pointer_type();
    assert_eq!(
        ptr,
        types::I64,
        "the prototype targets 64-bit platforms only"
    );

    let mut codegen = Codegen {
        program,
        interner,
        layouts: Layouts::new(),
        module,
        call_conv,
        fn_ids: ArenaMap::default(),
        alloc_fn: FuncId::from_u32(0),
        free_fn: FuncId::from_u32(0),
        int128_div_fn: FuncId::from_u32(0),
        int128_to_float_fn: FuncId::from_u32(0),
        float_to_int128_fn: FuncId::from_u32(0),
        strings: FxHashMap::default(),
        vtables: FxHashMap::default(),
        tables: FxHashMap::default(),
        drop_fns: FxHashMap::default(),
        pending_drops: Vec::new(),
        drop_in_place_fns: FxHashMap::default(),
        pending_drops_in_place: Vec::new(),
        locations,
        panic_fns: FxHashMap::default(),
        texts: FxHashMap::default(),
        fn_ctx: FunctionBuilderContext::new(),
        ctx: cranelift_codegen::Context::new(),
        this_module,
        arch: c.arch,
        shimmed: c.shimmed,
        c_calls: FxHashMap::default(),
        c_globals: FxHashMap::default(),
        runtime_words: None,
        described: Vec::new(),
        frame_tables: None,
        blobs: FxHashMap::default(),
        homes_locals: !settings.optimize,
        optimizes: settings.optimize,
        program_wide,
        found: Vec::new(),
        compiled: rustc_hash::FxHashSet::default(),
    };
    codegen.declare_functions();
    codegen
}

/// The object file of a module whose functions are all compiled.
fn finish_module(mut codegen: Codegen<'_>, locations: Locations<'_>) -> Vec<u8> {
    codegen.define_frame_tables();
    let Codegen {
        program,
        interner,
        this_module,
        module,
        described,
        mut layouts,
        ..
    } = codegen;
    let cie = module.isa().create_systemv_cie();
    let mut product = module.finish();
    unwind::write(&mut product, &described, cie);
    // Lines and functions as DWARF, in every build: a release build's go
    // beside the program when it is linked.
    let unit = match program.modules[this_module as usize].as_str() {
        "" => "main",
        path => path,
    };
    let mut types = debugtypes::DebugTypes::new(program, interner, &mut layouts);
    debuginfo::write(&mut product, &described, locations, unit, &mut types);
    let macho = product.object.format() == object::BinaryFormat::MachO;
    let mut bytes = product
        .emit()
        .expect("writing an object file to memory cannot fail");
    if macho {
        debuginfo::place_macho_addresses(&mut bytes);
    }
    bytes
}

fn host_isa(chosen: Settings) -> OwnedTargetIsa {
    let mut flags = settings::builder();
    let level = if chosen.optimize { "speed" } else { "none" };
    flags.set("opt_level", level).expect("known setting");
    // Every function keeps its frame record, which a sampling profiler
    // walks, and a debugger where there is no unwind information.
    flags
        .set("preserve_frame_pointers", "true")
        .expect("known setting");
    flags.set("is_pic", "true").expect("known setting");
    // The verifier catches malformed IR, which would be a bug in this crate.
    // It runs in debug builds of the compiler, and so in the tests. In a
    // release build it took about 40% of the time.
    let verify = if cfg!(debug_assertions) {
        "true"
    } else {
        "false"
    };
    flags.set("enable_verifier", verify).expect("known setting");
    // `i128` and `u128` are types a program may write, so
    // they stand in signatures: `extend i128: Eq` takes one by reference
    // and `Integer` answers one. Cranelift's x86-64 backend passes them
    // only with this on, and what it then does is what LLVM does, which
    // is what the C beside it does. On arm64 it is the default, which is
    // why only an x86-64 run found this.
    flags
        .set("enable_llvm_abi_extensions", "true")
        .expect("known setting");
    // For measuring: `WIP_CRANELIFT_FLAGS="name=value,..."` overrides any
    // Cranelift flag.
    if let Ok(overrides) = std::env::var("WIP_CRANELIFT_FLAGS") {
        for pair in overrides.split(',').filter(|pair| !pair.trim().is_empty()) {
            let (name, value) = pair.split_once('=').unwrap_or((pair, "true"));
            flags
                .set(name.trim(), value.trim())
                .unwrap_or_else(|err| panic!("WIP_CRANELIFT_FLAGS: `{pair}`: {err}"));
        }
    }
    // The processor's features: those of the level built for, not the
    // compiling machine's, unless it is the one built for.
    let mut isa = chosen.cpu.isa_builder();
    // Code for a Mac's `arm64` does not sign its return addresses: clang
    // signs them only for `arm64e`. Cranelift's detection of the host signs
    // them on every Mac, as a JIT may; the unwinder then read each frame's
    // return address as signed with a key it was not, and a panic's walk
    // ended at the first frame of Wip's.
    if cfg!(all(target_vendor = "apple", target_arch = "aarch64")) {
        isa.set("sign_return_address", "false")
            .expect("an arm64 setting");
        isa.set("sign_return_address_with_bkey", "false")
            .expect("an arm64 setting");
    }
    isa.finish(settings::Flags::new(flags))
        .expect("valid ISA flags")
}

/// Apple's arm64 ABI differs from AAPCS64 even for non-variadic calls — it
/// packs stack arguments differently and makes callers extend narrow
/// integers — so it is selected explicitly rather than left to a default.
fn call_conv(isa: &OwnedTargetIsa) -> CallConv {
    let triple = isa.triple();
    if triple.to_string().starts_with("aarch64-apple") {
        CallConv::AppleAarch64
    } else {
        CallConv::triple_default(triple)
    }
}

struct Codegen<'p> {
    program: &'p Program,
    interner: &'p Interner,
    layouts: Layouts,
    module: ObjectModule,
    call_conv: CallConv,
    fn_ids: ArenaMap<FnId, FuncId>,
    alloc_fn: FuncId,
    free_fn: FuncId,
    /// Runtime helpers for the 128-bit operations Cranelift does not lower on
    /// every target: division, and conversion to and from floats.
    int128_div_fn: FuncId,
    int128_to_float_fn: FuncId,
    float_to_int128_fn: FuncId,
    strings: FxHashMap<Symbol, DataId>,
    /// Where a span was written, for a panic's message.
    locations: Locations<'p>,
    /// The runtime's panic functions, by name.
    panic_fns: FxHashMap<&'static str, FuncId>,
    /// The bytes of the messages and file names panics pass.
    texts: FxHashMap<String, DataId>,
    /// A type's table of methods for an interface.
    vtables: FxHashMap<(wip_hir::InterfaceId, Ty), DataId>,
    /// The constant tables this object defines or reads.
    tables: FxHashMap<wip_hir::ConstId, DataId>,
    /// The drop functions of `own<T>` for each pointee `T`, declared on first
    /// use.
    drop_fns: FxHashMap<Ty, FuncId>,
    /// Drop functions declared but not defined yet.
    pending_drops: Vec<Ty>,
    /// The functions that drop a value of each type where it lies, for a
    /// buffer's elements, declared on first use.
    drop_in_place_fns: FxHashMap<Ty, FuncId>,
    pending_drops_in_place: Vec<Ty>,
    /// Reused across functions to avoid reallocating.
    fn_ctx: FunctionBuilderContext,
    /// Reused across functions, so that its buffers keep their capacity.
    ctx: cranelift_codegen::Context,
    /// The module this object file is for.
    this_module: u32,
    /// The processor whose C ABI calls are made in, where its rules are
    /// written.
    arch: Option<Arch>,
    /// The extern functions called through a wrapper in C.
    shimmed: &'p rustc_hash::FxHashSet<FnId>,
    /// How each function C is on the other side of passes its structs.
    c_calls: FxHashMap<FnId, Option<CCall>>,
    /// The variables C owns that this object reads or writes.
    c_globals: FxHashMap<wip_hir::GlobalId, DataId>,
    /// The runtime's block of words, once this object refers to it.
    runtime_words: Option<DataId>,
    /// What each function of the program defined so far is called, how
    /// long its code is, and where its code was written: for the table a
    /// panic reads, and the debug information.
    described: Vec<debuginfo::DebugFn>,
    /// The list of the modules' tables, once declared.
    frame_tables: Option<DataId>,
    /// The files embedded in this object, by where their bytes are.
    blobs: FxHashMap<usize, DataId>,
    /// Whether each variable and parameter of the source has a stack slot
    /// of its own, where a debugger finds it: a debug build's.
    homes_locals: bool,
    /// Whether this is a release build, whose bodies are optimised in the
    /// MIR before Cranelift.
    optimizes: bool,
    /// The functions that belong to no one module, and so are exported by
    /// the one that compiles them.
    program_wide: &'p rustc_hash::FxHashSet<FnId>,
    /// What this module's code referred to for the first time, since it
    /// was last asked: what must be compiled for it.
    found: Vec<Found>,
    /// The functions of this module compiled so far.
    compiled: rustc_hash::FxHashSet<FnId>,
}

/// What a module's code refers to, which some module must then hold: a
/// function, and a constant table another module declares.
#[derive(Clone, Copy)]
enum Found {
    Function(FnId),
    Table(wip_hir::ConstId),
}

impl Codegen<'_> {
    /// Compiles what this module was asked for, and what that refers to in
    /// this module in turn, and answers what it refers to in the others.
    fn compile_reachable(&mut self, asked: Vec<Found>) -> Vec<Found> {
        let program = self.program;
        let mut queue: std::collections::VecDeque<Found> = asked.into();
        let mut elsewhere = Vec::new();
        loop {
            while let Some(found) = queue.pop_front() {
                match found {
                    Found::Function(id) => {
                        if self.compiled.insert(id) {
                            self.define_function(id, &program.fns[id]);
                        }
                    }
                    // Defined here, by the module that declares it.
                    Found::Table(id) => {
                        self.table_data(id);
                    }
                }
                self.sort_found(&mut queue, &mut elsewhere);
            }
            // The drops of the types those functions end, which call the
            // `destroy` of each.
            self.define_drop_fns();
            self.sort_found(&mut queue, &mut elsewhere);
            if queue.is_empty() {
                return elsewhere;
            }
        }
    }

    /// What was found since the last look: this module's own functions to
    /// compile next, and the rest for the modules they belong to.
    fn sort_found(
        &mut self,
        own: &mut std::collections::VecDeque<Found>,
        elsewhere: &mut Vec<Found>,
    ) {
        for found in self.found.drain(..) {
            match found {
                Found::Function(id) => {
                    if !self.program.is_compiled(id) {
                        continue;
                    }
                    if self.program.fns[id].module == self.this_module {
                        if !self.compiled.contains(&id) {
                            own.push_back(found);
                        }
                    } else {
                        elsewhere.push(found);
                    }
                }
                Found::Table(_) => elsewhere.push(found),
            }
        }
    }
}

/// Where a variable or parameter of the source is kept for the debugger:
/// a stack slot holding it, or holding its address.
#[derive(Clone, Copy)]
pub(crate) struct Home {
    pub local: mir::Local,
    pub slot: StackSlot,
    pub indirect: bool,
}

/// What every module's object is compiled with about calls into C.
#[derive(Clone, Copy)]
struct CallsToC<'a> {
    arch: Option<Arch>,
    shimmed: &'a rustc_hash::FxHashSet<FnId>,
}

/// Where a local of a MIR body lives.
#[derive(Clone, Copy)]
enum Storage {
    /// A scalar whose address is never taken.
    Var(Variable),
    Slot(StackSlot),
    /// An aggregate parameter or result: the memory the caller supplied.
    /// Parameters cannot be assigned, so there is no need to copy them.
    Ptr(Value),
    /// A local of a type with no size.
    Empty,
}

struct FnCodegen<'c, 'p, 'f> {
    cg: &'c mut Codegen<'p>,
    b: FunctionBuilder<'f>,
    body: &'f mir::Body,
    storage: Vec<Storage>,
    /// The slots of the variables and parameters a debugger is told of.
    homes: Vec<Home>,
    blocks: Vec<ir::Block>,
    /// References to runtime functions (`wip_alloc`, `wip_free`, drop
    /// functions) in this function.
    runtime_refs: FxHashMap<FuncId, ir::FuncRef>,
    func_refs: FxHashMap<FnId, ir::FuncRef>,
    data_refs: FxHashMap<DataId, ir::GlobalValue>,
    /// How C passes this function's structs, when C calls it and one
    /// crosses by value.
    c_call: Option<CCall>,
}
/// Which arithmetic a panic is about, as the runtime's message names it.
pub(crate) fn overflow_code(op: wip_hir::BinaryOp) -> i32 {
    match op {
        wip_hir::BinaryOp::Add => 0,
        wip_hir::BinaryOp::Sub => 1,
        wip_hir::BinaryOp::Mul => 2,
        _ => 3,
    }
}
