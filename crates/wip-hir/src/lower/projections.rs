//! Projections: a function that lends a place to its caller.

use super::*;

impl<'a> Lowerer<'a> {
    /// `lend place`: lends a place that belongs to one of the projection's
    /// reference parameters, or to a constant table.
    pub(super) fn lend_expr(&mut self, value: ast::ExprId, span: Span) -> ExprId {
        let TyKind::Ref(pointee, kind) = self.kind(self.state.ret) else {
            let diagnostic = Diagnostic::error(
                codes::LEND_OUTSIDE_PROJECTION,
                "`lend` outside a projection",
                span,
                "not in a projection",
            )
            .with_help("a projection declares a reference result, as in `fn heaviest(bag: &var Bag): &var Item`");
            self.report(diagnostic);
            self.infer(value, None);
            return self.error_expr(span);
        };
        self.state.lends += 1;
        // A `yield` lends what it names, so a range of elements may be
        // yielded as it may be borrowed.
        let mut place = self.infer_borrowed(value, Some(pointee));
        // Lend what an `own` points to, as the `&own<T>` to `&T` coercion
        // does for an argument.
        loop {
            let ty = self.ty_of(place);
            if ty == pointee {
                break;
            }
            let TyKind::Own(inner) = self.kind(ty) else {
                break;
            };
            let span = self.state.body.exprs[place].span;
            place = self.alloc(ExprKind::Deref(place), inner, span);
        }
        // An array is lent as a slice where one is the result, as it is
        // passed where one is taken.
        if let (TyKind::Array(elem, _), TyKind::Slice(want)) =
            (self.kind(self.ty_of(place)), self.kind(pointee))
            && elem == want
        {
            let span = self.state.body.exprs[place].span;
            let whole = ExprKind::SubSlice {
                base: place,
                lo: None,
                hi: None,
            };
            place = self.alloc(whole, pointee, span);
        }
        let place = place;
        let ty = self.ty_of(place);
        let place_span = self.state.body.exprs[place].span;
        if self.is_poisoned(ty) {
            return self.alloc(ExprKind::Lend(place), Types::NEVER, span);
        }
        if ty != pointee {
            let (found, expected) = (self.ty_name(ty), self.ty_name(pointee));
            let diagnostic = Diagnostic::error(
                codes::MISMATCHED_TYPES,
                "mismatched types",
                place_span,
                format!("expected {expected}, found {found}"),
            );
            self.report(diagnostic);
        }
        match self.lent_root(place) {
            // A call to another projection lends what that one lends, and
            // what that is cannot be known until every body is checked, so
            // it is worked out afterwards.
            LendRoot::Forwarded(_) => {}
            LendRoot::Param(index) => match self.state.lent_param {
                // The caller borrows the argument that was lent from, so
                // every path must lend from the same one.
                Some(first) if first != index => {
                    let name = |this: &Self, i: u32| {
                        let local = this.state.body.params[i as usize];
                        this.text(this.state.body.locals[local].name).to_string()
                    };
                    let (first_name, this_name) = (name(self, first), name(self, index));
                    let diagnostic = Diagnostic::error(
                        codes::PROJECTION_BODY,
                        format!(
                            "every `lend` must name a place in the same parameter, but an earlier one lends part of `{first_name}`"
                        ),
                        place_span,
                        format!("part of `{this_name}`"),
                    )
                    .with_note("the call borrows the argument a projection lends from, and that is one argument");
                    self.report(diagnostic);
                }
                _ => self.state.lent_param = Some(index),
            },
            // A constant table lives as long as the program, and nothing
            // writes it: the call holds nothing for it.
            LendRoot::Table(id) if kind == crate::RefKind::Var => {
                let name = self.text(self.program.consts[id].name).to_string();
                let diagnostic = Diagnostic::error(
                    codes::PROJECTION_BODY,
                    format!("a `&var` projection cannot lend `{name}`, a constant"),
                    place_span,
                    "a constant cannot change",
                )
                .with_help("declare the result `&`: a constant is lent for reading")
                .with_note("a top-level `val` is kept once, in memory nothing writes");
                self.report(diagnostic);
                return self.alloc(ExprKind::Lend(place), Types::NEVER, span);
            }
            LendRoot::Table(_) => self.state.lent_table = true,
            LendRoot::None => {
                // A number, a `bool` or a variant of nothing is written where
                // it is used, so there is no place to lend.
                let value = matches!(
                    self.state.body.exprs[place].kind,
                    ExprKind::Int(_)
                        | ExprKind::Float(_)
                        | ExprKind::Bool(_)
                        | ExprKind::Variant { .. }
                );
                let diagnostic = match value {
                    true => Diagnostic::error(
                        codes::PROJECTION_BODY,
                        "a projection lends a place, and this is a value",
                        place_span,
                        "a value, not a place",
                    )
                    .with_help("answer it by value: declare the result without `&`")
                    .with_note("a number, a `bool` or a variant of nothing is written where it is used, even from a top-level `val`, so there is no place to lend"),
                    false => Diagnostic::error(
                        codes::PROJECTION_BODY,
                        "a projection yields a place that belongs to one of its reference parameters",
                        place_span,
                        "not part of a reference parameter",
                    )
                    .with_note("what a projection lends belongs to its caller, or to a constant table, which is why it needs no lifetime"),
                };
                self.report(diagnostic);
            }
        }
        if kind == crate::RefKind::Var && !self.writable_for_writing(place) {
            // The writing half of a `lend fn` is its body checked again: a
            // place it can only read means it only ever reads.
            let diagnostic = match self.state.lent_half {
                true => Diagnostic::error(
                    codes::PROJECTION_BODY,
                    "a `lend fn` lends a place its writing half cannot write",
                    place_span,
                    "only ever read",
                )
                .with_help("declare it `fn`: what it lends is for reading only")
                .with_note("`lend fn` is checked twice, once reading and once writing, and each must lend what it names"),
                false => Diagnostic::error(
                    codes::PROJECTION_BODY,
                    "a `&var` projection yields a place it cannot write",
                    place_span,
                    "cannot be written",
                )
                .with_help("take the parameter as `&var`, or declare the result `&`"),
            };
            self.report(diagnostic);
        }
        self.alloc(ExprKind::Lend(place), Types::NEVER, span)
    }

    /// Where a yielded place is rooted.
    fn lent_root(&self, place: ExprId) -> LendRoot {
        lent_root(&self.state.body, place, |ty| {
            matches!(self.kind(ty), TyKind::Ref(..))
        })
    }

    /// Whether every path through an expression leaves: it yields, panics,
    /// returns, or never ends. A projection's body is checked against its
    /// result type, so its own type does not say.
    fn always_leaves(&self, id: ExprId) -> bool {
        match &self.state.body.exprs[id].kind {
            ExprKind::Lend(_) | ExprKind::Panic { .. } => true,
            ExprKind::Block(block) => match block.value {
                Some(value) => self.always_leaves(value),
                None => false,
            },
            ExprKind::If {
                then_block,
                else_block: Some(else_block),
                ..
            } => {
                let leaves = |block: &Block| match block.value {
                    Some(value) => self.always_leaves(value),
                    None => false,
                };
                leaves(then_block) && leaves(else_block)
            }
            ExprKind::Match { arms, .. } => {
                !arms.is_empty() && arms.iter().all(|arm| self.always_leaves(arm.body))
            }
            // A `return`, a `break`, or a call that does not return.
            _ => self.ty_of(id) == Types::NEVER,
        }
    }

    /// A projection yields exactly once, as its last expression, and cannot
    /// `defer` yet.
    pub(super) fn check_projection(&mut self, id: FnId, value: ExprId) {
        let span = self.state.body.exprs[value].span;
        let yields_everywhere = self.always_leaves(value);
        if self.state.lends == 0 {
            let diagnostic = Diagnostic::error(
                codes::PROJECTION_BODY,
                "a projection must end with `lend`",
                span,
                "yields nothing",
            )
            .with_help("lend the place it hands over, as in `= lend bag.items[i]`");
            self.report(diagnostic);
        } else if !yields_everywhere {
            let diagnostic = Diagnostic::error(
                codes::PROJECTION_BODY,
                "every path through a projection must `lend`",
                span,
                "a path lends nothing",
            )
            .with_note("an `if` without `else`, or anything after the `lend`, leaves a path that reaches the end without lending");
            self.report(diagnostic);
        }
        let returns: Vec<Span> = self
            .state
            .body
            .stmts
            .iter()
            .filter(|(_, stmt)| matches!(stmt.kind, StmtKind::Return(_)))
            .map(|(_, stmt)| stmt.span)
            .collect();
        for ret in returns {
            let diagnostic = Diagnostic::error(
                codes::PROJECTION_BODY,
                "a projection ends with `lend`, not `return`",
                ret,
                "returns instead of lending",
            )
            .with_note("a projection lends a place that belongs to its caller; there is no value to return");
            self.report(diagnostic);
        }
        let deferred: Vec<Span> = self
            .state
            .body
            .stmts
            .iter()
            .filter(|(_, stmt)| matches!(stmt.kind, StmtKind::Defer(_)))
            .map(|(_, stmt)| stmt.span)
            .collect();
        for defer in deferred {
            let diagnostic = Diagnostic::error(
                codes::PROJECTION_BODY,
                "a projection cannot `defer` yet",
                defer,
                "runs after the caller is done",
            )
            .with_note("a projection's `defer`s are to run when the borrow ends; this version returns the address and stops");
            self.report(diagnostic);
        }
        // What the call holds: the parameter, where any path lends from
        // one, and nothing where every path lends a table.
        self.program.fns[id].projects = match (self.state.lent_param, self.state.lent_table) {
            (Some(index), _) => Some(Lent::Param(index)),
            (None, true) if self.state.lends > 0 && !self.lends_forwarded() => Some(Lent::Tables),
            _ => None,
        };
    }

    /// Whether any `lend` in the body forwards another projection's, which
    /// is worked out once every body is checked.
    fn lends_forwarded(&self) -> bool {
        self.state
            .body
            .exprs
            .iter()
            .any(|(_, expr)| match expr.kind {
                ExprKind::Lend(place) => matches!(self.lent_root(place), LendRoot::Forwarded(_)),
                _ => false,
            })
    }
}

/// Where a yielded place is rooted.
pub(super) enum LendRoot {
    /// A reference parameter of the projection, by index.
    Param(u32),
    /// A call to another projection: what it lends is rooted wherever that
    /// projection lends from, and the call is this one.
    Forwarded(ExprId),
    /// A constant table, which lives as long as the program.
    Table(ConstId),
    /// Nothing the projection may lend.
    None,
}

/// The walk down a yielded place to what it belongs to, which the check in
/// a body and the one that follows every body share.
fn lent_root(body: &Body, place: ExprId, is_ref: impl Fn(Ty) -> bool) -> LendRoot {
    let mut e = place;
    loop {
        match body.exprs[e].kind {
            ExprKind::Field { base, .. }
            | ExprKind::Index { base, .. }
            | ExprKind::SubSlice { base, .. }
            | ExprKind::Deref(base) => e = base,
            // A reference refers to what it borrows, which is how the
            // argument of a forwarded call is written.
            ExprKind::Ref(base) => e = base,
            // A call to a projection is a place of whatever that projection
            // lends from.
            ExprKind::Call { .. } => return LendRoot::Forwarded(e),
            ExprKind::Table(id) => return LendRoot::Table(id),
            // A binding aliases part of the place it was matched from.
            ExprKind::Local(local)
                if body.locals[local].kind == LocalKind::Binding
                    && body.aliases.contains_key(&local) =>
            {
                e = body.aliases[&local];
            }
            ExprKind::Local(local) => {
                let local_def = &body.locals[local];
                if local_def.kind != LocalKind::Param || !is_ref(local_def.ty) {
                    return LendRoot::None;
                }
                return match body.params.iter().position(|&p| p == local) {
                    Some(i) => LendRoot::Param(i as u32),
                    None => LendRoot::None,
                };
            }
            _ => return LendRoot::None,
        }
    }
}

/// What a projection lends, once the ones it calls are known.
enum Lends {
    /// The reference parameter it is rooted in.
    Param(u32),
    /// A constant table.
    Table,
    /// A projection it forwards to has not been worked out yet.
    Waiting,
    /// Nothing it may lend.
    Nothing,
}

impl Lowerer<'_> {
    /// What every projection that forwards to another lends, once each body
    /// is checked. A body cannot know what another lends
    /// while both are being checked, so the answers settle here, in rounds:
    /// a chain resolves from the end, `Grid::at` after the `Vec::at` it
    /// calls.
    pub(super) fn resolve_forwarded(&mut self) {
        let is_ref = |this: &Self, ty: Ty| matches!(this.kind(ty), TyKind::Ref(..));
        // The projections that forward, and the places they yield, in the
        // order they were declared.
        let mut work: Vec<(FnId, Vec<ExprId>)> = Vec::new();
        for (id, def) in self.program.fns.iter() {
            let Some(body) = &def.body else { continue };
            if !is_ref(self, def.ret) {
                continue;
            }
            let forwarded: Vec<ExprId> = body
                .exprs
                .iter()
                .filter_map(|(_, expr)| match expr.kind {
                    ExprKind::Lend(place) => matches!(
                        lent_root(body, place, |ty| is_ref(self, ty)),
                        LendRoot::Forwarded(_)
                    )
                    .then_some(place),
                    _ => None,
                })
                .collect();
            if !forwarded.is_empty() {
                work.push((id, forwarded));
            }
        }
        // Each round answers the projections whose callees were answered in
        // the one before, so a chain of `n` needs at most `n` of them.
        for _ in 0..=work.len() {
            let mut moved = false;
            for (id, places) in &work {
                if self.program.fns[*id].projects.is_some() {
                    continue;
                }
                let body = self.program.fns[*id].body.as_ref().expect("a body");
                let mut lends = None;
                let mut settled = true;
                for &place in places {
                    match self.lends(body, place) {
                        Lends::Param(index) if lends.is_none() => lends = Some(index),
                        Lends::Waiting => settled = false,
                        _ => {}
                    }
                }
                // Every path settled and none lends a parameter: tables
                // alone, here or in what it forwards to. A
                // `lend` of a parameter's place elsewhere in the body was
                // recorded where the body was checked.
                let tables = settled
                    && lends.is_none()
                    && places
                        .iter()
                        .all(|&place| matches!(self.lends(body, place), Lends::Table));
                if let Some(index) = lends {
                    self.program.fns[*id].projects = Some(Lent::Param(index));
                    moved = true;
                } else if tables && !lends_param_directly(body, |ty| is_ref(self, ty)) {
                    self.program.fns[*id].projects = Some(Lent::Tables);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        // What is still unanswered is reported, and what disagrees with an
        // earlier `yield` is reported as it is inside a body.
        let mut diagnostics = Vec::new();
        for (id, places) in &work {
            let body = self.program.fns[*id].body.as_ref().expect("a body");
            let first = self.program.fns[*id].projects;
            for &place in places {
                let span = body.exprs[place].span;
                let first = match first {
                    Some(Lent::Param(index)) => Some(index),
                    _ => None,
                };
                match self.lends(body, place) {
                    Lends::Table => {}
                    Lends::Param(index) => match first {
                        Some(earlier) if earlier != index => {
                            let name = |i: u32| {
                                let local = body.params[i as usize];
                                self.text(body.locals[local].name).to_string()
                            };
                            let (first_name, this_name) = (name(earlier), name(index));
                            diagnostics.push(
                                Diagnostic::error(
                                    codes::PROJECTION_BODY,
                                    format!(
                                        "every `lend` must name a place in the same parameter, but an earlier one lends part of `{first_name}`"
                                    ),
                                    span,
                                    format!("part of `{this_name}`"),
                                )
                                .with_note("the call borrows the argument a projection lends from, and that is one argument"),
                            );
                        }
                        _ => {}
                    },
                    lends => {
                        let mut diagnostic = Diagnostic::error(
                            codes::PROJECTION_BODY,
                            "a projection yields a place that belongs to one of its reference parameters",
                            span,
                            "not part of a reference parameter",
                        )
                        .with_note("what a projection lends belongs to its caller, or to a constant table, which is why it needs no lifetime");
                        // A projection that reaches itself never arrives at
                        // a place, and neither does anything it forwards to.
                        if matches!(lends, Lends::Waiting) {
                            diagnostic = diagnostic.with_help(
                                "this lends what another projection lends, and that one never reaches a place of its own",
                            );
                        }
                        diagnostics.push(diagnostic);
                    }
                }
            }
        }
        for diagnostic in diagnostics {
            self.report(diagnostic);
        }
    }

    /// What a yielded place is rooted in, following the projections it
    /// forwards to.
    fn lends(&self, body: &Body, place: ExprId) -> Lends {
        let mut place = place;
        loop {
            match lent_root(body, place, |ty| matches!(self.kind(ty), TyKind::Ref(..))) {
                LendRoot::Param(index) => return Lends::Param(index),
                LendRoot::Table(_) => return Lends::Table,
                LendRoot::None => return Lends::Nothing,
                LendRoot::Forwarded(call) => {
                    let ExprKind::Call {
                        callee, ref args, ..
                    } = body.exprs[call].kind
                    else {
                        return Lends::Nothing;
                    };
                    match self.program.fns[callee].projects {
                        Some(Lent::Param(index)) => match args.get(index as usize) {
                            Some(&arg) => place = arg,
                            None => return Lends::Nothing,
                        },
                        Some(Lent::Tables) => return Lends::Table,
                        None => return Lends::Waiting,
                    }
                }
            }
        }
    }
}

/// Whether a body lends a place of one of its parameters directly, not
/// through another projection.
fn lends_param_directly(body: &Body, is_ref: impl Fn(Ty) -> bool + Copy) -> bool {
    body.exprs.iter().any(|(_, expr)| match expr.kind {
        ExprKind::Lend(place) => matches!(lent_root(body, place, is_ref), LendRoot::Param(_)),
        _ => false,
    })
}
