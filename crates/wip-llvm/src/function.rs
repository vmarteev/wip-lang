//! One MIR body as an LLVM function: every local a stack slot, every
//! statement a few instructions, every block a label.

use std::fmt::Write as _;

use wip_hir::{BinaryOp, FloatTy, Ty, TyKind, Types, UnaryOp};
use wip_mir::{
    self as mir, AtomicOp, Callee, CheckKind, Const, Local, LocalKind, Operand, Place, Projection,
    Rvalue, Statement, Tag, Terminator,
};

use crate::debug::Subprogram;
use crate::{Llvm, Lt, Sig};

/// A value of the function: what LLVM calls it, and its type.
#[derive(Clone, Debug)]
struct Val {
    v: String,
    t: Lt,
}

impl Val {
    fn new(v: impl Into<String>, t: Lt) -> Val {
        Val { v: v.into(), t }
    }

    fn typed(&self) -> String {
        format!("{} {}", self.t.text(), self.v)
    }
}

/// What a move leaves in a build that checks moves.
const POISON: u64 = 0xA5A5_A5A5_A5A5_A5A5;

struct FnGen<'a, 'p> {
    m: &'a mut Llvm<'p>,
    body: &'a mir::Body,
    out: String,
    /// The entry block's stack slots, wherever in the body they are asked
    /// for: one made in a loop's block would grow the stack each pass.
    entry: String,
    next: u32,
    /// The address each local lives at.
    storage: Vec<String>,
    subprogram: Subprogram,
    /// Where the code being written was written: what every instruction
    /// says after it, as `, !dbg !n`.
    place: String,
    /// Each variable's lexical block: where its name is, where its scope
    /// ends, and its node, outer before inner.
    blocks: Vec<(u32, u32, usize)>,
}

/// The text of a function defined from `body`, described by `subprogram`;
/// declared at `at`, or code the compiler writes on its own; `never` where
/// it does not return.
#[allow(clippy::too_many_arguments)]
pub(crate) fn define(
    m: &mut Llvm<'_>,
    body: &mir::Body,
    symbol: &str,
    linkage: &str,
    sig: &Sig,
    c_abi: bool,
    never: bool,
    subprogram: Subprogram,
    at: Option<wip_syntax::Span>,
) -> String {
    // The code before the first statement is at the declaration's line,
    // as Cranelift's is; the compiler's own at none.
    let first = at.and_then(|at| m.debug.location(subprogram, at, None));
    let first = first.unwrap_or_else(|| m.debug.no_line(subprogram));
    let mut g = FnGen {
        m,
        body,
        out: String::new(),
        entry: String::new(),
        next: 0,
        storage: vec![String::new(); body.locals.len()],
        subprogram,
        place: format!(", !dbg !{first}"),
        blocks: Vec::new(),
    };
    let params: Vec<String> = sig
        .params
        .iter()
        .enumerate()
        .map(|(i, slot)| format!("{} %p{i}", slot.text()))
        .collect();
    let mut text = format!(
        "define {linkage}{} {symbol}({}) #{} !dbg !{} {{\nstart:\n",
        sig.ret_text(),
        params.join(", "),
        u8::from(never),
        subprogram.node
    );
    g.prologue(sig, c_abi);
    if at.is_some() {
        g.declare_variables();
    }
    let prologue = std::mem::take(&mut g.out);
    for (i, block) in body.blocks.iter().enumerate() {
        let _ = writeln!(g.out, "bb{i}:");
        for statement in &block.statements {
            g.statement(statement);
        }
        g.terminator(&block.terminator);
    }
    text.push_str(&g.entry);
    text.push_str(&prologue);
    text.push_str("  br label %bb0\n");
    text.push_str(&g.out);
    text.push_str("}\n\n");
    text
}

impl FnGen<'_, '_> {
    /// The function's variables and parameters, each declared where it
    /// lives, as `wip-codegen` describes a debug build's:
    /// a parameter is seen by the whole function; a
    /// variable by the code written between its name and the end of its
    /// scope, as a lexical block of that code, nested in the block of the
    /// variable whose scope holds its own, so that the innermost of two of
    /// one name is the one a debugger shows. A binding that holds the
    /// address of what it matched is shown as what it refers to.
    fn declare_variables(&mut self) {
        let body = self.body;
        let mut variables: Vec<(Local, mir::Source, Option<usize>)> = body
            .locals
            .iter()
            .enumerate()
            .filter_map(|(i, decl)| {
                let source = decl.source?;
                let local = Local(i as u32);
                let param = body.params.iter().position(|&p| p == local);
                Some((local, source, param))
            })
            .collect();
        variables.sort_by_key(|&(_, source, param)| {
            (
                param.is_none(),
                param,
                source.at.lo,
                std::cmp::Reverse(source.seen_until),
            )
        });
        let mut open: Vec<(u32, usize)> = Vec::new();
        for (local, source, param) in variables {
            let storage = self.storage[local.0 as usize].clone();
            if storage.is_empty() {
                continue;
            }
            let decl = body.local(local);
            let ty = match (source.by_address, self.kind(decl.ty)) {
                (true, TyKind::Ref(inner, _)) => inner,
                _ => decl.ty,
            };
            let Some(di_type) = self.m.di_type(ty) else {
                continue;
            };
            let (file, line, column) = self.m.debug.at(source.at);
            let scope = match param {
                Some(_) => self.subprogram.node,
                None => {
                    while open.last().is_some_and(|&(end, _)| end < source.seen_until) {
                        open.pop();
                    }
                    let outer = open
                        .last()
                        .map_or(self.subprogram.node, |&(_, block)| block);
                    let block = self.m.debug.add(format!(
                        "distinct !DILexicalBlock(scope: !{outer}, file: !{file}, \
                         line: {line}, column: {column})"
                    ));
                    open.push((source.seen_until, block));
                    self.blocks.push((source.at.lo, source.seen_until, block));
                    block
                }
            };
            let name = crate::escape(self.m.interner.resolve(source.name).as_bytes());
            let arg = match param {
                Some(index) => format!("arg: {}, ", index + 1),
                None => String::new(),
            };
            let variable = self.m.debug.add(format!(
                "!DILocalVariable(name: \"{name}\", {arg}scope: !{scope}, file: !{file}, \
                 line: {line}, type: !{di_type})"
            ));
            let expression = match source.by_address {
                true => "!DIExpression(DW_OP_deref)",
                false => "!DIExpression()",
            };
            let place = self.m.debug.constant(&format!(
                "!DILocation(line: {line}, column: {column}, scope: !{scope})"
            ));
            self.m
                .intrinsic("declare void @llvm.dbg.declare(metadata, metadata, metadata)");
            let _ = writeln!(
                self.entry,
                "  call void @llvm.dbg.declare(metadata ptr {storage}, metadata !{variable}, \
                 metadata {expression}), !dbg !{place}"
            );
        }
    }

    /// The innermost variable's block whose scope holds code written at
    /// `lo`, if any.
    fn block_at(&self, lo: u32) -> Option<usize> {
        self.blocks
            .iter()
            .rev()
            .find(|&&(start, end, _)| start <= lo && lo < end)
            .map(|&(_, _, block)| block)
    }

    fn temp(&mut self) -> String {
        self.next += 1;
        format!("%t{}", self.next)
    }

    fn label(&mut self, what: &str) -> String {
        self.next += 1;
        format!("{what}{}", self.next)
    }

    fn emit(&mut self, line: impl AsRef<str>) {
        self.out.push_str("  ");
        self.out.push_str(line.as_ref());
        self.out.push_str(&self.place);
        self.out.push('\n');
    }

    /// An instruction whose answer is a new temporary.
    fn value(&mut self, instruction: impl AsRef<str>, t: Lt) -> Val {
        let name = self.temp();
        let _ = writeln!(
            self.out,
            "  {name} = {}{}",
            instruction.as_ref(),
            self.place
        );
        Val::new(name, t)
    }

    fn kind(&self, ty: Ty) -> TyKind {
        self.m.kind(ty)
    }

    /// A stack slot in the entry block, of `size` bytes at `align`.
    fn alloca(&mut self, size: u32, align: u32) -> String {
        let name = self.temp();
        let _ = writeln!(
            self.entry,
            "  {name} = alloca [{} x i8], align {}",
            size.max(1),
            align.max(1)
        );
        name
    }

    fn place_ty(&self, place: &Place) -> Ty {
        mir::place_ty(self.m.program, self.body, place)
    }

    fn operand_ty(&self, operand: &Operand) -> Ty {
        mir::operand_ty(self.m.program, self.body, operand)
    }

    fn align(&mut self, ty: Ty) -> u32 {
        self.m.layout(ty).align.max(1)
    }

    /// Every local's slot, and the parameters put in theirs.
    fn prologue(&mut self, sig: &Sig, c_abi: bool) {
        let body = self.body;
        let mut incoming = 0usize;
        let mut sret = None;
        if sig.sret {
            sret = Some("%p0".to_string());
            incoming = 1;
        }
        let mut param_values: Vec<(Local, Vec<String>)> = Vec::new();
        for &param in &body.params {
            let ty = body.local(param).ty;
            let count = if c_abi && self.m.is_c_pair(ty) { 2 } else { 1 };
            let values = (incoming..incoming + count)
                .map(|i| format!("%p{i}"))
                .collect();
            incoming += count;
            param_values.push((param, values));
        }
        for (i, decl) in body.locals.iter().enumerate() {
            let local = Local(i as u32);
            let aggregate = self.m.is_aggregate(decl.ty);
            let storage = match decl.kind {
                LocalKind::Return if aggregate => {
                    sret.clone().expect("an aggregate result's address")
                }
                LocalKind::Param if aggregate => {
                    let values = &param_values
                        .iter()
                        .find(|(p, _)| *p == local)
                        .expect("a parameter's value")
                        .1;
                    if values.len() == 2 {
                        // A pointer and a length from C, put back together.
                        let slot = format!("%l{i}");
                        let _ = writeln!(self.entry, "  {slot} = alloca [16 x i8], align 8");
                        self.emit(format!("store ptr {}, ptr {slot}, align 8", values[0]));
                        let length = self.temp();
                        self.emit(format!(
                            "{length} = getelementptr inbounds i8, ptr {slot}, i64 8"
                        ));
                        self.emit(format!("store i64 {}, ptr {length}, align 8", values[1]));
                        slot
                    } else {
                        values[0].clone()
                    }
                }
                _ => {
                    let layout = self.m.layout(decl.ty);
                    let slot = format!("%l{i}");
                    let _ = writeln!(
                        self.entry,
                        "  {slot} = alloca [{} x i8], align {}",
                        layout.size.max(1),
                        layout.align.max(1)
                    );
                    slot
                }
            };
            self.storage[i] = storage;
        }
        for (local, values) in param_values {
            let ty = body.local(local).ty;
            if self.m.is_aggregate(ty) {
                continue;
            }
            if self.m.lt(ty).is_some() {
                // As the signature has it, which a `usize` is a pointer in.
                let index: usize = values[0][2..].parse().expect("a parameter's number");
                let incoming = Val::new(values[0].clone(), sig.params[index].lt);
                let slot = self.storage[local.0 as usize].clone();
                self.write_at(&incoming, ty, &slot);
            }
        }
    }

    // Places and values.

    fn gep(&mut self, base: &str, offset: i64) -> String {
        if offset == 0 {
            return base.to_string();
        }
        self.value(
            format!("getelementptr inbounds i8, ptr {base}, i64 {offset}"),
            Lt::Ptr,
        )
        .v
    }

    fn load(&mut self, addr: &str, t: Lt, align: u32) -> Val {
        self.value(format!("load {}, ptr {addr}, align {align}", t.text()), t)
    }

    /// A reference read from memory, with what LLVM may assume of where it
    /// points.
    fn load_reference(&mut self, addr: &str, layout: wip_mir::Layout) -> Val {
        let nonnull = self.m.debug.constant("!{}");
        let size = self.m.debug.constant(&format!("!{{i64 {}}}", layout.size));
        let align = self
            .m
            .debug
            .constant(&format!("!{{i64 {}}}", layout.align.max(1)));
        self.value(
            format!(
                "load ptr, ptr {addr}, align 8, !nonnull !{nonnull}, \
                 !dereferenceable !{size}, !align !{align}"
            ),
            Lt::Ptr,
        )
    }

    fn store(&mut self, value: &Val, addr: &str, align: u32) {
        self.emit(format!(
            "store {}, ptr {addr}, align {align}",
            value.typed()
        ));
    }

    /// The address of a place.
    fn place_addr(&mut self, place: &Place) -> String {
        let mut addr = self.storage[place.local.0 as usize].clone();
        let mut ty = self.body.local(place.local).ty;
        for &projection in &place.projections {
            addr = self.project(&addr, ty, projection);
            ty = mir::project_ty(self.m.program, ty, projection);
        }
        addr
    }

    fn project(&mut self, addr: &str, ty: Ty, projection: Projection) -> String {
        match projection {
            Projection::Deref => match self.m.referent(ty) {
                Some(layout) => self.load_reference(addr, layout).v,
                None => self.load(addr, Lt::Ptr, 8).v,
            },
            Projection::Field(index) => {
                let offset = match self.kind(ty) {
                    TyKind::Struct(..) => self.m.layouts.field_offset(self.m.program, ty, index),
                    _ => index * 8,
                };
                self.gep(addr, i64::from(offset))
            }
            Projection::VariantField { variant, field } => {
                let offset = self
                    .m
                    .layouts
                    .variant_offset(self.m.program, ty, variant, field);
                self.gep(addr, i64::from(offset))
            }
            Projection::Index(_) | Projection::ConstIndex(_) => {
                let elem = mir::element_ty(self.m.program, ty).expect("elements");
                let first = match self.kind(ty) {
                    TyKind::Array(..) => addr.to_string(),
                    _ => self.load(addr, Lt::Ptr, 8).v,
                };
                let stride = i64::from(self.m.layout(elem).stride());
                match projection {
                    Projection::Index(index) => {
                        let index = self.operand(&Operand::Copy(Place::local(index)));
                        let index = self.coerce(&index, Lt::I(64));
                        let offset = self.value(format!("mul i64 {index}, {stride}"), Lt::I(64));
                        self.value(
                            format!("getelementptr inbounds i8, ptr {first}, i64 {}", offset.v),
                            Lt::Ptr,
                        )
                        .v
                    }
                    Projection::ConstIndex(index) => {
                        self.gep(&first, stride.wrapping_mul(index as i64))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    /// A value as another width of integer, where MIR's type and the
    /// LLVM value differ only in that — a `bool` compared, a shift's
    /// amount. An address and a number are never taken for each other
    /// here: MIR types an address as one, and only a cast
    /// converts.
    fn coerce(&mut self, value: &Val, to: Lt) -> String {
        match (value.t, to) {
            (from, to) if from == to => value.v.clone(),
            (Lt::I(_), Lt::I(bits)) => self.int_resize(value, bits),
            (from, to) => panic!("no conversion of {from:?} to {to:?} but a cast's"),
        }
    }

    /// A pointer as the number it is, and back: a cast's conversion.
    fn convert(&mut self, value: &Val, to: Lt) -> String {
        match (value.t, to) {
            (Lt::Ptr, Lt::I(64)) => self.value(format!("ptrtoint ptr {} to i64", value.v), to).v,
            (Lt::Ptr, Lt::I(bits)) => {
                let word = self.value(format!("ptrtoint ptr {} to i64", value.v), Lt::I(64));
                self.int_resize(&word, bits)
            }
            (Lt::I(64), Lt::Ptr) => self.value(format!("inttoptr i64 {} to ptr", value.v), to).v,
            (Lt::I(_), Lt::Ptr) => {
                let word = self.int_resize(value, 64);
                self.value(format!("inttoptr i64 {word} to ptr"), to).v
            }
            _ => self.coerce(value, to),
        }
    }

    /// An integer to another width: its low bits, or extended with zeros.
    fn int_resize(&mut self, value: &Val, bits: u32) -> String {
        let Lt::I(from) = value.t else {
            panic!("{:?} is not an integer", value.t)
        };
        match from.cmp(&bits) {
            std::cmp::Ordering::Equal => value.v.clone(),
            std::cmp::Ordering::Less => {
                self.value(format!("zext {} to i{bits}", value.typed()), Lt::I(bits))
                    .v
            }
            std::cmp::Ordering::Greater => {
                self.value(format!("trunc {} to i{bits}", value.typed()), Lt::I(bits))
                    .v
            }
        }
    }

    /// The value of a scalar operand.
    fn operand(&mut self, operand: &Operand) -> Val {
        match operand {
            Operand::Const(constant) => self.constant(constant),
            Operand::Copy(place) => {
                let ty = self.place_ty(place);
                let lt = self
                    .m
                    .lt(ty)
                    .expect("an aggregate operand is passed by address");
                let align = self.align(ty);
                let addr = self.place_addr(place);
                match self.m.referent(ty) {
                    Some(layout) => self.load_reference(&addr, layout),
                    None => self.load(&addr, lt, align),
                }
            }
        }
    }

    fn int_text(bits: u128, width: u32) -> String {
        // Written as the signed number the bits are, which LLVM takes for
        // every width.
        if width >= 128 {
            return (bits as i128).to_string();
        }
        let shift = 128 - width;
        (((bits << shift) as i128) >> shift).to_string()
    }

    fn constant(&mut self, constant: &Const) -> Val {
        match *constant {
            Const::Int { bits, ty } => match self.m.lt(ty).expect("an integer's type") {
                Lt::Ptr if bits == 0 => Val::new("null", Lt::Ptr),
                Lt::Ptr => Val::new(
                    format!("inttoptr (i64 {} to ptr)", bits as u64 as i64),
                    Lt::Ptr,
                ),
                Lt::I(width) => Val::new(Self::int_text(bits, width), Lt::I(width)),
                lt => panic!("an integer constant of {lt:?}"),
            },
            Const::Float { value, ty } => match self.kind(ty) {
                TyKind::Float(FloatTy::F32) => Val::new(
                    format!("0x{:016X}", f64::from(value as f32).to_bits()),
                    Lt::F32,
                ),
                _ => Val::new(format!("0x{:016X}", value.to_bits()), Lt::F64),
            },
            Const::Bool(value) => Val::new(u8::from(value).to_string(), Lt::I(8)),
            Const::CStr(sym) => Val::new(self.m.string(sym), Lt::Ptr),
            Const::Fn { id, .. } => Val::new(self.m.symbol(id), Lt::Ptr),
            Const::VTable { interface, ty } => Val::new(self.m.vtable(interface, ty), Lt::Ptr),
            Const::Table { id, .. } => Val::new(self.m.table(id), Lt::Ptr),
            Const::SizeOf { of, ty } | Const::AlignOf { of, ty } => {
                let layout = self.m.layout(of);
                let bytes = match *constant {
                    Const::SizeOf { .. } => layout.stride(),
                    _ => layout.align,
                };
                let Some(Lt::I(width)) = self.m.lt(ty) else {
                    panic!("a size is an integer")
                };
                Val::new(bytes.to_string(), Lt::I(width))
            }
            Const::FrameTables => Val::new("@\"wip.frame_tables\"", Lt::Ptr),
            Const::RuntimeWords => Val::new("@wip_runtime_words", Lt::Ptr),
            Const::DropFn(ty) => Val::new(self.m.drop_fn(ty, false), Lt::Ptr),
        }
    }

    /// Stores a scalar in a local's own slot, as its type.
    fn write_at(&mut self, value: &Val, ty: Ty, slot: &str) {
        let Some(lt) = self.m.lt(ty) else {
            return;
        };
        let v = self.coerce(value, lt);
        let align = self.align(ty);
        self.store(&Val::new(v, lt), slot, align);
    }

    /// Stores a scalar in a place, as the place's type.
    fn write(&mut self, place: &Place, value: &Val) {
        let ty = self.place_ty(place);
        let Some(lt) = self.m.lt(ty) else {
            return;
        };
        let v = self.coerce(value, lt);
        let align = self.align(ty);
        let addr = self.place_addr(place);
        self.store(&Val::new(v, lt), &addr, align);
    }

    fn memset(&mut self, addr: &str, byte: u8, size: u32) {
        if size == 0 {
            return;
        }
        self.m
            .intrinsic("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)");
        self.emit(format!(
            "call void @llvm.memset.p0.i64(ptr {addr}, i8 {}, i64 {size}, i1 false)",
            byte as i8
        ));
    }

    fn memmove(&mut self, dest: &str, src: &str, size: u32) {
        if size == 0 {
            return;
        }
        self.m
            .intrinsic("declare void @llvm.memmove.p0.p0.i64(ptr, ptr, i64, i1)");
        self.emit(format!(
            "call void @llvm.memmove.p0.p0.i64(ptr {dest}, ptr {src}, i64 {size}, i1 false)"
        ));
    }

    // Statements.

    fn statement(&mut self, statement: &Statement) {
        match statement {
            // What follows was written here, until the next says otherwise;
            // a span with no line leaves the place as it was, as
            // Cranelift's table passes over it.
            Statement::At(span) => {
                let block = self.block_at(span.lo);
                if let Some(place) = self.m.debug.location(self.subprogram, *span, block) {
                    self.place = format!(", !dbg !{place}");
                }
            }
            Statement::Assign(place, rvalue) => self.assign(place, rvalue),
            Statement::SetVariant(place, variant) => self.set_variant(place, *variant),
            Statement::Zero(place) => {
                let ty = self.place_ty(place);
                let size = self.m.layout(ty).size;
                let addr = self.place_addr(place);
                self.memset(&addr, 0, size);
            }
            Statement::Poison(place) => {
                let ty = self.place_ty(place);
                let size = self.m.layout(ty).size;
                let addr = self.place_addr(place);
                self.memset(&addr, POISON as u8, size);
            }
            Statement::CheckMoved { place, at } => {
                let moved = self.poisoned(place);
                self.check(&moved, |g| g.panic_call("wip_panic_moved", Vec::new(), *at));
            }
            Statement::Call { callee, args, dest } => self.call(callee, args, dest.as_ref()),
            Statement::Alloc { dest, ty } => {
                let size = self.m.layout(*ty).size.max(1);
                let alloc = self.m.runtime("wip_alloc");
                let ptr = self
                    .call_values(alloc, vec![Val::new(size.to_string(), Lt::I(64))])
                    .expect("an allocation's address");
                self.write(dest, &ptr);
            }
            Statement::AllocBuffer { dest, elem, count } => {
                let count = self.operand(count);
                let count = Val::new(self.coerce(&count, Lt::I(64)), Lt::I(64));
                let stride = self.m.layout(*elem).stride();
                // More bytes than an `i64` counts is more than memory holds.
                self.m
                    .intrinsic("declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)");
                let both = self.value(
                    format!(
                        "call {{ i64, i1 }} @llvm.smul.with.overflow.i64(i64 {}, i64 {stride})",
                        count.v
                    ),
                    Lt::I(64),
                );
                let size = self.value(
                    format!("extractvalue {{ i64, i1 }} {}, 0", both.v),
                    Lt::I(64),
                );
                let over = self.value(
                    format!("extractvalue {{ i64, i1 }} {}, 1", both.v),
                    Lt::I(1),
                );
                self.trap_if(&over.v);
                let alloc = self.m.runtime("wip_alloc");
                let ptr = self
                    .call_values(alloc, vec![size])
                    .expect("an allocation's address");
                let addr = self.place_addr(dest);
                self.store(&ptr, &addr, 8);
                let length = self.gep(&addr, 8);
                self.store(&count, &length, 8);
            }
            Statement::Free(ptr) => {
                let ptr = self.operand(ptr);
                let free = self.m.runtime("wip_free");
                self.call_values(free, vec![ptr]);
            }
            Statement::DropFn { ty, ptr } => {
                let ptr = self.operand(ptr);
                let ptr = self.coerce(&ptr, Lt::Ptr);
                let drop = self.m.drop_fn(*ty, false);
                self.emit(format!("call void {drop}(ptr {ptr})"));
            }
            Statement::DropInPlace { ty, ptr } => {
                let ptr = self.operand(ptr);
                let ptr = self.coerce(&ptr, Lt::Ptr);
                let drop = self.m.drop_fn(*ty, true);
                self.emit(format!("call void {drop}(ptr {ptr})"));
            }
            Statement::Arith {
                dest,
                op,
                lhs,
                rhs,
                at,
            } => {
                let ty = self.place_ty(dest);
                let signed = matches!(self.kind(ty), TyKind::Int(int) if int.signed());
                let lhs = self.operand(lhs);
                let rhs = self.operand(rhs);
                let lt = lhs.t;
                let rhs = Val::new(self.coerce(&rhs, lt), lt);
                let name = match (op, signed) {
                    (BinaryOp::Add, true) => "sadd",
                    (BinaryOp::Add, false) => "uadd",
                    (BinaryOp::Sub, true) => "ssub",
                    (BinaryOp::Sub, false) => "usub",
                    (BinaryOp::Mul, true) => "smul",
                    (BinaryOp::Mul, false) => "umul",
                    (op, _) => unreachable!("{op:?} does not overflow"),
                };
                let t = lt.text();
                self.m.intrinsic(&format!(
                    "declare {{ {t}, i1 }} @llvm.{name}.with.overflow.{t}({t}, {t})"
                ));
                let both = self.value(
                    format!(
                        "call {{ {t}, i1 }} @llvm.{name}.with.overflow.{t}({}, {})",
                        lhs.typed(),
                        rhs.typed()
                    ),
                    lt,
                );
                let over = self.value(
                    format!("extractvalue {{ {t}, i1 }} {}, 1", both.v),
                    Lt::I(1),
                );
                let which = i64::from(overflow_code(*op));
                self.check_i1(&over.v, |g| {
                    g.panic_call(
                        "wip_panic_overflow",
                        vec![Val::new(which.to_string(), Lt::I(64))],
                        *at,
                    )
                });
                // Past the check the answer fits, which LLVM's flags say of
                // the operation computed again here, where it is used: a
                // loop's counter that cannot wrap is one LLVM can reason
                // about. LLVM finds the two the same.
                let instruction = match op {
                    BinaryOp::Add => "add",
                    BinaryOp::Sub => "sub",
                    _ => "mul",
                };
                let flag = if signed { "nsw" } else { "nuw" };
                let answer = self.value(
                    format!("{instruction} {flag} {}, {}", lhs.typed(), rhs.v),
                    lt,
                );
                self.write(dest, &answer);
            }
            Statement::Atomic {
                op,
                ty,
                address,
                value,
                expected,
                dest,
            } => {
                let address = self.operand(address);
                let address = self.coerce(&address, Lt::Ptr);
                let lt = self.m.lt(*ty).expect("an atomic's value is a scalar");
                let align = self.align(*ty);
                let t = lt.text();
                let answer = match op {
                    AtomicOp::Add | AtomicOp::Subtract | AtomicOp::Swap => {
                        let operand = self.operand(value.as_ref().expect("an atomic's value"));
                        let operand = self.coerce(&operand, lt);
                        let operation = match op {
                            AtomicOp::Add => "add",
                            AtomicOp::Subtract => "sub",
                            _ => "xchg",
                        };
                        Some(self.value(
                            format!("atomicrmw {operation} ptr {address}, {t} {operand} seq_cst, align {align}"),
                            lt,
                        ))
                    }
                    AtomicOp::CompareSwap => {
                        let wanted = self.operand(expected.as_ref().expect("an expected value"));
                        let wanted = self.coerce(&wanted, lt);
                        let stored = self.operand(value.as_ref().expect("an atomic's value"));
                        let stored = self.coerce(&stored, lt);
                        let both = self.value(
                            format!("cmpxchg ptr {address}, {t} {wanted}, {t} {stored} seq_cst seq_cst, align {align}"),
                            lt,
                        );
                        Some(self.value(format!("extractvalue {{ {t}, i1 }} {}, 0", both.v), lt))
                    }
                    AtomicOp::Load => Some(self.value(
                        format!("load atomic {t}, ptr {address} seq_cst, align {align}"),
                        lt,
                    )),
                    AtomicOp::Store => {
                        let stored = self.operand(value.as_ref().expect("a store has a value"));
                        let stored = self.coerce(&stored, lt);
                        self.emit(format!(
                            "store atomic {t} {stored}, ptr {address} seq_cst, align {align}"
                        ));
                        None
                    }
                };
                if let (Some(dest), Some(answer)) = (dest, answer) {
                    self.write(dest, &answer);
                }
            }
            Statement::Check { fails, kind, at } => {
                let fails = self.operand(fails);
                let kind = kind.clone();
                let at = *at;
                self.check(&fails, |g| {
                    let (name, numbers) = match &kind {
                        CheckKind::Bounds { index, length } => {
                            let index = g.operand(index);
                            let length = g.operand(length);
                            ("wip_panic_index", vec![index, length])
                        }
                        CheckKind::Length { length } => {
                            let length = g.operand(length);
                            ("wip_panic_length", vec![length])
                        }
                        CheckKind::Division => ("wip_panic_division", Vec::new()),
                        CheckKind::Overflow(op) => (
                            "wip_panic_overflow",
                            vec![Val::new(overflow_code(*op).to_string(), Lt::I(64))],
                        ),
                    };
                    g.panic_call(name, numbers, at);
                });
            }
        }
    }

    /// Branches to what `fails` says when the `bool` it holds is true, and
    /// carries on otherwise.
    fn check(&mut self, fails: &Val, panics: impl FnOnce(&mut Self)) {
        let flag = self.value(format!("icmp ne {}, 0", fails.typed()), Lt::I(1));
        self.check_i1(&flag.v, panics);
    }

    fn check_i1(&mut self, flag: &str, panics: impl FnOnce(&mut Self)) {
        let failed = self.label("fail");
        let fine = self.label("fine");
        let unlikely = self.m.debug.unlikely();
        self.emit(format!(
            "br i1 {flag}, label %{failed}, label %{fine}, !prof !{unlikely}"
        ));
        let _ = writeln!(self.out, "{failed}:");
        panics(self);
        self.emit("unreachable");
        let _ = writeln!(self.out, "{fine}:");
    }

    fn trap_if(&mut self, flag: &str) {
        self.m.intrinsic("declare void @llvm.trap()");
        self.check_i1(flag, |g| g.emit("call void @llvm.trap()"));
    }

    /// A call of a runtime panic: its own numbers, then the file, the line
    /// and the column.
    fn panic_call(&mut self, name: &str, numbers: Vec<Val>, at: wip_syntax::Span) {
        let (file, line, column) = (self.m.locations)(at);
        let file_data = self.m.text(&file);
        let mut args = numbers;
        args.push(Val::new(file_data, Lt::Ptr));
        args.push(Val::new(file.len().to_string(), Lt::I(64)));
        args.push(Val::new(line.to_string(), Lt::I(64)));
        args.push(Val::new(column.to_string(), Lt::I(64)));
        let id = self.m.runtime(name);
        self.call_values(id, args);
    }

    /// Whether a place begins with what [`Statement::Poison`] leaves.
    fn poisoned(&mut self, place: &Place) -> Val {
        let ty = self.place_ty(place);
        let size = self.m.layout(ty).size;
        let bits = match size {
            0 => return Val::new("0", Lt::I(8)),
            8.. => 64,
            4..=7 => 32,
            2..=3 => 16,
            _ => 8,
        };
        let addr = self.place_addr(place);
        let first = self.load(&addr, Lt::I(bits), 1);
        let pattern = (POISON as u128) & ((1u128 << bits) - 1);
        let equal = self.value(
            format!(
                "icmp eq {}, {}",
                first.typed(),
                Self::int_text(pattern, bits)
            ),
            Lt::I(1),
        );
        self.value(format!("zext i1 {} to i8", equal.v), Lt::I(8))
    }

    fn set_variant(&mut self, place: &Place, variant: u32) {
        let ty = self.place_ty(place);
        match self.m.layouts.enum_tag(self.m.program, ty) {
            Tag::Byte { size } => {
                let lt = if size == 1 { Lt::I(8) } else { Lt::I(32) };
                let addr = self.place_addr(place);
                self.store(&Val::new(variant.to_string(), lt), &addr, size.min(4));
            }
            Tag::Niche { empty, offset, .. } if variant == empty => {
                let addr = self.place_addr(place);
                let at = self.gep(&addr, i64::from(offset));
                self.store(&Val::new("null", Lt::Ptr), &at, 8);
            }
            Tag::Niche { .. } => {}
        }
    }

    fn variant_index(&mut self, addr: &str, ty: Ty) -> Val {
        match self.m.layouts.enum_tag(self.m.program, ty) {
            Tag::Byte { size: 1 } => {
                let tag = self.load(addr, Lt::I(8), 1);
                self.value(format!("zext i8 {} to i32", tag.v), Lt::I(32))
            }
            Tag::Byte { .. } => self.load(addr, Lt::I(32), 4),
            Tag::Niche {
                payload,
                empty,
                offset,
            } => {
                let at = self.gep(addr, i64::from(offset));
                let ptr = self.load(&at, Lt::Ptr, 8);
                let null = self.value(format!("icmp eq ptr {}, null", ptr.v), Lt::I(1));
                self.value(
                    format!("select i1 {}, i32 {empty}, i32 {payload}", null.v),
                    Lt::I(32),
                )
            }
        }
    }

    fn assign(&mut self, place: &Place, rvalue: &Rvalue) {
        let ty = self.place_ty(place);
        if self.m.is_aggregate(ty) {
            let Rvalue::Use(Operand::Copy(src)) = rvalue else {
                unreachable!("an aggregate is assigned by copying a place")
            };
            let size = self.m.layout(ty).size;
            let dest = self.place_addr(place);
            let src = self.place_addr(src);
            self.memmove(&dest, &src, size);
            return;
        }
        if self.m.lt(ty).is_none() {
            return;
        }
        let value = self.rvalue(rvalue, ty);
        self.write(place, &value);
    }

    fn rvalue(&mut self, rvalue: &Rvalue, ty: Ty) -> Val {
        match rvalue {
            Rvalue::Use(operand) => self.operand(operand),
            Rvalue::Unary(op, operand) => {
                let value = self.operand(operand);
                match (op, self.kind(ty)) {
                    (UnaryOp::Neg, TyKind::Float(_)) => {
                        self.value(format!("fneg {}", value.typed()), value.t)
                    }
                    (UnaryOp::Neg, _) => {
                        self.value(format!("sub {} 0, {}", value.t.text(), value.v), value.t)
                    }
                    (UnaryOp::Not, TyKind::Bool) => {
                        self.value(format!("xor {}, 1", value.typed()), value.t)
                    }
                    (UnaryOp::Not, _) => self.value(format!("xor {}, -1", value.typed()), value.t),
                }
            }
            Rvalue::Binary(op, lhs, rhs) => self.binary(*op, lhs, rhs),
            Rvalue::Cast(operand, to) => self.cast(operand, *to),
            Rvalue::AddressOf(place) => Val::new(self.place_addr(place), Lt::Ptr),
            Rvalue::VTableFn {
                table,
                index,
                methods,
            } => {
                let table = self.operand(table);
                let table = self.coerce(&table, Lt::Ptr);
                let at = self.gep(&table, i64::from(*index) * 8);
                match methods {
                    // A table of methods is a constant, never freed: what is
                    // read from it is the same wherever it is read, which
                    // lets LLVM read it once where a loop calls through one
                    // `&dyn`.
                    true => {
                        let invariant = self.m.debug.constant("!{}");
                        self.value(
                            format!("load ptr, ptr {at}, align 8, !invariant.load !{invariant}"),
                            Lt::Ptr,
                        )
                    }
                    false => self.load(&at, Lt::Ptr, 8),
                }
            }
            Rvalue::Float(op, operand) => {
                let value = self.operand(operand);
                let name = match op {
                    mir::FloatOp::Sqrt => "sqrt",
                    mir::FloatOp::Floor => "floor",
                    mir::FloatOp::Ceil => "ceil",
                    mir::FloatOp::Trunc => "trunc",
                };
                let (t, suffix) = float_names(value.t);
                self.m
                    .intrinsic(&format!("declare {t} @llvm.{name}.{suffix}({t})"));
                self.value(
                    format!("call {t} @llvm.{name}.{suffix}({})", value.typed()),
                    value.t,
                )
            }
            Rvalue::MulAdd(first, second, third) => {
                let first = self.operand(first);
                let second = self.operand(second);
                let third = self.operand(third);
                let (t, suffix) = float_names(first.t);
                self.m
                    .intrinsic(&format!("declare {t} @llvm.fma.{suffix}({t}, {t}, {t})"));
                self.value(
                    format!(
                        "call {t} @llvm.fma.{suffix}({}, {}, {})",
                        first.typed(),
                        second.typed(),
                        third.typed()
                    ),
                    first.t,
                )
            }
            Rvalue::Integer(op, operand) => {
                let value = self.operand(operand);
                let t = value.t.text();
                match op {
                    mir::IntegerOp::SwapBytes if value.t == Lt::I(8) => value,
                    mir::IntegerOp::CountOnes
                    | mir::IntegerOp::SwapBytes
                    | mir::IntegerOp::ReverseBits => {
                        let name = match op {
                            mir::IntegerOp::CountOnes => "ctpop",
                            mir::IntegerOp::SwapBytes => "bswap",
                            _ => "bitreverse",
                        };
                        self.m
                            .intrinsic(&format!("declare {t} @llvm.{name}.{t}({t})"));
                        self.value(
                            format!("call {t} @llvm.{name}.{t}({})", value.typed()),
                            value.t,
                        )
                    }
                    mir::IntegerOp::LeadingZeros | mir::IntegerOp::TrailingZeros => {
                        let name = match op {
                            mir::IntegerOp::LeadingZeros => "ctlz",
                            _ => "cttz",
                        };
                        self.m
                            .intrinsic(&format!("declare {t} @llvm.{name}.{t}({t}, i1)"));
                        // The width, for 0, as the instruction answers.
                        self.value(
                            format!("call {t} @llvm.{name}.{t}({}, i1 false)", value.typed()),
                            value.t,
                        )
                    }
                }
            }
            Rvalue::Rotate(turn, value, amount) => {
                let value = self.operand(value);
                let amount = self.operand(amount);
                let amount = self.coerce(&amount, value.t);
                let t = value.t.text();
                let name = match turn {
                    mir::Turn::Left => "fshl",
                    mir::Turn::Right => "fshr",
                };
                self.m
                    .intrinsic(&format!("declare {t} @llvm.{name}.{t}({t}, {t}, {t})"));
                self.value(
                    format!(
                        "call {t} @llvm.{name}.{t}({t} {}, {t} {}, {t} {amount})",
                        value.v, value.v
                    ),
                    value.t,
                )
            }
            Rvalue::Bits(operand) => {
                let value = self.operand(operand);
                let target = self.m.lt(ty).expect("bits are a scalar's");
                self.value(
                    format!("bitcast {} to {}", value.typed(), target.text()),
                    target,
                )
            }
            Rvalue::CstrLen(operand) => {
                let pointer = self.operand(operand);
                let id = self.m.runtime("wip_cstr_len");
                self.call_values(id, vec![pointer]).expect("a length")
            }
            Rvalue::Variant(place) => {
                let ty = self.place_ty(place);
                let addr = self.place_addr(place);
                self.variant_index(&addr, ty)
            }
        }
    }

    fn binary(&mut self, op: BinaryOp, lhs: &Operand, rhs: &Operand) -> Val {
        let operand_ty = self.operand_ty(lhs);
        let operand = self.kind(operand_ty);
        let l = self.operand(lhs);
        let r = self.operand(rhs);
        let t = l.t;
        if matches!(t, Lt::F32 | Lt::F64) {
            let instruction = match op {
                BinaryOp::Add => "fadd",
                BinaryOp::Sub => "fsub",
                BinaryOp::Mul => "fmul",
                BinaryOp::Div => "fdiv",
                BinaryOp::Rem => "frem",
                BinaryOp::Eq => "fcmp oeq",
                BinaryOp::Ne => "fcmp une",
                BinaryOp::Lt => "fcmp olt",
                BinaryOp::Le => "fcmp ole",
                BinaryOp::Gt => "fcmp ogt",
                BinaryOp::Ge => "fcmp oge",
                _ => unreachable!("the type checker rejects `{op:?}` on floats"),
            };
            let comparison = instruction.starts_with("fcmp");
            let answer = self.value(
                format!("{instruction} {}, {}", l.typed(), r.v),
                if comparison { Lt::I(1) } else { t },
            );
            return if comparison {
                self.value(format!("zext i1 {} to i8", answer.v), Lt::I(8))
            } else {
                answer
            };
        }
        let unsigned = !matches!(operand, TyKind::Int(int) if int.signed());
        let r = Val::new(self.coerce(&r, t), t);
        let compare = |cc: &str| Some(cc.to_string());
        let comparison = match op {
            BinaryOp::Eq => compare("eq"),
            BinaryOp::Ne => compare("ne"),
            BinaryOp::Lt => compare(if unsigned { "ult" } else { "slt" }),
            BinaryOp::Le => compare(if unsigned { "ule" } else { "sle" }),
            BinaryOp::Gt => compare(if unsigned { "ugt" } else { "sgt" }),
            BinaryOp::Ge => compare(if unsigned { "uge" } else { "sge" }),
            _ => None,
        };
        if let Some(cc) = comparison {
            let answer = self.value(format!("icmp {cc} {}, {}", l.typed(), r.v), Lt::I(1));
            return self.value(format!("zext i1 {} to i8", answer.v), Lt::I(8));
        }
        let Lt::I(bits) = t else {
            panic!("arithmetic on {t:?}")
        };
        // MIR checks the most negative number divided by -1 only to 64 bits;
        // past them the runtime's division aborts there,
        // as Cranelift's code calls it, where LLVM's `sdiv` and `srem`
        // would be undefined.
        if bits > 64 && !unsigned && matches!(op, BinaryOp::Div | BinaryOp::Rem) {
            let most_negative = Self::int_text(1u128 << (bits - 1), bits);
            let edge = self.value(format!("icmp eq {}, {most_negative}", l.typed()), Lt::I(1));
            let by = self.value(format!("icmp eq {}, -1", r.typed()), Lt::I(1));
            let both = self.value(format!("and i1 {}, {}", edge.v, by.v), Lt::I(1));
            self.check_i1(&both.v, |g| {
                let abort = g.m.abort();
                g.emit(format!("call void {abort}()"));
            });
        }
        let instruction = match op {
            BinaryOp::Add => "add",
            BinaryOp::Sub => "sub",
            BinaryOp::Mul => "mul",
            // A divisor of zero, and the most negative number divided by
            // -1, were checked before, and past
            // 64 bits just above.
            BinaryOp::Div if unsigned => "udiv",
            BinaryOp::Div => "sdiv",
            BinaryOp::Rem if unsigned => "urem",
            BinaryOp::Rem => "srem",
            BinaryOp::BitAnd => "and",
            BinaryOp::BitOr => "or",
            BinaryOp::BitXor => "xor",
            BinaryOp::Shl | BinaryOp::Shr => {
                // The amount is taken modulo the width, which
                // LLVM leaves undefined past it.
                let masked = self.value(format!("and {}, {}", r.typed(), bits - 1), t);
                let instruction = match (op, unsigned) {
                    (BinaryOp::Shl, _) => "shl",
                    (_, true) => "lshr",
                    _ => "ashr",
                };
                return self.value(format!("{instruction} {}, {}", l.typed(), masked.v), t);
            }
            BinaryOp::And | BinaryOp::Or => unreachable!("`&&` and `||` are branches in MIR"),
            _ => unreachable!("compared above"),
        };
        self.value(format!("{instruction} {}, {}", l.typed(), r.v), t)
    }

    /// `operand as to`.
    fn cast(&mut self, operand: &Operand, to: Ty) -> Val {
        let as_number = |kind: TyKind| match kind {
            TyKind::Char => TyKind::Int(wip_hir::IntTy::U32),
            other => other,
        };
        let from = as_number(self.kind(self.operand_ty(operand)));
        let target = self.m.lt(to).expect("casts are between scalar types");
        let value = self.operand(operand);
        let to_kind = as_number(self.kind(to));
        match (from, to_kind) {
            (TyKind::Int(a), TyKind::Int(b)) => match a.bits().cmp(&b.bits()) {
                std::cmp::Ordering::Less if a.signed() => self.value(
                    format!("sext {} to {}", value.typed(), target.text()),
                    target,
                ),
                std::cmp::Ordering::Less => self.value(
                    format!("zext {} to {}", value.typed(), target.text()),
                    target,
                ),
                std::cmp::Ordering::Greater => self.value(
                    format!("trunc {} to {}", value.typed(), target.text()),
                    target,
                ),
                std::cmp::Ordering::Equal => value,
            },
            (TyKind::Int(a), TyKind::Float(_)) => {
                let instruction = if a.signed() { "sitofp" } else { "uitofp" };
                self.value(
                    format!("{instruction} {} to {}", value.typed(), target.text()),
                    target,
                )
            }
            // Toward zero, saturating at the ends of the range, NaN to 0:
            // what LLVM's saturating conversion does.
            (TyKind::Float(_), TyKind::Int(b)) => {
                let name = if b.signed() { "fptosi" } else { "fptoui" };
                let (ft, suffix) = float_names(value.t);
                let t = target.text();
                self.m
                    .intrinsic(&format!("declare {t} @llvm.{name}.sat.{t}.{suffix}({ft})"));
                self.value(
                    format!("call {t} @llvm.{name}.sat.{t}.{suffix}({})", value.typed()),
                    target,
                )
            }
            (TyKind::Float(FloatTy::F32), TyKind::Float(FloatTy::F64)) => {
                self.value(format!("fpext {} to double", value.typed()), target)
            }
            (TyKind::Float(FloatTy::F64), TyKind::Float(FloatTy::F32)) => {
                self.value(format!("fptrunc {} to float", value.typed()), target)
            }
            _ => {
                // A pointer and the number it is, an `own` as a pointer:
                // the same word.
                let v = self.convert(&value, target);
                Val::new(v, target)
            }
        }
    }

    // Calls.

    /// A call: an aggregate argument by the address of its place, an
    /// aggregate result written to the address of `dest`.
    fn call(&mut self, callee: &Callee, args: &[Operand], dest: Option<&Place>) {
        let program = self.m.program;
        if let Callee::Fn(id) = callee {
            let def = &program.fns[*id];
            // A variable C owns: a load, or a store.
            if let Some(wip_hir::Access::Global(global)) = def.accesses
                && !self.m.shimmed.contains(id)
            {
                let symbol = self.m.c_global(global);
                let ty = program.globals[global].ty;
                let lt = self.m.lt(ty).expect("C's variable is a scalar");
                let align = self.align(ty);
                match args {
                    [value] => {
                        let value = self.operand(value);
                        let v = self.coerce(&value, lt);
                        self.store(&Val::new(v, lt), &symbol, align);
                    }
                    _ => {
                        let value = self.load(&symbol, lt, align);
                        self.write(dest.expect("a read has a destination"), &value);
                    }
                }
                return;
            }
            // More arguments than the declaration names, made as LLVM's own
            // variadic call.
            if let Some(original) = def.variadic_of
                && !self.m.shimmed.contains(id)
            {
                return self.variadic_call(*id, original, args, dest);
            }
            if let Some(c_call) = self.m.c_call(*id) {
                return self.c_call(*id, &c_call, args, dest);
            }
        }
        let (params, ret, c_abi) = match callee {
            Callee::StrCmp => (vec![Types::STR, Types::STR], Types::I64, true),
            Callee::Fn(id) => {
                let def = &program.fns[*id];
                (
                    def.params.iter().map(|p| p.ty).collect::<Vec<_>>(),
                    def.ret,
                    crate::uses_c_abi(def),
                )
            }
            Callee::Value(value) => {
                let TyKind::Fn(params, ret) = self.kind(self.operand_ty(value)) else {
                    unreachable!("a value that is called has a function type")
                };
                (program.types.list(params).to_vec(), ret, false)
            }
        };
        // A named function's call is made with its own signature, as its
        // definition has it.
        let sig = match callee {
            Callee::Fn(id) => self.m.signature(*id),
            _ => self.m.indirect_signature(&params, ret, c_abi),
        };
        let mut values = Vec::new();
        if sig.sret {
            let dest = dest.expect("an aggregate result has a destination");
            values.push(Val::new(self.place_addr(dest), Lt::Ptr));
        }
        for arg in args {
            let ty = self.operand_ty(arg);
            if c_abi && self.m.is_c_pair(ty) {
                let Operand::Copy(place) = arg else {
                    unreachable!("a pointer-and-length operand is a place")
                };
                let addr = self.place_addr(place);
                let pointer = self.load(&addr, Lt::Ptr, 8);
                let at = self.gep(&addr, 8);
                let length = self.load(&at, Lt::I(64), 8);
                values.push(pointer);
                values.push(length);
            } else if self.m.is_aggregate(ty) {
                let Operand::Copy(place) = arg else {
                    unreachable!("an aggregate operand is a place")
                };
                values.push(Val::new(self.place_addr(place), Lt::Ptr));
            } else {
                values.push(self.operand(arg));
            }
        }
        let target = match callee {
            Callee::StrCmp => {
                let id = self.m.runtime("wip_str_cmp");
                self.m.symbol(id)
            }
            Callee::Fn(id) => self.m.symbol(*id),
            Callee::Value(value) => {
                let value = self.operand(value);
                self.coerce(&value, Lt::Ptr)
            }
        };
        let answer = self.emit_call(&target, &sig, values);
        if let (Some(answer), Some(dest)) = (answer, dest)
            && !sig.sret
        {
            self.write(dest, &answer);
        }
    }

    /// A call of a function C passes a struct by value to, or answers one
    /// from, made as `c_call` says.
    fn c_call(
        &mut self,
        id: wip_hir::FnId,
        c_call: &mir::c_abi::CCall,
        args: &[Operand],
        dest: Option<&Place>,
    ) {
        use mir::c_abi::{Answer, Pass};
        let sig = self.m.signature(id);
        let def_ret = self.m.program.fns[id].ret;
        let mut values: Vec<Val> = Vec::new();
        if c_call.ret == Answer::Hidden {
            let dest = dest.expect("an aggregate result has a destination");
            values.push(Val::new(self.place_addr(dest), Lt::Ptr));
        }
        for (arg, pass) in args.iter().zip(&c_call.params) {
            let ty = self.operand_ty(arg);
            let place = |arg: &Operand| match arg {
                Operand::Copy(place) => place.clone(),
                Operand::Const(_) => unreachable!("an aggregate operand is a place"),
            };
            match pass {
                Pass::Parts(parts) | Pass::Spilled { parts, .. } => {
                    if let Pass::Spilled { pad, .. } = pass {
                        values.extend((0..pad.ints).map(|_| Val::new("0", Lt::I(64))));
                        values.extend((0..pad.floats).map(|_| Val::new("0.0", Lt::F64)));
                    }
                    // Copied to a slot as large as the pieces, which may
                    // reach past the struct's own bytes.
                    let src = self.place_addr(&place(arg));
                    let scratch = self.scratch(ty, parts);
                    let size = self.m.layout(ty).size;
                    self.memmove(&scratch, &src, size);
                    for part in parts {
                        let at = self.gep(&scratch, i64::from(part.offset));
                        values.push(self.load(&at, crate::reg_lt(part.reg), 1));
                    }
                }
                Pass::Copy => {
                    let src = self.place_addr(&place(arg));
                    let layout = self.m.layout(ty);
                    let copy = self.alloca(layout.size, layout.align);
                    self.memmove(&copy, &src, layout.size);
                    values.push(Val::new(copy, Lt::Ptr));
                }
                Pass::Stack(_) => values.push(Val::new(self.place_addr(&place(arg)), Lt::Ptr)),
                Pass::Plain if self.m.is_c_pair(ty) => {
                    let addr = self.place_addr(&place(arg));
                    let pointer = self.load(&addr, Lt::Ptr, 8);
                    let at = self.gep(&addr, 8);
                    let length = self.load(&at, Lt::I(64), 8);
                    values.push(pointer);
                    values.push(length);
                }
                Pass::Plain => values.push(self.operand(arg)),
            }
        }
        let target = self.m.symbol(id);
        let mut typed = Vec::new();
        for (value, slot) in values.iter().zip(&sig.params) {
            let v = self.coerce(value, slot.lt);
            typed.push(format!("{} {v}", slot.text()));
        }
        let call = format!("call {} {target}({})", sig.ret_text(), typed.join(", "));
        match &c_call.ret {
            // The pieces, stored where they belong through a slot as large
            // as all of them.
            Answer::Parts(parts) => {
                let both = self.value(call, Lt::Ptr);
                let scratch = self.scratch(def_ret, parts);
                let ty = crate::parts_type(parts);
                for (i, part) in parts.iter().enumerate() {
                    let lt = crate::reg_lt(part.reg);
                    let piece = self.value(format!("extractvalue {ty} {}, {i}", both.v), lt);
                    let at = self.gep(&scratch, i64::from(part.offset));
                    self.store(&piece, &at, 1);
                }
                let dest = dest.expect("an aggregate result has a destination");
                let dest = self.place_addr(dest);
                let size = self.m.layout(def_ret).size;
                self.memmove(&dest, &scratch, size);
            }
            Answer::Hidden => self.emit(call),
            Answer::Plain => match &sig.ret {
                Some(slot) => {
                    let answer = self.value(call, slot.lt);
                    if let Some(dest) = dest {
                        self.write(dest, &answer);
                    }
                }
                None => self.emit(call),
            },
        }
    }

    /// A slot for a struct that travels in registers: as large as the
    /// struct, and as the pieces it is loaded as.
    fn scratch(&mut self, ty: Ty, parts: &[mir::c_abi::Part]) -> String {
        let layout = self.m.layout(ty);
        let reach = parts
            .iter()
            .map(|part| {
                part.offset
                    + match part.reg {
                        mir::c_abi::Reg::F32 => 4,
                        _ => 8,
                    }
            })
            .max()
            .unwrap_or(0);
        self.alloca(layout.size.max(reach), 8)
    }

    /// A call that passes more than the declaration names: the named
    /// arguments as they always go, and the rest promoted as C promotes
    /// them, in LLVM's variadic call, which puts them where the target's
    /// C does.
    fn variadic_call(
        &mut self,
        id: wip_hir::FnId,
        original: wip_hir::FnId,
        args: &[Operand],
        dest: Option<&Place>,
    ) {
        let program = self.m.program;
        let named = program.fns[original].params.len();
        let sig = self.m.signature(original);
        let target = self.m.symbol(original);
        let mut typed = Vec::new();
        let mut slot = 0;
        for arg in &args[..named] {
            let ty = self.operand_ty(arg);
            if self.m.is_c_pair(ty) {
                let Operand::Copy(place) = arg else {
                    unreachable!("a pointer-and-length operand is a place")
                };
                let addr = self.place_addr(place);
                let pointer = self.load(&addr, Lt::Ptr, 8);
                let at = self.gep(&addr, 8);
                let length = self.load(&at, Lt::I(64), 8);
                typed.push(format!("ptr {}", pointer.v));
                typed.push(format!("i64 {}", length.v));
                slot += 2;
                continue;
            }
            let value = self.operand(arg);
            let param = &sig.params[slot];
            let v = self.coerce(&value, param.lt);
            typed.push(format!("{} {v}", param.text()));
            slot += 1;
        }
        for arg in &args[named..] {
            let ty = self.operand_ty(arg);
            let value = self.operand(arg);
            match mir::c_abi::promoted(program, ty).expect("a scalar extra argument") {
                mir::c_abi::Promoted::Double => {
                    let v = match value.t {
                        Lt::F32 => {
                            self.value(format!("fpext float {} to double", value.v), Lt::F64)
                                .v
                        }
                        _ => value.v,
                    };
                    typed.push(format!("double {v}"));
                }
                mir::c_abi::Promoted::Int { bits, signed } => {
                    let v = match value.t {
                        Lt::I(from) if from < bits => {
                            let extend = if signed { "sext" } else { "zext" };
                            self.value(
                                format!("{extend} {} to i{bits}", value.typed()),
                                Lt::I(bits),
                            )
                            .v
                        }
                        Lt::Ptr => value.v,
                        _ => value.v,
                    };
                    let t = if value.t == Lt::Ptr {
                        "ptr".to_string()
                    } else {
                        format!("i{bits}")
                    };
                    typed.push(format!("{t} {v}"));
                }
            }
        }
        // A function's type names types alone: what C's ABI asks of a
        // parameter is said of its argument, and of the result before the
        // type.
        let named_types: Vec<String> = sig.params.iter().map(|slot| slot.lt.text()).collect();
        let (attr, ret) = match (&sig.parts, &sig.ret) {
            (Some(parts), _) => (String::new(), crate::parts_type(parts)),
            (None, Some(slot)) if !slot.attr.is_empty() => {
                (format!("{} ", slot.attr), slot.lt.text())
            }
            (None, Some(slot)) => (String::new(), slot.lt.text()),
            (None, None) => (String::new(), "void".to_string()),
        };
        let fn_type = format!("{ret} ({}, ...)", named_types.join(", "));
        let call = format!("call {attr}{fn_type} {target}({})", typed.join(", "));
        let _ = id;
        match &sig.ret {
            Some(slot) => {
                let answer = self.value(call, slot.lt);
                if let Some(dest) = dest {
                    self.write(dest, &answer);
                }
            }
            None => self.emit(call),
        }
    }

    /// A call of a function of the program by its signature, with the
    /// values already split as it takes them.
    fn call_values(&mut self, id: wip_hir::FnId, values: Vec<Val>) -> Option<Val> {
        let sig = self.m.signature(id);
        let target = self.m.symbol(id);
        self.emit_call(&target, &sig, values)
    }

    fn emit_call(&mut self, target: &str, sig: &Sig, values: Vec<Val>) -> Option<Val> {
        assert_eq!(
            values.len(),
            sig.params.len(),
            "a call of {target} passes what it takes"
        );
        let mut args = Vec::new();
        for (value, slot) in values.iter().zip(&sig.params) {
            let v = self.coerce(value, slot.lt);
            args.push(format!("{} {v}", slot.text()));
        }
        let call = format!("call {} {target}({})", sig.ret_text(), args.join(", "));
        match &sig.ret {
            Some(slot) => Some(self.value(call, slot.lt)),
            None => {
                self.emit(call);
                None
            }
        }
    }

    // Terminators.

    fn terminator(&mut self, terminator: &Terminator) {
        match terminator {
            Terminator::Goto(target) => self.emit(format!("br label %bb{}", target.0)),
            Terminator::Branch {
                cond,
                then,
                otherwise,
            } => {
                let cond = self.operand(cond);
                let flag = self.value(format!("icmp ne {}, 0", cond.typed()), Lt::I(1));
                self.emit(format!(
                    "br i1 {}, label %bb{}, label %bb{}",
                    flag.v, then.0, otherwise.0
                ));
            }
            Terminator::Switch {
                value,
                cases,
                otherwise,
            } => {
                let value = self.operand(value);
                let value = self.coerce(&value, Lt::I(32));
                let cases: Vec<String> = cases
                    .iter()
                    .map(|&(case, target)| format!("i32 {}, label %bb{}", case as i32, target.0))
                    .collect();
                self.emit(format!(
                    "switch i32 {value}, label %bb{} [ {} ]",
                    otherwise.0,
                    cases.join(" ")
                ));
            }
            Terminator::Return => {
                let ret = self.body.ret;
                match ret {
                    Some(ret)
                        if !self.m.is_aggregate(self.body.local(ret).ty)
                            && self.m.lt(self.body.local(ret).ty).is_some() =>
                    {
                        let ty = self.body.local(ret).ty;
                        let value = self.operand(&Operand::Copy(Place::local(ret)));
                        let slot = self.m.slot(ty).expect("a scalar result");
                        let v = self.coerce(&value, slot.lt);
                        self.emit(format!("ret {} {v}", slot.lt.text()));
                    }
                    _ => self.emit("ret void"),
                }
            }
            Terminator::Panic { note, message, at } => {
                let text =
                    message.map_or(String::new(), |m| self.m.interner.resolve(m).to_string());
                let data = self.m.text(&text);
                let message = vec![
                    Val::new(data, Lt::Ptr),
                    Val::new(text.len().to_string(), Lt::I(64)),
                ];
                match note {
                    None => self.panic_call("wip_panic", message, *at),
                    Some(Operand::Copy(place)) => {
                        let addr = self.place_addr(place);
                        let pointer = self.load(&addr, Lt::Ptr, 8);
                        let at_length = self.gep(&addr, 8);
                        let length = self.load(&at_length, Lt::I(64), 8);
                        let mut numbers = vec![pointer, length];
                        numbers.extend(message);
                        self.panic_call("wip_panic_noted", numbers, *at);
                    }
                    Some(Operand::Const(_)) => unreachable!("a note is a place"),
                }
                self.emit("unreachable");
            }
            Terminator::Unreachable => self.emit("unreachable"),
        }
    }
}

/// A float's LLVM type and the suffix its intrinsics take.
fn float_names(t: Lt) -> (&'static str, &'static str) {
    match t {
        Lt::F32 => ("float", "f32"),
        _ => ("double", "f64"),
    }
}

/// Which arithmetic a panic is about, as the runtime's message names it.
fn overflow_code(op: BinaryOp) -> i32 {
    match op {
        BinaryOp::Add => 0,
        BinaryOp::Sub => 1,
        BinaryOp::Mul => 2,
        _ => 3,
    }
}
