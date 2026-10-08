//! Expressions, and how each use of a place is checked.

use super::lends::Lends;
use super::*;

impl Checker<'_> {
    pub(super) fn expr(&mut self, id: ExprId, ctx: Ctx, state: &mut State) {
        let body = self.body;
        let expr = &body.exprs[id];
        match &expr.kind {
            ExprKind::Local(_)
            | ExprKind::Field { .. }
            | ExprKind::Deref(_)
            | ExprKind::Index { .. } => self.use_place(id, ctx, state),
            ExprKind::Move(inner) => {
                // The type checker has already rejected `move` of a non-place.
                let ctx = if self.place(*inner).is_some() {
                    Ctx::Move(expr.span)
                } else {
                    Ctx::Value
                };
                self.expr(*inner, ctx, state);
            }
            ExprKind::Ref(inner) => {
                let ctx = if self.place(*inner).is_some() {
                    Ctx::Inspect
                } else {
                    Ctx::Value
                };
                self.expr(*inner, ctx, state);
            }
            // A `String` over a `str`'s bytes, made for the call it is lent
            // to: the text is read, and the `String` is a temporary.
            ExprKind::Undropped(inner) => self.expr(*inner, Ctx::Value, state),
            ExprKind::Assign {
                place, op, value, ..
            } => {
                self.assignment(*place, op.is_some(), *value, state)
            }
            ExprKind::Binary {
                op: BinaryOp::And | BinaryOp::Or,
                lhs,
                rhs,
                ..
            } => {
                // The right operand may not run, so its moves are only
                // possible ones.
                self.expr(*lhs, Ctx::Value, state);
                let mut rhs_state = state.clone();
                self.expr(*rhs, Ctx::Value, &mut rhs_state);
                *state = state.merge(&rhs_state);
            }
            ExprKind::Binary { lhs, rhs, .. } | ExprKind::StrCmp { lhs, rhs } => {
                self.expr(*lhs, Ctx::Value, state);
                self.expr(*rhs, Ctx::Value, state);
            }
            ExprKind::Unary { operand, .. } | ExprKind::Own(operand) => {
                self.expr(*operand, Ctx::Value, state)
            }
            // In the order written.
            ExprKind::Call {
                callee,
                args,
                order,
                ..
            } => {
                self.call_args(args, order, Some(*callee), state);
                self.record_copy(id, *callee, args, state);
                // A call that is a jump leaves this frame in place, so
                // nothing of it may still need cleaning up.
                if self.body.tail_calls.contains(&id) {
                    self.check_tail_call(expr.span, state);
                }
            }
            // The function value is read first: it is plain data.
            ExprKind::CallValue { callee, args } => {
                self.expr(*callee, Ctx::Value, state);
                self.call_args(args, &[], None, state);
            }
            // Lending a place reads it; it belongs to a parameter, so nothing
            // moves.
            ExprKind::Lend(place) => self.expr(*place, Ctx::Inspect, state),
            // A union holds one field's value over zeroed bytes.
            ExprKind::Union { field, .. } => {
                if let Some((_, value)) = field {
                    self.expr(*value, Ctx::Value, state);
                }
            }
            ExprKind::Variant {
                args: elems, order, ..
            }
            | ExprKind::Struct {
                fields: elems,
                order,
                ..
            } => {
                for i in evaluation_order(order, elems.len()) {
                    self.expr(elems[i], Ctx::Value, state);
                }
            }
            ExprKind::Array(elems) => {
                for &elem in elems {
                    self.expr(elem, Ctx::Value, state);
                }
            }
            ExprKind::ArrayRepeat { elem, count } => {
                self.expr(*elem, Ctx::Value, state);
                if *count > 1 && self.owns(self.ty(*elem)) {
                    let diagnostic = Diagnostic::error(
                        codes::CANNOT_REPEAT_OWN,
                        "cannot repeat a value that contains `own`",
                        body.exprs[*elem].span,
                        format!("would give {count} owners to one allocation"),
                    )
                    .with_help("write each element out, so each gets its own allocation");
                    self.report(diagnostic);
                }
            }
            ExprKind::OwnRepeat { elem, count } => {
                self.expr(*elem, Ctx::Value, state);
                self.expr(*count, Ctx::Value, state);
                // The count is known only at run time, so the element's type
                // decides.
                if self.owns(self.ty(*elem)) {
                    let diagnostic = Diagnostic::error(
                        codes::CANNOT_REPEAT_OWN,
                        "cannot repeat a value that contains `own`",
                        body.exprs[*elem].span,
                        "would give every element of the buffer the same allocation",
                    )
                    .with_help("fill the buffer with a value that owns nothing");
                    self.report(diagnostic);
                }
            }
            ExprKind::Unsize(inner) => self.expr(*inner, Ctx::Value, state),
            // The reference is borrowed as any other is; the table beside it
            // is the type's own.
            ExprKind::DynRef { value, .. } => self.expr(*value, Ctx::Value, state),
            // What a closure captured is borrowed where it is made.
            ExprKind::Closure { env, .. } => self.expr(*env, Ctx::Value, state),
            // An owned closure lent for a call is read where it is lent, as
            // any reference to it would be.
            ExprKind::LendClosure(inner) => self.expr(*inner, Ctx::Inspect, state),
            // The address of a drop function: plain data.
            ExprKind::DropRef(_) => {}
            ExprKind::CallClosure { callee, args, .. } => {
                // A call reads the closure where it is: an owned one stays
                // with its owner.
                let ctx = if self.place(*callee).is_some() {
                    Ctx::Inspect
                } else {
                    Ctx::Value
                };
                self.expr(*callee, ctx, state);
                self.call_args(args, &[], None, state);
            }
            // The receiver is the first argument.
            ExprKind::DynCall {
                interface,
                index,
                args,
                order,
                ..
            } => {
                let method = self.program.interfaces[*interface].methods[*index as usize].id;
                self.call_args(args, order, Some(method), state);
            }
            ExprKind::Cast(inner)
            | ExprKind::VariantIndex(inner)
            | ExprKind::IsNull(inner)
            | ExprKind::CstrToStr(inner) => {
                self.expr(*inner, Ctx::Value, state)
            }
            ExprKind::Len(base) => {
                let ctx = if self.place(*base).is_some() {
                    Ctx::Inspect
                } else {
                    Ctx::Value
                };
                self.expr(*base, ctx, state);
            }
            // A range of elements is always borrowed: its base is looked at,
            // not taken.
            ExprKind::SubSlice { base, lo, hi } => {
                let ctx = if self.place(*base).is_some() {
                    Ctx::Inspect
                } else {
                    Ctx::Value
                };
                self.expr(*base, ctx, state);
                for &bound in lo.iter().chain(hi.iter()) {
                    self.expr(bound, Ctx::Value, state);
                }
            }
            ExprKind::Block(block) => {
                self.block(block, state);
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => self.if_expr(*cond, then_block, else_block.as_ref(), state),
            // `value is pattern`: its bindings are made here,
            // and the borrow they hold lasts until the `if` or `while` that
            // tests it ends its block, which releases it.
            ExprKind::Is { scrutinee, pattern } => {
                let ctx = if self.place(*scrutinee).is_some() {
                    Ctx::Inspect
                } else {
                    Ctx::Value
                };
                self.expr(*scrutinee, ctx, state);
                self.bind_pattern(pattern, *scrutinee, Holder::Test, state);
                self.bind_views(pattern, *scrutinee, state);
            }
            // A step of a walk one place at a time: the
            // binding refers into the container, which may not change while
            // the body runs, as for a `for` over a slice.
            ExprKind::Match { scrutinee, arms } if body.sequence_walks.contains(&id) => {
                self.expr(*scrutinee, Ctx::Inspect, state);
                let walked = self.access_path(*scrutinee);
                let span = body.exprs[*scrutinee].span;
                for arm in arms {
                    let mark = self.borrows.len();
                    if let Some(path) = &walked {
                        let mut element = path.clone();
                        element.projs.push(Proj::Element);
                        let mut binders = Vec::new();
                        crate::bindings_of(&arm.pattern, Some(element), &mut binders);
                        for (local, path) in binders {
                            if let Some(path) = path {
                                self.aliases.insert(local, path);
                            }
                        }
                        self.borrows.push(Borrow {
                            heap: false,
                            path: path.clone(),
                            span,
                            var: false,
                            holder: Holder::Loop,
                        });
                    }
                    self.expr(arm.body, Ctx::Value, state);
                    self.borrows.truncate(mark);
                }
            }
            // A tuple matched in place: each element is a scrutinee of its
            // own, and binds as a `match` on it alone would.
            ExprKind::Match { scrutinee, arms }
                if let ExprKind::Places(elements) = &body.exprs[*scrutinee].kind =>
            {
                for &element in elements {
                    let ctx = if self.place(element).is_some() {
                        Ctx::Inspect
                    } else {
                        Ctx::Value
                    };
                    self.expr(element, ctx, state);
                }
                let mut joined: Option<State> = None;
                for arm in arms {
                    let patterns = arm.pattern.elements(elements.len());
                    let mut arm_state = state.clone();
                    let mark = self.borrows.len();
                    for (pattern, &element) in patterns.iter().zip(elements) {
                        self.bind_pattern(pattern, element, Holder::Match, &mut arm_state);
                        self.bind_views(pattern, element, &mut arm_state);
                    }
                    self.expr(arm.body, Ctx::Value, &mut arm_state);
                    for pattern in &patterns {
                        self.arm_ends(pattern, arm.body, &mut arm_state);
                    }
                    self.borrows.truncate(mark);
                    let returns = self.ty(arm.body) == Types::NEVER;
                    if !returns {
                        joined = Some(match joined {
                            Some(prev) => prev.merge(&arm_state),
                            None => arm_state,
                        });
                    }
                }
                if let Some(joined) = joined {
                    *state = joined;
                }
            }
            // Read only by the `match` above.
            ExprKind::Places(elements) => {
                for &element in elements {
                    self.expr(element, Ctx::Value, state);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                let ctx = if self.place(*scrutinee).is_some() {
                    Ctx::Inspect
                } else {
                    Ctx::Value
                };
                self.expr(*scrutinee, ctx, state);
                let mut joined: Option<State> = None;
                for arm in arms {
                    let mut arm_state = state.clone();
                    let mark = self.borrows.len();
                    self.bind_pattern(&arm.pattern, *scrutinee, Holder::Match, &mut arm_state);
                    self.bind_views(&arm.pattern, *scrutinee, &mut arm_state);
                    self.expr(arm.body, Ctx::Value, &mut arm_state);
                    self.arm_ends(&arm.pattern, arm.body, &mut arm_state);
                    self.borrows.truncate(mark);
                    let returns = self.ty(arm.body) == Types::NEVER;
                    if !returns {
                        joined = Some(match joined {
                            Some(prev) => prev.merge(&arm_state),
                            None => arm_state,
                        });
                    }
                }
                if let Some(joined) = joined {
                    *state = joined;
                }
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            // A constant table is kept once and never written, and what is
            // read from it is copied.
            | ExprKind::Table(_)
            // A C pointer is plain data: it is copied, never dropped, and
            // the move checker leaves it alone.
            | ExprKind::Null
            | ExprKind::Zeroed
            | ExprKind::FnRef { .. }
            | ExprKind::Error => {}
            // A panic ends the program: nothing after it runs, and nothing
            // is dropped. A note built for it is read there.
            ExprKind::Panic { note, .. } => {
                if let Some(note) = note {
                    self.expr(*note, Ctx::Value, state);
                }
            }
        }
    }

    /// What an arm's pattern bound as a value of its own is dropped where
    /// the arm ends: the arm's value may not borrow it.
    fn arm_ends(&mut self, pattern: &Pattern, value: ExprId, state: &mut State) {
        let mut locals = Vec::new();
        pattern.locals(&mut locals);
        let owned: Vec<LocalId> = locals
            .into_iter()
            .filter(|l| !self.body.alias_bindings.contains(l))
            .collect();
        if self.ty(value) != Types::NEVER {
            self.check_block_value(value, &owned, "arm", state);
        }
        let span = self.body.exprs[value].span;
        let end = Span::new(span.hi.saturating_sub(1), span.hi);
        for local in owned {
            let path = Path {
                local,
                projs: Vec::new(),
            };
            self.changed(&path, end, "dropped", state);
        }
    }

    /// `place = value` and `place op= value`, in the order they run. What
    /// the place's expression computes comes first; then, for `op=`, the
    /// place is found and its old value read, and the value may not change
    /// what it is in (E0414). For `=` the value comes next, and the place
    /// is found where it is written, in what the value left: the value may
    /// change what it is in, but not what its expression lends (E0414),
    /// nor move what it is found from (E0408).
    fn assignment(&mut self, place: ExprId, compound: bool, value: ExprId, state: &mut State) {
        let body = self.body;
        self.place_parts(place, state);
        let found = self
            .place(place)
            .expect("an assignment's target is a place");
        if compound && let Place::Path(path) = &found {
            let name = self.display(place);
            self.check_live(path, &name, body.exprs[place].span, state);
            self.read_after_copy(path.local, false, state);
        }
        let mark = self.borrows.len();
        if compound {
            if let Some(path) = self.access_path(place) {
                self.borrows.push(Borrow {
                    heap: false,
                    path,
                    span: body.exprs[place].span,
                    var: true,
                    holder: Holder::Assign { target: place },
                });
            }
        } else {
            for (path, span, var) in self.operand_borrows(place, state) {
                self.borrows.push(Borrow {
                    heap: false,
                    path,
                    span,
                    var,
                    holder: Holder::Operand { target: place },
                });
            }
        }
        self.expr(value, Ctx::Value, state);
        self.borrows.truncate(mark);
        // The place is found now: what it is found from must still be
        // there. A variable's own fields are checked as it is assigned.
        if !compound
            && !matches!(found, Place::Path(_))
            && let Some(path) = self.access_path(place)
        {
            let name = self.root_name(&path);
            self.check_live(&path, &name, body.exprs[place].span, state);
        }
        self.use_place(place, Ctx::Assign, state);
        // A `str` variable borrows what its new value borrows, and so does
        // one holding a `&` reference; one assigned through a
        // `&var` parameter goes to the caller.
        if self.borrows_as_view(self.ty(place))
            && let Some(path) = self.access_path(place)
        {
            let whole = matches!(body.exprs[place].kind, ExprKind::Local(_));
            if whole && body.locals[path.local].kind != LocalKind::Param {
                self.bind_str(path.local, value, state);
            } else if body.locals[path.local].kind != LocalKind::Param {
                self.add_str(path.local, value, state);
            } else if body.locals[path.local].kind == LocalKind::Param {
                self.check_leaving(value, borrowed::Leaving::Param(path.local), state);
            }
        }
    }

    /// A `@tailrec` function's call to itself is compiled as a jump back to
    /// the top, so what the frame would have dropped or deferred at its exit
    /// never runs. Both are errors here rather than silent leaks: drops are not
    /// reordered to make a call fit.
    fn check_tail_call(&mut self, span: Span, state: &State) {
        let live: Vec<LocalId> = self
            .body
            .params
            .iter()
            .copied()
            .chain(self.scopes.iter().flatten().copied())
            .collect();
        for local in live {
            let ty = self.body.locals[local].ty;
            if !self.owns(ty) {
                continue;
            }
            let path = Path {
                local,
                projs: Vec::new(),
            };
            // Gone already: moved out on every path to here.
            if state
                .conflict(&path)
                .is_some_and(|(moved, m)| *moved == path && m.definite)
            {
                continue;
            }
            let name = self.interner.resolve(self.body.locals[local].name);
            let diagnostic = Diagnostic::error(
                codes::TAIL_CALL_LEAVES_WORK,
                format!("`{name}` is still owned here, so this call cannot be a jump"),
                span,
                format!("`{name}` would be freed after this call returns"),
            )
            .with_secondary(self.body.locals[local].span, format!("`{name}` declared here"))
            .with_note(
                "a `@tailrec` function's call to itself reuses its frame, so nothing of the frame may be left to drop",
            )
            .with_help(format!(
                "move `{name}` into the call, or end its scope before it"
            ));
            self.report(diagnostic);
            return;
        }
        if self.deferred.iter().any(|defers| !defers.is_empty()) {
            let diagnostic = Diagnostic::error(
                codes::TAIL_CALL_LEAVES_WORK,
                "a `defer` is waiting here, so this call cannot be a jump",
                span,
                "the `defer` would run after this call returns",
            )
            .with_note(
                "a `@tailrec` function's call to itself reuses its frame, and a `defer` runs where the frame exits",
            );
            self.report(diagnostic);
        }
    }

    /// Evaluates the parts of a place that are not a path: the reference it
    /// is behind, an array and its index, or a temporary.
    pub(super) fn place_parts(&mut self, id: ExprId, state: &mut State) {
        match self.place(id).expect("place_parts is called on places") {
            Place::Path(_) => {}
            Place::ThroughRef { reference } => self.expr(reference, Ctx::Inspect, state),
            Place::Element { array, index } => {
                self.expr(array, Ctx::Inspect, state);
                self.expr(index, Ctx::Value, state);
            }
            Place::Temporary { base } => self.expr(base, Ctx::Value, state),
        }
    }

    /// What a place's expression lends, held from where it is written to
    /// where the place is used: each `&` argument of a projection the place
    /// is found through, and what a view argument borrows, but the place
    /// the projection lends from, which is lent where the place is used.
    /// Each with whether it is lent `&var`.
    pub(super) fn operand_borrows(&mut self, id: ExprId, state: &State) -> Vec<(Path, Span, bool)> {
        let body = self.body;
        let mut held = Vec::new();
        match body.exprs[id].kind {
            ExprKind::Field { base, .. }
            | ExprKind::Index { base, .. }
            | ExprKind::SubSlice { base, .. } => held.extend(self.operand_borrows(base, state)),
            ExprKind::Deref(pointer) => match body.exprs[pointer].kind {
                ExprKind::Call {
                    callee, ref args, ..
                } => {
                    let from = match self.program.fns[callee].projects {
                        Some(wip_hir::Lent::Param(from)) => Some(from as usize),
                        _ => None,
                    };
                    for (i, &arg) in args.iter().enumerate() {
                        let kind = self.program.types.kind(body.exprs[arg].ty);
                        let var = matches!(kind, TyKind::Ref(_, RefKind::Var));
                        match body.exprs[arg].kind {
                            ExprKind::Ref(inner) if Some(i) == from => {
                                held.extend(self.operand_borrows(inner, state))
                            }
                            ExprKind::Ref(inner) => {
                                if let Some(path) = self.access_path(inner) {
                                    held.push((path, body.exprs[arg].span, var));
                                }
                            }
                            _ if self.borrows_as_view(body.exprs[arg].ty) => {
                                let roots = self.lent_roots(arg, Lends::ALL, state);
                                held.extend(
                                    roots
                                        .paths
                                        .into_iter()
                                        .map(|path| (path, body.exprs[arg].span, false)),
                                );
                            }
                            _ => {}
                        }
                    }
                }
                _ => held.extend(self.operand_borrows(pointer, state)),
            },
            _ => {}
        }
        held
    }

    /// Checks a use of a place. The parts of an assignment's target were
    /// evaluated before its value, so they are not evaluated again.
    pub(super) fn use_place(&mut self, id: ExprId, ctx: Ctx, state: &mut State) {
        if matches!(ctx, Ctx::Move(_) | Ctx::Assign) {
            self.check_not_borrowed(id, ctx);
        }
        if ctx != Ctx::Assign {
            self.place_parts(id, state);
        }
        self.use_found(id, ctx, state);
    }

    /// Checks a use of a place whose parts are checked already: where it is
    /// lent to a call, after the arguments its expression was written
    /// before.
    pub(super) fn use_found(&mut self, id: ExprId, ctx: Ctx, state: &mut State) {
        let body = self.body;
        let (span, ty) = (body.exprs[id].span, body.exprs[id].ty);
        let name = self.display(id);
        let found = self.place(id).expect("use_place is called on places");
        if let Place::Path(path) = &found {
            self.read_after_copy(path.local, ctx == Ctx::Assign, state);
        }
        // A `str` is read wherever it is used, and so is
        // the reference a place lies behind, where it is kept.
        if ctx != Ctx::Assign {
            let holder = if self.is_view(ty) || self.is_kept_closure(ty) {
                self.access_path(id)
            } else {
                self.kept_reference_behind(id)
                    .and_then(|reference| self.access_path(reference))
            };
            if let Some(path) = holder {
                self.check_stale(path.local, &name, span, state);
            }
        }
        match ctx {
            Ctx::Move(move_span) => {
                if let Place::Path(path) = &found {
                    self.changed(path, move_span, "moved", state);
                }
            }
            Ctx::Assign => {
                if let Some(path) = self.access_path(id) {
                    self.changed(&path, span, "assigned", state);
                }
            }
            Ctx::Value | Ctx::Inspect => {}
        }
        match found {
            Place::Path(path) => match ctx {
                Ctx::Inspect => {
                    self.check_live(&path, &name, span, state);
                }
                Ctx::Value => {
                    if self.check_live(&path, &name, span, state) && self.owns(ty) {
                        // An atomic holds no memory, and is moved all the
                        // same.
                        let note = if self.program.is_intrinsic_type(ty) {
                            format!(
                                "`{}` is moved, never copied: a copy would read it in more than one step, which another thread could see half of",
                                self.ty_name(ty)
                            )
                        } else {
                            "a value that contains `own` has exactly one owner, so it is never copied".to_string()
                        };
                        let diagnostic = Diagnostic::error(
                            codes::CANNOT_COPY_OWN,
                            format!("cannot copy `{name}`"),
                            span,
                            format!("`{name}` has type `{}`", self.ty_name(ty)),
                        )
                        .with_note(note)
                        .with_fix(
                            format!("to transfer ownership, write `move {name}`"),
                            [Edit::insert(span.lo, "move ")],
                        );
                        self.report(diagnostic);
                    }
                }
                Ctx::Move(move_span) => {
                    if self.check_live(&path, &name, span, state) {
                        state.record_move(path, move_span, name, self.exit.is_some());
                    }
                }
                Ctx::Assign => self.assign(path, &name, span, state),
            },
            Place::ThroughRef { reference } => {
                let taken = match ctx {
                    Ctx::Move(_) => Some("move"),
                    Ctx::Value if self.owns(ty) => Some("copy"),
                    _ => None,
                };
                if let Some(verb) = taken {
                    let binding = matches!(
                        body.exprs[reference].kind,
                        ExprKind::Local(l) if body.locals[l].kind == LocalKind::Binding
                    );
                    // A `for` binding refers to an element.
                    let element = matches!(
                        body.exprs[reference].kind,
                        ExprKind::Local(l) if self.aliases.get(&l).is_some_and(|path| {
                            path.projs.iter().any(|p| matches!(p, Proj::Element))
                        }) || self.kept_elements.contains(&l)
                    );
                    let lent = matches!(body.exprs[reference].kind, ExprKind::Call { .. });
                    // A binding behind a reference the pattern tested
                    // through, or the matched value was behind: `match
                    // move` takes nothing from there either.
                    let kept = matches!(
                        body.exprs[reference].kind,
                        ExprKind::Local(l) if binding && self.kept_bindings.contains(&l)
                    );
                    // What a closure captured: its own, and kept, since it
                    // may be called again.
                    if self.is_capture(id) {
                        let diagnostic = Diagnostic::error(
                            codes::CANNOT_MOVE_OUT_OF_BORROW,
                            format!("cannot {verb} `{name}`, which this closure captured"),
                            span,
                            "captured, and kept while the closure lives",
                        )
                        .with_note(
                            "a closure may be called again, so what it captured stays where it is",
                        )
                        .with_help("take what is used up as a parameter of the closure");
                        self.report(diagnostic);
                        return;
                    }
                    let message = if lent {
                        format!("cannot {verb} what a projection lends")
                    } else {
                        format!("cannot {verb} `{name}`, which is behind a reference")
                    };
                    let mut diagnostic = Diagnostic::error(
                        codes::CANNOT_MOVE_OUT_OF_BORROW,
                        message,
                        span,
                        if element {
                            "a `for` binding, which refers to an element"
                        } else if binding {
                            "a `match` binding, which refers to part of the matched value"
                        } else if lent {
                            "lent by a projection, and owned by the caller"
                        } else {
                            "behind a `&` reference"
                        },
                    )
                    .with_note("a reference can read a value but cannot take ownership of it");
                    if kept {
                        diagnostic = diagnostic.with_help(
                            "it lies behind a reference, which neither `move` nor `match move` takes from; copy it with `clone()`",
                        );
                    } else if element {
                        diagnostic = diagnostic.with_help(
                            "elements stay where they are while a loop walks them; to take one out, move the whole array",
                        );
                    } else if binding {
                        diagnostic = diagnostic
                            .with_help("to take the value apart, match it with `match move …`");
                    } else if lent {
                        diagnostic = diagnostic.with_help(
                            "a projection lends a place that belongs to its caller; nothing is taken out of it",
                        );
                    }
                    self.report(diagnostic);
                }
            }
            Place::Element { .. } => {
                let taken = match ctx {
                    Ctx::Move(_) => Some("move out of"),
                    Ctx::Value if self.owns(ty) => Some("copy"),
                    _ => None,
                };
                if let Some(verb) = taken {
                    let diagnostic = Diagnostic::error(
                        codes::CANNOT_MOVE_OUT_OF_ARRAY,
                        format!("cannot {verb} an array element that contains `own`"),
                        span,
                        "an element of an array",
                    )
                    .with_note("taking one element would leave a hole in the array; an array of `own` values is moved as a whole");
                    self.report(diagnostic);
                }
            }
            Place::Temporary { .. } => {
                if ctx == Ctx::Value && self.owns(ty) {
                    let diagnostic = Diagnostic::error(
                        codes::CANNOT_MOVE_OUT_OF_TEMPORARY,
                        format!("cannot take `{name}` out of a temporary"),
                        span,
                        "part of a temporary value",
                    )
                    .with_help("bind the temporary with `val`, then `move` this part out of it");
                    self.report(diagnostic);
                }
            }
        }
    }

    /// Binds a pattern's names to the fields of `scrutinee`, the value tested.
    /// An alias binding refers into the place, which must not change while the
    /// bindings are in scope, so a borrow is pushed, for the caller to release;
    /// an owned binding is a new variable. A place behind a kept `&` reference
    /// is not this function's: its aliases are kept references too, which
    /// borrow what that one does and hold nothing of the variable, which may be
    /// pointed somewhere else while they are used.
    pub(super) fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee: ExprId,
        holder: Holder,
        state: &mut State,
    ) {
        let body = self.body;
        let scrut_path = self.access_path(scrutinee);
        let scrut_path = scrut_path.as_ref();
        let scrut_span = body.exprs[scrutinee].span;
        // Each binding, and the place it refers into, at any depth.
        let mut binders: Vec<(LocalId, Option<Path>)> = Vec::new();
        crate::bindings_of(pattern, scrut_path.cloned(), &mut binders);
        // An alias behind a reference the pattern tests through is a kept
        // reference, which borrows what the value tested does.
        let mut behind = Vec::new();
        crate::behind_references(pattern, &mut behind);
        for local in behind {
            if body.alias_bindings.contains(&local) {
                self.kept_bindings.insert(local);
            }
        }
        if let Some(reference) = self.kept_reference_behind(scrutinee) {
            let roots = self.str_roots(reference, state).paths;
            for (local, _) in binders {
                if body.alias_bindings.contains(&local) {
                    self.kept_bindings.insert(local);
                    state.roots.insert(local, roots.clone());
                    state.stale.remove(&local);
                } else {
                    state.reinit(&Path {
                        local,
                        projs: Vec::new(),
                    });
                }
            }
            return;
        }
        let mut aliased = false;
        for (local, path) in binders {
            if body.alias_bindings.contains(&local) {
                if let Some(path) = path {
                    self.aliases.insert(local, path);
                    aliased = true;
                }
            } else {
                state.reinit(&Path {
                    local,
                    projs: Vec::new(),
                });
            }
        }
        if aliased && let Some(path) = scrut_path {
            self.borrows.push(Borrow {
                heap: false,
                path: path.clone(),
                span: scrut_span,
                var: false,
                holder,
            });
        }
    }
}
