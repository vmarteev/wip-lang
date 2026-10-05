//! Lowering a type-checked function body to MIR.
//!
//! The builder walks the typed IR once, in evaluation order. It keeps what
//! the old code generator kept: a stack of scopes whose cleanups run at each
//! exit, and the temporaries of the statements being lowered, which are
//! dropped where their statements end.

mod control;
mod drops;
mod expr;
mod generator;
mod numbers;
mod place;

use la_arena::ArenaMap;
use rustc_hash::FxHashMap;
use wip_hir::{
    self as hir, ExprId, ExprKind, FnDef, Intrinsic, LocalId, Program, StmtId, StmtKind,
    evaluation_order,
};
use wip_syntax::Interner;

use crate::*;

/// Lowers the body of `def`. A generator's `next` keeps its locals in the
/// generator.
pub fn lower_fn(program: &Program, interner: &Interner, def: &FnDef, body: &hir::Body) -> Body {
    let mut builder = Builder::new(program, interner, body);
    let Some(id) = def.generator else {
        builder.function(def);
        return builder.finish();
    };
    builder.generator(def);
    let mut body = builder.finish();
    crate::frame::keep_in_frame(&mut body, program.structs[id].fields.len() as u32);
    body
}

/// The frame of every generator the program has: what its `next` keeps
/// in it between calls, after its declared fields. Each
/// is the locals of the `next` of that type, so it is known once the
/// monomorphizer has made them, and before anything is laid out.
pub fn generator_frames(program: &mut Program, interner: &Interner) {
    // Each frame is its own, so the order they are worked out in does not
    // matter.
    let generators: Vec<(Ty, FnId)> = program
        .generator_next
        .iter()
        .map(|(&ty, &next)| (ty, next))
        .collect();
    let mut frames = Vec::new();
    for (ty, next) in generators {
        let def = &program.fns[next];
        let Some(body) = &def.body else { continue };
        let mut builder = Builder::new(program, interner, body);
        builder.generator(def);
        frames.push((ty, crate::frame::frame_tys(&builder.finish())));
    }
    program.frames.extend(frames);
}

/// `count` for an enum whose variants carry nothing: how many there are.
pub fn lower_count_fn(program: &Program, interner: &Interner, count: u64) -> Body {
    let empty = hir::Body::default();
    let mut builder = Builder::new(program, interner, &empty);
    builder.count_fn(count);
    builder.finish()
}

/// `fromIndex` for one: the variant with that number, or nothing.
pub fn lower_from_index_fn(program: &Program, interner: &Interner, ty: Ty, option: Ty) -> Body {
    let empty = hir::Body::default();
    let mut builder = Builder::new(program, interner, &empty);
    builder.index_to_variant_fn(ty, option);
    builder.finish()
}

/// `all` for one: every variant, in order, as an array.
pub fn lower_all_variants_fn(program: &Program, interner: &Interner, ty: Ty, array: Ty) -> Body {
    let empty = hir::Body::default();
    let mut builder = Builder::new(program, interner, &empty);
    builder.all_variants_fn(ty, array);
    builder.finish()
}

/// The drop function of an `own<ty>`: given a pointer, which may be null, it
/// drops the value the pointer points to and frees it. A chain of `own`
/// values of the same type is followed in a loop, not by recursion.
pub fn lower_drop_fn(program: &Program, interner: &Interner, ty: Ty) -> Body {
    let empty = hir::Body::default();
    let mut builder = Builder::new(program, interner, &empty);
    builder.drop_fn(ty);
    builder.finish()
}

/// The function that drops a value of `ty` where it lies: given its
/// address, it drops what the value holds and frees nothing. A buffer's
/// elements are dropped by it, one call each, so that an element holding a
/// buffer of its own type is a call, not a drop written inside itself
/// without end.
pub fn lower_drop_in_place_fn(program: &Program, interner: &Interner, ty: Ty) -> Body {
    let empty = hir::Body::default();
    let mut builder = Builder::new(program, interner, &empty);
    builder.drop_in_place_fn(ty);
    builder.finish()
}

struct Builder<'a> {
    program: &'a Program,
    interner: &'a Interner,
    hir: &'a hir::Body,
    locals: Vec<LocalDecl>,
    params: Vec<Local>,
    ret: Option<Local>,
    /// Statements, and the terminator once there is one.
    blocks: Vec<(Vec<Statement>, Option<Terminator>)>,
    current: BlockId,
    /// Control cannot reach what follows: a `return` has ended the path.
    dead: bool,
    vars: ArenaMap<LocalId, Local>,
    /// What each enclosing block does at its exits, innermost last.
    scopes: Vec<Vec<Cleanup>>,
    /// Temporaries of the statements being lowered that own memory, dropped
    /// when their statements end.
    temps: Vec<(Place, Ty)>,
    /// Whether values of a type own memory.
    owns: FxHashMap<Ty, bool>,
    /// The loops around the statement being lowered, innermost last.
    loops: Vec<LoopFrame>,
    /// The top of a `@tailrec` function's body, where its calls to itself
    /// jump back to.
    top: Option<BlockId>,
    /// For a local that may have been moved away by the time its scope ends,
    /// a `bool` that says whether it still holds a value.
    flags: ArenaMap<LocalId, Local>,
    /// The same, for a temporary a `match` or a `?` takes a payload out of:
    /// zeroing says nothing about a type that cleans up after itself, so it
    /// says so here instead.
    temp_flags: FxHashMap<Local, Local>,
    /// For a reference whose referent the body moves out — `move self` in
    /// a `var fn` — whether the referent holds a value: an assignment puts
    /// one back, and drops what was there only where there was one.
    referent_flags: FxHashMap<LocalId, Local>,
    /// For a field the body moves out of a local — `going.work.get()`, or
    /// one inside another, `all.row.entries` — whether it still holds a
    /// value, so that dropping the local drops only what is left of it.
    /// The field is its path of field indices from the local.
    field_flags: FxHashMap<(LocalId, Vec<u32>), Local>,
    /// The same for a field the body moves out of what a reference
    /// parameter refers to — `move self.dialog` in a `var fn` — so that
    /// putting one back drops what was there only where there was one.
    referent_field_flags: FxHashMap<(LocalId, u32), Local>,
    /// While a generator's `next` is lowered: where it stops and goes on.
    generator: Option<generator::Generator>,
    /// Whether moves leave poison and drops check for it.
    check_moves: bool,
    /// Whether a type holds a `destroy` of its own, which would run on the
    /// zeros a move leaves.
    destroys: FxHashMap<Ty, bool>,
    /// Where a drop being lowered is, for the check's message: the local's
    /// declaration, or the assignment that replaces a value.
    drop_at: Span,
    /// Where each scope being lowered ends, innermost last: what a
    /// variable declared now is seen until.
    seen_until: Vec<u32>,
}

/// Whether a build checks moves: a move leaves poison where it would leave
/// zeros, and each drop that would run a `destroy` on zeros panics where it
/// finds poison. `WIP_CHECK_MOVES` says so, `1` or `0`;
/// where it is not set, the build does: a debug build checks, a release
/// build does not.
fn checking_moves(program: &Program) -> bool {
    match std::env::var_os("WIP_CHECK_MOVES") {
        Some(value) => !value.is_empty() && value != "0",
        None => program.check_moves,
    }
}

/// A loop being lowered: where `break` and `continue` go, how
/// many scopes and temporaries were open when it started, and whether a
/// `break` leaves it.
#[derive(Clone, Copy)]
struct LoopFrame {
    exit: BlockId,
    next: BlockId,
    scopes: usize,
    temps: usize,
    breaks: bool,
}

/// One entry of a block's cleanups, run newest first.
#[derive(Clone, Copy)]
enum Cleanup {
    Drop(LocalId),
    /// Lower a deferred expression, whole, at this exit.
    Defer(ExprId),
}

/// A lowered expression.
enum Value {
    /// No value: `void`, or a path that has ended.
    Unit,
    /// A scalar: a constant, or a temporary that nothing changes later.
    Scalar(Operand),
    /// An aggregate, where it is.
    Place(Place),
}

impl<'a> Builder<'a> {
    fn new(program: &'a Program, interner: &'a Interner, hir: &'a hir::Body) -> Builder<'a> {
        Builder {
            program,
            interner,
            hir,
            locals: Vec::new(),
            params: Vec::new(),
            ret: None,
            blocks: vec![(Vec::new(), None)],
            current: BlockId(0),
            dead: false,
            vars: ArenaMap::default(),
            scopes: Vec::new(),
            temps: Vec::new(),
            owns: FxHashMap::default(),
            loops: Vec::new(),
            top: None,
            flags: ArenaMap::default(),
            temp_flags: FxHashMap::default(),
            referent_flags: FxHashMap::default(),
            field_flags: FxHashMap::default(),
            referent_field_flags: FxHashMap::default(),
            generator: None,
            check_moves: checking_moves(program),
            destroys: FxHashMap::default(),
            // A drop function's body has no value of its own to point at.
            drop_at: hir
                .value
                .map_or_else(Span::default, |value| hir.exprs[value].span),
            seen_until: Vec::new(),
        }
    }

    /// What a move leaves where the value was: zeros, or, in a build that
    /// checks moves, poison where zeros would let a `destroy` run on
    /// nothing.
    fn vacate(&mut self, place: Place, ty: Ty) {
        if self.check_moves && self.poisons(ty) {
            self.push(Statement::Poison(place));
        } else {
            self.push(Statement::Zero(place));
        }
    }

    /// Whether a moved-out value of this type is poisoned in a build that
    /// checks moves: it holds a `destroy` of its own, and does not say it
    /// was moved by its own zeros, as an `own` does.
    fn poisons(&mut self, ty: Ty) -> bool {
        !self.zeroes_itself(ty) && self.holds_destroy(ty)
    }

    /// Whether a value of this type holds, in place, one with a `destroy`
    /// of its own. What an `own` points to is not in place.
    fn holds_destroy(&mut self, ty: Ty) -> bool {
        if let Some(&known) = self.destroys.get(&ty) {
            return known;
        }
        // A type that holds itself is answered for while it is asked.
        self.destroys.insert(ty, false);
        let program = self.program;
        let holds = program.drop_fns.contains_key(&ty)
            || match self.kind(ty) {
                TyKind::Struct(..) => program
                    .field_tys(ty)
                    .into_iter()
                    .any(|field| self.holds_destroy(field)),
                TyKind::Enum(..) => program
                    .variant_field_tys(ty)
                    .into_iter()
                    .flatten()
                    .any(|field| self.holds_destroy(field)),
                TyKind::Array(elem, len) => len > 0 && self.holds_destroy(elem),
                _ => false,
            };
        self.destroys.insert(ty, holds);
        holds
    }

    fn finish(self) -> Body {
        Body {
            locals: self.locals,
            params: self.params,
            ret: self.ret,
            blocks: self
                .blocks
                .into_iter()
                .map(|(statements, terminator)| Block {
                    statements,
                    terminator: terminator.unwrap_or(Terminator::Unreachable),
                })
                .collect(),
        }
    }

    /// A function: its parameters, its body's value, and the cleanups of its
    /// outermost scope, which drop the parameters it owns.
    fn function(&mut self, def: &FnDef) {
        let hir = self.hir;
        // A parameter is seen by the whole body.
        self.seen_until.push(hir.exprs[hir.value()].span.hi);
        for &param in &hir.params {
            let local = self.source_local(param, LocalKind::Param);
            self.params.push(local);
            self.vars.insert(param, local);
        }
        if def.ret != Types::UNIT {
            self.ret = Some(self.new_local(def.ret, LocalKind::Return));
        }
        // A parameter holds a value from the start; a flag only says when it
        // stops.
        for &param in &hir.params.clone() {
            self.set_flag(param, true);
            if self.needs_referent_flag(param) {
                let flag = self.new_local(Types::BOOL, LocalKind::Var);
                self.referent_flags.insert(param, flag);
                self.assign(
                    Place::local(flag),
                    Rvalue::Use(Operand::Const(Const::Bool(true))),
                );
            }
            for index in self.moved_referent_fields(param) {
                let flag = self.new_local(Types::BOOL, LocalKind::Var);
                self.referent_field_flags.insert((param, index), flag);
                self.assign(
                    Place::local(flag),
                    Rvalue::Use(Operand::Const(Const::Bool(true))),
                );
            }
        }
        self.scopes
            .push(hir.params.iter().map(|&p| Cleanup::Drop(p)).collect());
        // A `@tailrec` function's calls to itself set the parameters and jump
        // here.
        if !hir.tail_calls.is_empty() {
            let top = self.new_block();
            self.terminate(Terminator::Goto(top));
            self.switch_to(top);
            self.top = Some(top);
        }
        let value = hir.value();
        // A block marks its own statements; a body that is one expression
        // is written at that expression.
        if !matches!(hir.exprs[value].kind, hir::ExprKind::Block(_)) {
            self.at(hir.exprs[value].span);
        }
        let result = match self.ret {
            Some(ret) if self.is_aggregate(def.ret) => {
                self.store_expr(value, Place::local(ret));
                Value::Unit
            }
            _ => self.expr(value),
        };
        if self.dead {
            return;
        }
        self.drop_temps(0);
        self.drop_scope();
        match (result, self.ret) {
            (Value::Scalar(operand), Some(ret)) => {
                self.assign(Place::local(ret), Rvalue::Use(operand));
                self.terminate(Terminator::Return);
            }
            _ if def.ret == Types::UNIT || self.is_aggregate(def.ret) => {
                self.terminate(Terminator::Return);
            }
            // The type checker guarantees a value on every path.
            _ => self.terminate(Terminator::Unreachable),
        }
    }

    /// Records whether a local holds a value, where it needs a flag.
    /// The flag is made on first use, false until then.
    fn set_flag(&mut self, local: LocalId, live: bool) {
        // A whole value put in holds every field again.
        if live {
            for path in self.moved_fields(local) {
                self.set_field_flag(local, &path, true);
            }
        }
        let flag = match self.flags.get(local) {
            Some(&flag) => flag,
            None => {
                if !self.needs_flag(local) {
                    return;
                }
                let flag = self.new_local(Types::BOOL, LocalKind::Var);
                self.flags.insert(local, flag);
                flag
            }
        };
        self.assign(
            Place::local(flag),
            Rvalue::Use(Operand::Const(Const::Bool(live))),
        );
    }

    /// A whole local was moved away, so its flag says it holds nothing.
    fn moved_from(&mut self, id: ExprId) {
        if let ExprKind::Local(local) = self.hir.exprs[id].kind {
            self.set_flag(local, false);
        }
        self.set_referent_flag(id, false);
        if let Some((local, path)) = self.field_of_local(id) {
            self.set_field_flag(local, &path, false);
        }
        self.set_referent_field_flag(id, false);
    }

    /// A whole local was assigned, so it holds a value again.
    fn assigned_to(&mut self, id: ExprId) {
        if let ExprKind::Local(local) = self.hir.exprs[id].kind {
            self.set_flag(local, true);
        }
        self.set_referent_flag(id, true);
        // A field put in holds it again, and every field inside it.
        if let Some((local, path)) = self.field_of_local(id) {
            for moved in self.moved_fields(local) {
                if moved.starts_with(&path) {
                    self.set_field_flag(local, &moved, true);
                }
            }
        }
        self.set_referent_field_flag(id, true);
    }

    /// The reference and the field `id` is, where it is a field of what a
    /// local reference refers to: `self.dialog` in a `var fn`.
    fn field_of_referent(&self, id: ExprId) -> Option<(LocalId, u32)> {
        match self.hir.exprs[id].kind {
            ExprKind::Field { base, index } => {
                self.referent_of(base).map(|reference| (reference, index))
            }
            _ => None,
        }
    }

    /// Records whether a field of a referent holds a value, where it has a
    /// flag.
    fn set_referent_field_flag(&mut self, id: ExprId, live: bool) {
        if let Some(field) = self.field_of_referent(id)
            && let Some(&flag) = self.referent_field_flags.get(&field)
        {
            self.assign(
                Place::local(flag),
                Rvalue::Use(Operand::Const(Const::Bool(live))),
            );
        }
    }

    /// The local and the field `id` is, where it is a field of a local or
    /// a field of one of its fields: the path of field indices from it.
    pub(super) fn field_of_local(&self, id: ExprId) -> Option<(LocalId, Vec<u32>)> {
        let mut path = Vec::new();
        let mut at = id;
        loop {
            match self.hir.exprs[at].kind {
                ExprKind::Field { base, index } => {
                    path.push(index);
                    at = base;
                }
                ExprKind::Local(local) if !path.is_empty() => {
                    path.reverse();
                    return Some((local, path));
                }
                _ => return None,
            }
        }
    }

    /// Records whether a field of a local holds a value, where the body
    /// moves it out; the flag is made on first use.
    fn set_field_flag(&mut self, local: LocalId, path: &[u32], live: bool) {
        if !self.moved_fields(local).iter().any(|moved| moved == path) {
            return;
        }
        let key = (local, path.to_vec());
        let flag = match self.field_flags.get(&key) {
            Some(&flag) => flag,
            None => {
                let flag = self.new_local(Types::BOOL, LocalKind::Var);
                self.field_flags.insert(key, flag);
                flag
            }
        };
        self.assign(
            Place::local(flag),
            Rvalue::Use(Operand::Const(Const::Bool(live))),
        );
    }

    /// The reference whose referent `id` is, where `id` is `*r` for a
    /// local `r`: `self` in a `var fn`.
    fn referent_of(&self, id: ExprId) -> Option<LocalId> {
        match self.hir.exprs[id].kind {
            ExprKind::Deref(inner) => match self.hir.exprs[inner].kind {
                ExprKind::Local(local) => Some(local),
                _ => None,
            },
            _ => None,
        }
    }

    /// Records whether `id`'s referent holds a value, where it has a flag.
    fn set_referent_flag(&mut self, id: ExprId, live: bool) {
        if let Some(reference) = self.referent_of(id)
            && let Some(&flag) = self.referent_flags.get(&reference)
        {
            self.assign(
                Place::local(flag),
                Rvalue::Use(Operand::Const(Const::Bool(live))),
            );
        }
    }

    fn kind(&self, ty: Ty) -> TyKind {
        self.program.types.kind(ty)
    }

    fn ty(&self, id: ExprId) -> Ty {
        self.hir.exprs[id].ty
    }

    fn is_aggregate(&self, ty: Ty) -> bool {
        is_aggregate(self.program, ty)
    }

    fn is_scalar(&self, ty: Ty) -> bool {
        is_scalar(self.program, ty)
    }

    /// Where an expression was written, for a check's message.
    fn span(&self, id: ExprId) -> Span {
        self.hir.exprs[id].span
    }

    /// Whether a function of this result does not return.
    fn ty_is_never(&self, ty: Ty) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Never)
    }

    fn needs_drop(&mut self, ty: Ty) -> bool {
        let program = self.program;
        *self
            .owns
            .entry(ty)
            .or_insert_with(|| program.owns_memory(ty))
    }

    fn new_local(&mut self, ty: Ty, kind: LocalKind) -> Local {
        self.locals.push(LocalDecl::unnamed(ty, kind));
        Local(self.locals.len() as u32 - 1)
    }

    /// The local of a variable or parameter of the source, named as it is
    /// there, and seen until the end of the scope being lowered.
    /// One the compiler named has no name a debugger
    /// shows.
    fn source_local(&mut self, id: LocalId, kind: LocalKind) -> Local {
        let hir = &self.hir.locals[id];
        if self.interner.is_hidden(hir.name) {
            return self.new_local(hir.ty, kind);
        }
        let source = Source {
            name: hir.name,
            at: hir.span,
            seen_until: self.seen_until.last().copied().unwrap_or(hir.span.hi),
            by_address: self.hir.alias_bindings.contains(&id),
        };
        self.locals.push(LocalDecl {
            ty: hir.ty,
            kind,
            source: Some(source),
        });
        Local(self.locals.len() as u32 - 1)
    }

    fn temp(&mut self, ty: Ty) -> Local {
        self.new_local(ty, LocalKind::Temp)
    }

    fn new_block(&mut self) -> BlockId {
        self.blocks.push((Vec::new(), None));
        BlockId(self.blocks.len() as u32 - 1)
    }

    fn switch_to(&mut self, block: BlockId) {
        self.current = block;
    }

    fn push(&mut self, statement: Statement) {
        let (statements, terminator) = &mut self.blocks[self.current.0 as usize];
        assert!(
            terminator.is_none(),
            "a statement after the end of block {}",
            self.current.0
        );
        statements.push(statement);
    }

    fn terminate(&mut self, terminator: Terminator) {
        let (_, slot) = &mut self.blocks[self.current.0 as usize];
        assert!(
            slot.is_none(),
            "block {} is terminated twice",
            self.current.0
        );
        *slot = Some(terminator);
    }

    fn assign(&mut self, place: Place, rvalue: Rvalue) {
        self.push(Statement::Assign(place, rvalue));
    }

    /// A new temporary holding `rvalue`, as an operand.
    fn value(&mut self, ty: Ty, rvalue: Rvalue) -> Operand {
        let temp = Place::local(self.temp(ty));
        self.assign(temp.clone(), rvalue);
        Operand::Copy(temp)
    }

    /// The local an operand is in, putting a constant in a temporary first.
    fn local_of(&mut self, operand: Operand, ty: Ty) -> Local {
        match operand {
            Operand::Copy(Place { local, projections }) if projections.is_empty() => local,
            operand => {
                let temp = self.temp(ty);
                self.assign(Place::local(temp), Rvalue::Use(operand));
                temp
            }
        }
    }

    fn int(bits: u128, ty: Ty) -> Operand {
        Operand::Const(Const::Int { bits, ty })
    }

    /// Starts a block that nothing reaches, if the path has ended.
    fn revive(&mut self) {
        if self.dead {
            let block = self.new_block();
            self.switch_to(block);
            self.dead = false;
        }
    }
}
