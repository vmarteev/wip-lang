//! An LLVM backend, as an experiment: the MIR every backend takes, written as
//! LLVM's textual IR, which `clang -O2` compiles. Nothing of LLVM is linked
//! into the compiler.
//!
//! The whole program is one LLVM module, so LLVM sees every function
//! together and inlines across what were Wip's modules. Each local is a
//! stack slot of its layout's size, read and written at byte offsets;
//! LLVM's `mem2reg` and SROA turn what can be into registers. Calls are
//! made as the Cranelift backend makes them: an aggregate by the address
//! of its place, an aggregate result through an address the caller
//! supplies, and a `str` or a `&[T]` to C as a pointer and a length. A
//! struct C takes by value travels as `wip_mir::c_abi` says, as it does in
//! Cranelift's code, and a variadic call is LLVM's own.
//!
//! Debug information is LLVM's metadata, which LLVM writes as DWARF and
//! keeps through its inlining; the table of functions a
//! panic reads to say its calls is made from that DWARF
//! once LLVM has compiled the program (`frame_tables`).

mod debug;
mod debugtypes;
mod export;
mod frames;
mod function;

pub use frames::{frame_table_assembly, with_frame_tables};

use std::collections::{BTreeSet, VecDeque};
use std::fmt::Write as _;

use rustc_hash::{FxHashMap, FxHashSet};
use wip_hir::{FloatTy, FnId, Program, Ty, TyKind, Types};
use wip_mir::{self as mir, Layout, Layouts};
use wip_syntax::{Interner, Span, Symbol};

/// Where a span was written, for a panic's message: the
/// file as diagnostics name it, and the line and column.
pub type Locations<'a> = &'a (dyn Fn(Span) -> (String, u32, u32) + Sync);

/// What the program starts at: its `main`, or the tests `wip test` runs.
#[derive(Clone, Copy)]
pub enum Entry<'a> {
    Main(FnId),
    Tests(&'a [FnId]),
}

/// The extern functions called through a wrapper in C: those Cranelift's
/// code calls so.
pub fn shimmed(program: &Program) -> FxHashSet<FnId> {
    mir::c_abi::shimmed(program, mir::c_abi::Arch::host())
}

/// The program as LLVM IR, and the files its DWARF names.
pub struct Ir {
    pub text: String,
    /// Each file as the DWARF names it, with the name a panic gives it.
    pub files: FxHashMap<String, String>,
}

/// The program as LLVM IR.
pub fn compile(
    program: &Program,
    interner: &Interner,
    entry: Option<Entry<'_>>,
    locations: Locations<'_>,
) -> Ir {
    let mut llvm = Llvm::new(program, interner, locations);
    for (id, def) in program.fns.iter() {
        if def.exports_c && program.is_compiled(id) {
            llvm.symbol(id);
        }
    }
    match entry {
        Some(Entry::Main(main)) => llvm.define_entry(main),
        Some(Entry::Tests(tests)) => llvm.define_test_entry(tests),
        None => {}
    }
    loop {
        if let Some(id) = llvm.queue.pop_front() {
            llvm.define_function(id);
        } else if let Some((ty, in_place)) = llvm.pending_drops.pop() {
            llvm.define_drop(ty, in_place);
        } else {
            break;
        }
    }
    llvm.finish()
}

/// A value's type in LLVM: an integer of its width, a float, or a pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Lt {
    I(u32),
    F32,
    F64,
    Ptr,
}

impl Lt {
    pub(crate) fn text(self) -> String {
        match self {
            Lt::I(bits) => format!("i{bits}"),
            Lt::F32 => "float".to_string(),
            Lt::F64 => "double".to_string(),
            Lt::Ptr => "ptr".to_string(),
        }
    }
}

/// A parameter or a result as a signature has it: its type, and what C's
/// ABI asks of it — a narrow integer extended, a struct's address marked as
/// where the result goes or as the bytes on the stack.
#[derive(Clone, Debug)]
pub(crate) struct Slot {
    pub lt: Lt,
    pub attr: String,
}

impl Slot {
    pub(crate) fn new(lt: Lt) -> Slot {
        Slot {
            lt,
            attr: String::new(),
        }
    }

    pub(crate) fn text(&self) -> String {
        match self.attr.as_str() {
            "" => self.lt.text(),
            attr => format!("{} {attr}", self.lt.text()),
        }
    }
}

/// A function's signature in LLVM, as Wip calls it: the address an
/// aggregate result is written to first, then the parameters.
#[derive(Clone, Debug)]
pub(crate) struct Sig {
    pub params: Vec<Slot>,
    pub ret: Option<Slot>,
    /// The first parameter is where an aggregate result goes.
    pub sret: bool,
    /// A struct C answers in registers: the pieces, as LLVM returns a
    /// struct of them.
    pub parts: Option<Vec<mir::c_abi::Part>>,
}

impl Sig {
    pub(crate) fn ret_text(&self) -> String {
        if let Some(parts) = &self.parts {
            return parts_type(parts);
        }
        match &self.ret {
            Some(slot) => match slot.attr.as_str() {
                "" => slot.lt.text(),
                attr => format!("{attr} {}", slot.lt.text()),
            },
            None => "void".to_string(),
        }
    }
}

/// The LLVM type of a register a piece of a struct travels in.
pub(crate) fn reg_lt(reg: mir::c_abi::Reg) -> Lt {
    match reg {
        mir::c_abi::Reg::I64 => Lt::I(64),
        mir::c_abi::Reg::F32 => Lt::F32,
        mir::c_abi::Reg::F64 => Lt::F64,
    }
}

/// The struct LLVM returns pieces in.
pub(crate) fn parts_type(parts: &[mir::c_abi::Part]) -> String {
    let fields: Vec<String> = parts.iter().map(|part| reg_lt(part.reg).text()).collect();
    format!("{{ {} }}", fields.join(", "))
}

pub(crate) struct Llvm<'p> {
    pub program: &'p Program,
    pub interner: &'p Interner,
    pub locations: Locations<'p>,
    pub layouts: Layouts,
    pub shimmed: FxHashSet<FnId>,
    /// The processor whose C ABI calls are made in.
    arch: Option<mir::c_abi::Arch>,
    /// Whether a `&var` parameter is marked `noalias`.
    noalias: bool,
    c_calls: FxHashMap<FnId, Option<mir::c_abi::CCall>>,
    /// The variables C owns that the program reads or writes.
    c_globals: FxHashMap<wip_hir::GlobalId, String>,
    /// The functions defined so far.
    functions: String,
    /// What the program's data is: strings, tables, the runtime's words.
    data: String,
    /// What is declared and not defined: C's functions and LLVM's
    /// intrinsics.
    pub declarations: BTreeSet<String>,
    /// Each function's symbol, once referred to.
    names: FxHashMap<FnId, String>,
    /// The C symbols declared, so that one declared twice under two names
    /// is one declaration.
    c_declared: FxHashSet<String>,
    pub queue: VecDeque<FnId>,
    pub debug: debug::Debug<'p>,
    /// Each type's node of debug information, once written.
    di_types: FxHashMap<Ty, Option<usize>>,
    strings: FxHashMap<Symbol, String>,
    texts: FxHashMap<String, String>,
    tables: FxHashMap<wip_hir::ConstId, String>,
    vtables: FxHashMap<(wip_hir::InterfaceId, Ty), String>,
    blobs: FxHashMap<usize, String>,
    drops: FxHashMap<(Ty, bool), String>,
    pub pending_drops: Vec<(Ty, bool)>,
    /// The runtime's functions, which C's names reach: `wip_alloc`,
    /// `wip_panic_index` and the rest.
    runtime: FxHashMap<String, FnId>,
}

impl<'p> Llvm<'p> {
    fn new(program: &'p Program, interner: &'p Interner, locations: Locations<'p>) -> Self {
        let runtime = program
            .fns
            .iter()
            .filter(|(_, def)| def.exports_c)
            .map(|(id, def)| {
                let name = interner.resolve(def.symbol.unwrap_or(def.name)).to_string();
                (name, id)
            })
            .collect();
        Llvm {
            program,
            interner,
            locations,
            layouts: Layouts::new(),
            shimmed: shimmed(program),
            arch: mir::c_abi::Arch::host(),
            noalias: std::env::var("WIP_LLVM_NOALIAS").map_or(true, |v| v != "0"),
            c_calls: FxHashMap::default(),
            c_globals: FxHashMap::default(),
            functions: String::new(),
            data: String::new(),
            declarations: BTreeSet::new(),
            names: FxHashMap::default(),
            c_declared: FxHashSet::default(),
            queue: VecDeque::new(),
            debug: debug::Debug::new(locations),
            di_types: FxHashMap::default(),
            strings: FxHashMap::default(),
            texts: FxHashMap::default(),
            tables: FxHashMap::default(),
            vtables: FxHashMap::default(),
            blobs: FxHashMap::default(),
            drops: FxHashMap::default(),
            pending_drops: Vec::new(),
            runtime,
        }
    }

    fn finish(self) -> Ir {
        let mut out = String::new();
        out.push_str("; Written by wip's LLVM backend.\n\n");
        out.push_str(&self.data);
        // The runtime's words, and the list of the tables a
        // panic reads its calls from, which is written once
        // LLVM has compiled the program and its DWARF says what is where
        // (`frame_tables`).
        out.push_str("@wip_runtime_words = global [8 x i64] zeroinitializer, align 8\n");
        out.push_str("@\"wip.frame_tables\" = external hidden global [0 x i64], align 8\n\n");
        for declaration in &self.declarations {
            out.push_str(declaration);
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&self.functions);
        // Every function keeps its frame record, as Cranelift's do, which a
        // profiler walks; nothing unwinds through Wip's code. A call stays
        // a call, as in Cranelift's code, so that a panic's calls are all
        // there: one in the tail of a function would be a jump, and the
        // function would be missing from them. A function that does not
        // return — a panic — is never called from two places by one call,
        // which would have one line for both.
        out.push_str(
            "attributes #0 = { nounwind uwtable \"frame-pointer\"=\"all\" \
             \"disable-tail-calls\"=\"true\" }\n",
        );
        out.push_str(
            "attributes #1 = { nounwind uwtable \"frame-pointer\"=\"all\" \
             \"disable-tail-calls\"=\"true\" noreturn nomerge }\n\n",
        );
        self.debug.write(&mut out);
        Ir {
            text: out,
            files: self.debug.paths,
        }
    }

    pub(crate) fn kind(&self, ty: Ty) -> TyKind {
        self.program.types.kind(ty)
    }

    pub(crate) fn layout(&mut self, ty: Ty) -> Layout {
        self.layouts.of(self.program, ty)
    }

    pub(crate) fn is_aggregate(&self, ty: Ty) -> bool {
        mir::is_aggregate(self.program, ty)
    }

    /// A scalar's LLVM type; `None` for an aggregate or a type with no size.
    pub(crate) fn lt(&self, ty: Ty) -> Option<Lt> {
        if mir::is_slice_pointer(self.program, ty)
            || mir::is_dyn_pointer(self.program, ty)
            || mir::is_closure_pointer(self.program, ty)
        {
            return None;
        }
        match self.kind(ty) {
            TyKind::Int(t) => Some(Lt::I(t.bits())),
            TyKind::Float(FloatTy::F32) => Some(Lt::F32),
            TyKind::Float(FloatTy::F64) => Some(Lt::F64),
            TyKind::Bool => Some(Lt::I(8)),
            TyKind::Char => Some(Lt::I(32)),
            TyKind::Cstring
            | TyKind::Fn(..)
            | TyKind::Own(_)
            | TyKind::Ptr(_)
            | TyKind::Ref(..) => Some(Lt::Ptr),
            _ => None,
        }
    }

    /// A scalar parameter or result: values narrower than 32 bits are
    /// extended by the caller, as Apple's arm64 ABI requires.
    pub(crate) fn slot(&self, ty: Ty) -> Option<Slot> {
        let lt = self.lt(ty)?;
        let attr = match self.kind(ty) {
            // A `&var` argument is the only way to its place while the call
            // runs (E0413), which is what LLVM's `noalias`
            // says. `WIP_LLVM_NOALIAS=0` leaves it out, to measure.
            TyKind::Ref(_, wip_hir::RefKind::Var) if self.noalias => "noalias",
            TyKind::Bool => "zeroext",
            TyKind::Int(t) if t.bits() < 32 && t.signed() => "signext",
            TyKind::Int(t) if t.bits() < 32 => "zeroext",
            _ => "",
        };
        Some(Slot {
            lt,
            attr: attr.to_string(),
        })
    }

    /// A scalar result, as a parameter is but for `noalias`: on a result,
    /// LLVM reads it as `malloc`'s promise of memory nothing else reaches,
    /// which a projection's `&var` into its argument is not.
    pub(crate) fn result(&self, ty: Ty) -> Option<Slot> {
        let mut slot = self.slot(ty)?;
        if slot.attr.starts_with("noalias") {
            slot.attr.clear();
        }
        Some(slot)
    }

    /// Whether a `&` parameter of this type reaches a place that nothing
    /// changes while the call runs, which LLVM's `noalias readonly` says:
    /// a `&` excludes every `&var` to its place (E0413),
    /// and through a `&` only an `@intrinsic` struct —
    /// `std::sync::Atomic` — changes what it holds, so a
    /// place that holds none of them by value does not change. What it
    /// reaches through an `own`, a `ptr` or another reference is other
    /// memory, of which the attribute says nothing; and C, which may
    /// write where it likes, is never on either side of the call.
    fn only_read(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Ref(inner, wip_hir::RefKind::Shared) => {
                self.lt(ty) == Some(Lt::Ptr) && !self.changes_through_a_ref(inner)
            }
            _ => false,
        }
    }

    /// Whether a value of `ty` holds, by value, what changes through a `&`.
    fn changes_through_a_ref(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Struct(id, _) => {
                self.program.structs[id].is_intrinsic
                    || self
                        .program
                        .field_tys(ty)
                        .into_iter()
                        .any(|field| self.changes_through_a_ref(field))
            }
            TyKind::Enum(..) => self
                .program
                .variant_field_tys(ty)
                .into_iter()
                .flatten()
                .any(|field| self.changes_through_a_ref(field)),
            TyKind::Array(elem, _) => self.changes_through_a_ref(elem),
            _ => false,
        }
    }

    /// Whether C takes the type as two arguments, a pointer and a length.
    pub(crate) fn is_c_pair(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Str | TyKind::Slice(_) => true,
            TyKind::Ref(inner, _) => matches!(self.kind(inner), TyKind::Slice(_)),
            _ => false,
        }
    }

    /// The signature of a function of `params` and `ret`; `c_abi` where C
    /// is on the other side, and a `str` is two arguments.
    pub(crate) fn signature_with(&mut self, params: &[Ty], ret: Ty, c_abi: bool) -> Sig {
        let mut slots = Vec::new();
        let sret = self.is_aggregate(ret);
        if sret {
            slots.push(Slot::new(Lt::Ptr));
        }
        for &param in params {
            if c_abi && self.is_c_pair(param) {
                slots.push(Slot::new(Lt::Ptr));
                slots.push(Slot::new(Lt::I(64)));
                continue;
            }
            let mut slot = self.slot(param).unwrap_or(Slot::new(Lt::Ptr));
            if !c_abi && self.noalias && self.only_read(param) {
                slot.attr = "noalias readonly".to_string();
            }
            // An aggregate taken by value is passed by the address of a
            // place nothing else reaches while the call runs: a copy the
            // caller made of a place it keeps, or what it moved or built,
            // which it no longer names. The callee
            // never assigns to it (E0304), and moves out of it only what
            // the caller gave up.
            if !c_abi && self.noalias && self.is_aggregate(param) {
                slot.attr = "noalias".to_string();
            }
            add_attr(&mut slot, self.referent_attrs(param));
            slots.push(slot);
        }
        let mut ret_slot = self.result(ret);
        if let Some(slot) = ret_slot.as_mut() {
            add_attr(slot, self.referent_attrs(ret));
        }
        Sig {
            params: slots,
            ret: ret_slot,
            sret,
            parts: None,
        }
    }

    /// The whole value a reference of type `ty` reaches, where it is one
    /// address and the value has a size: a Wip reference is never null and
    /// always reaches all of a value of its type, aligned as the type is,
    /// for as long as it is held. A value of no size may
    /// be at any address, and is given nothing.
    pub(crate) fn referent(&mut self, ty: Ty) -> Option<Layout> {
        match self.kind(ty) {
            TyKind::Ref(inner, _) if self.lt(ty) == Some(Lt::Ptr) => {
                let layout = self.layout(inner);
                (layout.size > 0).then_some(layout)
            }
            _ => None,
        }
    }

    /// What a parameter or result of type `ty` says of what it reaches.
    fn referent_attrs(&mut self, ty: Ty) -> String {
        match self.referent(ty) {
            Some(layout) => format!(
                "nonnull dereferenceable({}) align {}",
                layout.size,
                layout.align.max(1)
            ),
            None => String::new(),
        }
    }

    pub(crate) fn signature(&mut self, id: FnId) -> Sig {
        if let Some(c_call) = self.c_call(id) {
            return self.c_signature(id, &c_call);
        }
        let def = &self.program.fns[id];
        let params: Vec<Ty> = def.params.iter().map(|p| p.ty).collect();
        // For finding where `noalias` goes wrong: only the functions whose
        // names hold `WIP_LLVM_NOALIAS_ONLY`, where it is set.
        let keep = self.noalias;
        if let Ok(only) = std::env::var("WIP_LLVM_NOALIAS_ONLY") {
            let name = self.program.fn_name(id, self.interner);
            self.noalias = keep && only.split(',').any(|part| name.contains(part));
        }
        let sig = self.signature_with(&params, def.ret, uses_c_abi(def));
        self.noalias = keep;
        sig
    }

    /// The signature of a call through a value, with `noalias` only where
    /// no filter is set.
    pub(crate) fn indirect_signature(&mut self, params: &[Ty], ret: Ty, c_abi: bool) -> Sig {
        let keep = self.noalias;
        if std::env::var_os("WIP_LLVM_NOALIAS_ONLY").is_some() {
            self.noalias = false;
        }
        let sig = self.signature_with(params, ret, c_abi);
        self.noalias = keep;
        sig
    }

    /// How C passes `id`'s structs, where it passes one by value and the
    /// call is made here rather than in a wrapper.
    pub(crate) fn c_call(&mut self, id: FnId) -> Option<mir::c_abi::CCall> {
        if let Some(known) = self.c_calls.get(&id) {
            return known.clone();
        }
        let def = &self.program.fns[id];
        let c_call = match self.arch {
            Some(arch)
                if uses_c_abi(def)
                    && !self.shimmed.contains(&id)
                    && mir::c_abi::by_value(self.program, def) =>
            {
                mir::c_abi::classify(self.program, &mut self.layouts, def, arch)
                    .filter(|c_call| !c_call.is_plain())
            }
            _ => None,
        };
        self.c_calls.insert(id, c_call.clone());
        c_call
    }

    /// The signature C gives a function whose structs travel as `c_call`
    /// says: pieces in registers, a pointer to a copy, bytes on the stack,
    /// and a result in registers or through the address C is given.
    pub(crate) fn c_signature(&mut self, id: FnId, c_call: &mir::c_abi::CCall) -> Sig {
        use mir::c_abi::{Answer, Pass};
        let def = &self.program.fns[id];
        let mut params = Vec::new();
        let sret = c_call.ret == Answer::Hidden;
        if sret {
            let layout = self.layout(def.ret);
            params.push(Slot {
                lt: Lt::Ptr,
                attr: format!(
                    "sret([{} x i8]) align {}",
                    layout.size.max(1),
                    layout.align.max(1)
                ),
            });
        }
        for (param, pass) in def.params.iter().zip(&c_call.params) {
            match pass {
                Pass::Plain if self.is_c_pair(param.ty) => {
                    params.push(Slot::new(Lt::Ptr));
                    params.push(Slot::new(Lt::I(64)));
                }
                Pass::Plain => params.push(self.slot(param.ty).unwrap_or(Slot::new(Lt::Ptr))),
                Pass::Parts(parts) => {
                    params.extend(parts.iter().map(|part| Slot::new(reg_lt(part.reg))));
                }
                Pass::Spilled { pad, parts } => {
                    params.extend((0..pad.ints).map(|_| Slot::new(Lt::I(64))));
                    params.extend((0..pad.floats).map(|_| Slot::new(Lt::F64)));
                    params.extend(parts.iter().map(|part| Slot::new(reg_lt(part.reg))));
                }
                Pass::Copy => params.push(Slot::new(Lt::Ptr)),
                Pass::Stack(size) => params.push(Slot {
                    lt: Lt::Ptr,
                    attr: format!("byval([{size} x i8]) align 8"),
                }),
            }
        }
        let (ret, parts) = match &c_call.ret {
            Answer::Plain => (self.result(def.ret), None),
            Answer::Parts(parts) => (None, Some(parts.clone())),
            Answer::Hidden => (None, None),
        };
        Sig {
            params,
            ret,
            sret,
            parts,
        }
    }

    /// A variable C owns, as the symbol the linker resolves.
    pub(crate) fn c_global(&mut self, global: wip_hir::GlobalId) -> String {
        if let Some(name) = self.c_globals.get(&global) {
            return name.clone();
        }
        let def = &self.program.globals[global];
        let name = global_name(self.interner.resolve(def.symbol.unwrap_or(def.name)));
        self.declarations
            .insert(format!("{name} = external global i8"));
        self.c_globals.insert(global, name.clone());
        name
    }

    /// A function's symbol, `@"…"`, declared or asked to be defined the
    /// first time it is referred to.
    pub(crate) fn symbol(&mut self, id: FnId) -> String {
        if let Some(name) = self.names.get(&id) {
            return name.clone();
        }
        let def = &self.program.fns[id];
        // C's name for a function the program defines itself, exported to
        // C: the one function, in the one module.
        if def.is_extern && !self.shimmed.contains(&id) {
            let c_name = self.interner.resolve(def.symbol.unwrap_or(def.name));
            if let Some(&own) = self.runtime.get(c_name)
                && own != id
            {
                let name = self.symbol(own);
                self.names.insert(id, name.clone());
                return name;
            }
        }
        let name = if def.is_extern {
            let c_name = if self.shimmed.contains(&id) {
                mir::shim_name(id)
            } else {
                self.interner
                    .resolve(def.symbol.unwrap_or(def.name))
                    .to_string()
            };
            let symbol = global(&c_name);
            // One C function may be declared under two names, each with a
            // signature of its own; a call says its own, and the first is
            // the declaration's.
            if self.c_declared.insert(c_name) {
                let sig = self.signature(id);
                let mut params: Vec<String> = sig.params.iter().map(Slot::text).collect();
                // A function C declares with `...`, called with more.
                if def.is_variadic && !self.shimmed.contains(&id) {
                    params.push("...".to_string());
                }
                self.declarations.insert(format!(
                    "declare {} {symbol}({})",
                    sig.ret_text(),
                    params.join(", ")
                ));
            }
            symbol
        } else if def.exports_c {
            self.queue.push_back(id);
            global(self.interner.resolve(def.symbol.unwrap_or(def.name)))
        } else {
            self.queue.push_back(id);
            let written = self.program.fn_name(id, self.interner);
            global(&format!("wip.{written}.{}", u32::from(id.into_raw())))
        };
        self.names.insert(id, name.clone());
        name
    }

    /// A runtime function by the name C knows it by.
    pub(crate) fn runtime(&mut self, name: &str) -> FnId {
        *self
            .runtime
            .get(name)
            .unwrap_or_else(|| panic!("the prelude exports `{name}`"))
    }

    /// The drop function of an `own` of `ty`, or the one that drops a value
    /// of `ty` where it lies.
    pub(crate) fn drop_fn(&mut self, ty: Ty, in_place: bool) -> String {
        if let Some(name) = self.drops.get(&(ty, in_place)) {
            return name.clone();
        }
        let kind = if in_place { "drop_in_place" } else { "drop" };
        let name = global(&format!("wip.{kind}.{}", self.drops.len()));
        self.drops.insert((ty, in_place), name.clone());
        self.pending_drops.push((ty, in_place));
        name
    }

    /// A string literal's bytes and its NUL.
    pub(crate) fn string(&mut self, sym: Symbol) -> String {
        if let Some(name) = self.strings.get(&sym) {
            return name.clone();
        }
        let name = global(&format!("wip.str.{}", self.strings.len()));
        let mut bytes = self.interner.resolve(sym).as_bytes().to_vec();
        bytes.push(0);
        let _ = writeln!(
            self.data,
            "{name} = private unnamed_addr constant [{} x i8] c\"{}\", align 1",
            bytes.len(),
            escape(&bytes)
        );
        self.strings.insert(sym, name.clone());
        name
    }

    /// Bytes without a NUL: a panic's message and file name.
    pub(crate) fn text(&mut self, text: &str) -> String {
        if let Some(name) = self.texts.get(text) {
            return name.clone();
        }
        let name = global(&format!("wip.text.{}", self.texts.len()));
        let bytes = text.as_bytes();
        // An object holds a byte at least; an empty text's is never read.
        let bytes: &[u8] = if bytes.is_empty() { &[0] } else { bytes };
        let _ = writeln!(
            self.data,
            "{name} = private unnamed_addr constant [{} x i8] c\"{}\", align 1",
            bytes.len(),
            escape(bytes)
        );
        self.texts.insert(text.to_string(), name.clone());
        name
    }

    fn blob(&mut self, bytes: &std::sync::Arc<[u8]>) -> String {
        let key = std::sync::Arc::as_ptr(bytes) as *const u8 as usize;
        if let Some(name) = self.blobs.get(&key) {
            return name.clone();
        }
        let name = global(&format!("wip.embedded.{}", self.blobs.len()));
        let contents: &[u8] = if bytes.is_empty() { &[0] } else { bytes };
        let _ = writeln!(
            self.data,
            "{name} = private unnamed_addr constant [{} x i8] c\"{}\", align 1",
            contents.len(),
            escape(contents)
        );
        self.blobs.insert(key, name.clone());
        name
    }

    /// A constant table, kept once, in read-only data: its
    /// bytes, with the addresses of strings and files where they go.
    pub(crate) fn table(&mut self, id: wip_hir::ConstId) -> String {
        if let Some(name) = self.tables.get(&id) {
            return name.clone();
        }
        let name = global(&format!("wip.table.{}", id.into_raw().into_u32()));
        let def = &self.program.consts[id];
        let ty = def.ty;
        let value = def
            .value
            .as_ref()
            .expect("only a table that was worked out is kept");
        let table = mir::table_bytes(
            self.program,
            self.interner,
            &mut self.layouts,
            value,
            ty,
            cfg!(target_endian = "little"),
        );
        let mut addresses: Vec<(u32, String)> = Vec::new();
        for (offset, sym) in &table.strings {
            addresses.push((*offset, self.string(*sym)));
        }
        for (offset, bytes) in &table.blobs {
            addresses.push((*offset, self.blob(bytes)));
        }
        addresses.sort_by_key(|(offset, _)| *offset);
        let mut types = Vec::new();
        let mut values = Vec::new();
        let mut at = 0usize;
        let bytes = &table.bytes;
        for (offset, target) in addresses {
            let offset = offset as usize;
            if offset > at {
                types.push(format!("[{} x i8]", offset - at));
                values.push(format!(
                    "[{} x i8] c\"{}\"",
                    offset - at,
                    escape(&bytes[at..offset])
                ));
            }
            types.push("ptr".to_string());
            values.push(format!("ptr {target}"));
            at = offset + 8;
        }
        if at < bytes.len() || types.is_empty() {
            let rest = if at < bytes.len() {
                &bytes[at..]
            } else {
                &[0u8][..]
            };
            types.push(format!("[{} x i8]", rest.len()));
            values.push(format!("[{} x i8] c\"{}\"", rest.len(), escape(rest)));
        }
        let align = self.layout(ty).align.max(1);
        let _ = writeln!(
            self.data,
            "{name} = private unnamed_addr constant <{{ {} }}> <{{ {} }}>, align {align}",
            types.join(", "),
            values.join(", ")
        );
        self.tables.insert(id, name.clone());
        name
    }

    /// A type's table of methods for an interface.
    pub(crate) fn vtable(&mut self, interface: wip_hir::InterfaceId, ty: Ty) -> String {
        if let Some(name) = self.vtables.get(&(interface, ty)) {
            return name.clone();
        }
        let name = global(&format!("wip.vtable.{}", self.vtables.len()));
        self.vtables.insert((interface, ty), name.clone());
        let methods = self
            .program
            .vtables
            .iter()
            .find(|table| table.interface == interface && table.ty == ty)
            .expect("every table a program needs was made with its instances")
            .methods
            .clone();
        let entries: Vec<String> = methods
            .into_iter()
            .map(|method| format!("ptr {}", self.symbol(method)))
            .collect();
        let _ = writeln!(
            self.data,
            "{name} = private unnamed_addr constant [{} x ptr] [{}], align 8",
            entries.len(),
            entries.join(", ")
        );
        name
    }

    /// C's `abort`, declared once, whether or not the program declares it
    /// itself.
    pub(crate) fn abort(&mut self) -> String {
        let symbol = global("abort");
        if self.c_declared.insert("abort".to_string()) {
            self.declarations
                .insert(format!("declare void {symbol}() noreturn nounwind"));
        }
        symbol
    }

    /// An intrinsic of LLVM's, declared once.
    pub(crate) fn intrinsic(&mut self, declaration: &str) {
        self.declarations.insert(declaration.to_string());
    }

    fn define_function(&mut self, id: FnId) {
        let def = &self.program.fns[id];
        let Some(mut body) = mir::function_body(self.program, self.interner, id) else {
            return;
        };
        // The release build's MIR, as Cranelift is given it.
        mir::scalars(self.program, &mut body);
        // For finding what MIR reads before it writes.
        if std::env::var_os("WIP_MIR_UNWRITTEN").is_some() {
            for (local, block) in mir::reads_before_writes(&body) {
                let decl = body.local(local);
                let name = decl.source.map_or(String::from("-"), |s| {
                    self.interner.resolve(s.name).to_string()
                });
                eprintln!(
                    "unwritten: {} _{} ({name}, {:?}, {}) read in bb{}",
                    self.program.fn_name(id, self.interner),
                    local.0,
                    decl.kind,
                    self.program.ty_name(decl.ty, self.interner),
                    block.0
                );
            }
        }
        let mut sig = self.signature(id);
        let mut symbol = self.symbol(id);
        let mut linkage = if def.exports_c { "" } else { "internal " };
        let name = self.program.fn_name(id, self.interner);
        // C's side of a function C passes a struct by value to, or is
        // answered one by, is a function of its own, and the function is
        // Wip's as any other.
        if def.exports_c
            && let Some(c_call) = self.c_call(id)
        {
            let body = global(&format!("wip.{name}.{}", u32::from(id.into_raw())));
            let text = export::define(self, id, &symbol, &body, &c_call);
            self.functions.push_str(&text);
            let params: Vec<Ty> = def.params.iter().map(|p| p.ty).collect();
            sig = self.signature_with(&params, def.ret, true);
            symbol = body;
            linkage = "internal ";
        }
        let at = def.span;
        let subprogram = self.debug.subprogram(&name, Some(at));
        let text = function::define(
            self,
            &body,
            &symbol,
            linkage,
            &sig,
            uses_c_abi(def),
            def.ret == Types::NEVER,
            subprogram,
            Some(at),
        );
        self.functions.push_str(&text);
    }

    fn define_drop(&mut self, ty: Ty, in_place: bool) {
        let body = match in_place {
            false => mir::lower_drop_fn(self.program, self.interner, ty),
            true => mir::lower_drop_in_place_fn(self.program, self.interner, ty),
        };
        let symbol = self.drop_fn(ty, in_place);
        let sig = Sig {
            params: vec![Slot::new(Lt::Ptr)],
            ret: None,
            sret: false,
            parts: None,
        };
        let name = format!("drop<{}>", self.program.ty_name(ty, self.interner));
        let subprogram = self.debug.subprogram(&name, None);
        let text = function::define(
            self,
            &body,
            &symbol,
            "internal ",
            &sig,
            false,
            false,
            subprogram,
            None,
        );
        self.functions.push_str(&text);
    }

    /// `int main(int argc, char **argv)`: calls Wip's `main`, with the
    /// program's arguments as a slice if it takes them, and answers its
    /// exit code.
    fn define_entry(&mut self, main: FnId) {
        let def = &self.program.fns[main];
        let callee = self.symbol(main);
        let mut out = String::new();
        let mut args = Vec::new();
        if !def.params.is_empty() {
            out.push_str("  %args = alloca [16 x i8], align 8\n");
            out.push_str("  store ptr %argv, ptr %args, align 8\n");
            out.push_str("  %count = sext i32 %argc to i64\n");
            out.push_str("  %length = getelementptr inbounds i8, ptr %args, i64 8\n");
            out.push_str("  store i64 %count, ptr %length, align 8\n");
            args.push("ptr %args".to_string());
        }
        if let Some(report) = self.program.entry_report {
            let layout = self.layout(def.ret);
            let report = self.symbol(report);
            let _ = writeln!(
                out,
                "  %result = alloca [{} x i8], align {}",
                layout.size.max(1),
                layout.align.max(1)
            );
            let mut with_result = vec!["ptr %result".to_string()];
            with_result.extend(args);
            let _ = writeln!(out, "  call void {callee}({})", with_result.join(", "));
            let _ = writeln!(out, "  %code64 = call i64 {report}(ptr %result)");
            out.push_str("  %code = trunc i64 %code64 to i32\n  ret i32 %code\n");
        } else if def.ret == Types::I64 {
            let _ = writeln!(out, "  %code64 = call i64 {callee}({})", args.join(", "));
            out.push_str("  %code = trunc i64 %code64 to i32\n  ret i32 %code\n");
        } else {
            let sig = self.signature(main);
            let _ = writeln!(
                out,
                "  call {} {callee}({})",
                sig.ret_text(),
                args.join(", ")
            );
            out.push_str("  ret i32 0\n");
        }
        self.define_c_main(&out);
    }

    /// C's `main`, whose instructions are `body`: code the compiler writes
    /// on its own, which a panic's calls pass over, every instruction at no
    /// line, as LLVM asks of a call in a function it describes.
    fn define_c_main(&mut self, body: &str) {
        let subprogram = self.debug.subprogram("main", None);
        let place = self.debug.no_line(subprogram);
        let _ = writeln!(
            self.functions,
            "define i32 @main(i32 %argc, ptr %argv) #0 !dbg !{} {{\nstart:",
            subprogram.node
        );
        for line in body.lines() {
            let _ = writeln!(self.functions, "{line}, !dbg !{place}");
        }
        self.functions.push_str("}\n\n");
    }

    /// The runner `wip test` asks for: each test in turn, its name before
    /// and `ok` after.
    fn define_test_entry(&mut self, tests: &[FnId]) {
        let plural = if tests.len() == 1 { "test" } else { "tests" };
        let mut lines = vec![format!("running {} {plural}\n", tests.len())];
        for &id in tests {
            lines.push(format!(
                "test {} ... ",
                self.program.test_name(id, self.interner)
            ));
        }
        lines.push("ok\n".to_string());
        lines.push(format!("\n{} {plural} passed\n", tests.len()));
        let print = self.runtime("wip_print_text");
        let print = self.symbol(print);
        let texts: Vec<(String, usize)> = lines
            .iter()
            .map(|line| (self.text(line), line.len()))
            .collect();
        let callees: Vec<String> = tests.iter().map(|&id| self.symbol(id)).collect();
        let mut out = String::new();
        let say = |out: &mut String, (text, len): &(String, usize)| {
            let _ = writeln!(out, "  call void {print}(ptr {text}, i64 {len})");
        };
        say(&mut out, &texts[0]);
        for (index, callee) in callees.iter().enumerate() {
            say(&mut out, &texts[index + 1]);
            let _ = writeln!(out, "  call void {callee}()");
            say(&mut out, &texts[tests.len() + 1]);
        }
        say(&mut out, &texts[tests.len() + 2]);
        out.push_str("  ret i32 0\n");
        self.define_c_main(&out);
    }
}

/// `slot`'s attributes, and `more` after them.
fn add_attr(slot: &mut Slot, more: String) {
    if more.is_empty() {
        return;
    }
    if !slot.attr.is_empty() {
        slot.attr.push(' ');
    }
    slot.attr.push_str(&more);
}

/// Whether C is on the other side of a function's calls.
pub(crate) fn uses_c_abi(def: &wip_hir::FnDef) -> bool {
    def.is_extern || def.exports_c
}

/// A global's name, quoted as LLVM's IR takes any name.
pub(crate) fn global_name(name: &str) -> String {
    global(name)
}

/// A global's name, quoted as LLVM's IR takes any name.
pub(crate) fn global(name: &str) -> String {
    let mut out = String::from("@\"");
    for &byte in name.as_bytes() {
        match byte {
            b'"' | b'\\' => {
                let _ = write!(out, "\\{byte:02X}");
            }
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\{byte:02X}");
            }
        }
    }
    out.push('"');
    out
}

/// Bytes as LLVM writes them in `c"…"`.
pub(crate) fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            b'"' | b'\\' => {
                let _ = write!(out, "\\{byte:02X}");
            }
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\{byte:02X}");
            }
        }
    }
    out
}
