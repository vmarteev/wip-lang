//! Blocks, statements, `if`, loops, and the `defer`s that run at each exit.

use super::*;

impl Checker<'_> {
    /// Checks a block's statements, then its value, then the `defer`s that
    /// run where it ends. Returns whether the block always returns.
    pub(super) fn block(&mut self, block: &Block, state: &mut State) -> bool {
        // The borrows of guards in the block last until it ends.
        let borrows = self.borrows.len();
        self.deferred.push(Vec::new());
        self.scopes.push(Vec::new());
        // Statements after one that returns are unreachable and not checked.
        let returns = block.stmts.iter().any(|&stmt| self.stmt(stmt, state))
            || block.value.is_some_and(|value| {
                self.expr(value, Ctx::Value, state);
                self.ty(value) == Types::NEVER
            });
        let deferred = self.deferred.pop().expect("pushed above");
        self.scopes.pop().expect("pushed above");
        if !returns {
            let end = Span::new(block.span.hi.saturating_sub(1), block.span.hi);
            let label = "the block ends here, running the `defer`";
            self.run_deferred(&deferred, end, label, state);
            self.block_ends(block, end, state);
        }
        self.borrows.truncate(borrows);
        returns
    }

    /// Checks deferred expressions where an exit runs them, newest first,
    /// with the state there.
    pub(super) fn run_deferred(
        &mut self,
        defers: &[ExprId],
        exit: Span,
        label: &'static str,
        state: &mut State,
    ) {
        let outer = self.exit.replace((exit, label));
        for &e in defers.iter().rev() {
            self.expr(e, Ctx::Value, state);
        }
        self.exit = outer;
    }

    /// An `if`: each branch starts from the state before it, and the states
    /// of the branches that fall through are merged.
    pub(super) fn if_expr(
        &mut self,
        cond: ExprId,
        then_block: &Block,
        else_block: Option<&Block>,
        state: &mut State,
    ) {
        // The borrows of `is` tests in the condition last through the `then`
        // block.
        let mark = self.borrows.len();
        self.expr(cond, Ctx::Value, state);
        let mut then_state = state.clone();
        let then_returns = self.block(then_block, &mut then_state);
        self.borrows.truncate(mark);
        let mut else_state = state.clone();
        let else_returns = match else_block {
            Some(block) => self.block(block, &mut else_state),
            None => false,
        };
        *state = match (then_returns, else_returns) {
            (true, false) => else_state,
            (false, true) | (true, true) => then_state,
            (false, false) => then_state.merge(&else_state),
        };
    }

    /// Returns whether the statement always returns.
    pub(super) fn stmt(&mut self, id: StmtId, state: &mut State) -> bool {
        // What the block declares, for a `@tailrec` jump.
        let declared = self.body.stmts[id].kind.declares();
        if !declared.is_empty()
            && let Some(scope) = self.scopes.last_mut()
        {
            scope.extend(declared);
        }
        match &self.body.stmts[id].kind {
            StmtKind::Let { local, init } => {
                self.expr(*init, Ctx::Value, state);
                // In a loop, a `let` makes its variable whole again on every
                // iteration.
                state.reinit(&Path {
                    local: *local,
                    projs: Vec::new(),
                });
                if self.is_view(self.body.locals[*local].ty) || self.is_kept_ref(*local) {
                    self.bind_str(*local, *init, state);
                }
                if self.is_kept_closure(self.body.locals[*local].ty) {
                    self.bind_closure(*local, *init, state);
                }
                false
            }
            StmtKind::Expr(e) => {
                self.expr(*e, Ctx::Value, state);
                self.ty(*e) == Types::NEVER
            }
            // `yield value` hands `.Some(value)` over, as a result, and the
            // generator waits.
            StmtKind::Yield(value) => {
                self.expr(*value, Ctx::Value, state);
                let handed = match &self.body.exprs[*value].kind {
                    ExprKind::Variant { args, .. } if args.len() == 1 => args[0],
                    _ => *value,
                };
                if self.borrows_as_view(self.ty(handed)) {
                    self.check_leaving(handed, borrowed::Leaving::Yield, state);
                }
                self.yielded(self.body.stmts[id].span, state);
                false
            }
            // `val pattern = value else { … }`: the `else`
            // path leaves, and the bindings hold for the rest of the block.
            StmtKind::Guard {
                scrutinee,
                pattern,
                else_block,
            } => {
                let ctx = if self.place(*scrutinee).is_some() {
                    Ctx::Inspect
                } else {
                    Ctx::Value
                };
                self.expr(*scrutinee, ctx, state);
                let mut else_state = state.clone();
                if let Some(else_block) = else_block {
                    self.block(else_block, &mut else_state);
                }
                self.bind_pattern(pattern, *scrutinee, Holder::Guard, state);
                self.bind_views(pattern, *scrutinee, state);
                false
            }
            StmtKind::Defer(e) => {
                // Nothing happens here: the expression is checked at each
                // exit that runs it.
                self.deferred
                    .last_mut()
                    .expect("a `defer` is inside a block")
                    .push(*e);
                false
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.expr(*value, Ctx::Value, state);
                    if self.is_view(self.ty(*value)) {
                        self.check_leaving(*value, borrowed::Leaving::Result, state);
                    }
                }
                // A `return` unwinds every enclosing block, innermost first.
                let blocks = self.deferred.clone();
                let span = self.body.stmts[id].span;
                let mut s = state.clone();
                for defers in blocks.iter().rev() {
                    self.run_deferred(defers, span, "this `return` runs the `defer`", &mut s);
                }
                self.check_var_params(span, &s);
                true
            }
            StmtKind::While { cond, body } => {
                self.loop_stmt(Some(*cond), body, None, None, state);
                false
            }
            StmtKind::ForElements {
                binding,
                elements,
                body,
            } => {
                let ctx = if self.place(*elements).is_some() {
                    Ctx::Inspect
                } else {
                    Ctx::Value
                };
                self.expr(*elements, ctx, state);
                // The binding refers to each element, so what is walked must
                // stay put while the loop runs.
                let walked = self.access_path(*elements);
                // Each binding refers into the element: the name a `for`
                // binds, or a struct's fields.
                if let Some(path) = &walked {
                    let mut element = path.clone();
                    element.projs.push(Proj::Element);
                    let mut binders = Vec::new();
                    crate::bindings_of(binding, Some(element), &mut binders);
                    for (local, path) in binders {
                        if let Some(path) = path {
                            self.aliases.insert(local, path);
                        }
                    }
                }
                let span = self.body.exprs[*elements].span;
                let heap = self.on_heap(*elements);
                let borrow = walked.map(|path| Borrow {
                    path,
                    span,
                    var: false,
                    holder: Holder::Loop,
                    heap,
                });
                self.loop_stmt(None, body, None, borrow, state);
                false
            }
            StmtKind::ForRange {
                binding,
                lo,
                hi,
                body,
                ..
            } => {
                self.expr(*lo, Ctx::Value, state);
                self.expr(*hi, Ctx::Value, state);
                self.loop_stmt(None, body, *binding, None, state);
                false
            }
            StmtKind::Break { depth } | StmtKind::Continue { depth } => {
                let is_break = matches!(self.body.stmts[id].kind, StmtKind::Break { .. });
                let span = self.body.stmts[id].span;
                // Which loop it leaves: the innermost, or one further out
                // where it named a loop.
                let target = self.loop_exits.len().saturating_sub(1 + *depth as usize);
                let open = self
                    .loop_exits
                    .get(target)
                    .map_or(self.deferred.len(), |exits| exits.deferred);
                // The jump unwinds the blocks inside the loop, innermost
                // first.
                let blocks = self.deferred[open..].to_vec();
                let label = if is_break {
                    "this `break` runs the `defer`"
                } else {
                    "this `continue` runs the `defer`"
                };
                let mut s = state.clone();
                for defers in blocks.iter().rev() {
                    self.run_deferred(defers, span, label, &mut s);
                }
                if let Some(exits) = self.loop_exits.get_mut(target) {
                    let slot = if is_break {
                        &mut exits.breaks
                    } else {
                        &mut exits.continues
                    };
                    *slot = Some(match slot.take() {
                        Some(earlier) => earlier.merge(&s),
                        None => s,
                    });
                }
                true
            }
        }
    }
}

impl Checker<'_> {
    /// What the block declared is dropped where it ends: its value may not
    /// borrow it, and a `str` that does is stale from here.
    fn block_ends(&mut self, block: &Block, end: Span, state: &mut State) {
        let mut dying = Vec::new();
        for &stmt in &block.stmts {
            match &self.body.stmts[stmt].kind {
                StmtKind::Guard { pattern, .. } => pattern.locals(&mut dying),
                kind => dying.extend(kind.declares()),
            }
        }
        // The function's own block answers its result, which is checked as
        // one.
        let is_body = self.body.exprs[self.body.value()].span == block.span;
        if let Some(value) = block.value
            && !is_body
        {
            self.check_block_value(value, &dying, "block", state);
        }
        for local in dying {
            let path = Path {
                local,
                projs: Vec::new(),
            };
            self.changed(&path, end, "dropped", state);
        }
    }

    /// A loop's body, with the states its `break`s and `continue`s leave
    /// with. Returns whether the body always returns.
    fn loop_body(
        &mut self,
        body: &Block,
        state: &mut State,
        fresh: Option<LocalId>,
    ) -> (bool, LoopExits) {
        // A range's binding holds a new number on every pass.
        if let Some(local) = fresh {
            state.reinit(&Path {
                local,
                projs: Vec::new(),
            });
        }
        self.loop_exits.push(LoopExits {
            deferred: self.deferred.len(),
            breaks: None,
            continues: None,
        });
        let returns = self.block(body, state);
        let exits = self.loop_exits.pop().expect("pushed above");
        (returns, exits)
    }

    /// The state at the top of a loop after one more pass: the entry state,
    /// merged with the state at the end of the body and at each `continue`.
    fn next_head(
        &mut self,
        entry: &State,
        body: &Block,
        state: &mut State,
        fresh: Option<LocalId>,
    ) -> State {
        let (returns, exits) = self.loop_body(body, state, fresh);
        let mut next = if returns {
            entry.clone()
        } else {
            entry.merge(state)
        };
        if let Some(continues) = &exits.continues {
            next = next.merge(continues);
        }
        next
    }

    /// A loop: `cond`, if it is a `while`, and then `body`, on each pass.
    /// The state at the top of the loop is iterated until it stops changing;
    /// then a final pass reports what is wrong. The loop may run no times, so
    /// the state after it includes the state before.
    ///
    /// `fresh` is a range's binding, which holds a new number on every pass,
    /// and `borrow` keeps what a `for` walks in place while it runs.
    fn loop_stmt(
        &mut self,
        cond: Option<ExprId>,
        body: &Block,
        fresh: Option<LocalId>,
        borrow: Option<Borrow>,
        state: &mut State,
    ) {
        let mark = self.borrows.len();
        self.borrows.extend(borrow);
        let entry = state.clone();
        let mut head = entry.clone();
        self.quiet += 1;
        loop {
            let mut s = head.clone();
            // The borrows of `is` tests in a `while`'s condition last through
            // its body.
            let pass = self.borrows.len();
            if let Some(cond) = cond {
                self.expr(cond, Ctx::Value, &mut s);
            }
            let next = self.next_head(&entry, body, &mut s, fresh);
            self.borrows.truncate(pass);
            if next == head {
                break;
            }
            head = next;
        }
        self.quiet -= 1;

        // The final pass reports, starting from the fixed point. The loop
        // ends where its condition is false, or at a `break`.
        self.loops.push(body.span);
        let mut s = head;
        let pass = self.borrows.len();
        if let Some(cond) = cond {
            self.expr(cond, Ctx::Value, &mut s);
        }
        let mut exit = s.clone();
        let exits = self.loop_body(body, &mut s, fresh).1;
        self.borrows.truncate(pass);
        self.loops.pop();
        if let Some(breaks) = &exits.breaks {
            exit = exit.merge(breaks);
        }
        self.borrows.truncate(mark);
        *state = exit;
    }
}
