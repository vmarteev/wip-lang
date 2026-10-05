//! Move checking: the static half of destructive moves.
//!
//! At every program point the checker knows which *places* have been moved
//! out of: a local, or a path of fields into it (`car.engine`), looking
//! through `own` and `&var` parameters but not through `&` parameters or
//! array elements. Using a place that overlaps a moved one is an error;
//! assigning to a place makes it whole again. Branches are merged, so a place
//! moved on only some paths is "possibly moved", and each loop is iterated to
//! a fixed point so that a move late in its body is seen by uses early in the
//! next iteration.
//!
//! The rules:
//! - A value whose type contains `own` is never copied implicitly. Using such
//!   a place as a value needs `move` (E0404).
//! - After `move p`, `p` and everything inside it hold no value until assigned
//!   again (E0401, E0402, E0403). Moving a field leaves the other fields
//!   usable, but not the struct as a whole.
//! - Nothing is moved out of a `&` reference (E0405), an array element
//!   (E0406) or a temporary (E0409), and a value containing `own` cannot be
//!   repeated with `[x; n]` (E0407). A `&var` parameter can be moved out of,
//!   but must hold a value again at every exit (E0415).
//! - In one call, a `&var` argument cannot overlap another reference
//!   argument (E0413), and a later argument cannot move or assign what an
//!   earlier one borrowed (E0414).
//!
//! The code generator zeroes every place it moves out of, and drops skip null
//! pointers, so a place the checker accepts is never freed twice.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use rustc_hash::{FxHashMap, FxHashSet};
use wip_hir::{
    BinaryOp, Binder, Block, Body, ExprId, ExprKind, FnDef, FnId, LocalId, LocalKind, Pattern,
    Program, RefKind, SliceRest, StmtId, StmtKind, Ty, TyKind, Types, evaluation_order,
};
use wip_syntax::{Code, Diagnostic, Edit, Interner, Span, parallel};

mod borrowed;
mod expr;
mod heap;
mod lends;
mod moved;
mod references;
mod state;
mod stmt;

use wip_syntax::codes;

/// Checks every function body, on one thread per core
/// ([`parallel::threads`]).
pub fn check(program: &Program, interner: &Interner) -> Vec<Diagnostic> {
    check_with_threads(program, interner, parallel::threads())
}

/// Checks every function body on up to `threads` threads. Each body is
/// checked on its own, so the result is the same for any number of threads.
pub fn check_with_threads(
    program: &Program,
    interner: &Interner,
    threads: usize,
) -> Vec<Diagnostic> {
    let bodies: Vec<(FnId, &FnDef, &Body)> = program
        .fns
        .iter()
        .filter_map(|(id, def)| Some((id, def, def.body.as_ref()?)))
        .collect();
    let implemented = implemented_methods(program);
    let weights: Vec<usize> = bodies.iter().map(|(_, _, body)| body.exprs.len()).collect();
    let found = parallel::run(parallel::split(&weights, threads), |range| {
        bodies[range]
            .iter()
            .flat_map(|&(id, def, body)| {
                check_body(program, interner, def, body, implemented.get(&id).copied())
            })
            .collect::<Vec<_>>()
    });
    let mut diagnostics: Vec<Diagnostic> = found.into_iter().flatten().collect();
    diagnostics.sort_by_key(|d| d.primary.span.lo);
    diagnostics
}

/// Each method of an implementation, and the interface's method it
/// implements.
fn implemented_methods(program: &Program) -> FxHashMap<FnId, FnId> {
    let mut implemented = FxHashMap::default();
    for imp in &program.impls {
        let interface = &program.interfaces[imp.interface];
        for &method in &imp.methods {
            let name = program.fns[method].name;
            if let Some(declared) = interface
                .methods
                .iter()
                .find(|m| program.fns[m.id].name == name)
            {
                implemented.insert(method, declared.id);
            }
        }
    }
    implemented
}

/// Checks one body. `implements` is the interface's method it implements,
/// whose signature its result is held to as well as its own.
fn check_body(
    program: &Program,
    interner: &Interner,
    def: &FnDef,
    body: &Body,
    implements: Option<FnId>,
) -> Vec<Diagnostic> {
    let mut lends = lends::signature_lends(program, def);
    if let Some(declared) = implements {
        let declared = lends::signature_lends(program, &program.fns[declared]);
        for (own, declared) in lends.iter_mut().zip(declared) {
            own.place &= declared.place;
            own.borrows &= declared.borrows;
        }
    }
    let mut checker = Checker {
        program,
        interner,
        body,
        diagnostics: Vec::new(),
        quiet: 0,
        loops: Vec::new(),
        reported: FxHashSet::default(),
        stale_reported: FxHashSet::default(),
        deferred: Vec::new(),
        exit: None,
        seen: FxHashSet::default(),
        borrows: Vec::new(),
        var_params: body
            .params
            .iter()
            .copied()
            .filter(|&p| {
                matches!(
                    program.types.kind(body.locals[p].ty),
                    TyKind::Ref(_, RefKind::Var)
                )
            })
            .collect(),
        fn_name: interner.resolve(def.name).to_string(),
        generator: def.generator.is_some(),
        aliases: FxHashMap::default(),
        scopes: Vec::new(),
        loop_exits: Vec::new(),
        lends,
        implements,
        kept_bindings: FxHashSet::default(),
        callee_lends: RefCell::new(FxHashMap::default()),
    };
    let mut state = State::default();
    let value = body.value();
    checker.expr(value, Ctx::Value, &mut state);
    if body.exprs[value].ty != Types::NEVER && checker.is_view(def.ret) {
        checker.check_leaving(body_result(body, value), borrowed::Leaving::Result, &state);
    }
    if body.exprs[value].ty != Types::NEVER {
        let span = body.exprs[value].span;
        let end = Span::new(span.hi.saturating_sub(1), span.hi);
        checker.check_var_params(end, &state);
    }
    checker.diagnostics
}

/// The expression that answers a function's result: the value of its
/// block, where it is one.
fn body_result(body: &Body, value: ExprId) -> ExprId {
    match &body.exprs[value].kind {
        ExprKind::Block(block) => block.value.unwrap_or(value),
        _ => value,
    }
}

// ---- places and move state ----

/// Every name a pattern binds, with the place it refers into at whatever
/// depth: a binder may be a pattern of its own.
fn bindings_of(pattern: &Pattern, base: Option<Path>, out: &mut Vec<(LocalId, Option<Path>)>) {
    let deeper = |base: &Option<Path>, proj: Proj| {
        base.clone().map(|mut path| {
            path.projs.push(proj);
            path
        })
    };
    match pattern {
        Pattern::Binding(local) => out.push((*local, base)),
        Pattern::Variant { binders, .. } => {
            for (i, binder) in binders.iter().enumerate() {
                binder_bindings(binder, deeper(&base, Proj::Payload(i as u32)), out);
            }
        }
        Pattern::Fields(binders) => {
            for (i, binder) in binders.iter().enumerate() {
                binder_bindings(binder, deeper(&base, Proj::Field(i as u32)), out);
            }
        }
        // An element, and the slice of those between, refer to the
        // elements.
        Pattern::Slice {
            prefix,
            rest,
            suffix,
        } => {
            for binder in prefix.iter().chain(suffix) {
                binder_bindings(binder, deeper(&base, Proj::Elements), out);
            }
            if let Some(SliceRest::Bind(local)) = rest {
                out.push((*local, deeper(&base, Proj::Elements)));
            }
        }
        // A value the scrutinee must equal binds nothing.
        Pattern::Wildcard
        | Pattern::Error
        | Pattern::Int(_)
        | Pattern::Range { .. }
        | Pattern::Bool(_)
        | Pattern::Str(_) => {}
        Pattern::Any(alternatives) => {
            for alternative in alternatives {
                bindings_of(alternative, base.clone(), out);
            }
        }
    }
}

fn binder_bindings(binder: &Binder, base: Option<Path>, out: &mut Vec<(LocalId, Option<Path>)>) {
    match binder {
        Binder::Ignored => {}
        Binder::Bind(local) => out.push((*local, base)),
        Binder::Nested(pattern) => bindings_of(pattern, base, out),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Proj {
    /// Through an `own`, or a reference parameter.
    Deref,
    Field(u32),
    /// A field of the variant that a `match` arm matched. Only the arm's
    /// bindings reach it.
    Payload(u32),
    /// The element a `for` binding refers to. Only the binding reaches it.
    Element,
    /// The elements a slice pattern's bindings refer to: one, or the
    /// slice of those between. Only the bindings reach them.
    Elements,
}

/// A tracked place: a local and a path of projections into it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Path {
    local: wip_hir::LocalId,
    projs: Vec<Proj>,
}

#[derive(Clone, PartialEq, Debug)]
struct Moved {
    /// Moved on every path to this point, rather than only on some.
    definite: bool,
    /// Where it was moved, sorted.
    spans: Vec<Span>,
    /// The place as written, for messages: `car.engine`.
    name: String,
    /// Moved by a deferred expression, where its block exited.
    by_defer: bool,
}

/// Every place moved out of at a program point. Places not listed, and not
/// inside a listed one, hold values.
#[derive(Clone, PartialEq, Default, Debug)]
struct State {
    moved: BTreeMap<Path, Moved>,
    /// What each `str` variable borrows.
    roots: BTreeMap<LocalId, std::collections::BTreeSet<Path>>,
    /// The `str` variables whose roots changed since they were given their
    /// value.
    stale: BTreeMap<LocalId, borrowed::Stale>,
}

/// What a place expression refers to, as far as ownership is concerned.
enum Place {
    Path(Path),
    /// Behind a `&` reference, or a `match` binding of either kind, which
    /// cannot be moved out of; `reference` is the expression of reference
    /// type.
    ThroughRef {
        reference: ExprId,
    },
    /// Inside an array element.
    Element {
        array: ExprId,
        index: ExprId,
    },
    /// Inside a temporary, such as the result of a call.
    Temporary {
        base: ExprId,
    },
}

/// How an expression is used.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// Its value is taken: read if it is plain data, which needs `move` if it
    /// owns memory.
    Value,
    /// Looked at without taking ownership: borrowed, or the base of a field
    /// access or index, or matched on.
    Inspect,
    /// `move place`; the span is the whole `move` expression.
    Move(Span),
    /// The target of `place = value`.
    Assign,
}

struct Checker<'a> {
    program: &'a Program,
    interner: &'a Interner,
    body: &'a Body,
    diagnostics: Vec<Diagnostic>,
    /// Nonzero while a loop is being iterated to its fixed point; diagnostics
    /// are reported only in the final pass over it.
    quiet: u32,
    /// Bodies of the loops around the current point, innermost last.
    loops: Vec<Span>,
    /// Moves already reported, so that one mistake produces one error.
    reported: FxHashSet<Span>,
    /// Stale `str`s already reported, by the change that made them stale.
    stale_reported: FxHashSet<(LocalId, Span)>,
    /// For each enclosing block, innermost last: the expressions of its
    /// `defer`s so far, which run where the block exits.
    deferred: Vec<Vec<ExprId>>,
    /// While deferred expressions are checked as they run: the exit that
    /// runs them, and a label for it.
    exit: Option<(Span, &'static str)>,
    /// Diagnostics already reported: a deferred expression is checked at
    /// every exit that runs it, but each mistake is reported once.
    seen: FxHashSet<(Code, Span)>,
    /// The reference arguments of the calls whose arguments are being
    /// evaluated, outermost first. Each must stay valid until its call.
    borrows: Vec<Borrow>,
    /// The `&var` parameters, which must hold values at every exit.
    var_params: Vec<LocalId>,
    /// The function's name, for messages.
    fn_name: String,
    /// Whether the body is a generator's `next`, which stops at each
    /// `yield` and may be moved before it goes on.
    generator: bool,
    /// Alias bindings of `match`es on places, and the place each refers into.
    aliases: FxHashMap<LocalId, Path>,
    /// The loops around the current point, innermost last.
    loop_exits: Vec<LoopExits>,
    /// The locals declared in each enclosing block, innermost last: what a
    /// `@tailrec` function's jump would leave undropped.
    scopes: Vec<Vec<LocalId>>,
    /// What each parameter may lend the function's result, by its
    /// signature and the interface's it implements.
    lends: Vec<lends::Lends>,
    /// The interface's method this body implements, for messages.
    implements: Option<FnId>,
    /// The alias bindings of a place behind a kept `&` reference, which are
    /// kept references themselves.
    kept_bindings: FxHashSet<LocalId>,
    /// What each parameter of a function called here lends its result,
    /// worked out once per function.
    callee_lends: RefCell<FxHashMap<FnId, Rc<[lends::Lends]>>>,
}

/// A place that must stay as it is for a while: borrowed by a reference
/// argument of a call whose arguments are being evaluated, or referred to by
/// bindings.
struct Borrow {
    path: Path,
    span: Span,
    var: bool,
    holder: Holder,
    /// What is borrowed lies on the heap, where the place's own `own` or
    /// block of slots points: a loop over a `Vec` borrows its elements, not
    /// the `Vec`. A generator that holds the place may be moved, and the
    /// elements stay where they are.
    heap: bool,
}

/// What holds a [`Borrow`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holder {
    /// A reference argument, until the call starts.
    Argument,
    /// The bindings of a `match` arm.
    Match,
    /// The bindings of an `is` test, for its block.
    Test,
    /// The bindings of a guard, for the rest of the block.
    Guard,
    /// The binding of a `for` loop.
    Loop,
    /// The target of an assignment, found before its value is computed.
    /// With `replace`, for `=` on a variable or a field of
    /// one, the value may take or replace the target itself, but not a place
    /// that contains it. `compound` is for `op=`.
    Assign {
        target: ExprId,
        replace: bool,
        compound: bool,
    },
}

/// The states with which the `break`s and `continue`s of a loop leave its
/// body.
struct LoopExits {
    /// How many blocks were open when the loop started: a jump runs the
    /// `defer`s of the blocks inside it.
    deferred: usize,
    breaks: Option<State>,
    continues: Option<State>,
}

impl Checker<'_> {
    fn report(&mut self, diagnostic: Diagnostic) {
        if self.quiet == 0 && self.seen.insert((diagnostic.code, diagnostic.primary.span)) {
            self.diagnostics.push(diagnostic);
        }
    }

    fn ty(&self, id: ExprId) -> Ty {
        self.body.exprs[id].ty
    }

    fn owns(&self, ty: Ty) -> bool {
        self.program.owns_memory(ty)
    }

    fn ty_name(&self, ty: Ty) -> String {
        self.program.ty_name(ty, self.interner)
    }

    /// The expression as the programmer wrote it, for messages: `car.engine`.
    fn display(&self, id: ExprId) -> String {
        match &self.body.exprs[id].kind {
            ExprKind::Local(local) => self
                .interner
                .resolve(self.body.locals[*local].name)
                .to_string(),
            ExprKind::Field { base, index } => {
                let TyKind::Struct(s, _) = self.program.types.kind(self.ty(*base)) else {
                    return self.display(*base);
                };
                let def = &self.program.structs[s];
                let field = def.fields[*index as usize].name;
                // A capture is written as the name the lambda used, not as a
                // field of the environment the compiler built for it.
                if def.is_env {
                    return self.interner.resolve(field).to_string();
                }
                format!("{}.{}", self.display(*base), self.interner.resolve(field))
            }
            ExprKind::Deref(inner) | ExprKind::Ref(inner) | ExprKind::Move(inner) => {
                self.display(*inner)
            }
            ExprKind::Index { base, .. } | ExprKind::SubSlice { base, .. } => {
                format!("{}[…]", self.display(*base))
            }
            ExprKind::Call { callee, .. } => {
                let name = self.interner.resolve(self.program.fns[*callee].name);
                format!("{name}(…)")
            }
            ExprKind::Table(id) => self
                .interner
                .resolve(self.program.consts[*id].name)
                .to_string(),
            _ => "this value".to_string(),
        }
    }

    /// Whether the place is a field of a closure's environment: what it
    /// captured.
    fn is_capture(&self, id: ExprId) -> bool {
        let ExprKind::Field { base, .. } = self.body.exprs[id].kind else {
            return false;
        };
        matches!(self.program.types.kind(self.ty(base)), TyKind::Struct(s, _)
            if self.program.structs[s].is_env)
    }

    /// Classifies a place expression; `None` for anything else.
    fn place(&self, id: ExprId) -> Option<Place> {
        match self.body.exprs[id].kind {
            ExprKind::Local(local) => Some(Place::Path(Path {
                local,
                projs: Vec::new(),
            })),
            ExprKind::Field { base, index } => Some(match self.place(base) {
                Some(Place::Path(mut path)) => {
                    path.projs.push(Proj::Field(index));
                    Place::Path(path)
                }
                Some(other) => other,
                None => Place::Temporary { base },
            }),
            // Moves through `&var` are tracked like moves through `own`: the
            // parameter must hold a value again at every exit (E0415).
            ExprKind::Deref(inner) => Some(match self.program.types.kind(self.ty(inner)) {
                TyKind::Ref(_, RefKind::Shared) => Place::ThroughRef { reference: inner },
                // A writable binding cannot be moved out of either: nothing
                // checks that it holds a value again when its arm ends, as
                // E0415 does for a `&var` parameter.
                TyKind::Ref(_, RefKind::Var)
                    if matches!(
                        self.body.exprs[inner].kind,
                        ExprKind::Local(l) if self.body.locals[l].kind == LocalKind::Binding
                    ) =>
                {
                    Place::ThroughRef { reference: inner }
                }
                // What a projection lends cannot be moved out of either: it
                // belongs to the caller.
                TyKind::Ref(_, RefKind::Var)
                    if matches!(self.body.exprs[inner].kind, ExprKind::Call { .. }) =>
                {
                    Place::ThroughRef { reference: inner }
                }
                _ => match self.place(inner) {
                    Some(Place::Path(mut path)) => {
                        path.projs.push(Proj::Deref);
                        Place::Path(path)
                    }
                    Some(other) => other,
                    None => Place::Temporary { base: inner },
                },
            }),
            ExprKind::Index { base, index } => Some(Place::Element { array: base, index }),
            _ => None,
        }
    }
}
