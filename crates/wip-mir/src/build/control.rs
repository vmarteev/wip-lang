//! Blocks, statements, `if`, `match`, `&&` and `||`.

use super::*;

impl Builder<'_> {
    /// Marks what follows as written at `span`, for debug information;
    /// nothing where the path has ended.
    pub(super) fn at(&mut self, span: Span) {
        if !self.dead {
            self.push(Statement::At(span));
        }
    }

    /// A block: its statements, then its value (into `dest` if given), then
    /// its cleanups. A block whose path ended produces no value.
    pub(super) fn block_expr(&mut self, block: &hir::Block, dest: Option<Place>) -> Value {
        self.scopes.push(Vec::new());
        self.seen_until.push(block.span.hi);
        for &stmt in &block.stmts {
            self.stmt(stmt);
        }
        if let Some(value) = block.value
            && !self.dead
        {
            self.at(self.hir.exprs[value].span);
        }
        let value = match block.value {
            Some(value) if !self.dead => match dest {
                Some(dest) => {
                    self.store_expr(value, dest.clone());
                    Value::Place(dest)
                }
                None => self.expr(value),
            },
            _ => Value::Unit,
        };
        if self.dead {
            self.scopes.pop();
        } else {
            self.drop_scope();
        }
        self.seen_until.pop();
        value
    }

    fn stmt(&mut self, id: StmtId) {
        self.revive();
        self.at(self.hir.stmts[id].span);
        // Statements can nest inside expressions (`f(x, { … })`), so the
        // temporaries of enclosing statements may still be pending.
        let mark = self.temps.len();
        let hir = self.hir;
        match &hir.stmts[id].kind {
            StmtKind::Let { local, init } => {
                let var = self.source_local(*local, LocalKind::Var);
                self.store_expr(*init, Place::local(var));
                self.drop_temps(mark);
                self.vars.insert(*local, var);
                // It holds a value from here until something moves it away.
                self.set_flag(*local, true);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.push(Cleanup::Drop(*local));
                }
            }
            StmtKind::Return(None) if self.generator.is_some() => self.generator_return(),
            StmtKind::Yield(value) => self.yield_stmt(*value, mark),
            StmtKind::Return(value) => {
                match (value, self.ret) {
                    (Some(value), Some(ret)) if self.is_aggregate(self.local_ty(ret)) => {
                        self.store_expr(*value, Place::local(ret));
                        self.drop_temps(mark);
                        self.drop_pending_temps();
                        self.drop_all_scopes();
                    }
                    (Some(value), ret) => {
                        let value = self.expr(*value);
                        self.drop_temps(mark);
                        self.drop_pending_temps();
                        self.drop_all_scopes();
                        if let (Value::Scalar(operand), Some(ret)) = (value, ret) {
                            self.assign(Place::local(ret), Rvalue::Use(operand));
                        }
                    }
                    (None, _) => {
                        self.drop_pending_temps();
                        self.drop_all_scopes();
                    }
                }
                // A `@tailrec` function's `return f(x)` has already jumped
                // back to the top.
                if !self.dead {
                    self.terminate(Terminator::Return);
                    self.dead = true;
                }
            }
            StmtKind::While { cond, body } => {
                let forever = matches!(hir.exprs[*cond].kind, ExprKind::Bool(true));
                let header = self.new_block();
                let body_block = self.new_block();
                let exit = self.new_block();
                self.terminate(Terminator::Goto(header));
                self.switch_to(header);
                let bindings = self.condition_bindings(*cond);
                let cond = self.scalar(*cond);
                self.drop_temps(mark);
                let breaks = if bindings {
                    // The bindings of `is` tests in the condition are dropped
                    // at the end of each pass, and when the loop ends, by
                    // latches that `continue` and `break` go through too.
                    let next = self.new_block();
                    let leave = self.new_block();
                    self.terminate(Terminator::Branch {
                        cond,
                        then: body_block,
                        otherwise: leave,
                    });
                    let breaks = self.loop_body(body_block, body, next, leave, |_| {});
                    let scope = self.scopes.pop().expect("pushed for the bindings");
                    self.switch_to(next);
                    self.unwind(scope.clone());
                    self.terminate(Terminator::Goto(header));
                    self.switch_to(leave);
                    self.unwind(scope);
                    self.terminate(Terminator::Goto(exit));
                    breaks
                } else {
                    self.terminate(Terminator::Branch {
                        cond,
                        then: body_block,
                        otherwise: exit,
                    });
                    self.loop_body(body_block, body, header, exit, |_| {})
                };
                self.switch_to(exit);
                // `while true` with no `break` never gets here.
                if forever && !breaks {
                    self.dead = true;
                }
            }
            // A counter over the elements, whose number is read once; the
            // binding holds each element's address.
            StmtKind::ForElements {
                binding,
                elements,
                body,
            } => {
                let (place, len) = self.elements(*elements);
                let i = self.temp(Types::I64);
                self.assign(Place::local(i), Rvalue::Use(Self::int(0, Types::I64)));
                let binding = binding.clone();
                let elem_ty = match self.kind(self.ty(*elements)) {
                    TyKind::Array(elem, _) | TyKind::Slice(elem) => elem,
                    _ => Types::ERROR,
                };
                // The binding is seen by the loop's body.
                self.seen_until.push(hir.stmts[id].span.hi);
                self.counted_loop(i, End::Before(len), Types::I64, body, mark, |builder| {
                    let element = place.project(Projection::Index(i));
                    builder.bind(&binding, &element, elem_ty);
                });
                self.seen_until.pop();
            }
            StmtKind::ForRange {
                binding,
                lo,
                hi,
                inclusive,
                body,
            } => {
                let ty = self.ty(*lo);
                let lo = self.scalar(*lo);
                let hi = self.scalar(*hi);
                let i = self.temp(ty);
                self.assign(Place::local(i), Rvalue::Use(lo));
                let binding = *binding;
                let end = match inclusive {
                    true => End::Through(hi),
                    false => End::Before(hi),
                };
                self.seen_until.push(hir.stmts[id].span.hi);
                self.counted_loop(i, end, ty, body, mark, |builder| {
                    if let Some(local) = binding {
                        let var = builder.source_local(local, LocalKind::Var);
                        builder.assign(
                            Place::local(var),
                            Rvalue::Use(Operand::Copy(Place::local(i))),
                        );
                        builder.vars.insert(local, var);
                    }
                });
                self.seen_until.pop();
            }
            // A jump unwinds what the loop's body opened: the temporaries of
            // the statements around it, then its blocks, innermost first.
            StmtKind::Break { depth } | StmtKind::Continue { depth } => {
                let is_break = matches!(hir.stmts[id].kind, StmtKind::Break { .. });
                // Which loop it leaves, counted from the innermost.
                let target = self.loops.len() - 1 - *depth as usize;
                let frame = *self
                    .loops
                    .get(target)
                    .expect("the type checker allows `break` and `continue` only in loops");
                // What a `match` arm took out of a temporary is the arm's,
                // and its flag says so, as on `return`.
                let temps = self.temps[frame.temps..].to_vec();
                for (place, ty) in temps.into_iter().rev() {
                    self.drop_temp(&place, ty);
                }
                let scopes = self.scopes[frame.scopes..].to_vec();
                for scope in scopes.into_iter().rev() {
                    self.unwind(scope);
                }
                let goes_to = if is_break {
                    self.loops[target].breaks = true;
                    frame.exit
                } else {
                    frame.next
                };
                self.terminate(Terminator::Goto(goes_to));
                self.dead = true;
            }
            StmtKind::Guard {
                scrutinee,
                pattern,
                else_block,
            } => {
                self.guard(*scrutinee, pattern, else_block.as_ref());
                self.drop_temps(mark);
            }
            StmtKind::Expr(expr) => self.expr_stmt(*expr, mark),
            // Nothing is evaluated here: the expression runs, whole, at each
            // exit of the block.
            StmtKind::Defer(expr) => {
                if let Some(scope) = self.scopes.last_mut() {
                    scope.push(Cleanup::Defer(*expr));
                }
            }
        }
    }

    pub(super) fn local_ty(&self, local: Local) -> Ty {
        self.locals[local.0 as usize].ty
    }

    /// An expression whose value is not used. A result that owns memory and
    /// that nothing takes, such as `make()`, is dropped with the statement's
    /// other temporaries.
    pub(super) fn expr_stmt(&mut self, expr: ExprId, mark: usize) {
        let ty = self.ty(expr);
        match self.expr(expr) {
            Value::Scalar(Operand::Copy(place)) if matches!(self.kind(ty), TyKind::Own(_)) => {
                self.own_temp(place, ty);
            }
            Value::Place(place) if !self.is_place_kind(expr) => self.own_temp(place, ty),
            _ => {}
        }
        self.drop_temps(mark);
    }

    /// A loop's body, lowered into `start` after `bind`. It goes to `next`
    /// when it ends or continues, and to `exit` when it breaks. Returns
    /// whether a `break` leaves it.
    fn loop_body(
        &mut self,
        start: BlockId,
        body: &hir::Block,
        next: BlockId,
        exit: BlockId,
        bind: impl FnOnce(&mut Self),
    ) -> bool {
        self.loops.push(LoopFrame {
            exit,
            next,
            scopes: self.scopes.len(),
            temps: self.temps.len(),
            breaks: false,
        });
        self.switch_to(start);
        self.dead = false;
        bind(self);
        self.block_expr(body, None);
        if !self.dead {
            self.terminate(Terminator::Goto(next));
        }
        self.dead = false;
        self.loops.pop().expect("pushed above").breaks
    }

    /// A `for`: the counter `i`, of type `ty`, runs up to `end`, and the
    /// body runs after `bind` on each pass. The statement's temporaries,
    /// such as a walked temporary, are dropped after the loop.
    fn counted_loop(
        &mut self,
        i: Local,
        end: End,
        ty: Ty,
        body: &hir::Block,
        mark: usize,
        bind: impl FnOnce(&mut Self),
    ) {
        let header = self.new_block();
        let body_block = self.new_block();
        let step = self.new_block();
        let exit = self.new_block();
        self.terminate(Terminator::Goto(header));
        self.switch_to(header);
        let (past, end) = match end {
            End::Before(end) => (BinaryOp::Ge, end),
            End::Through(end) => (BinaryOp::Gt, end),
        };
        let done = self.value(
            Types::BOOL,
            Rvalue::Binary(past, Operand::Copy(Place::local(i)), end.clone()),
        );
        self.terminate(Terminator::Branch {
            cond: done,
            then: exit,
            otherwise: body_block,
        });
        self.loop_body(body_block, body, step, exit, bind);
        self.switch_to(step);
        // Through the end, the loop stops at it rather than step past it,
        // which the type may not hold: `0..=255` of a `u8`.
        if past == BinaryOp::Gt {
            let last = self.value(
                Types::BOOL,
                Rvalue::Binary(BinaryOp::Eq, Operand::Copy(Place::local(i)), end),
            );
            let increment = self.new_block();
            self.terminate(Terminator::Branch {
                cond: last,
                then: exit,
                otherwise: increment,
            });
            self.switch_to(increment);
        }
        let next = self.value(
            ty,
            Rvalue::Binary(
                BinaryOp::Add,
                Operand::Copy(Place::local(i)),
                Self::int(1, ty),
            ),
        );
        self.assign(Place::local(i), Rvalue::Use(next));
        self.terminate(Terminator::Goto(header));
        self.switch_to(exit);
        self.drop_temps(mark);
    }

    /// `if`/`else`: each branch assigns a scalar result to a temporary, or
    /// writes an aggregate one to `dest` (or a temporary).
    pub(super) fn if_expr(
        &mut self,
        cond: ExprId,
        then_block: &hir::Block,
        else_block: Option<&hir::Block>,
        ty: Ty,
        dest: Option<Place>,
    ) -> Value {
        let bindings = self.condition_bindings(cond);
        let cond = self.scalar(cond);
        let dest = if self.is_aggregate(ty) {
            Some(dest.unwrap_or_else(|| Place::local(self.temp(ty))))
        } else {
            None
        };
        let result = (dest.is_none() && self.is_scalar(ty)).then(|| self.temp(ty));
        let then_start = self.new_block();
        let else_start = self.new_block();
        let join = self.new_block();
        self.terminate(Terminator::Branch {
            cond,
            then: then_start,
            otherwise: else_start,
        });
        let mut joined = false;
        for (start, block) in [(then_start, Some(then_block)), (else_start, else_block)] {
            self.switch_to(start);
            self.dead = false;
            // Temporaries made in a branch exist on that path only, so they
            // are dropped before the paths join.
            let mark = self.temps.len();
            let value = match block {
                Some(block) => self.block_expr(block, dest.clone()),
                None => Value::Unit,
            };
            if self.dead {
                self.temps.truncate(mark);
                continue;
            }
            self.drop_temps(mark);
            match (result, value) {
                (Some(result), Value::Scalar(operand)) => {
                    self.assign(Place::local(result), Rvalue::Use(operand));
                }
                (Some(_), _) => unreachable!("a branch of a scalar `if` produced no scalar"),
                (None, _) => {}
            }
            self.terminate(Terminator::Goto(join));
            joined = true;
        }
        // Both branches returned: nothing follows the `if`.
        if !joined {
            if bindings {
                self.scopes.pop();
            }
            self.dead = true;
            return Value::Unit;
        }
        self.dead = false;
        self.switch_to(join);
        if bindings {
            self.drop_scope();
        }
        match (result, dest) {
            (Some(result), _) => Value::Scalar(Operand::Copy(Place::local(result))),
            (None, Some(dest)) => Value::Place(dest),
            (None, None) => Value::Unit,
        }
    }

    /// A `match`: a switch on the variant goes to the first
    /// arm that matches it. A scrutinee that is not a place is moved into a
    /// temporary, which the statement drops afterwards, minus whatever the
    /// arms take out of it.
    pub(super) fn match_expr(
        &mut self,
        scrutinee: ExprId,
        arms: &[hir::Arm],
        ty: Ty,
        dest: Option<Place>,
    ) -> Value {
        let scrutinee_ty = self.ty(scrutinee);
        // A tuple matched in place is a place for each element, each tested
        // as a `match` on it alone would test it.
        let tested = match &self.hir.exprs[scrutinee].kind {
            hir::ExprKind::Places(elements) => {
                Tested::Many(elements.iter().map(|&e| self.tested_place(e)).collect())
            }
            _ => {
                let (place, ty) = self.tested_place(scrutinee);
                Tested::One(place, ty)
            }
        };
        // What one place the switches below read; a tuple matched in place
        // is neither an enum nor a number, and they leave it be.
        let place = match &tested {
            Tested::One(place, _) => place.clone(),
            Tested::Many(_) => Place::local(self.temp(Types::BOOL)),
        };
        let variant = match self.kind(scrutinee_ty) {
            TyKind::Enum(..) => Some(self.value(Types::I32, Rvalue::Variant(place.clone()))),
            _ => None,
        };
        let dest = if self.is_aggregate(ty) {
            Some(dest.unwrap_or_else(|| Place::local(self.temp(ty))))
        } else {
            None
        };
        let result = (dest.is_none() && self.is_scalar(ty)).then(|| self.temp(ty));
        let join = self.new_block();
        let arm_blocks: Vec<BlockId> = arms.iter().map(|_| self.new_block()).collect();
        // Exhaustiveness is checked, so no value gets past the last arm.
        let unmatched = self.new_block();
        // An arm that tests one variant and nothing else is a case of one
        // switch, which is what a `match` on an enum has always been.
        // Anything else — a value to equal, alternatives, a guard — is a
        // test of its own, tried in order.
        // A pattern that tests what a field holds is a test of its own,
        // whatever its head is.
        let switchable = variant.is_some()
            && arms.iter().all(|arm| {
                arm.guard.is_none()
                    && !tests_within(&arm.pattern)
                    && matches!(
                        arm.pattern,
                        hir::Pattern::Variant { .. }
                            | hir::Pattern::Wildcard
                            | hir::Pattern::Binding(_)
                            | hir::Pattern::Fields(_)
                    )
            });
        // A `match` on a number whose arms are numbers — an interpreter's
        // opcodes, a byte, a character — is one switch too, where every
        // number fits a case and nothing is guarded; an arm that takes the
        // rest ends it. Tried in order, they were a comparison each.
        let numbers = self.numbers_switch(scrutinee_ty, arms, &arm_blocks, unmatched);
        let mut tests: Vec<BlockId> = Vec::new();
        match variant {
            None if numbers.is_some() => {
                let (cases, otherwise) = numbers.expect("just asked");
                self.terminate(Terminator::Switch {
                    value: Operand::Copy(place.clone()),
                    cases,
                    otherwise,
                });
            }
            Some(variant) if switchable => {
                let mut cases: Vec<(u32, BlockId)> = Vec::new();
                let mut otherwise = unmatched;
                for (arm, &block) in arms.iter().zip(&arm_blocks) {
                    match arm.pattern {
                        hir::Pattern::Variant { variant, .. } => {
                            if !cases.iter().any(|&(case, _)| case == variant) {
                                cases.push((variant, block));
                            }
                        }
                        _ => {
                            otherwise = block;
                            break;
                        }
                    }
                }
                self.terminate(Terminator::Switch {
                    value: variant,
                    cases,
                    otherwise,
                });
            }
            _ if !switchable || variant.is_none() => {
                // One block per arm's test, each falling to the next.
                tests = arms.iter().map(|_| self.new_block()).collect();
                let first = tests.first().copied().unwrap_or(unmatched);
                self.terminate(Terminator::Goto(first));
                for (i, (arm, &block)) in arms.iter().zip(&arm_blocks).enumerate() {
                    let next = *tests.get(i + 1).unwrap_or(&unmatched);
                    self.switch_to(tests[i]);
                    self.test_tested(&arm.pattern, &tested, block, next);
                }
            }
            _ => {
                let first = arm_blocks.first().copied().unwrap_or(unmatched);
                self.terminate(Terminator::Goto(first));
            }
        }
        let mut joined = false;
        for (i, (arm, &block)) in arms.iter().zip(&arm_blocks).enumerate() {
            self.switch_to(block);
            self.dead = false;
            self.scopes.push(Vec::new());
            // What the pattern binds is seen by the arm.
            self.seen_until.push(self.hir.exprs[arm.body].span.hi);
            self.bind_tested(&arm.pattern, &tested);
            // `if …` after the pattern: the arm matches only where it is
            // true, and where it is not the next arm is tried.
            // The bindings are made first, since the
            // guard reads them; they alias, so there is nothing to undo.
            if let Some(guard) = arm.guard {
                let cond = self.scalar(guard);
                let body = self.new_block();
                let next = *tests.get(i + 1).unwrap_or(&unmatched);
                self.terminate(Terminator::Branch {
                    cond,
                    then: body,
                    otherwise: next,
                });
                self.switch_to(body);
            }
            self.drop_untaken_tested(&arm.pattern, &tested);
            // An arm that is one expression is written where it is; a
            // block marks its own statements.
            if !matches!(self.hir.exprs[arm.body].kind, hir::ExprKind::Block(_)) {
                self.at(self.hir.exprs[arm.body].span);
            }
            // Temporaries made in an arm exist on that path only.
            let mark = self.temps.len();
            let value = match &dest {
                Some(dest) => {
                    self.store_expr(arm.body, dest.clone());
                    Value::Place(dest.clone())
                }
                None => self.expr(arm.body),
            };
            self.seen_until.pop();
            if self.dead {
                self.temps.truncate(mark);
                self.scopes.pop();
                continue;
            }
            self.drop_temps(mark);
            self.drop_scope();
            match (result, value) {
                (Some(result), Value::Scalar(operand)) => {
                    self.assign(Place::local(result), Rvalue::Use(operand));
                }
                (Some(_), _) => unreachable!("an arm of a scalar `match` produced no scalar"),
                (None, _) => {}
            }
            self.terminate(Terminator::Goto(join));
            joined = true;
        }
        self.switch_to(unmatched);
        self.terminate(Terminator::Unreachable);
        if !joined {
            self.dead = true;
            return Value::Unit;
        }
        self.dead = false;
        self.switch_to(join);
        match (result, dest) {
            (Some(result), _) => Value::Scalar(Operand::Copy(Place::local(result))),
            (None, Some(dest)) => Value::Place(dest),
            (None, None) => Value::Unit,
        }
    }

    /// The variables of the bindings of `is` tests in a condition,
    /// made before the condition is evaluated. A test binds
    /// only on the path where it succeeds, so each starts as zeros, which
    /// dropping leaves alone. The scope this pushes drops them; it is pushed
    /// only if there are bindings, and the result says whether it was.
    fn condition_bindings(&mut self, cond: ExprId) -> bool {
        fn collect(hir: &hir::Body, e: ExprId, locals: &mut Vec<LocalId>) {
            match &hir.exprs[e].kind {
                ExprKind::Binary {
                    op: BinaryOp::And,
                    lhs,
                    rhs,
                    ..
                } => {
                    collect(hir, *lhs, locals);
                    collect(hir, *rhs, locals);
                }
                ExprKind::Is { pattern, .. } => pattern.locals(locals),
                _ => {}
            }
        }
        let mut locals = Vec::new();
        collect(self.hir, cond, &mut locals);
        if locals.is_empty() {
            return false;
        }
        let mut scope = Vec::new();
        for local in locals {
            let var = self.source_local(local, LocalKind::Var);
            self.push(Statement::Zero(Place::local(var)));
            self.vars.insert(local, var);
            scope.push(Cleanup::Drop(local));
        }
        self.scopes.push(scope);
        true
    }

    /// `val pattern = value else { … }`: on the path where
    /// the value does not match, the `else` block, which leaves; on the
    /// other, the bindings, which the enclosing block drops.
    fn guard(
        &mut self,
        scrutinee: ExprId,
        pattern: &hir::Pattern,
        else_block: Option<&hir::Block>,
    ) {
        // A struct pattern always matches: it tests nothing and only binds.
        let Some(else_block) = else_block else {
            let (place, scrutinee_ty) = self.tested_place(scrutinee);
            self.bind(pattern, &place, scrutinee_ty);
            self.drop_untaken(pattern, &place, scrutinee_ty);
            return;
        };
        let (place, scrutinee_ty) = self.tested_place(scrutinee);
        let matched = self.new_block();
        let unmatched = self.new_block();
        self.test_pattern(pattern, &place, scrutinee_ty, matched, unmatched);
        self.switch_to(unmatched);
        self.block_expr(else_block, None);
        if !self.dead {
            // The type checker requires the block to leave.
            self.terminate(Terminator::Unreachable);
        }
        self.switch_to(matched);
        self.dead = false;
        self.bind(pattern, &place, scrutinee_ty);
        self.drop_untaken(pattern, &place, scrutinee_ty);
    }

    /// The place of a tested value, and its type. A value that is not a
    /// place is moved into a temporary, which the statement drops.
    /// An arm's test of what a `match` tests: its place, or each element of a
    /// tuple matched in place in turn, going to `matched` where every one
    /// matches.
    fn test_tested(
        &mut self,
        pattern: &hir::Pattern,
        tested: &Tested,
        matched: BlockId,
        next: BlockId,
    ) {
        match tested {
            Tested::One(place, ty) => self.test_pattern(pattern, place, *ty, matched, next),
            Tested::Many(places) => {
                let patterns = pattern.elements(places.len());
                for (i, (pattern, (place, ty))) in patterns.iter().zip(places).enumerate() {
                    let pass = if i + 1 == places.len() {
                        matched
                    } else {
                        self.new_block()
                    };
                    self.test_pattern(pattern, place, *ty, pass, next);
                    if pass != matched {
                        self.switch_to(pass);
                    }
                }
            }
        }
    }

    /// What an arm binds of what a `match` tests.
    fn bind_tested(&mut self, pattern: &hir::Pattern, tested: &Tested) {
        match tested {
            Tested::One(place, ty) => self.bind(pattern, place, *ty),
            Tested::Many(places) => {
                for (pattern, (place, ty)) in pattern.elements(places.len()).iter().zip(places) {
                    self.bind(pattern, place, *ty);
                }
            }
        }
    }

    /// What an arm leaves of a value it matched, dropped.
    fn drop_untaken_tested(&mut self, pattern: &hir::Pattern, tested: &Tested) {
        match tested {
            Tested::One(place, ty) => self.drop_untaken(pattern, place, *ty),
            Tested::Many(places) => {
                for (pattern, (place, ty)) in pattern.elements(places.len()).iter().zip(places) {
                    self.drop_untaken(pattern, place, *ty);
                }
            }
        }
    }

    fn tested_place(&mut self, scrutinee: ExprId) -> (Place, Ty) {
        let scrutinee_ty = self.ty(scrutinee);
        let place = if self.is_place_kind(scrutinee) {
            self.place(scrutinee)
        } else {
            let temp = Place::local(self.temp(scrutinee_ty));
            self.store_expr(scrutinee, temp.clone());
            self.own_temp(temp.clone(), scrutinee_ty);
            temp
        };
        (place, scrutinee_ty)
    }

    /// The cases of a switch on a number, and where the rest go, where every
    /// arm of a `match` is numbers that fit a case, with no guard, or takes
    /// every value. Nothing where an arm is anything else.
    fn numbers_switch(
        &self,
        ty: Ty,
        arms: &[hir::Arm],
        blocks: &[BlockId],
        unmatched: BlockId,
    ) -> Option<(Vec<(u32, BlockId)>, BlockId)> {
        // A case is the value's bits, read without a sign; a negative
        // number of a signed type is not one.
        let limit: u128 = match self.kind(ty) {
            TyKind::Int(int) if int.signed() => 1u128 << (int.bits().min(33) - 1),
            TyKind::Int(_) | TyKind::Char => 1u128 << 32,
            _ => return None,
        };
        let mut cases: Vec<(u32, BlockId)> = Vec::new();
        let add = |bits: u128, block: BlockId, cases: &mut Vec<(u32, BlockId)>| {
            if bits >= limit {
                return false;
            }
            let case = bits as u32;
            // An earlier arm has it: that one is taken.
            if !cases.iter().any(|&(earlier, _)| earlier == case) {
                cases.push((case, block));
            }
            true
        };
        for (arm, &block) in arms.iter().zip(blocks) {
            if arm.guard.is_some() {
                return None;
            }
            match &arm.pattern {
                hir::Pattern::Int(bits) => {
                    if !add(*bits, block, &mut cases) {
                        return None;
                    }
                }
                hir::Pattern::Any { alternatives, .. } => {
                    for alternative in alternatives {
                        let hir::Pattern::Int(bits) = alternative else {
                            return None;
                        };
                        if !add(*bits, block, &mut cases) {
                            return None;
                        }
                    }
                }
                hir::Pattern::Wildcard | hir::Pattern::Binding(_) => return Some((cases, block)),
                _ => return None,
            }
        }
        Some((cases, unmatched))
    }

    /// `value is pattern`: a `bool`, and on the path where
    /// the value is the pattern's variant, its bindings.
    pub(super) fn is_test(&mut self, scrutinee: ExprId, pattern: &hir::Pattern) -> Operand {
        let (place, scrutinee_ty) = self.tested_place(scrutinee);
        let result = self.temp(Types::BOOL);
        let matched = self.new_block();
        let unmatched = self.new_block();
        let join = self.new_block();
        // What a nested pattern tests is tested too.
        self.test_pattern(pattern, &place, scrutinee_ty, matched, unmatched);
        self.switch_to(matched);
        self.bind(pattern, &place, scrutinee_ty);
        self.drop_untaken(pattern, &place, scrutinee_ty);
        self.assign(
            Place::local(result),
            Rvalue::Use(Operand::Const(Const::Bool(true))),
        );
        self.terminate(Terminator::Goto(join));
        self.switch_to(unmatched);
        self.assign(
            Place::local(result),
            Rvalue::Use(Operand::Const(Const::Bool(false))),
        );
        self.terminate(Terminator::Goto(join));
        self.switch_to(join);
        Operand::Copy(Place::local(result))
    }

    /// Emits the test a pattern makes, going to `matched` where the value
    /// matches and to `next` where it does not.
    fn test_pattern(
        &mut self,
        pattern: &hir::Pattern,
        place: &Place,
        scrutinee_ty: Ty,
        matched: BlockId,
        next: BlockId,
    ) {
        match pattern {
            // These match every value of their type: the arm is taken.
            hir::Pattern::Wildcard | hir::Pattern::Binding(_) => {
                self.terminate(Terminator::Goto(matched));
            }
            // A struct tests nothing itself; what its fields hold may.
            hir::Pattern::Fields(binders) => {
                let fields: Vec<(u32, &hir::Pattern)> = binders
                    .iter()
                    .enumerate()
                    .filter_map(|(i, binder)| match binder {
                        hir::Binder::Nested(pattern) if tests(pattern) => Some((i as u32, pattern)),
                        _ => None,
                    })
                    .collect();
                self.test_fields(&fields, place, scrutinee_ty, None, matched, next);
            }
            hir::Pattern::Variant { variant, binders } => {
                let tag = self.value(Types::I32, Rvalue::Variant(place.clone()));
                // The fields a nested pattern tests are tested once the
                // variant is known, each failure going on to the next arm.
                let nested = self.new_block();
                self.terminate(Terminator::Switch {
                    value: tag,
                    cases: vec![(*variant, nested)],
                    otherwise: next,
                });
                self.switch_to(nested);
                let fields: Vec<(u32, &hir::Pattern)> = binders
                    .iter()
                    .enumerate()
                    .filter_map(|(i, binder)| match binder {
                        hir::Binder::Nested(pattern) if tests(pattern) => Some((i as u32, pattern)),
                        _ => None,
                    })
                    .collect();
                self.test_fields(&fields, place, scrutinee_ty, Some(*variant), matched, next);
            }
            hir::Pattern::Int(bits) => {
                let value = Operand::Const(Const::Int {
                    bits: *bits,
                    ty: scrutinee_ty,
                });
                let same = self.value(
                    Types::BOOL,
                    Rvalue::Binary(BinaryOp::Eq, Operand::Copy(place.clone()), value),
                );
                self.terminate(Terminator::Branch {
                    cond: same,
                    then: matched,
                    otherwise: next,
                });
            }
            // `lo..=hi`: at least `lo`, then at most `hi`, an end left off
            // not tested.
            hir::Pattern::Range { lo, hi } => {
                let ends = [(BinaryOp::Ge, *lo), (BinaryOp::Le, *hi)];
                let tested: Vec<(BinaryOp, u128)> = ends
                    .into_iter()
                    .filter_map(|(op, end)| end.map(|bits| (op, bits)))
                    .collect();
                for (i, &(op, bits)) in tested.iter().enumerate() {
                    let end = Operand::Const(Const::Int {
                        bits,
                        ty: scrutinee_ty,
                    });
                    let within = self.value(
                        Types::BOOL,
                        Rvalue::Binary(op, Operand::Copy(place.clone()), end),
                    );
                    let last = i + 1 == tested.len();
                    let then = match last {
                        true => matched,
                        false => self.new_block(),
                    };
                    self.terminate(Terminator::Branch {
                        cond: within,
                        then,
                        otherwise: next,
                    });
                    if !last {
                        self.switch_to(then);
                    }
                }
                if tested.is_empty() {
                    self.terminate(Terminator::Goto(matched));
                }
            }
            hir::Pattern::Bool(wanted) => {
                let same = self.value(
                    Types::BOOL,
                    Rvalue::Binary(
                        BinaryOp::Eq,
                        Operand::Copy(place.clone()),
                        Operand::Const(Const::Bool(*wanted)),
                    ),
                );
                self.terminate(Terminator::Branch {
                    cond: same,
                    then: matched,
                    otherwise: next,
                });
            }
            // Text is compared by its bytes, wherever they are: a `String`
            // by the text it lends, `toStr()`.
            hir::Pattern::Str(text) => {
                let held = if self.program.is_string(scrutinee_ty) {
                    let to_str = self
                        .program
                        .prelude_items
                        .function(hir::KnownFn::StringToStr)
                        .expect("the prelude declares `String.toStr`");
                    let receiver_ty = self.program.fns[to_str].params[0].ty;
                    let receiver = self.value(receiver_ty, Rvalue::AddressOf(place.clone()));
                    let lent = Place::local(self.temp(Types::STR));
                    self.push(Statement::Call {
                        callee: Callee::Fn(to_str),
                        args: vec![receiver],
                        dest: Some(lent.clone()),
                    });
                    lent
                } else {
                    place.clone()
                };
                let literal = Place::local(self.temp(Types::STR));
                let len = self.interner.resolve(*text).len() as u128;
                self.assign(
                    literal.clone().project(Projection::Field(0)),
                    Rvalue::Use(Operand::Const(Const::CStr(*text))),
                );
                self.assign(
                    literal.clone().project(Projection::Field(1)),
                    Rvalue::Use(Self::int(len, Types::I64)),
                );
                let order = Place::local(self.temp(Types::I64));
                self.push(Statement::Call {
                    callee: Callee::StrCmp,
                    args: vec![
                        Operand::Copy(held.clone().project(Projection::Field(0))),
                        Operand::Copy(held.project(Projection::Field(1))),
                        Operand::Copy(literal.clone().project(Projection::Field(0))),
                        Operand::Copy(literal.project(Projection::Field(1))),
                    ],
                    dest: Some(order.clone()),
                });
                let same = self.value(
                    Types::BOOL,
                    Rvalue::Binary(BinaryOp::Eq, Operand::Copy(order), Self::int(0, Types::I64)),
                );
                self.terminate(Terminator::Branch {
                    cond: same,
                    then: matched,
                    otherwise: next,
                });
            }
            // Any one of several: each is tried, and the last one that
            // fails goes on to the next arm. Where they bind, which one
            // matched is kept, for the arm to bind from once all of its
            // pattern has matched.
            hir::Pattern::Any {
                alternatives,
                binding,
            } => {
                let which = binding.map(|id| self.alternative_matched(id));
                for (i, alternative) in alternatives.iter().enumerate() {
                    let otherwise = if i + 1 == alternatives.len() {
                        next
                    } else {
                        self.new_block()
                    };
                    let pass = match which {
                        Some(_) => self.new_block(),
                        None => matched,
                    };
                    self.test_pattern(alternative, place, scrutinee_ty, pass, otherwise);
                    if let Some(which) = which {
                        self.switch_to(pass);
                        self.assign(
                            Place::local(which),
                            Rvalue::Use(Self::int(i as u128, Types::I32)),
                        );
                        self.terminate(Terminator::Goto(matched));
                    }
                    if i + 1 < alternatives.len() {
                        self.switch_to(otherwise);
                    }
                }
            }
            // A slice's length, where it may be another, then what the
            // elements hold.
            hir::Pattern::Slice {
                prefix,
                rest,
                suffix,
            } => {
                let (elements, len) = self.pattern_elements(place, scrutinee_ty);
                if matches!(self.kind(scrutinee_ty), TyKind::Slice(_)) {
                    let named = (prefix.len() + suffix.len()) as u128;
                    let op = match rest {
                        Some(_) => BinaryOp::Ge,
                        None => BinaryOp::Eq,
                    };
                    let fits = self.value(
                        Types::BOOL,
                        Rvalue::Binary(op, len.clone(), Self::int(named, Types::I64)),
                    );
                    let then = self.new_block();
                    self.terminate(Terminator::Branch {
                        cond: fits,
                        then,
                        otherwise: next,
                    });
                    self.switch_to(then);
                }
                let elem = self.program.element_ty(scrutinee_ty);
                let tested: Vec<(Place, &hir::Pattern)> = self
                    .slice_parts(prefix, suffix, &elements, scrutinee_ty, &len)
                    .into_iter()
                    .filter_map(|(part, binder)| match binder {
                        hir::Binder::Nested(pattern) if tests(pattern) => Some((part, pattern)),
                        _ => None,
                    })
                    .collect();
                for (i, (part, pattern)) in tested.iter().enumerate() {
                    // The last one that matches takes the arm.
                    let ok = match i + 1 == tested.len() {
                        true => matched,
                        false => self.new_block(),
                    };
                    self.test_pattern(pattern, part, elem, ok, next);
                    self.switch_to(ok);
                }
                if tested.is_empty() {
                    self.terminate(Terminator::Goto(matched));
                }
            }
            // What the reference refers to is tested.
            hir::Pattern::Deref(inner) => {
                let (referent, ty) = self.referent(place, scrutinee_ty);
                self.test_pattern(inner, &referent, ty, matched, next);
            }
            // Reported already; nothing reaches the arm.
            hir::Pattern::Error => self.terminate(Terminator::Goto(next)),
        }
    }

    /// The place a reference at `place` refers to, and its type: through
    /// the pointer, or the reference itself where it is a slice's, whose
    /// pointer and length are the slice's place.
    fn referent(&self, place: &Place, ty: Ty) -> (Place, Ty) {
        let TyKind::Ref(pointee, _) = self.kind(ty) else {
            unreachable!("a pattern tests through a value that is not a reference")
        };
        if crate::is_slice_pointer(self.program, ty) {
            return (place.clone(), pointee);
        }
        (place.clone().project(Projection::Deref), pointee)
    }

    /// The place of an array's or a slice's elements, and how many there
    /// are, for a slice pattern. A slice's place is its
    /// pointer and length.
    fn pattern_elements(&mut self, place: &Place, ty: Ty) -> (Place, Operand) {
        match self.kind(ty) {
            TyKind::Array(_, len) => (place.clone(), Self::int(u128::from(len), Types::I64)),
            _ => {
                let len = self.value(
                    Types::I64,
                    Rvalue::Use(Operand::Copy(place.clone().project(Projection::Field(1)))),
                );
                (place.clone(), len)
            }
        }
    }

    /// The elements a slice pattern names, each with its binder: `prefix`
    /// from the start, `suffix` from the end. The length
    /// was tested, so none is checked.
    fn slice_parts<'b>(
        &mut self,
        prefix: &'b [hir::Binder],
        suffix: &'b [hir::Binder],
        elements: &Place,
        ty: Ty,
        len: &Operand,
    ) -> Vec<(Place, &'b hir::Binder)> {
        let mut parts = Vec::new();
        for (i, binder) in prefix.iter().enumerate() {
            let at = self.slice_element(elements, ty, len, i as u64, false);
            parts.push((at, binder));
        }
        for (j, binder) in suffix.iter().enumerate() {
            let back = (suffix.len() - j) as u64;
            let at = self.slice_element(elements, ty, len, back, true);
            parts.push((at, binder));
        }
        parts
    }

    /// The element `index` from the start, or `index` back from the end,
    /// of an array's or a slice's elements.
    fn slice_element(
        &mut self,
        elements: &Place,
        ty: Ty,
        len: &Operand,
        index: u64,
        from_end: bool,
    ) -> Place {
        if let TyKind::Array(_, n) = self.kind(ty) {
            let at = if from_end { n - index } else { index };
            return elements.clone().project(Projection::ConstIndex(at));
        }
        let at = match from_end {
            true => self.value(
                Types::I64,
                Rvalue::Binary(
                    BinaryOp::Sub,
                    len.clone(),
                    Self::int(u128::from(index), Types::I64),
                ),
            ),
            false => Self::int(u128::from(index), Types::I64),
        };
        let at = self.local_of(at, Types::I64);
        elements.clone().project(Projection::Index(at))
    }

    /// `..rest`: the elements between those a slice pattern names, as a
    /// slice's pointer and length.
    fn bind_rest(
        &mut self,
        local: LocalId,
        elements: &Place,
        len: Operand,
        prefix: u64,
        suffix: u64,
    ) {
        let made = self.vars.get(local).copied();
        let var = made.unwrap_or_else(|| self.source_local(local, LocalKind::Var));
        let first = self.local_of(Self::int(u128::from(prefix), Types::I64), Types::I64);
        let start = self.value(
            Types::PTR_U8,
            Rvalue::AddressOf(elements.clone().project(Projection::Index(first))),
        );
        let count = self.value(
            Types::I64,
            Rvalue::Binary(
                BinaryOp::Sub,
                len,
                Self::int(u128::from(prefix + suffix), Types::I64),
            ),
        );
        let slice = Place::local(var);
        self.assign(
            slice.clone().project(Projection::Field(0)),
            Rvalue::Use(start),
        );
        self.assign(slice.project(Projection::Field(1)), Rvalue::Use(count));
        if made.is_some() {
            return;
        }
        self.vars.insert(local, var);
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(Cleanup::Drop(local));
        }
    }

    /// Tests what the fields of a value hold, in turn: the value matches
    /// where every one of them does, and the first that does not goes on to
    /// the next arm. `variant` says which variant's fields
    /// they are, where the value is an enum's.
    fn test_fields(
        &mut self,
        fields: &[(u32, &hir::Pattern)],
        place: &Place,
        scrutinee_ty: Ty,
        variant: Option<u32>,
        matched: BlockId,
        next: BlockId,
    ) {
        for (i, (field, pattern)) in fields.iter().enumerate() {
            let (inner, ty) = match variant {
                Some(variant) => (
                    place.clone().project(Projection::VariantField {
                        variant,
                        field: *field,
                    }),
                    self.program.variant_field_ty(scrutinee_ty, variant, *field),
                ),
                None => (
                    place.clone().project(Projection::Field(*field)),
                    self.program.field_ty(scrutinee_ty, *field),
                ),
            };
            // The last one that matches takes the arm.
            let ok = match i + 1 == fields.len() {
                true => matched,
                false => self.new_block(),
            };
            self.test_pattern(pattern, &inner, ty, ok, next);
            self.switch_to(ok);
        }
        if fields.is_empty() {
            self.terminate(Terminator::Goto(matched));
        }
    }

    /// Gives a pattern's bindings the fields of the value at `place`.
    fn bind(&mut self, pattern: &hir::Pattern, place: &Place, scrutinee_ty: Ty) {
        match pattern {
            hir::Pattern::Binding(local) => self.bind_local(*local, place.clone(), scrutinee_ty),
            hir::Pattern::Variant { variant, binders } => {
                let program = self.program;
                for (i, binder) in binders.iter().enumerate() {
                    if matches!(binder, hir::Binder::Ignored) {
                        continue;
                    }
                    let field = place.project(Projection::VariantField {
                        variant: *variant,
                        field: i as u32,
                    });
                    let field_ty = program.variant_field_ty(scrutinee_ty, *variant, i as u32);
                    self.binder(binder, field, field_ty);
                }
            }
            // A struct's fields, bound where they lie.
            hir::Pattern::Fields(binders) => {
                let program = self.program;
                for (i, binder) in binders.iter().enumerate() {
                    if matches!(binder, hir::Binder::Ignored) {
                        continue;
                    }
                    let field = place.project(Projection::Field(i as u32));
                    let field_ty = program.field_ty(scrutinee_ty, i as u32);
                    self.binder(binder, field, field_ty);
                }
            }
            // The elements it names, and the slice of those between.
            hir::Pattern::Slice {
                prefix,
                rest,
                suffix,
            } => {
                let (elements, len) = self.pattern_elements(place, scrutinee_ty);
                let elem = self.program.element_ty(scrutinee_ty);
                for (part, binder) in
                    self.slice_parts(prefix, suffix, &elements, scrutinee_ty, &len)
                {
                    self.binder(binder, part, elem);
                }
                if let Some(hir::SliceRest::Bind(local)) = rest {
                    let (p, s) = (prefix.len() as u64, suffix.len() as u64);
                    self.bind_rest(*local, &elements, len, p, s);
                }
            }
            // A value the scrutinee must equal binds nothing; the test
            // is made where the arm is chosen.
            hir::Pattern::Wildcard
            | hir::Pattern::Error
            | hir::Pattern::Int(_)
            | hir::Pattern::Range { .. }
            | hir::Pattern::Bool(_)
            | hir::Pattern::Str(_) => {}
            // From whichever alternative matched.
            hir::Pattern::Any {
                alternatives,
                binding: Some(id),
            } => {
                let which = self.alternative_matched(*id);
                self.each_alternative(alternatives, which, |this, alternative| {
                    this.bind(alternative, place, scrutinee_ty);
                });
            }
            hir::Pattern::Any { binding: None, .. } => {}
            // Aliases of what the reference refers to.
            hir::Pattern::Deref(inner) => {
                let (referent, ty) = self.referent(place, scrutinee_ty);
                self.bind(inner, &referent, ty);
            }
        }
    }

    /// The local that says which alternative of the `|` numbered `id`
    /// matched.
    fn alternative_matched(&mut self, id: u32) -> Local {
        if let Some(&which) = self.alternatives_matched.get(&id) {
            return which;
        }
        let which = self.temp(Types::I32);
        self.alternatives_matched.insert(id, which);
        which
    }

    /// Does `each` for the alternative `which` says matched, on its own
    /// path, the paths joining after.
    fn each_alternative<'p>(
        &mut self,
        alternatives: &'p [hir::Pattern],
        which: Local,
        mut each: impl FnMut(&mut Self, &'p hir::Pattern),
    ) {
        let join = self.new_block();
        let blocks: Vec<BlockId> = alternatives.iter().map(|_| self.new_block()).collect();
        let Some((&last, rest)) = blocks.split_last() else {
            self.terminate(Terminator::Goto(join));
            self.switch_to(join);
            return;
        };
        self.terminate(Terminator::Switch {
            value: Operand::Copy(Place::local(which)),
            cases: rest
                .iter()
                .enumerate()
                .map(|(i, &block)| (i as u32, block))
                .collect(),
            otherwise: last,
        });
        for (alternative, &block) in alternatives.iter().zip(&blocks) {
            self.switch_to(block);
            each(self, alternative);
            self.terminate(Terminator::Goto(join));
        }
        self.switch_to(join);
    }

    /// What a binder does with the field it took: bind it, or take it apart
    /// in turn.
    fn binder(&mut self, binder: &hir::Binder, field: Place, field_ty: Ty) {
        match binder {
            hir::Binder::Ignored => {}
            hir::Binder::Bind(local) => self.bind_local(*local, field, field_ty),
            hir::Binder::Nested(pattern) => self.bind(pattern, &field, field_ty),
        }
    }

    /// Where a pattern took part of a temporary it moved out of — a field
    /// bound by value — the temporary is no longer dropped whole, so the
    /// fields nothing took are dropped here, where the pattern binds:
    /// `.Edits(changes, ..)` drops the listing `..` leaves. A temporary
    /// the pattern took nothing from is still dropped whole, where its
    /// statement ends.
    fn drop_untaken(&mut self, pattern: &hir::Pattern, place: &Place, ty: Ty) {
        if !self.temp_flags.contains_key(&place.local) || !self.takes(pattern, ty) {
            return;
        }
        self.drop_untaken_parts(pattern, place, ty);
    }

    fn drop_untaken_parts(&mut self, pattern: &hir::Pattern, place: &Place, ty: Ty) {
        // What the alternative that matched left.
        if let hir::Pattern::Any {
            alternatives,
            binding: Some(id),
        } = pattern
        {
            let which = self.alternative_matched(*id);
            self.each_alternative(alternatives, which, |this, alternative| {
                this.drop_untaken_parts(alternative, place, ty);
            });
            return;
        }
        let program = self.program;
        let parts: Vec<(Place, Ty, &hir::Binder)> = match pattern {
            hir::Pattern::Variant { variant, binders } => binders
                .iter()
                .enumerate()
                .map(|(i, binder)| {
                    let field = place.project(Projection::VariantField {
                        variant: *variant,
                        field: i as u32,
                    });
                    (
                        field,
                        program.variant_field_ty(ty, *variant, i as u32),
                        binder,
                    )
                })
                .collect(),
            hir::Pattern::Fields(binders) => binders
                .iter()
                .enumerate()
                .map(|(i, binder)| {
                    let field = place.project(Projection::Field(i as u32));
                    (field, program.field_ty(ty, i as u32), binder)
                })
                .collect(),
            // An array taken apart: what it names, and what `..` passes
            // over. A slice is only ever lent.
            hir::Pattern::Slice {
                prefix,
                rest,
                suffix,
            } => {
                let TyKind::Array(elem, n) = self.kind(ty) else {
                    return;
                };
                if rest.is_some() {
                    for i in prefix.len() as u64..n - suffix.len() as u64 {
                        self.drop_place(&place.project(Projection::ConstIndex(i)), elem);
                    }
                }
                let len = Self::int(u128::from(n), Types::I64);
                self.slice_parts(prefix, suffix, place, ty, &len)
                    .into_iter()
                    .map(|(part, binder)| (part, elem, binder))
                    .collect()
            }
            _ => return,
        };
        for (field, field_ty, binder) in parts {
            match binder {
                hir::Binder::Ignored => self.drop_place(&field, field_ty),
                hir::Binder::Nested(inner) => self.drop_untaken_parts(inner, &field, field_ty),
                hir::Binder::Bind(_) => {}
            }
        }
    }

    /// Whether a pattern takes anything that owns memory by value.
    fn takes(&mut self, pattern: &hir::Pattern, ty: Ty) -> bool {
        let program = self.program;
        let fields: Vec<(Ty, &hir::Binder)> = match pattern {
            hir::Pattern::Variant { variant, binders } => binders
                .iter()
                .enumerate()
                .map(|(i, binder)| (program.variant_field_ty(ty, *variant, i as u32), binder))
                .collect(),
            hir::Pattern::Fields(binders) => binders
                .iter()
                .enumerate()
                .map(|(i, binder)| (program.field_ty(ty, i as u32), binder))
                .collect(),
            hir::Pattern::Slice { prefix, suffix, .. } => prefix
                .iter()
                .chain(suffix)
                .map(|binder| (program.element_ty(ty), binder))
                .collect(),
            hir::Pattern::Any {
                alternatives,
                binding: Some(_),
            } => {
                return alternatives
                    .iter()
                    .any(|alternative| self.takes(alternative, ty));
            }
            _ => return false,
        };
        fields.into_iter().any(|(field_ty, binder)| match binder {
            hir::Binder::Ignored => false,
            hir::Binder::Bind(local) => {
                !self.hir.alias_bindings.contains(local) && self.needs_drop(field_ty)
            }
            hir::Binder::Nested(inner) => self.takes(inner, field_ty),
        })
    }

    /// An alias binding holds the address of its field. Any other binding
    /// takes the field, a reference included, leaving zeros
    /// behind so that it is not dropped twice.
    fn bind_local(&mut self, local: LocalId, field: Place, ty: Ty) {
        // The binding of an `is` test already has its variable, and its
        // cleanup.
        let made = self.vars.get(local).copied();
        let var = made.unwrap_or_else(|| self.source_local(local, LocalKind::Var));
        if self.hir.alias_bindings.contains(&local) {
            self.assign(Place::local(var), Rvalue::AddressOf(field));
        } else {
            self.assign(Place::local(var), Rvalue::Use(Operand::Copy(field.clone())));
            if self.needs_drop(ty) {
                // What the binding took is no longer the value's to drop.
                // Zeros say so for an `own`; a type that cleans up after
                // itself needs the temporary's flag.
                self.taken_from_temp(&field);
                self.vacate(field, ty);
            }
            // It holds a value now: a binding moved away on one path only is
            // dropped on the others, which its flag says. Without this the flag
            // was only ever false.
            self.set_flag(local, true);
        }
        if made.is_some() {
            return;
        }
        self.vars.insert(local, var);
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(Cleanup::Drop(local));
        }
    }

    /// `&&` and `||` evaluate the right operand only when needed.
    pub(super) fn short_circuit(&mut self, op: BinaryOp, lhs: ExprId, rhs: ExprId) -> Operand {
        let lhs = self.scalar(lhs);
        let result = self.temp(Types::BOOL);
        self.assign(Place::local(result), Rvalue::Use(lhs.clone()));
        let rhs_block = self.new_block();
        let merge = self.new_block();
        let (then, otherwise) = if op == BinaryOp::And {
            (rhs_block, merge)
        } else {
            (merge, rhs_block)
        };
        self.terminate(Terminator::Branch {
            cond: lhs,
            then,
            otherwise,
        });
        self.switch_to(rhs_block);
        // Temporaries made on the right only exist on this path, so they are
        // dropped before the paths join.
        let mark = self.temps.len();
        let rhs = self.scalar(rhs);
        self.drop_temps(mark);
        self.assign(Place::local(result), Rvalue::Use(rhs));
        self.terminate(Terminator::Goto(merge));
        self.switch_to(merge);
        Operand::Copy(Place::local(result))
    }
}

/// Whether a pattern can fail: what always matches needs no test.
fn tests(pattern: &hir::Pattern) -> bool {
    match pattern {
        hir::Pattern::Wildcard | hir::Pattern::Binding(_) => false,
        hir::Pattern::Fields(binders) => binders.iter().any(|binder| match binder {
            hir::Binder::Nested(nested) => tests(nested),
            _ => false,
        }),
        hir::Pattern::Deref(inner) => tests(inner),
        _ => true,
    }
}

/// Whether anything inside a pattern tests a value: the head of the
/// pattern aside, a nested pattern may fail.
fn tests_within(pattern: &hir::Pattern) -> bool {
    let binders: Vec<&hir::Binder> = match pattern {
        hir::Pattern::Variant { binders, .. } | hir::Pattern::Fields(binders) => {
            binders.iter().collect()
        }
        hir::Pattern::Slice { prefix, suffix, .. } => prefix.iter().chain(suffix).collect(),
        _ => return false,
    };
    binders.into_iter().any(|binder| match binder {
        hir::Binder::Nested(nested) => tests(nested),
        _ => false,
    })
}

/// Where a counted loop ends: before its end, or at it, which it runs for
/// too.
enum End {
    Before(Operand),
    Through(Operand),
}

/// What a `match` tests: one place, or the elements of a tuple matched in
/// place, each where it is.
enum Tested {
    One(Place, Ty),
    Many(Vec<(Place, Ty)>),
}
