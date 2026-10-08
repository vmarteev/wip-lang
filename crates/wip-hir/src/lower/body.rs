//! Function bodies, blocks and statements, and `if`.

use super::*;

/// Whether an `if`, followed along its `else if` chain, ends without `else`.
pub(super) fn lacks_else(ast: &Ast, e: ast::ExprId) -> bool {
    match ast.exprs[e].kind {
        ast::ExprKind::If {
            else_branch: None, ..
        } => true,
        ast::ExprKind::If {
            else_branch: Some(next),
            ..
        } => lacks_else(ast, next),
        _ => false,
    }
}

/// The `-` that an expression statement starts with, if any.
pub(super) fn leading_neg(ast: &Ast, stmt: ast::StmtId) -> Option<Span> {
    let ast::StmtKind::Expr(mut e) = ast.stmts[stmt].kind else {
        return None;
    };
    loop {
        match ast.exprs[e].kind {
            ast::ExprKind::Unary {
                op: ast::UnaryOp::Neg,
                op_span,
                ..
            } => return Some(op_span),
            ast::ExprKind::Binary { lhs, .. } => e = lhs,
            ast::ExprKind::Cast { expr, .. } => e = expr,
            _ => return None,
        }
    }
}

/// What a `for` found to walk.
pub(super) enum Walked {
    /// The slice it lends, and what one of its elements is.
    Items(ExprId, Ty),
    /// Nothing, and nothing reported: the caller says so.
    No,
}

impl<'a> Lowerer<'a> {
    /// Checks one body: a function's, a method's, or an interface method's
    /// default. Where a constraint pins a decided type — `I: Iterator<T>`,
    /// and `Self: Iterator<Item>` in the interface's own methods — `I::Item`
    /// is that type in the body.
    pub(super) fn check_fn(&mut self, body: ast::ExprId, id: FnId) {
        let generics = self.program.fns[id].generics.clone();
        let pins = self.decided_pins(&generics);
        self.program.types.set_pins(pins);
        self.check_body_of(body, id);
        self.program.types.set_pins(Vec::new());
    }

    /// The decided types the constraints of `generics` pin: `I::Item` is `T`
    /// under `I: Iterator<T>`.
    pub(super) fn decided_pins(
        &mut self,
        generics: &[GenericParamDef],
    ) -> Vec<(Ty, crate::InterfaceId, u32, Ty)> {
        let mut pins = Vec::new();
        for (index, param) in generics.iter().enumerate() {
            let ty = self.intern(TyKind::Param(crate::TyParam {
                index: index as u32,
                name: param.name,
                copy: param.copy,
            }));
            for constraint in &param.interfaces {
                let decided = &self.program.interfaces[constraint.interface].generics;
                let args = self.program.types.list(constraint.args);
                for (at, (&arg, param)) in args.iter().zip(decided).enumerate() {
                    let left_to_it = matches!(self.program.types.kind(arg), TyKind::Assoc(base, ..) if base == ty);
                    if param.decided && !left_to_it {
                        pins.push((ty, constraint.interface, at as u32, arg));
                    }
                }
            }
        }
        pins
    }

    fn check_body_of(&mut self, body: ast::ExprId, id: FnId) {
        let def = &self.program.fns[id];
        self.type_params = def.generics.clone();
        self.derived = self.program.derived.contains(&id);
        let owner = def.owner;
        // An expression's id is its file's, so what the last body noted of
        // its arguments must not be read as this one's.
        self.state.named_by_equals.clear();
        self.state.index_operands.clear();
        // A function that yields answers a generator.
        if let Some(gen_id) = self.generator_of_fn(id) {
            self.self_ty = owner.map(|owner| self.self_ty(owner));
            self.state.lent_half = false;
            self.state.lends = 0;
            self.state.lent_param = None;
            self.state.lent_table = false;
            self.check_generator_fn(body, id, gen_id);
            return;
        }
        self.state.ret = def.ret;
        self.state.ret_span = def.ret_span;
        self.state.lent_half = self.program.lent_halves.contains(&id);
        let params = def.params.clone();
        // `Self` in the body is the type the method belongs to.
        self.self_ty = owner.map(|owner| self.self_ty(owner));
        self.state.body = Body::default();
        self.state.scopes = vec![FxHashMap::default()];
        self.state.lends = 0;
        self.state.lent_param = None;
        self.state.lent_table = false;
        for p in &params {
            let local = self.declare(p.name, p.ty, LocalKind::Param, p.name_span);
            self.state.body.params.push(local);
        }
        let value = if self.state.ret == Types::UNIT {
            self.unit_body(body, id)
        } else {
            self.value_body(body, id)
        };
        self.state.body.value = Some(value);
        if matches!(self.kind(self.state.ret), TyKind::Ref(..)) {
            self.check_projection(id, value);
        }
        // `@tailrec`: its calls to itself must be tail calls.
        if self.program.fns[id].is_tailrec {
            self.check_tailrec(id, value);
        }
        self.settle_bindings();
        self.program.fns[id].body = Some(std::mem::take(&mut self.state.body));
    }

    /// The body of a function that returns nothing. Its value, if it has one,
    /// is discarded.
    pub(super) fn unit_body(&mut self, body: ast::ExprId, id: FnId) -> ExprId {
        let value = self.infer(body, Some(Types::UNIT));
        let ty = self.ty_of(value);
        if ty == Types::UNIT || self.is_poisoned(ty) {
            return value;
        }
        if !self.has_effect(value) {
            // Most likely a forgotten return type: `fn add(a: i64, b: i64) = a + b`.
            let name = self.program.ty_name(ty, self.interner);
            let def = &self.program.fns[id];
            let (fn_name, sig_span) = (def.name, def.span);
            let diagnostic = Diagnostic::warning(
                codes::DISCARDED_VALUE,
                format!("`{}` computes a value and discards it", self.text(fn_name)),
                self.state.body.exprs[value].span,
                format!("a value of type `{name}` that is never used"),
            )
            .with_note("a function without `: T` returns nothing")
            .with_fix(
                format!("to return it, declare the return type: `: {name}`"),
                [Edit::insert(sig_span.hi, format!(": {name}"))],
            );
            self.report(diagnostic);
        }
        self.wrap_discarded(value)
    }

    /// The body of a function that returns a value: its value is the result.
    pub(super) fn value_body(&mut self, body: ast::ExprId, id: FnId) -> ExprId {
        let ret = self.state.ret;
        let value = self.infer(body, Some(ret));
        let expr = &self.state.body.exprs[value];
        // A block that ends with a statement instead of a value.
        if let ExprKind::Block(block) = &expr.kind
            && block.value.is_none()
            && expr.ty == Types::UNIT
        {
            if !self.has_error(ret) {
                let end = Span::new(block.span.hi.saturating_sub(1), block.span.hi);
                let mut diagnostic = Diagnostic::error(
                    codes::MISSING_RETURN,
                    format!(
                        "`{}` can reach its end without returning a value",
                        self.text(self.program.fns[id].name)
                    ),
                    end,
                    "reaches the end here",
                );
                if let Some(ret_span) = self.state.ret_span {
                    diagnostic = diagnostic.with_secondary(
                        ret_span,
                        format!("declared to return {}", self.ty_name(ret)),
                    );
                }
                self.report(diagnostic);
            }
            return value;
        }
        let context = self
            .state
            .ret_span
            .map(|s| (s, "return type declared here"));
        self.coerce(value, ret, context)
    }

    pub(super) fn declare(&mut self, name: Symbol, ty: Ty, kind: LocalKind, span: Span) -> LocalId {
        let id = self.state.body.locals.alloc(Local {
            name,
            ty,
            kind,
            span,
        });
        self.state
            .scopes
            .last_mut()
            .expect("inside a scope")
            .insert(name, id);
        id
    }

    pub(super) fn lookup(&self, name: Symbol) -> Option<LocalId> {
        self.state
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&name).copied())
    }

    /// Checks a block. Its last expression is its value, checked against
    /// `expected` — unless `()` is expected, in which case it is discarded like
    /// any other statement. Returns the block and its type,
    /// which is `!` if the block always returns.
    pub(super) fn check_block(
        &mut self,
        block: &'a ast::Block,
        expected: Option<Ty>,
    ) -> (Block, Ty) {
        self.block_of(block, expected, false)
    }

    /// A block that is a call's argument, whose value is the argument's:
    /// the value it answers, having left the block, is lent as a `str` or
    /// for the call where the argument would be.
    /// An interpolated literal is one of these. An `&`
    /// written as its value is not the argument's, since it could point at
    /// what the block itself holds.
    pub(super) fn argument_block(
        &mut self,
        block: &'a ast::Block,
        expected: Option<Ty>,
    ) -> (Block, Ty) {
        self.block_of(block, expected, true)
    }

    fn block_of(
        &mut self,
        block: &'a ast::Block,
        expected: Option<Ty>,
        argument: bool,
    ) -> (Block, Ty) {
        let ast = self.ast;
        self.state.scopes.push(FxHashMap::default());
        let mut stmts = Vec::new();
        let mut value = None;
        let mut diverges = false;
        let last = block.stmts.len().checked_sub(1);
        for (i, &stmt) in block.stmts.iter().enumerate() {
            if Some(i) == last
                && expected != Some(Types::UNIT)
                && let ast::StmtKind::Expr(e) = ast.stmts[stmt].kind
                // An `if` without `else` has no value: where one is expected,
                // it is a statement and the block falls off its end.
                && !(expected.is_some() && lacks_else(ast, e))
            {
                let v = match expected {
                    // A block that is an argument answers its value as it
                    // is: what the argument is lent as is the call's to
                    // say, of the value the block answers, which has
                    // left the block.
                    Some(t) if argument => self.infer(e, Some(t)),
                    Some(t) => self.check(e, t),
                    None => self.infer(e, None),
                };
                diverges |= self.ty_of(v) == Types::NEVER;
                value = Some(v);
                break;
            }
            // A statement that starts with `-` may be meant to continue
            // the expression above it, which the warning says.
            let next_neg = match ast.stmts[stmt].kind {
                ast::StmtKind::Expr(_) => block
                    .stmts
                    .get(i + 1)
                    .and_then(|&next| leading_neg(ast, next)),
                _ => None,
            };
            let (id, d) = self.stmt(stmt, next_neg);
            stmts.push(id);
            diverges |= d;
        }
        self.state.scopes.pop();
        let ty = match value {
            _ if diverges => Types::NEVER,
            // The value was checked against `expected`, and any mismatch
            // reported there; an argument's is checked by the call.
            Some(v) if argument => self.ty_of(v),
            Some(v) => expected.unwrap_or(self.ty_of(v)),
            None => Types::UNIT,
        };
        let block = Block {
            stmts,
            value,
            span: block.span,
        };
        (block, ty)
    }

    /// An expression statement, whose value is not used. `if`, `match` and
    /// blocks are checked with `()` expected, so the values of their branches
    /// are discarded too. `next_neg` is where the statement after it starts
    /// with `-`, which the warning for a pointless value mentions.
    pub(super) fn discarded(&mut self, e: ast::ExprId, next_neg: Option<Span>) -> ExprId {
        let block_like = matches!(
            self.ast.exprs[e].kind,
            ast::ExprKind::Block(_) | ast::ExprKind::If { .. } | ast::ExprKind::Match { .. }
        );
        let id = self.infer(e, block_like.then_some(Types::UNIT));
        self.warn_if_pointless(id, next_neg);
        id
    }

    /// Discards the value of `id` where `()` is expected.
    pub(super) fn discard(&mut self, id: ExprId) -> ExprId {
        let ty = self.ty_of(id);
        if ty == Types::UNIT || self.is_poisoned(ty) {
            return id;
        }
        self.warn_if_pointless(id, None);
        self.wrap_discarded(id)
    }

    /// A block of type `()` whose one statement evaluates `id` and drops its
    /// value.
    pub(super) fn wrap_discarded(&mut self, id: ExprId) -> ExprId {
        let span = self.state.body.exprs[id].span;
        let stmt = self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Expr(id),
            span,
        });
        let block = Block {
            stmts: vec![stmt],
            value: None,
            span,
        };
        self.alloc(ExprKind::Block(block), Types::UNIT, span)
    }

    /// E0320: a value computed without side effects and never used.
    /// Warns about a discarded value that has no effect. `next_neg` is the `-`
    /// starting the next statement, if there is one.
    pub(super) fn warn_if_pointless(&mut self, id: ExprId, next_neg: Option<Span>) {
        let ty = self.ty_of(id);
        if ty == Types::UNIT || self.is_poisoned(ty) || self.has_effect(id) {
            return;
        }
        let expr = &self.state.body.exprs[id];
        let mut diagnostic = Diagnostic::warning(
            codes::DISCARDED_VALUE,
            "this value is computed and never used",
            expr.span,
            format!("a value of type {}", self.ty_name(ty)),
        );
        if let Some(neg) = next_neg {
            diagnostic = diagnostic
                .with_secondary(neg, "this `-` starts a new statement")
                .with_note("a line break ends a complete expression, so a line that starts with `-` is a new statement")
                .with_help("to subtract, move `-` to the end of the previous line");
        } else if let ExprKind::Unary {
            op: UnaryOp::Neg, ..
        } = expr.kind
        {
            diagnostic = diagnostic
                .with_note("a line break ends a complete expression, so a line that starts with `-` is a new statement")
                .with_help("to continue the previous line, move `-` to its end");
        }
        self.report(diagnostic);
    }

    /// Whether evaluating `id` does anything besides producing its value: a
    /// call, an assignment, a move, an allocation, or any statement.
    pub(super) fn has_effect(&self, id: ExprId) -> bool {
        let has = |e: ExprId| self.has_effect(e);
        let kind = &self.state.body.exprs[id].kind;
        // A block runs its statements whether or not its value is used, and
        // the same goes for the branches of an `if`.
        let block_has = |b: &Block| !b.stmts.is_empty() || b.value.is_some_and(has);
        match kind {
            ExprKind::Call { .. }
            | ExprKind::CallClosure { .. }
            | ExprKind::DynCall { .. }
            | ExprKind::CallValue { .. }
            | ExprKind::Assign { .. }
            | ExprKind::Move(_)
            | ExprKind::Own(_)
            | ExprKind::OwnRepeat { .. }
            | ExprKind::Match { .. }
            | ExprKind::Lend(_)
            | ExprKind::Panic { .. }
            | ExprKind::Error => true,
            // Everything else does what the expressions inside it do.
            _ => kind.children().into_iter().any(has) || kind.blocks().into_iter().any(block_has),
        }
    }

    /// A statement, and whether it never finishes. `next_neg` is what an
    /// expression statement passes to [`Self::discarded`].
    pub(super) fn stmt(&mut self, id: ast::StmtId, next_neg: Option<Span>) -> (StmtId, bool) {
        let ast = self.ast;
        let stmt = &ast.stmts[id];
        let (kind, diverges) = match &stmt.kind {
            ast::StmtKind::Yield(value) => (self.yield_stmt(*value, stmt.span), false),
            ast::StmtKind::Let {
                mutable,
                name,
                ty,
                init,
            } => {
                let (init, ty) = match ty {
                    Some(t) => {
                        let mut ty = self.resolve_ty(*t);
                        // A `val` may keep a closure that reads what it
                        // captures: `&(…) => R`, which a
                        // `var` may not. A `&` reference is a view, kept as
                        // one, and a `&var` is a parameter's.
                        let lent_closure = matches!(self.kind(ty), TyKind::Ref(inner, _)
                                if matches!(self.kind(inner), TyKind::Fn(..)));
                        if (*mutable
                            && lent_closure
                            && self.no_ref(ty, *t, "a `var` cannot hold a lent closure", None))
                            || self.no_var_ref(ty, *t, "a variable")
                            || self.no_never(ty, *t, "a variable")
                            || self.bare_slice(ty, *t, false)
                        {
                            ty = Types::ERROR;
                        }
                        let context = (ast.types[*t].span, "expected because of this type");
                        (self.check_in(*init, ty, Some(context)), ty)
                    }
                    None => {
                        // `val f = (x) => …` may keep what it captures.
                        self.state.keeping_closure = !*mutable
                            && matches!(ast.exprs[*init].kind, ast::ExprKind::Lambda { .. });
                        // `val r = &place` holds a reference, as `val r: &T
                        // = &place` does.
                        self.state.local_reference = matches!(
                            ast.exprs[*init].kind,
                            ast::ExprKind::Unary {
                                op: ast::UnaryOp::Ref,
                                ..
                            }
                        );
                        let checked = self.infer(*init, None);
                        self.state.keeping_closure = false;
                        self.state.local_reference = false;
                        let mut ty = self.state.body.exprs[checked].ty;
                        // `val r = if c { 1 }`: an `if` without `else` has
                        // no value to give, which is said here rather than
                        // where `r` is first used.
                        if ty == Types::UNIT
                            && matches!(ast.exprs[*init].kind, ast::ExprKind::If { .. })
                            && lacks_else(ast, *init)
                        {
                            let diagnostic = Diagnostic::error(
                                codes::NO_VALUE,
                                "an `if` without `else` has no value",
                                ast.exprs[*init].span,
                                "nothing when the condition does not hold",
                            )
                            .with_help("give it an `else`, or say that the value may not be there: `if c { .Some(x) } else { .None }`")
                            .with_note("an `if` without `else` is a statement; what may be missing is an `Option`");
                            // That its branch's value goes unused is this
                            // error again.
                            let inside = ast.exprs[*init].span;
                            self.diagnostics.retain(|d| {
                                d.code != codes::DISCARDED_VALUE
                                    || !(inside.lo <= d.primary.span.lo
                                        && d.primary.span.hi <= inside.hi)
                            });
                            self.report(diagnostic);
                            ty = Types::ERROR;
                        }
                        (checked, ty)
                    }
                };
                // A slice read through a reference is kept as the
                // reference; any other slice stands for elements that belong
                // to the caller, with nothing of fixed size to copy.
                let (init, ty) = match self.kept_reference(init) {
                    Some(reference) => (reference, self.ty_of(reference)),
                    None => (init, ty),
                };
                let ty = if matches!(self.kind(ty), TyKind::Slice(_)) {
                    let diagnostic = Diagnostic::error(
                        codes::UNSIZED_SLICE,
                        "a slice cannot be stored in a variable",
                        self.state.body.exprs[init].span,
                        format!("has type {}", self.ty_name(ty)),
                    )
                    .with_note("a slice is a view of elements that belong to someone else, so it exists only as a `&` or `&var` argument")
                    .with_help("use it directly, or pass it on with `&`");
                    self.report(diagnostic);
                    Types::ERROR
                } else {
                    ty
                };
                let kind = if *mutable {
                    LocalKind::Var
                } else {
                    LocalKind::Let {
                        keyword: Span::new(stmt.span.lo, stmt.span.lo + 3),
                    }
                };
                let local = self.declare(name.sym, ty, kind, name.span);
                (StmtKind::Let { local, init }, ty == Types::NEVER)
            }
            ast::StmtKind::Defer(e) => {
                let keyword = Span::new(stmt.span.lo, stmt.span.lo + 5);
                self.state.defers.push((keyword, self.state.loops.len()));
                let e = self.discarded(*e, None);
                self.state.defers.pop();
                (StmtKind::Defer(e), false)
            }
            ast::StmtKind::Return(value) => {
                if let Some(&(keyword, _)) = self.state.defers.last() {
                    let diagnostic = Diagnostic::error(
                        codes::RETURN_IN_DEFER,
                        "cannot `return` from a deferred expression",
                        stmt.span,
                        "would leave the function in the middle of its cleanup",
                    )
                    .with_secondary(keyword, "inside this `defer`")
                    .with_note("a `defer` runs while its block exits: it cannot replace the result or skip the cleanup after it");
                    self.report(diagnostic);
                }
                // A loop that is a generator has no function to leave.
                self.return_in_generator(stmt.span);
                (StmtKind::Return(self.return_value(*value, stmt.span)), true)
            }
            ast::StmtKind::Guard {
                mutable,
                pattern,
                value,
                else_block,
            } => (
                self.guard(*mutable, pattern, *value, else_block.as_ref()),
                false,
            ),
            ast::StmtKind::While { label, cond, body } => {
                let forever = matches!(ast.exprs[*cond].kind, ast::ExprKind::Bool(true));
                self.state.scopes.push(FxHashMap::default());
                let cond = self.binding_condition(*cond);
                self.push_loop(label.as_ref());
                let (body, _) = self.check_block(body, Some(Types::UNIT));
                let (_, breaks) = self.state.loops.pop().expect("pushed above");
                self.state.scopes.pop();
                // `while true` with no `break` never finishes normally.
                (StmtKind::While { cond, body }, forever && !breaks)
            }
            ast::StmtKind::For {
                label,
                binding,
                source,
                body,
            } => (self.for_stmt(label.as_ref(), binding, *source, body), false),
            ast::StmtKind::Break(label) | ast::StmtKind::Continue(label) => {
                let is_break = matches!(stmt.kind, ast::StmtKind::Break(_));
                let word = if is_break { "break" } else { "continue" };
                // Which loop it leaves: the innermost, or the one whose
                // name it wrote. Outside a loop there is
                // no name to look for; that is reported below.
                let depth = match label {
                    Some(label) if !self.state.loops.is_empty() => self.loop_named(*label, word),
                    Some(_) => None,
                    None => Some(0),
                };
                let target = self
                    .state
                    .loops
                    .len()
                    .saturating_sub(1 + depth.unwrap_or(0) as usize);
                if self.state.loops.is_empty() {
                    let diagnostic = Diagnostic::error(
                        codes::LOOP_JUMP_OUTSIDE,
                        format!("`{word}` outside a loop"),
                        stmt.span,
                        "not inside `while` or `for`",
                    )
                    .with_note(
                        "`break` leaves the innermost loop, and `continue` starts its next pass",
                    );
                    self.report(diagnostic);
                } else if let Some(&(keyword, loops)) = self.state.defers.last()
                    && loops > target
                {
                    let diagnostic = Diagnostic::error(
                        codes::JUMP_IN_DEFER,
                        format!("cannot `{word}` from a deferred expression"),
                        stmt.span,
                        "would leave the loop in the middle of a block's cleanup",
                    )
                    .with_secondary(keyword, "inside this `defer`")
                    .with_note("a `defer` runs while its block exits, and cannot jump elsewhere; a loop written inside the deferred expression may use `break` and `continue`");
                    self.report(diagnostic);
                } else if is_break {
                    self.state.loops[target].1 = true;
                }
                let depth = depth.unwrap_or(0);
                let kind = if is_break {
                    StmtKind::Break { depth }
                } else {
                    StmtKind::Continue { depth }
                };
                (kind, true)
            }
            ast::StmtKind::Expr(e) => {
                let e = self.discarded(*e, next_neg);
                (
                    StmtKind::Expr(e),
                    self.state.body.exprs[e].ty == Types::NEVER,
                )
            }
        };
        (
            self.state.body.stmts.alloc(Stmt {
                kind,
                span: stmt.span,
            }),
            diverges,
        )
    }

    /// What one element of the slice a container lends is, where it lends one
    /// kind: what `items_of` finds, without the call, for inference and for an
    /// argument lent as its elements. Nothing for a type that lends none.
    pub(super) fn items_element(&self, ty: Ty) -> Option<Ty> {
        let interface = self
            .program
            .prelude_items
            .interface(KnownInterface::Items)?;
        if let TyKind::Param(param) = self.kind(ty) {
            return self
                .type_params
                .get(param.index as usize)
                .and_then(|p| p.interfaces.iter().find(|c| c.interface == interface))
                .and_then(|c| self.program.types.list(c.args).first().copied());
        }
        let owner = self.owner_of(ty)?;
        let items = *self
            .program
            .impls
            .iter()
            .find(|i| i.interface == interface && i.ty == owner)?
            .methods
            .first()?;
        let args: Vec<Ty> = match self.kind(ty) {
            TyKind::Struct(_, args) | TyKind::Enum(_, args) => {
                self.program.types.list(args).to_vec()
            }
            _ => Vec::new(),
        };
        // The element alone is substituted: the slice of it may be a type
        // nothing has made yet, which a lookup cannot find.
        let TyKind::Ref(slice, _) = self.kind(self.program.fns[items].ret) else {
            return None;
        };
        let TyKind::Slice(elem) = self.kind(slice) else {
            return None;
        };
        self.program.types.try_subst_find(elem, &args)
    }

    /// The elements a container lends, as the slice `for` walks, and what
    /// one of them is. A type is walked through its
    /// implementation of the prelude's `Items`, which is a projection, so
    /// what the loop visits belongs to the container.
    pub(super) fn items_of(&mut self, receiver: ExprId, ty: Ty) -> Walked {
        let Some(interface) = self.program.prelude_items.interface(KnownInterface::Items) else {
            return Walked::No;
        };
        // A type parameter lends its elements because a constraint says so,
        // and the call goes through the interface's own method, which its
        // instance replaces.
        let (items, args) = match self.kind(ty) {
            TyKind::Param(param) => {
                let found = self
                    .type_params
                    .get(param.index as usize)
                    .and_then(|p| p.interfaces.iter().find(|c| c.interface == interface))
                    .and_then(|c| self.program.types.list(c.args).first().copied());
                let Some(elem) = found else {
                    return Walked::No;
                };
                let method = self.program.interfaces[interface].methods[0].id;
                (method, vec![ty, elem])
            }
            _ => {
                let Some(owner) = self.owner_of(ty) else {
                    return Walked::No;
                };
                // Its element is what its implementation decides, and it
                // has one.
                let Some(items) = self
                    .program
                    .impls
                    .iter()
                    .find(|i| i.interface == interface && i.ty == owner)
                    .and_then(|i| i.methods.first())
                    .copied()
                else {
                    return Walked::No;
                };
                // A method of a generic container is generic in the
                // container's own parameters.
                let args: Vec<Ty> = match self.kind(ty) {
                    TyKind::Struct(_, args) | TyKind::Enum(_, args) => {
                        self.program.types.list(args).to_vec()
                    }
                    _ => Vec::new(),
                };
                (items, args)
            }
        };
        let type_args = self.program.types.intern_list(&args);
        let ret = self.program.types.subst(self.program.fns[items].ret, &args);
        let TyKind::Ref(slice, _) = self.kind(ret) else {
            return Walked::No;
        };
        let TyKind::Slice(elem) = self.kind(slice) else {
            return Walked::No;
        };
        let span = self.state.body.exprs[receiver].span;
        // The container is lent for the call, as `c.items()` lends it.
        let borrowed = self.intern(TyKind::Ref(ty, crate::RefKind::Shared));
        let lent = self.alloc(ExprKind::Ref(receiver), borrowed, span);
        let call = self.alloc(
            ExprKind::Call {
                callee: items,
                args: vec![lent],
                type_args,
                order: vec![0],
            },
            ret,
            span,
        );
        // A projection's call is a place.
        Walked::Items(self.alloc(ExprKind::Deref(call), slice, span), elem)
    }

    /// `for` over elements or a range.
    pub(super) fn for_stmt(
        &mut self,
        label: Option<&ast::Name>,
        binding: &'a ast::Pattern,
        source: ast::ForSource,
        body: &'a ast::Block,
    ) -> StmtKind {
        match source {
            // `for x in move c`: `c` gives up its elements, and each is the
            // loop's to keep; what a loop that stops early leaves is dropped
            // with the iterator that holds it.
            ast::ForSource::Elements(elements)
                if matches!(
                    self.ast.exprs[elements].kind,
                    ast::ExprKind::Unary {
                        op: ast::UnaryOp::Move,
                        ..
                    }
                ) =>
            {
                let moved = self.operand(elements, None);
                let ty = self.ty_of(moved);
                if let Some((taken, next)) = self.given_up_iterator(moved) {
                    return self.walk_iterator(label, binding, taken, next, body);
                }
                // An iterator moved into the loop is walked as it is.
                if let Some(next) = self.iterator_of(ty) {
                    return self.walk_iterator(label, binding, moved, next, body);
                }
                if !self.is_poisoned(ty) {
                    let span = self.ast.exprs[elements].span;
                    let diagnostic = Diagnostic::error(
                        codes::NOT_ITERABLE,
                        format!("{} does not give up its elements", self.ty_name(ty)),
                        span,
                        "`move` takes each element",
                    )
                    .with_help("walk it without `move`, and each element is lent to the loop: `for x in c`")
                    .with_note("`for x in move c` walks `c.intoIterator()`, which a `Vec`, a `Deque`, a `Set` and a `Map` answer, and a type of your own by `extend …: IntoIterator<I>`");
                    self.report(diagnostic);
                }
                self.state.scopes.push(FxHashMap::default());
                let binding =
                    self.element_pattern(binding, Types::ERROR, None, crate::RefKind::Shared, None);
                let body = self.loop_body(label, body);
                self.state.scopes.pop();
                StmtKind::ForElements {
                    binding,
                    elements: moved,
                    body,
                }
            }
            ast::ForSource::Elements(elements) => {
                // A range of elements is lent to the loop for as long as
                // it runs, as a container lends its slice: `for x in
                // xs[1..n]` walks part of it.
                let e = self.infer_borrowed(elements, None);
                let mut e = self.autoderef(e);
                let ty = self.ty_of(e);
                // A temporary is the loop's already, as it is moved where it
                // is used (E0317): one that gives up its elements does.
                if !self.state.body.is_place(e)
                    && let Some((taken, next)) = self.given_up_iterator(e)
                {
                    return self.walk_iterator(label, binding, taken, next, body);
                }
                let elem = match self.kind(ty) {
                    TyKind::Array(elem, _) | TyKind::Slice(elem) => Some(elem),
                    _ if self.is_poisoned(ty) => None,
                    // A container is walked through the elements it lends.
                    _ => match self.items_of(e, ty) {
                        Walked::Items(items, elem) => {
                            // A container that can be written lends its
                            // elements for writing, as an array in a `var`
                            // does: the writing half of the pair, where it
                            // has one.
                            if let ExprKind::Deref(call) = self.state.body.exprs[items].kind
                                && self.writable(e)
                                && self.swap_in_writer(call)
                            {
                                let ty = self.ty_of(call);
                                if let TyKind::Ref(slice, _) = self.kind(ty) {
                                    self.state.body.exprs[items].ty = slice;
                                }
                            }
                            e = items;
                            Some(elem)
                        }
                        // What lends no slice may lend its elements one
                        // place at a time …
                        Walked::No if let Some(sequence) = self.sequence_of(ty) => {
                            return self.walk_sequence(label, binding, e, sequence, body);
                        }
                        // … or hand them out one at a time.
                        Walked::No if let Some(next) = self.iterator_of(ty) => {
                            return self.walk_iterator(label, binding, e, next, body);
                        }
                        Walked::No => {
                            let diagnostic = Diagnostic::error(
                                codes::NOT_ITERABLE,
                                format!("cannot iterate over {}", self.ty_name(ty)),
                                self.state.body.exprs[e].span,
                                "not an array, a slice or a buffer",
                            )
                            .with_help("to count, walk a range: `for i in 0..n`")
                            .with_note("a type of your own is walked when it lends its elements, `extend Deck: Items<Card>`, lends them one place at a time, `extend Ring: Sequence<Card>`, or hands them out one at a time, `extend Words: Iterator<str>`, which a generator writes for it: `fn words(): Iterator<str> = { … yield … }`");
                            self.report(diagnostic);
                            None
                        }
                    },
                };
                // The binding refers to each element, and can write it when
                // the elements can be written.
                let kind = if self.writable(e) {
                    crate::RefKind::Var
                } else {
                    crate::RefKind::Shared
                };
                self.state.scopes.push(FxHashMap::default());
                // The binding refers to the element, so it is checked
                // against a reference to it, as a `match` binding of a
                // place is.
                let element_ty = match elem {
                    Some(elem) => self.intern(TyKind::Ref(elem, kind)),
                    None => Types::ERROR,
                };
                let binding = self.element_pattern(binding, element_ty, elem, kind, Some(e));
                let body = self.loop_body(label, body);
                self.state.scopes.pop();
                StmtKind::ForElements {
                    binding,
                    elements: e,
                    body,
                }
            }
            ast::ForSource::Range { lo, hi, inclusive } => {
                // A literal bound takes the other bound's type, as a
                // literal operand does.
                let (lo, hi) = if self.number_literal(lo) {
                    let hi = self.operand(hi, None);
                    let lo = self.operand(lo, Some(self.ty_of(hi)));
                    (lo, hi)
                } else {
                    let lo = self.operand(lo, None);
                    let hi = self.operand(hi, Some(self.ty_of(lo)));
                    (lo, hi)
                };
                let (lo_ty, hi_ty) = (self.ty_of(lo), self.ty_of(hi));
                let ty = if self.is_poisoned(lo_ty) || self.is_poisoned(hi_ty) {
                    Types::ERROR
                } else if lo_ty == hi_ty && self.program.types.is_integer(lo_ty) {
                    lo_ty
                } else {
                    let span = self.state.body.exprs[lo]
                        .span
                        .to(self.state.body.exprs[hi].span);
                    let diagnostic = Diagnostic::error(
                        codes::INVALID_OPERANDS,
                        "a range counts with integers of one type",
                        span,
                        format!("from {} to {}", self.ty_name(lo_ty), self.ty_name(hi_ty)),
                    );
                    self.report(diagnostic);
                    Types::ERROR
                };
                self.state.scopes.push(FxHashMap::default());
                let binding = match &binding.kind {
                    ast::PatternKind::Binding(sym) => {
                        Some(self.declare(*sym, ty, LocalKind::Binding, binding.span))
                    }
                    ast::PatternKind::Wildcard | ast::PatternKind::Error => None,
                    _ => {
                        let diagnostic = Diagnostic::error(
                            codes::INVALID_PATTERN,
                            "a range counts with numbers, which have no fields",
                            binding.span,
                            "not a name",
                        )
                        .with_help("write a name, or `_`");
                        self.report(diagnostic);
                        None
                    }
                };
                let body = self.loop_body(label, body);
                self.state.scopes.pop();
                StmtKind::ForRange {
                    binding,
                    lo,
                    hi,
                    inclusive,
                    body,
                }
            }
        }
    }

    /// A loop's body, inside the loop for `break` and `continue`.
    pub(super) fn loop_body(&mut self, label: Option<&ast::Name>, body: &'a ast::Block) -> Block {
        self.push_loop(label);
        let (body, _) = self.check_block(body, Some(Types::UNIT));
        self.state.loops.pop();
        body
    }

    /// Enters a loop, with the name it was given. A name
    /// already in use around it is reported: `break` would mean the inner
    /// one, and the outer one could no longer be named.
    pub(super) fn push_loop(&mut self, label: Option<&ast::Name>) {
        if let Some(label) = label
            && self
                .state
                .loops
                .iter()
                .any(|&(name, _)| name == Some(label.sym))
        {
            let text = self.text(label.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::LOOP_JUMP_OUTSIDE,
                format!("`{text}` is the name of a loop around this one"),
                label.span,
                "a name already in use",
            )
            .with_note(
                "a loop's name says which one a `break` leaves, so two around each other cannot share it",
            );
            self.report(diagnostic);
        }
        self.state.loops.push((label.map(|l| l.sym), false));
    }

    /// How many loops out the one with this name is, counted from the
    /// innermost.
    fn loop_named(&mut self, label: ast::Name, word: &str) -> Option<u32> {
        let found = self
            .state
            .loops
            .iter()
            .rev()
            .position(|&(name, _)| name == Some(label.sym));
        if let Some(depth) = found {
            return Some(depth as u32);
        }
        let text = self.text(label.sym).to_string();
        let names: Vec<String> = self
            .state
            .loops
            .iter()
            .filter_map(|&(name, _)| name)
            .map(|name| format!("`{}`", self.text(name)))
            .collect();
        let mut diagnostic = Diagnostic::error(
            codes::LOOP_JUMP_OUTSIDE,
            format!("no loop around this one is called `{text}`"),
            label.span,
            "unknown loop",
        )
        .with_note(format!(
            "`{word} name` leaves the loop of that name, which is written `name: while …` or `name: for …`"
        ));
        if !names.is_empty() {
            diagnostic = diagnostic.with_help(format!("the loops here are {}", names.join(", ")));
        }
        self.report(diagnostic);
        None
    }

    pub(super) fn return_value(
        &mut self,
        value: Option<ast::ExprId>,
        span: Span,
    ) -> Option<ExprId> {
        // A lambda whose result is not known yet takes the type of what it
        // first returns.
        if self.state.inferring_ret {
            self.state.inferring_ret = false;
            let Some(v) = value else {
                self.state.ret = Types::UNIT;
                return None;
            };
            let e = self.infer(v, None);
            let ty = self.ty_of(e);
            self.state.ret = if ty == Types::NEVER { Types::UNIT } else { ty };
            return Some(e);
        }
        let ret = self.state.ret;
        match value {
            Some(v) if ret == Types::UNIT => {
                let e = self.infer(v, None);
                let ty = self.state.body.exprs[e].ty;
                if ty != Types::UNIT && !self.is_poisoned(ty) {
                    let diagnostic = Diagnostic::error(
                        codes::MISMATCHED_TYPES,
                        "this function returns nothing, but a value is returned",
                        self.state.body.exprs[e].span,
                        format!("a value of type {}", self.ty_name(ty)),
                    )
                    .with_help(format!(
                        "to return it, declare the return type: `: {}`",
                        self.program.ty_name(ty, self.interner)
                    ));
                    self.report(diagnostic);
                }
                Some(e)
            }
            Some(v) => {
                let context = self
                    .state
                    .ret_span
                    .map(|s| (s, "return type declared here"));
                Some(self.check_in(v, ret, context))
            }
            None => {
                if ret != Types::UNIT && !self.has_error(ret) {
                    let mut diagnostic = Diagnostic::error(
                        codes::MISMATCHED_TYPES,
                        "missing return value",
                        span,
                        format!("expected a value of type {}", self.ty_name(ret)),
                    );
                    if let Some(ret_span) = self.state.ret_span {
                        diagnostic =
                            diagnostic.with_secondary(ret_span, "return type declared here");
                    }
                    self.report(diagnostic);
                }
                None
            }
        }
    }

    /// `assert(condition)` and `assert(condition, note)`: the condition
    /// holds, or the program prints the note and the message the lexer made
    /// and ends. It is `if !condition { panic(message) }`, so
    /// nothing below the checker needs to know of it.
    pub(super) fn assert(
        &mut self,
        cond: ast::ExprId,
        note: Option<ast::ExprId>,
        message: Option<Symbol>,
        span: Span,
    ) -> ExprId {
        // An `is` test binds nothing here: there is no block for a binding
        // to live in, since a failure ends the program.
        let cond = self.condition(cond);
        // A note written as one string is in the message the lexer made; any
        // other is built when the assert fails, before it.
        let note = note.and_then(|note| {
            self.message_text(note, codes::ASSERT_NOTE, "an assert's note")
                .filter(|&value| !matches!(self.state.body.exprs[value].kind, ExprKind::Str(_)))
        });
        let Some(message) = message else {
            return self.error_expr(span);
        };
        let cond = self.alloc(
            ExprKind::Unary {
                op: UnaryOp::Not,
                operand: cond,
            },
            Types::BOOL,
            span,
        );
        let panic = self.alloc(
            ExprKind::Panic {
                note,
                message: Some(message),
            },
            Types::NEVER,
            span,
        );
        let then_block = Block {
            stmts: Vec::new(),
            value: Some(panic),
            span,
        };
        let kind = ExprKind::If {
            cond,
            then_block,
            else_block: None,
        };
        self.alloc(kind, Types::UNIT, span)
    }

    pub(super) fn if_expr(
        &mut self,
        cond: ast::ExprId,
        then_block: &'a ast::Block,
        else_branch: Option<ast::ExprId>,
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        // The bindings of `is` tests in the condition are in scope in the
        // `then` block only.
        self.state.scopes.push(FxHashMap::default());
        let cond = self.binding_condition(cond);
        let Some(else_branch) = else_branch else {
            // Without `else` there is no value: the branch's is discarded.
            // Where a value is expected, the branch is checked against its
            // type instead, so that the missing `else` is the only error.
            let wanted = hint.filter(|&t| t != Types::UNIT && !self.is_poisoned(t));
            let (then_block, _) = self.check_block(then_block, wanted.or(Some(Types::UNIT)));
            self.state.scopes.pop();
            let kind = ExprKind::If {
                cond,
                then_block,
                else_block: None,
            };
            return self.alloc(kind, Types::UNIT, span);
        };
        let (then_block, then_ty) = self.check_block(then_block, hint);
        self.state.scopes.pop();
        // Without an expected type, the `then` branch sets the type of the
        // `else` branch.
        let expected = hint.or((then_ty != Types::NEVER).then_some(then_ty));
        let (else_block, else_ty) = self.else_block(else_branch, expected);
        let ty = if then_ty == Types::NEVER && else_ty == Types::NEVER {
            Types::NEVER
        } else {
            expected.unwrap_or(else_ty)
        };
        let kind = ExprKind::If {
            cond,
            then_block,
            else_block: Some(else_block),
        };
        self.alloc(kind, ty, span)
    }

    /// The `else` branch: a block, or an `else if` held as a block's value.
    pub(super) fn else_block(
        &mut self,
        else_branch: ast::ExprId,
        expected: Option<Ty>,
    ) -> (Block, Ty) {
        let ast = self.ast;
        let expr = &ast.exprs[else_branch];
        if let ast::ExprKind::Block(block) = &expr.kind {
            return self.check_block(block, expected);
        }
        let value = match expected {
            Some(t) => self.check(else_branch, t),
            None => self.infer(else_branch, None),
        };
        let ty = self.ty_of(value);
        let block = Block {
            stmts: Vec::new(),
            value: Some(value),
            span: expr.span,
        };
        (block, ty)
    }
}
