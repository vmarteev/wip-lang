//! Blocks and statements.

use super::*;

impl<'a> Parser<'a> {
    pub(super) fn block(&mut self) -> Block {
        if !self.at(T::LBrace) {
            let diagnostic = self.expected("`{`");
            self.report(diagnostic);
            return Block {
                stmts: Vec::new(),
                span: Span::at(self.prev_span().hi),
            };
        }
        let open = self.bump();
        self.in_braces(|p| p.block_body(open))
    }

    pub(super) fn block_body(&mut self, open: Span) -> Block {
        let mut stmts = Vec::new();
        loop {
            while self.eat(T::Semi) {}
            match self.peek() {
                T::RBrace => {
                    let close = self.bump();
                    self.brace_pairs.push((open, close));
                    return Block {
                        stmts,
                        span: open.to(close),
                    };
                }
                kind if kind == T::Eof || is_item_start(kind) => {
                    self.unclosed_brace(open);
                    return Block {
                        stmts,
                        span: open.to(self.prev_span()),
                    };
                }
                _ => {}
            }
            self.begin_region();
            if self.leading_operator() {
                continue;
            }
            let before = self.pos;
            stmts.extend(self.stmt());
            if self.pos == before {
                self.bump();
                continue;
            }
            if let [.., previous, last] = stmts.as_slice() {
                self.variant_under_expression(*previous, *last);
            }
            self.stmt_separator();
        }
    }

    /// A variant at the start of a line, directly under a statement that a `.`
    /// could have continued: `io::println(…)` and then `.Ok(value)` reads as a
    /// method call on the line above, though a line break ends the expression
    /// there. It means what it says, so this is a warning and not an error
    /// (E0126).
    fn variant_under_expression(&mut self, previous: StmtId, last: StmtId) {
        let StmtKind::Expr(value) = self.ast.stmts[last].kind else {
            return;
        };
        let Some(name) = self.dot_name(value) else {
            return;
        };
        // A name in lower case is a method's or a field's:
        // a chain meant to go on, which a `val`'s value can be too.
        let first = self.text(name.span).chars().next().unwrap_or('A');
        if first.is_lowercase() {
            let above = match self.ast.stmts[previous].kind {
                StmtKind::Expr(above) | StmtKind::Let { init: above, .. } => above,
                _ => return,
            };
            self.chain_under_expression(above, last, name);
            return;
        }
        // Only what a `.` could have carried on: a binding, a loop, an
        // assignment and an `if` without `else` have no value to take one,
        // so nothing there can be misread.
        let StmtKind::Expr(above) = self.ast.stmts[previous].kind else {
            return;
        };
        if matches!(
            self.ast.exprs[above].kind,
            ExprKind::Assign { .. }
                | ExprKind::If {
                    else_branch: None,
                    ..
                }
        ) {
            return;
        }
        let (above, below) = (self.ast.stmts[previous].span, self.ast.stmts[last].span);
        let between = self.local(above.hi)..self.local(below.lo);
        if !self.src[between].contains('\n') {
            return;
        }
        let diagnostic = Diagnostic::warning(
            codes::VARIANT_UNDER_EXPRESSION,
            "this reads as a method call on the line above",
            below,
            "a variant, on a line of its own",
        )
        .with_secondary(above, "the expression this seems to continue")
        .with_note(
            "a line break ends an expression, and a `.` that starts a line begins a variant of the expected type, so these are two statements",
        )
        .with_fix("say so with `return`", [Edit::insert(below.lo, "return ")]);
        self.report(diagnostic);
    }

    /// A chain continued with a `.` at the start of a line: `xs` and then
    /// `.walk()` under it, which a line break ended, so
    /// that `.walk` names a variant. A line that starts with `.` begins a
    /// variant; a chain goes on with the `.` at the end of the line above,
    /// or inside parentheses.
    fn chain_under_expression(&mut self, above: ExprId, last: StmtId, name: Name) {
        let above_span = self.expr_span(above);
        let below = self.ast.stmts[last].span;
        let between = self.local(above_span.hi)..self.local(below.lo);
        if !self.src[between].contains('\n') {
            return;
        }
        let method = self.text(name.span);
        let dot = Span::new(name.span.lo - 1, name.span.lo);
        let diagnostic = Diagnostic::warning(
            codes::VARIANT_UNDER_EXPRESSION,
            format!("`.{method}` starts a line, so it does not continue the line above"),
            below,
            "a variant of its own, not a call on the line above",
        )
        .with_secondary(above_span, "the expression it seems to continue")
        .with_note("a line break ends an expression, and a `.` that starts a line begins a variant; a chain goes on with the `.` at the end of the line above, or inside parentheses")
        .with_fix(
            "end the line above with the `.`",
            [Edit::replace(dot, ""), Edit::insert(above_span.hi, ".")],
        );
        self.report(diagnostic);
    }

    /// The name after the `.` an expression begins with, through the
    /// postfix forms that keep their value on the left; nothing where it
    /// does not begin with one.
    fn dot_name(&self, id: ExprId) -> Option<Name> {
        match &self.ast.exprs[id].kind {
            ExprKind::Path {
                leading_dot: true,
                segments,
                ..
            } => segments.first().copied(),
            ExprKind::Call { callee, .. } => self.dot_name(*callee),
            ExprKind::Field { base, .. } => self.dot_name(*base),
            ExprKind::Index { base, .. } | ExprKind::SubSlice { base, .. } => self.dot_name(*base),
            ExprKind::Try(inner) => self.dot_name(*inner),
            ExprKind::Binary { lhs, .. } => self.dot_name(*lhs),
            ExprKind::Cast { expr, .. } => self.dot_name(*expr),
            _ => None,
        }
    }

    /// After a statement: `;`, a line break or `}` must follow (E0112).
    pub(super) fn stmt_separator(&mut self) {
        let kind = self.peek();
        if matches!(kind, T::Semi | T::RBrace | T::Eof) || is_item_start(kind) || self.line_break()
        {
            return;
        }
        let diagnostic = Diagnostic::error(
            codes::SEPARATOR,
            "missing `;` between statements on one line",
            self.span(),
            "starts another statement on the same line",
        )
        .with_note("statements on one line are separated by `;`")
        .with_fix("add `;`", [Edit::insert(self.prev_span().hi, ";")]);
        self.report(diagnostic);
    }

    /// `val pattern = value else { … }`, after the keyword,
    /// or `var`, whose names are variables.
    fn guard(&mut self, mutable: bool, start: Span, refutable: bool) -> StmtId {
        let pattern = self.pattern();
        let value = if self.expect(T::Eq) {
            self.expr()
        } else {
            self.error_expr()
        };
        let else_block = if self.eat(T::Else) {
            Some(self.block())
        } else {
            if refutable {
                let diagnostic = Diagnostic::error(
                    codes::GUARD_SYNTAX,
                    if mutable {
                        "a pattern in `var` needs `else`"
                    } else {
                        "a pattern in `val` needs `else`"
                    },
                    Span::at(self.prev_span().hi),
                    "expected `else { … }` here",
                )
                .with_note("`val pattern = value else { … }` runs the `else` block when the value does not match; the block must leave")
                .with_help("to test without leaving, write `if value is pattern { … }`");
                self.report(diagnostic);
            }
            None
        };
        let kind = StmtKind::Guard {
            mutable,
            pattern,
            value,
            else_block,
        };
        let span = start.to(self.prev_span());
        self.ast.stmts.alloc(Stmt { kind, span })
    }

    /// `break outer`: which loop the jump leaves, where it is not the
    /// innermost. It is on the same line as the keyword.
    fn jump_label(&mut self) -> Option<Name> {
        if !is_ident(self.peek()) || self.line_break() {
            return None;
        }
        self.name("a loop's name")
    }

    pub(super) fn stmt(&mut self) -> Option<StmtId> {
        let start = self.span();
        let kind = match self.peek() {
            T::Let | T::Val | T::Var => {
                if self.at(T::Let) {
                    let span = self.span();
                    let diagnostic = Diagnostic::error(
                        codes::LET,
                        "immutable variables are declared with `val`",
                        span,
                        "`let` is not used in Wip",
                    )
                    .with_fix("write `val`", [Edit::replace(span, "val")]);
                    self.report(diagnostic);
                }
                let mutable = self.at(T::Var);
                self.bump();
                // `val .Some(x) = value else { … }`, and
                // `val Point(x, y) = value` or `val (x, y) = value`, which
                // cannot fail; `var` for each.
                let refutable =
                    self.at(T::Dot) || (is_ident(self.peek()) && self.nth(1) == T::ColonColon);
                // `val [a, b] = pair`, or `val [first, ..] = xs else { … }`,
                // which the checker says needs `else`.
                let takes_apart = self.at(T::LParen)
                    || self.at(T::LBracket)
                    || (is_ident(self.peek()) && self.nth(1) == T::LParen);
                if refutable || takes_apart {
                    return Some(self.guard(mutable, start, refutable));
                }
                let Some(name) = self.name("a variable name") else {
                    self.skip_stmt();
                    return None;
                };
                let ty = if self.eat(T::Colon) {
                    Some(self.ty())
                } else {
                    None
                };
                let init = if self.expect(T::Eq) {
                    self.expr()
                } else {
                    self.error_expr()
                };
                StmtKind::Let {
                    mutable,
                    name,
                    ty,
                    init,
                }
            }
            T::Defer => {
                self.bump();
                StmtKind::Defer(self.expr())
            }
            T::Return => {
                self.bump();
                // A line break after `return` means it returns nothing.
                let value =
                    (can_start_expr(self.peek()) && !self.line_break()).then(|| self.expr());
                StmtKind::Return(value)
            }
            // `yield value`: to the list being built.
            T::Yield => {
                self.bump();
                StmtKind::Yield(self.expr())
            }
            T::While => {
                self.bump();
                let cond = self.cond();
                let body = self.block();
                StmtKind::While {
                    label: None,
                    cond,
                    body,
                }
            }
            // `outer: while …`, `outer: for …`: a loop with a name, which
            // `break outer` leaves.
            T::Ident(_) if self.nth(1) == T::Colon && matches!(self.nth(2), T::While | T::For) => {
                let label = self.name("a name for the loop")?;
                self.bump();
                let id = self.stmt()?;
                match &mut self.ast.stmts[id].kind {
                    StmtKind::While { label: it, .. } | StmtKind::For { label: it, .. } => {
                        *it = Some(label);
                    }
                    _ => unreachable!("a label is followed by a loop"),
                }
                self.ast.stmts[id].span = start.to(self.prev_span());
                return Some(id);
            }
            // `for x in xs { … }` or `for i in lo..hi { … }`.
            T::For => {
                self.bump();
                let Some((binding, source)) = self.for_header() else {
                    self.skip_stmt();
                    return None;
                };
                let body = self.block();
                StmtKind::For {
                    label: None,
                    binding,
                    source,
                    body,
                }
            }
            // `break`, or `break outer`: the loop of that name.
            T::Break => {
                self.bump();
                StmtKind::Break(self.jump_label())
            }
            T::Continue => {
                self.bump();
                StmtKind::Continue(self.jump_label())
            }
            kind if can_start_expr(kind) => StmtKind::Expr(self.expr()),
            _ => {
                let diagnostic = self.expected("a statement");
                self.report(diagnostic);
                self.bump();
                self.skip_stmt();
                return None;
            }
        };
        let span = start.to(self.prev_span());
        Some(self.ast.stmts.alloc(Stmt { kind, span }))
    }
}

impl Parser<'_> {
    /// What follows `for`: what each element is bound to, `in`, and what
    /// is walked — a statement's and a list literal's element's alike.
    /// `None` where there is no binding.
    pub(super) fn for_header(&mut self) -> Option<(Pattern, ForSource)> {
        // `for x in …`, `for _ in …`, or a value taken apart,
        // `for MapEntry(key, value) in …` and `for (a, b) in …`.
        // An array's elements too, `for [a, b] in pairs`.
        let binding = if self.at(T::LParen)
            || self.at(T::LBracket)
            || (is_ident(self.peek()) && self.nth(1) == T::LParen)
        {
            self.pattern()
        } else if self.at(T::Underscore) {
            let span = self.bump();
            Pattern {
                kind: PatternKind::Wildcard,
                span,
            }
        } else {
            let name = self.name("a name for each element")?;
            Pattern {
                kind: PatternKind::Binding(name.sym),
                span: name.span,
            }
        };
        self.expect(T::In);
        let first = self.cond();
        let source = if self.at(T::DotDot) || self.at(T::DotDotEq) {
            let inclusive = self.at(T::DotDotEq);
            self.bump();
            ForSource::Range {
                lo: first,
                hi: self.cond(),
                inclusive,
            }
        } else {
            ForSource::Elements(first)
        };
        Some((binding, source))
    }
}
