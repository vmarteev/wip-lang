//! What a `str` borrows.
//!
//! A `str` is a pointer and a length into bytes it does not own. Each value
//! of one has *roots*: the places whose bytes it may point into. A literal
//! has none, and neither has a `str` parameter, which is valid for the whole
//! call. A call that answers a `str` borrows from the arguments whose types
//! can hold what it refers to, by the callee's signature
//! (`lends.rs`), so its roots follow from its arguments alone.
//!
//! Changing a root — assigning it, moving it, passing it as `&var`, or
//! dropping it where its block ends — makes every `str` that borrows it
//! stale, and using a stale `str` is an error (E0436). Nothing is reported
//! for a `str` that is not used again, which is what freezing a place until
//! the last use of what borrows it means. A `str` that would outlive its
//! roots — as a result, as the value of the block that declares them, kept
//! from a temporary, or given to a `&var` parameter — is refused where it
//! would leave (E0437).

use std::collections::BTreeSet;

use super::lends::{Lends, lends_to, signature_lends};
use super::*;

/// What a `str` value borrows.
#[derive(Default)]
pub(super) struct Roots {
    pub(super) paths: BTreeSet<Path>,
    /// A temporary it borrows, which is dropped at the end of its
    /// statement.
    pub(super) temporary: Option<Span>,
}

impl Roots {
    fn extend(&mut self, other: Roots) {
        self.paths.extend(other.paths);
        self.temporary = self.temporary.or(other.temporary);
    }
}

/// How a kind of view borrows, said in messages.
struct Borrowing {
    /// Why it must not be used after what it borrows changed (E0436).
    stale: &'static str,
    /// What it may borrow where it leaves a function (E0437).
    leaving: &'static str,
    /// What to keep instead.
    instead: &'static str,
}

/// A `str` whose roots changed after it was made.
#[derive(Clone, PartialEq, Debug)]
pub(super) struct Stale {
    /// Changed on every path to this point, rather than only on some.
    pub(super) definite: bool,
    /// Where the root was changed, and how: `assigned`, `moved`, `passed
    /// as `&var`` or `dropped`.
    pub(super) change: Span,
    pub(super) how: &'static str,
    /// The root, as written: `owned`.
    pub(super) root: String,
}

/// How a generator's local changes at a `yield`: it may be moved with the
/// generator before the generator goes on.
const ACROSS_YIELD: &str = "is held across a `yield`";

/// Where a `str` goes that outlives the function or block it was made in.
#[derive(Clone, Copy)]
pub(super) enum Leaving {
    /// Returned from the function.
    Result,
    /// Handed over by a generator's `yield`.
    Yield,
    /// Assigned through a `&var` parameter, to the caller's variable.
    Param(LocalId),
}

impl Checker<'_> {
    /// Whether a value of `ty` borrows: a `str`, a view, or what holds one.
    /// A reference is a parameter's, and is tracked as a place instead.
    pub(super) fn is_view(&self, ty: Ty) -> bool {
        self.program.holds_view(ty) && !matches!(self.program.types.kind(ty), TyKind::Ref(..))
    }

    /// Whether a value of `ty` borrows as a view does: a view, or a `&`
    /// reference kept as one. A lent closure is not one; it
    /// borrows what it captured.
    pub(super) fn borrows_as_view(&self, ty: Ty) -> bool {
        self.is_view(ty) || self.is_shared_ref(ty)
    }

    /// Whether `ty` is a `&` reference to a value, not to a closure.
    fn is_shared_ref(&self, ty: Ty) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Ref(inner, RefKind::Shared)
            if !matches!(self.program.types.kind(inner), TyKind::Fn(..)))
    }

    /// Whether `local` holds a `&` reference of its own, kept as a view: a
    /// `val` or `var` declared as one, or a binding of a reference that is
    /// not an alias for part of a place. It borrows what the
    /// reference points into, as a `str` variable borrows its bytes, and a
    /// parameter or an alias does not: they stand for the place itself.
    pub(super) fn is_kept_ref(&self, local: LocalId) -> bool {
        let local_def = &self.body.locals[local];
        self.is_shared_ref(local_def.ty)
            && match local_def.kind {
                LocalKind::Param => false,
                LocalKind::Binding => {
                    !self.body.alias_bindings.contains(&local)
                        || self.kept_bindings.contains(&local)
                }
                LocalKind::Let { .. } | LocalKind::Var => true,
            }
    }

    /// The reference `place` lies behind, where that reference is kept — in
    /// a local, a view's field, an element, or the place an alias refers
    /// to — rather than being a parameter or an alias itself: the outermost
    /// such. What `&place` borrows is what that reference borrows, and a
    /// use of the place is a use of what holds it.
    pub(super) fn kept_reference_behind(&self, place: ExprId) -> Option<ExprId> {
        let mut e = place;
        loop {
            match self.body.exprs[e].kind {
                ExprKind::Field { base, .. }
                | ExprKind::Index { base, .. }
                | ExprKind::SubSlice { base, .. } => e = base,
                ExprKind::Deref(inner) => {
                    let kept = self.is_shared_ref(self.ty(inner))
                        && match self.body.exprs[inner].kind {
                            ExprKind::Local(local) => self.is_kept_ref(local),
                            ExprKind::Field { .. }
                            | ExprKind::Index { .. }
                            | ExprKind::Deref(_) => true,
                            _ => false,
                        };
                    if kept {
                        return Some(inner);
                    }
                    e = inner;
                }
                // What a projection lends lies in the argument it lends
                // from, `names[0]` in `names`; and `&place` is the place.
                ExprKind::Call {
                    callee, ref args, ..
                } => {
                    let wip_hir::Lent::Param(base) = self.program.fns[callee].projects? else {
                        return None;
                    };
                    e = *args.get(base as usize)?;
                }
                ExprKind::Ref(inner) => e = inner,
                _ => return None,
            }
        }
    }

    /// Whether a value of `ty` is a closure that reads what it captured,
    /// `&(…) => R`, which a `val` may keep: it borrows what it captured as
    /// a view borrows.
    pub(super) fn is_kept_closure(&self, ty: Ty) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Ref(inner, wip_hir::RefKind::Shared)
            if matches!(self.program.types.kind(inner), TyKind::Fn(..)))
    }

    /// `local`, a closure kept in a `val`, borrows what its value captured.
    pub(super) fn bind_closure(&mut self, local: LocalId, value: ExprId, state: &mut State) {
        let mut roots = self.lent_roots(value, Lends::ALL, state).paths;
        // What it captured may borrow in turn: another kept closure, or a
        // `str`. It reaches that too.
        let reached: Vec<Path> = roots
            .iter()
            .filter(|path| path.projs.is_empty())
            .filter_map(|path| state.roots.get(&path.local))
            .flat_map(|paths| paths.iter().cloned())
            .collect();
        roots.extend(reached);
        state.roots.insert(local, roots);
        state.stale.remove(&local);
    }

    /// What each parameter of `callee` lends its result.
    fn call_lends(&self, callee: FnId) -> Rc<[Lends]> {
        if let Some(lends) = self.callee_lends.borrow().get(&callee) {
            return lends.clone();
        }
        let lends: Rc<[Lends]> = signature_lends(self.program, &self.program.fns[callee]).into();
        self.callee_lends.borrow_mut().insert(callee, lends.clone());
        lends
    }

    /// What the arguments of a call of `callee` lend its result, each as
    /// its parameter says: every one lends all it can where the callee is a
    /// function value, or where there are more arguments than parameters.
    fn call_roots(&self, callee: Option<FnId>, args: &[ExprId], state: &State) -> Roots {
        let lends = callee.map(|callee| self.call_lends(callee));
        let mut roots = Roots::default();
        for (i, &arg) in args.iter().enumerate() {
            let lends = lends
                .as_ref()
                .and_then(|lends| lends.get(i).copied())
                .unwrap_or(Lends::ALL);
            let lends = self.arg_lends(callee, i, arg, lends);
            roots.extend(self.lent_roots(arg, lends, state));
        }
        roots
    }

    /// What argument `i` of a call of `callee` lends, as its parameter's
    /// `lends` say. A reference passed where the parameter is not one — a
    /// type parameter given `&Node` — is the value the parameter holds, and
    /// what that value borrows is the place it points to.
    fn arg_lends(&self, callee: Option<FnId>, i: usize, arg: ExprId, lends: Lends) -> Lends {
        let declared = callee.and_then(|callee| self.program.fns[callee].params.get(i));
        let by_value = declared
            .is_some_and(|param| !matches!(self.program.types.kind(param.ty), TyKind::Ref(..)));
        if lends.borrows && by_value && self.is_shared_ref(self.ty(arg)) {
            Lends::ALL
        } else {
            lends
        }
    }

    /// The roots of the value of `id`, an expression of type `str`, in
    /// `state`, after it has been checked.
    pub(super) fn str_roots(&self, id: ExprId, state: &State) -> Roots {
        let body = self.body;
        let mut roots = Roots::default();
        match &body.exprs[id].kind {
            // A variable, an element of one, or a `for` binding that refers
            // to an element: what the variable borrows.
            ExprKind::Local(_)
            | ExprKind::Index { .. }
            | ExprKind::Deref(_)
            | ExprKind::Field { .. } => {
                if let Some(path) = self.access_path(id)
                    && let Some(paths) = state.roots.get(&path.local)
                {
                    roots.paths.extend(paths.iter().cloned());
                }
            }
            // A view borrows what its fields do.
            ExprKind::Struct { fields: parts, .. } | ExprKind::Variant { args: parts, .. } => {
                for &part in parts {
                    roots.extend(self.lent_roots(part, Lends::ALL, state));
                }
            }
            ExprKind::Array(elems) => {
                for &elem in elems {
                    roots.extend(self.str_roots(elem, state));
                }
            }
            ExprKind::ArrayRepeat { elem, .. } => roots = self.str_roots(*elem, state),
            // `own view` borrows what the view does.
            ExprKind::Own(inner) => roots = self.str_roots(*inner, state),
            // A value moved on borrows what it did where it was: `f(move
            // words)` answers what `words` borrows.
            ExprKind::Move(inner) => roots = self.str_roots(*inner, state),
            ExprKind::SubSlice { base, .. } => roots = self.str_roots(*base, state),
            // `&place`: the place, or what the reference it lies behind
            // borrows.
            ExprKind::Ref(_) | ExprKind::LendClosure(_) => {
                roots = self.lent_roots(id, Lends::ALL, state);
            }
            ExprKind::Call { callee, args, .. } => {
                roots = self.call_roots(Some(*callee), args, state)
            }
            ExprKind::DynCall {
                interface,
                index,
                args,
                ..
            } => {
                let method = self.program.interfaces[*interface].methods[*index as usize].id;
                roots = self.call_roots(Some(method), args, state);
            }
            // A function value or closure may answer what it captured, so
            // the callee is borrowed as well.
            ExprKind::CallValue { callee, args } | ExprKind::CallClosure { callee, args, .. } => {
                roots = self.call_roots(None, args, state);
                if let Some(path) = self.access_path(*callee) {
                    roots.paths.insert(path);
                }
            }
            ExprKind::Block(block) => roots = self.block_roots(block, state),
            ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                roots = self.block_roots(then_block, state);
                if let Some(else_block) = else_block {
                    roots.extend(self.block_roots(else_block, state));
                }
            }
            ExprKind::Match { arms, .. } => {
                for arm in arms {
                    if self.ty(arm.body) != Types::NEVER {
                        roots.extend(self.str_roots(arm.body, state));
                    }
                }
            }
            // A literal's bytes live forever, and C's are the program's to
            // keep. A `str` reached through a parameter is valid for the whole
            // call.
            _ => {}
        }
        roots
    }

    fn block_roots(&self, block: &Block, state: &State) -> Roots {
        match block.value {
            Some(value) if self.ty(value) != Types::NEVER => self.str_roots(value, state),
            _ => Roots::default(),
        }
    }

    /// What an argument lends to what the call answers, as `lends` says:
    /// the place a reference refers to, what the argument
    /// borrows — a `str`'s roots, a view's — or both.
    pub(super) fn lent_roots(&self, arg: ExprId, lends: Lends, state: &State) -> Roots {
        let body = self.body;
        let ty = self.ty(arg);
        if self.is_view(ty) {
            return if lends.borrows {
                self.str_roots(arg, state)
            } else {
                Roots::default()
            };
        }
        if !matches!(self.program.types.kind(ty), TyKind::Ref(..)) || lends == Lends::default() {
            return Roots::default();
        }
        let mut roots = Roots::default();
        let mut e = arg;
        // The conversions a reference passes through keep what it refers to.
        while let ExprKind::Unsize(inner) | ExprKind::DynRef { value: inner, .. } =
            body.exprs[e].kind
        {
            e = inner;
        }
        match &body.exprs[e].kind {
            // A lent closure refers to what it captured.
            ExprKind::Closure { env, .. } => {
                if let ExprKind::Struct { fields, .. } = &body.exprs[*env].kind {
                    for &field in fields {
                        if let Some(path) = self.access_path(field) {
                            roots.paths.insert(path);
                        }
                    }
                }
            }
            ExprKind::Ref(inner) | ExprKind::LendClosure(inner) => {
                // `&lexer` lends what the view borrows; `&"text"` lends a
                // literal, whose bytes live forever.
                if lends.borrows && self.is_view(self.ty(*inner)) {
                    roots.extend(self.str_roots(*inner, state));
                }
                // A place behind a kept reference lies in what that
                // reference borrows, and not in what holds it.
                if lends.place
                    && let Some(reference) = self.kept_reference_behind(*inner)
                {
                    roots.extend(self.str_roots(reference, state));
                } else if lends.place {
                    match self.access_path(*inner) {
                        Some(path) => roots.extend(self.place_roots(path, state)),
                        None if matches!(
                            self.place(*inner),
                            None | Some(Place::Temporary { .. })
                        ) =>
                        {
                            roots.temporary = Some(body.exprs[arg].span);
                        }
                        None => {}
                    }
                }
            }
            // A reference kept in a variable points where its roots are.
            ExprKind::Local(local) if self.is_kept_ref(*local) => {
                roots = self.str_roots(e, state);
            }
            // A reference parameter, or a binding that refers into a place:
            // the place, and what it borrows.
            ExprKind::Local(_) => {
                if lends.borrows {
                    roots.extend(self.str_roots(e, state));
                }
                if lends.place
                    && let Some(path) = self.access_path(e)
                {
                    roots.extend(self.place_roots(path, state));
                }
            }
            // A reference read from a view's field: what the view borrows,
            // which is where the reference points.
            _ => roots = self.str_roots(e, state),
        }
        roots
    }

    /// What a place borrows, as a root: the place itself, or, where it lies
    /// behind a reference kept in a variable — an element of what a
    /// `&Vec<T>` variable points to — what that reference borrows.
    fn place_roots(&self, path: Path, state: &State) -> Roots {
        let mut roots = Roots::default();
        if !path.projs.is_empty() && self.is_kept_ref(path.local) {
            if let Some(paths) = state.roots.get(&path.local) {
                roots.paths.extend(paths.iter().cloned());
            }
            return roots;
        }
        roots.paths.insert(path);
        roots
    }

    /// What a value is, for messages: "a `str`", or its type.
    fn a_view(&self, value: ExprId) -> String {
        let ty = self.ty(value);
        if ty == Types::STR {
            "a `str`".to_string()
        } else {
            let name = self.ty_name(ty);
            let article = match name.chars().next() {
                Some('A' | 'E' | 'I' | 'O' | 'U' | 'a' | 'e' | 'i' | 'o' | 'u') => "an",
                _ => "a",
            };
            format!("{article} `{name}`")
        }
    }

    /// The interface that declares `method`, for messages.
    fn interface_name(&self, method: FnId) -> String {
        match self.program.fns[method].interface {
            Some(interface) => self
                .interner
                .resolve(self.program.interfaces[interface].name)
                .to_string(),
            None => self
                .interner
                .resolve(self.program.fns[method].name)
                .to_string(),
        }
    }

    /// The root, as the programmer wrote it, for messages.
    pub(super) fn root_name(&self, path: &Path) -> String {
        self.interner
            .resolve(self.body.locals[path.local].name)
            .to_string()
    }

    /// The bindings of `pattern` that are values of their own, and views:
    /// they borrow what `scrutinee` borrows.
    pub(super) fn bind_views(&mut self, pattern: &Pattern, scrutinee: ExprId, state: &mut State) {
        let mut locals = Vec::new();
        pattern.locals(&mut locals);
        let views: Vec<LocalId> = locals
            .into_iter()
            .filter(|&l| self.is_view(self.body.locals[l].ty) || self.is_kept_ref(l))
            .collect();
        if views.is_empty() {
            return;
        }
        let roots = self.str_roots(scrutinee, state);
        for local in views {
            state.roots.insert(local, roots.paths.clone());
            state.stale.remove(&local);
        }
    }

    /// After a call, a `&var` argument that holds views may hold what the
    /// other arguments borrow: `words.push(piece)`. `args`
    /// are in the order of `callee`'s parameters, and each lends what its
    /// type can hold of what the `&var` argument's views refer to.
    pub(super) fn var_takes_roots(
        &mut self,
        args: &[ExprId],
        callee: Option<FnId>,
        state: &mut State,
    ) {
        for (i, &arg) in args.iter().enumerate() {
            let TyKind::Ref(inner, RefKind::Var) = self.program.types.kind(self.ty(arg)) else {
                continue;
            };
            if !self.is_view(inner) {
                continue;
            }
            let Some(path) = self.access_path(arg) else {
                continue;
            };
            // What each parameter lends to the value the `&var` one refers
            // to, as the callee declares them.
            let lends = callee.map(|callee| {
                let def = &self.program.fns[callee];
                let params: Vec<Ty> = def.params.iter().map(|p| p.ty).collect();
                let pointee = match params.get(i).map(|&p| self.program.types.kind(p)) {
                    Some(TyKind::Ref(pointee, _)) => pointee,
                    _ => inner,
                };
                lends_to(self.program, &def.generics, &params, pointee)
            });
            let mut taken = BTreeSet::new();
            for (j, &other) in args.iter().enumerate() {
                // What another `&var` argument reaches is lent for the call
                // alone, and nothing of it stays.
                let lent_for_writing = matches!(
                    self.program.types.kind(self.ty(other)),
                    TyKind::Ref(_, RefKind::Var)
                );
                if other != arg && !lent_for_writing {
                    let lends = lends
                        .as_ref()
                        .and_then(|lends| lends.get(j).copied())
                        .unwrap_or(Lends::ALL);
                    let lends = self.arg_lends(callee, j, other, lends);
                    taken.extend(self.lent_roots(other, lends, state).paths);
                }
            }
            if self.is_ref_param(path.local) {
                let span = self.body.exprs[arg].span;
                self.check_var_contents_kept(
                    path.local,
                    &taken,
                    span,
                    "keeps a view of `{lent}` here",
                );
            }
            if !taken.is_empty() {
                state.roots.entry(path.local).or_default().extend(taken);
            }
        }
    }

    /// Reports a view kept in `param`, a reference parameter, that points
    /// into what another `&var` parameter reaches: that is lent for the
    /// call alone, and its caller writes it again.
    fn check_var_contents_kept(
        &mut self,
        param: LocalId,
        roots: &BTreeSet<Path>,
        span: Span,
        label: &str,
    ) {
        let Some(root) = roots.iter().find(|root| {
            !root.is_lent()
                && root.local != param
                && self.body.locals[root.local].kind == LocalKind::Param
                && matches!(
                    self.program.types.kind(self.body.locals[root.local].ty),
                    TyKind::Ref(_, RefKind::Var)
                )
        }) else {
            return;
        };
        let kept = self
            .interner
            .resolve(self.body.locals[param].name)
            .to_string();
        let lent = self
            .interner
            .resolve(self.body.locals[root.local].name)
            .to_string();
        let diagnostic = Diagnostic::error(
            codes::VAR_CONTENTS_KEPT,
            format!("`{kept}` would keep what `{lent}` reaches, which is lent for this call alone"),
            span,
            label.replace("{lent}", &lent),
        )
        .with_secondary(
            self.body.locals[root.local].span,
            format!("`{lent}` is lent for writing"),
        )
        .with_note("what a `&var` argument reaches is not kept by another argument: its caller writes it again, and would find what kept it stale")
        .with_help(format!(
            "keep a copy, an index or a handle into `{lent}` instead, as an `Arena`'s handle names what it holds"
        ));
        self.report(diagnostic);
    }

    fn is_ref_param(&self, local: LocalId) -> bool {
        self.body.locals[local].kind == LocalKind::Param
            && matches!(
                self.program.types.kind(self.body.locals[local].ty),
                TyKind::Ref(..)
            )
    }

    /// `path` changes at `span`: every `str` that borrows it is stale from
    /// here on.
    pub(super) fn changed(&self, path: &Path, span: Span, how: &'static str, state: &mut State) {
        let stale: Vec<LocalId> = state
            .roots
            .iter()
            .filter(|(_, roots)| roots.iter().any(|root| root.overlaps(path)))
            .map(|(&local, _)| local)
            .collect();
        for local in stale {
            state.stale.insert(
                local,
                Stale {
                    definite: true,
                    change: span,
                    how,
                    root: self.root_name(path),
                },
            );
        }
    }

    /// `local`, a `str` variable, now holds the value of `value`.
    pub(super) fn bind_str(&mut self, local: LocalId, value: ExprId, state: &mut State) {
        let roots = self.str_roots(value, state);
        if let Some(temporary) = roots.temporary {
            self.kept_temporary(local, value, temporary);
        }
        state.roots.insert(local, roots.paths);
        state.stale.remove(&local);
    }

    /// An element of `local`, an array of `str`s, now holds the value of
    /// `value`: the array borrows what it borrowed, and that as well.
    pub(super) fn add_str(&mut self, local: LocalId, value: ExprId, state: &mut State) {
        let roots = self.str_roots(value, state);
        if let Some(temporary) = roots.temporary {
            self.kept_temporary(local, value, temporary);
        }
        state.roots.entry(local).or_default().extend(roots.paths);
    }

    /// A generator stops at a `yield`, and may be moved before it goes
    /// on, with its locals inside it: nothing may point into them across
    /// one. What a loop walks or a `match` binds into is
    /// refused here; a view that borrows one is stale from here on.
    pub(super) fn yielded(&mut self, span: Span, state: &mut State) {
        let held: Vec<(Path, Span, Holder)> = self
            .borrows
            .iter()
            .filter(|b| !self.is_ref_param(b.path.local) && !b.heap)
            .map(|b| (b.path.clone(), b.span, b.holder))
            .collect();
        for (path, at, holder) in held {
            let name = self.root_name(&path);
            let (by, help) = match holder {
                // A parameter the function that yields was given by value
                // is the generator's; given by reference, it would not be.
                Holder::Loop if self.body.locals[path.local].kind == LocalKind::Param => (
                    "the loop walks it",
                    format!(
                        "take `{name}` by reference, `&`, which the generator then borrows, or walk it by index, `for i in 0..{name}.len() {{ yield {name}[i] }}`"
                    ),
                ),
                Holder::Loop => (
                    "the loop walks it",
                    format!(
                        "walk `{name}` by index, `for i in 0..{name}.len() {{ yield {name}[i] }}`, or make it outside the generator, which then borrows it"
                    ),
                ),
                Holder::Match | Holder::Test | Holder::Guard => (
                    "a pattern binds into it",
                    format!("take what is yielded out of `{name}` before the `yield`"),
                ),
                _ => (
                    "borrowed here",
                    format!("borrow `{name}` after the `yield`"),
                ),
            };
            let diagnostic = Diagnostic::error(
                codes::BORROWED_ACROSS_YIELD,
                format!("`{name}` is borrowed across a `yield`"),
                span,
                format!("the generator stops here, with `{name}` borrowed"),
            )
            .with_secondary(at, by)
            .with_note("a generator keeps its locals inside itself, and may be moved while it waits, so nothing may point into them across a `yield`; what lies on the heap, as a `Vec`'s elements do, stays where it is")
            .with_help(help);
            self.report(diagnostic);
        }
        let own: std::collections::BTreeSet<LocalId> = state
            .roots
            .values()
            .flatten()
            .map(|path| path.local)
            .filter(|&local| !self.is_ref_param(local))
            .collect();
        for local in own {
            let path = Path {
                local,
                projs: Vec::new(),
            };
            self.changed(&path, span, ACROSS_YIELD, state);
        }
    }

    /// Reports a use of a `str` variable whose roots changed (E0436).
    /// `name` is the use as written: `words[…]`, or a `for` binding.
    pub(super) fn check_stale(&mut self, local: LocalId, name: &str, span: Span, state: &State) {
        let Some(stale) = state.stale.get(&local) else {
            return;
        };
        // One report for each change, however many uses follow it.
        if self.quiet > 0 || !self.stale_reported.insert((local, stale.change)) {
            return;
        }
        let root = &stale.root;
        // A generator's local, which the generator may have been moved
        // with while it waited.
        if stale.how == ACROSS_YIELD {
            let given = self.body.locals.iter().any(|(_, l)| {
                l.kind == LocalKind::Param && self.interner.resolve(l.name) == root.as_str()
            });
            let help = if given {
                format!(
                    "take `{root}` by reference, `&`, which the generator then borrows, or keep a copy that owns its bytes with `String::of(…)`"
                )
            } else {
                format!(
                    "make `{root}` outside the generator, which then borrows it, or keep a copy that owns its bytes with `String::of(…)`"
                )
            };
            // The iterator a `for` walks has no name a program wrote.
            if self.body.locals[local].name == wip_syntax::Symbol::walked() {
                let diagnostic = Diagnostic::error(
                    codes::BORROWED_ACROSS_YIELD,
                    format!("this loop's iterator borrows `{root}` across a `yield`"),
                    span,
                    "each pass calls `next()` here, after the generator went on",
                )
                .with_secondary(stale.change, "the generator stops here")
                .with_note("a generator keeps its locals inside itself, and may be moved while it waits, so nothing may point into them across a `yield`; what lies on the heap, as a `Vec`'s elements do, stays where it is")
                .with_help(help);
                self.report(diagnostic);
                return;
            }
            let variable = self.interner.resolve(self.body.locals[local].name);
            let diagnostic = Diagnostic::error(
                codes::BORROWED_ACROSS_YIELD,
                format!("`{name}` borrows `{root}` across a `yield`"),
                span,
                "used here, after the generator went on",
            )
            .with_secondary(stale.change, "the generator stops here")
            .with_secondary(
                self.body.locals[local].span,
                format!("`{variable}` borrows `{root}`, the generator's own"),
            )
            .with_note("a generator keeps its locals inside itself, and may be moved while it waits, so nothing may point into them across a `yield`; what lies on the heap, as a `Vec`'s elements do, stays where it is")
            .with_help(help);
            self.report(diagnostic);
            return;
        }
        // The variable a `for` walks an iterator in has no name a program
        // wrote.
        if self.body.locals[local].name == wip_syntax::Symbol::walked() {
            let diagnostic = Diagnostic::error(
                codes::STR_OUTLIVED_ITS_BYTES,
                format!("this loop's iterator borrows `{root}`, which changes inside it"),
                span,
                "each pass calls `next()` here",
            )
            .with_secondary(stale.change, format!("`{root}` {} here", stale.how))
            .with_note(
                "what the iterator was made from must not change while the loop still walks it",
            )
            .with_help(format!(
                "change `{root}` after the loop, or walk a copy that owns its bytes"
            ));
            self.report(diagnostic);
            return;
        }
        let variable = self.interner.resolve(self.body.locals[local].name);
        let verb = if stale.definite {
            "was"
        } else {
            "may have been"
        };
        // A closure kept in a `val` reads what it captured where it is.
        if self.is_kept_closure(self.body.locals[local].ty) {
            let diagnostic = Diagnostic::error(
                codes::STR_OUTLIVED_ITS_BYTES,
                format!("`{name}` is used after `{root}`, which it captures, {verb} {}", stale.how),
                span,
                "used here",
            )
            .with_secondary(stale.change, format!("`{root}` {} here", stale.how))
            .with_secondary(
                self.body.locals[local].span,
                format!("`{variable}` captures `{root}`"),
            )
            .with_note("a closure kept in a `val` reads what it captures where it is, so that must not change while the closure is still used")
            .with_help(format!(
                "call `{name}` before `{root}` changes, or give it `{root}` as an argument"
            ));
            self.report(diagnostic);
            return;
        }
        let what = if stale.how == "dropped" {
            "dropped"
        } else {
            "changed"
        };
        let diagnostic = Diagnostic::error(
            codes::STR_OUTLIVED_ITS_BYTES,
            format!("`{name}` is used after `{root}`, which it borrows, {verb} {what}"),
            span,
            "used here",
        )
        .with_secondary(stale.change, format!("`{root}` {} here", stale.how))
        .with_secondary(
            self.body.locals[local].span,
            format!("`{variable}` borrows `{root}`"),
        )
        .with_note(self.borrowing(self.body.locals[local].ty).stale)
        .with_help(format!(
            "use `{name}` before `{root}` changes, or {}",
            self.borrowing(self.body.locals[local].ty).instead
        ));
        self.report(diagnostic);
    }

    /// How a value of `ty` borrows, for messages: a `str` its bytes, a `&`
    /// reference its place, a view what its parts borrow.
    fn borrowing(&self, ty: Ty) -> Borrowing {
        if ty == Types::STR {
            Borrowing {
                stale: "a `str` points into the bytes of what it borrows, so those must not change while the `str` is still used",
                leaving: "a `str` that leaves a function may only borrow what its reference parameters refer to",
                instead: "keep a copy that owns its bytes with `String::of(…)`",
            }
        } else if self.is_shared_ref(ty) {
            Borrowing {
                stale: "a `&` reference points into the place it borrows, so that must not change while the reference is still used",
                leaving: "a `&` reference that leaves a function may only point into what its reference parameters refer to",
                instead: "keep a copy of the value it points to",
            }
        } else {
            Borrowing {
                stale: "a view points into what it borrows — a `str` into bytes, a `&` into a place — so that must not change while the view is still used",
                leaving: "a view that leaves a function may only borrow what its reference parameters refer to",
                instead: "keep a value that owns its data",
            }
        }
    }

    /// In one call, a `str` argument must not borrow what a `&var` argument
    /// lets the call change: `text.push(text.toStr())` reads bytes that
    /// the push may move (E0436).
    pub(super) fn check_call_strs(&mut self, args: &[ExprId], state: &State) {
        let body = self.body;
        let vars: Vec<(ExprId, Path)> = args
            .iter()
            .filter(|&&arg| {
                matches!(
                    self.program.types.kind(self.ty(arg)),
                    TyKind::Ref(_, RefKind::Var)
                )
            })
            .filter_map(|&arg| Some((arg, self.access_path(arg)?)))
            .collect();
        if vars.is_empty() {
            return;
        }
        for &arg in args {
            if !self.is_view(self.ty(arg)) {
                continue;
            }
            let roots = self.str_roots(arg, state);
            let Some((var_arg, path)) = vars
                .iter()
                .find(|(_, path)| roots.paths.iter().any(|root| root.overlaps(path)))
            else {
                continue;
            };
            let root = self.root_name(path);
            let diagnostic = Diagnostic::error(
                codes::STR_OUTLIVED_ITS_BYTES,
                format!("this argument borrows `{root}`, which the same call may change"),
                body.exprs[arg].span,
                format!("borrows `{root}`"),
            )
            .with_secondary(body.exprs[*var_arg].span, "passed as `&var` here")
            .with_note("a `&var` argument may change its place while the call still reads the `str`, which points into it")
            .with_help("copy the text first: `String::of(…)` owns its bytes");
            self.report(diagnostic);
        }
    }

    /// Reports a `str` kept in `local` that borrows a temporary (E0437).
    fn kept_temporary(&mut self, local: LocalId, value: ExprId, temporary: Span) {
        let name = self.interner.resolve(self.body.locals[local].name);
        let diagnostic = Diagnostic::error(
            codes::STR_OUTLIVES_ITS_ROOT,
            format!("`{name}` would borrow a temporary"),
            self.body.exprs[value].span,
            format!("{} is kept here", self.a_view(value)),
        )
        .with_secondary(
            temporary,
            "a temporary, dropped at the end of the statement",
        )
        .with_note("a view points into what it borrows, which must live as long as it does")
        .with_help("bind the value with `val` first, then borrow it");
        self.report(diagnostic);
    }

    /// A `str` that leaves the function: every root must be something a
    /// reference parameter refers to, which outlives the call (E0437).
    pub(super) fn check_leaving(&mut self, value: ExprId, leaving: Leaving, state: &State) {
        let roots = self.str_roots(value, state);
        let span = self.body.exprs[value].span;
        if let Leaving::Param(param) = leaving {
            self.check_var_contents_kept(param, &roots.paths, span, "points into `{lent}`");
        }
        let what = match leaving {
            Leaving::Result => "return".to_string(),
            Leaving::Yield => "yield".to_string(),
            Leaving::Param(param) => format!(
                "give `{}`",
                self.interner.resolve(self.body.locals[param].name)
            ),
        };
        if let Some(temporary) = roots.temporary {
            let diagnostic = Diagnostic::error(
                codes::STR_OUTLIVES_ITS_ROOT,
                format!(
                    "cannot {what} {} that borrows a temporary",
                    self.a_view(value)
                ),
                span,
                "leaves the function here",
            )
            .with_secondary(
                temporary,
                "a temporary, dropped at the end of the statement",
            )
            .with_note(self.borrowing(self.ty(value)).leaving)
            .with_help("answer a value that owns its data, as a `String` owns its bytes");
            self.report(diagnostic);
            return;
        }
        // A result points into a reference parameter's place only where
        // the signature lends it: its callers keep only what the signature
        // says.
        if matches!(leaving, Leaving::Result)
            && let Some((root, param)) = roots.paths.iter().filter(|p| !p.is_lent()).find_map(|p| {
                let param = self.body.params.iter().position(|&l| l == p.local)?;
                let lends = self.lends.get(param)?;
                (self.is_ref_param(p.local) && !lends.place).then_some((p, param))
            })
        {
            let name = self.root_name(root);
            let declared = match self.implements {
                Some(method) => format!(
                    "`{}`, as `{}` declares it,",
                    self.fn_name,
                    self.interface_name(method)
                ),
                None => format!("`{}`", self.fn_name),
            };
            let diagnostic = Diagnostic::error(
                codes::STR_OUTLIVES_ITS_ROOT,
                format!("cannot {what} {} that points into `{name}`", self.a_view(value)),
                span,
                format!("points into `{name}`"),
            )
            .with_secondary(
                self.body.locals[self.body.params[param]].span,
                format!("`{name}` is `{}`", self.ty_name(self.body.locals[root.local].ty)),
            )
            .with_note(if self.says_from {
                format!(
                    "{declared} says with `from` what its result borrows, and `{name}` itself is not among it; its callers keep only that"
                )
            } else {
                format!(
                    "{declared} answers what its parameters' types can hold, and `{name}`'s does not hold, as its own, what the result refers to; its callers keep only that"
                )
            })
            .with_help(format!("answer what `{name}` borrows, or a value that owns its data"));
            self.report(diagnostic);
            return;
        }
        // What a parameter borrows leaves only where `from` names it.
        if matches!(leaving, Leaving::Result)
            && let Some((root, param)) = roots.paths.iter().filter(|p| p.is_lent()).find_map(|p| {
                let param = self.body.params.iter().position(|&l| l == p.local)?;
                (!self.lends.get(param)?.borrows).then_some((p, param))
            })
        {
            let name = self.root_name(root);
            let diagnostic = Diagnostic::error(
                codes::STR_OUTLIVES_ITS_ROOT,
                format!(
                    "cannot {what} {} that borrows what `{name}` borrows",
                    self.a_view(value)
                ),
                span,
                format!("borrows what `{name}` borrows"),
            )
            .with_secondary(
                self.body.locals[self.body.params[param]].span,
                format!("`{name}` is not named after `from`"),
            )
            .with_note(format!(
                "`{}` says with `from` what its result borrows, and its callers keep only that",
                self.fn_name
            ))
            .with_help(format!(
                "name `{name}` after `from`, or answer from what it names"
            ));
            self.report(diagnostic);
            return;
        }
        let Some(root) = roots
            .paths
            .iter()
            .find(|p| !p.is_lent() && !self.is_ref_param(p.local))
        else {
            return;
        };
        let name = self.root_name(root);
        let owner = if self.generator {
            format!("`{name}` is the generator's own, and moves with it")
        } else if self.body.locals[root.local].kind == LocalKind::Param {
            format!(
                "`{name}` belongs to `{}`, and is dropped when it returns",
                self.fn_name
            )
        } else {
            format!(
                "`{name}` is a local of `{}`, dropped when it returns",
                self.fn_name
            )
        };
        let (note, help) = match leaving {
            Leaving::Yield => (
                "what a generator yields may borrow what it was given, but not what it holds itself, which moves with it",
                "yield a `String`, which owns its bytes, or make the value outside the generator, which then borrows it",
            ),
            _ => (
                self.borrowing(self.ty(value)).leaving,
                "answer a value that owns its data, as a `String` owns its bytes",
            ),
        };
        let diagnostic = Diagnostic::error(
            codes::STR_OUTLIVES_ITS_ROOT,
            format!("cannot {what} {} that borrows `{name}`", self.a_view(value)),
            span,
            owner,
        )
        .with_secondary(
            self.body.locals[root.local].span,
            format!("`{name}` declared here"),
        )
        .with_note(note)
        .with_help(help);
        self.report(diagnostic);
    }

    /// The value of a block, or of a `match` arm, that borrows one of
    /// `dying`, which are dropped where it ends (E0437).
    pub(super) fn check_block_value(
        &mut self,
        value: ExprId,
        dying: &[LocalId],
        what: &str,
        state: &State,
    ) {
        if !self.borrows_as_view(self.ty(value)) || dying.is_empty() {
            return;
        }
        let roots = self.str_roots(value, state);
        let Some(root) = roots.paths.iter().find(|p| dying.contains(&p.local)) else {
            return;
        };
        let name = self.root_name(root);
        let diagnostic = Diagnostic::error(
            codes::STR_OUTLIVES_ITS_ROOT,
            format!("the value of this {what} borrows `{name}`, which is dropped where it ends"),
            self.body.exprs[value].span,
            format!("borrows `{name}`"),
        )
        .with_secondary(self.body.locals[root.local].span, format!("`{name}` declared here"))
        .with_note("a `str`, a view or a `&` reference points into what it borrows, which must live as long as it does")
        .with_help(format!("declare `{name}` outside the {what}, or answer a value that owns its data"));
        self.report(diagnostic);
    }
}
