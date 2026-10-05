//! `@tailrec`: a function whose calls to itself are jumps.
//!
//! Tail position is decided here, where the spans are: the value of the
//! body, the value of a block in tail position, every arm of an `if` or a
//! `match` in tail position, and the operand of `return`. A call to the
//! function itself anywhere else is an error, and so is an argument that
//! refers into the frame the call is about to reuse. What is left — that no
//! local still owns memory, and that no `defer` is pending — needs to know
//! what has been moved, so the move checker asks it.

use super::*;

impl Lowerer<'_> {
    /// Checks a `@tailrec` function once its body is checked, and records the
    /// calls the mid-level IR turns into jumps.
    pub(super) fn check_tailrec(&mut self, id: FnId, value: ExprId) {
        // A projection lends a place of its caller; `yield` is not a call.
        if self.program.fns[id].projects.is_some()
            || matches!(self.kind(self.program.fns[id].ret), TyKind::Ref(..))
        {
            let span = self.program.fns[id].span;
            let diagnostic = Diagnostic::error(
                codes::NOT_A_TAIL_CALL,
                "a projection cannot be `@tailrec`",
                span,
                "a projection",
            )
            .with_note(
                "a projection lends a place that belongs to its caller, and ends with `lend` rather than a call",
            );
            self.report(diagnostic);
            return;
        }
        let mut tails = Vec::new();
        self.tail_positions(value, &mut tails);
        // `return f(x)` is a tail call too, wherever the `return` is written.
        self.returned_values(value, &mut tails);
        let tails: FxHashSet<ExprId> = tails.into_iter().collect();
        // Every call the function makes to itself, in the order written.
        let mut calls = Vec::new();
        self.self_calls(value, id, &mut calls);
        if calls.is_empty() {
            let span = self.program.fns[id].name_span;
            let name = self.text(self.program.fns[id].name).to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_A_TAIL_CALL,
                format!("`{name}` never calls itself"),
                span,
                "no call to itself",
            )
            .with_note(
                "`@tailrec` promises that a function's calls to itself are jumps; a function that makes none has nothing to promise",
            )
            .with_help("remove `@tailrec`");
            self.report(diagnostic);
            return;
        }
        let mut tail_calls = Vec::new();
        for call in calls {
            if !tails.contains(&call) {
                let span = self.state.body.exprs[call].span;
                let diagnostic = Diagnostic::error(
                    codes::NOT_A_TAIL_CALL,
                    "this call to itself is not a tail call",
                    span,
                    "its value is used by what is around it",
                )
                .with_note(
                    "a tail call's value is the function's value, and nothing of the function runs after it",
                )
                .with_help(
                    "in a `@tailrec` function every call to itself is a jump: carry what is computed after the call in a parameter instead",
                );
                self.report(diagnostic);
                continue;
            }
            if self.borrows_frame(call) {
                continue;
            }
            tail_calls.push(call);
        }
        self.state.body.tail_calls = tail_calls;
    }

    /// The expressions in tail position, starting from the body's value.
    fn tail_positions(&self, id: ExprId, tails: &mut Vec<ExprId>) {
        tails.push(id);
        let body = &self.state.body;
        match &body.exprs[id].kind {
            // A block's value is in tail position; its statements are not,
            // except a `return`, which the walk below finds anyway.
            ExprKind::Block(block) => {
                if let Some(value) = block.value {
                    self.tail_positions(value, tails);
                }
            }
            ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                for block in [Some(then_block), else_block.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    if let Some(value) = block.value {
                        self.tail_positions(value, tails);
                    }
                }
            }
            ExprKind::Match { arms, .. } => {
                for arm in arms {
                    self.tail_positions(arm.body, tails);
                }
            }
            _ => {}
        }
    }

    /// The operands of every `return` in the body, which are in tail
    /// position wherever they are written, and what is in tail position
    /// inside them.
    fn returned_values(&self, id: ExprId, tails: &mut Vec<ExprId>) {
        let kind = &self.state.body.exprs[id].kind;
        for child in kind.children() {
            self.returned_values(child, tails);
        }
        for block in kind.blocks() {
            self.returned_values_in_block(block, tails);
        }
        for arm in kind.arms() {
            self.returned_values(arm.body, tails);
        }
    }

    fn returned_values_in_block(&self, block: &Block, tails: &mut Vec<ExprId>) {
        for &stmt in &block.stmts {
            let kind = &self.state.body.stmts[stmt].kind;
            if let StmtKind::Return(Some(value)) = kind {
                self.tail_positions(*value, tails);
            }
            for child in kind.children() {
                self.returned_values(child, tails);
            }
            for inner in kind.blocks() {
                self.returned_values_in_block(inner, tails);
            }
        }
        if let Some(value) = block.value {
            self.returned_values(value, tails);
        }
    }

    /// Every call of the function to itself, wherever it is written. The
    /// operand of a `return` is in tail position too, so the statements are
    /// walked for those.
    fn self_calls(&self, id: ExprId, function: FnId, calls: &mut Vec<ExprId>) {
        let body = &self.state.body;
        let kind = &body.exprs[id].kind;
        if let ExprKind::Call { callee, .. } = kind
            && *callee == function
        {
            calls.push(id);
        }
        for child in kind.children() {
            self.self_calls(child, function, calls);
        }
        for block in kind.blocks() {
            self.self_calls_in_block(block, function, calls);
        }
        for arm in kind.arms() {
            self.self_calls(arm.body, function, calls);
        }
    }

    fn self_calls_in_stmt(&self, id: StmtId, function: FnId, calls: &mut Vec<ExprId>) {
        let kind = &self.state.body.stmts[id].kind;
        for child in kind.children() {
            self.self_calls(child, function, calls);
        }
        for block in kind.blocks() {
            self.self_calls_in_block(block, function, calls);
        }
    }

    fn self_calls_in_block(&self, block: &Block, function: FnId, calls: &mut Vec<ExprId>) {
        for &stmt in &block.stmts {
            self.self_calls_in_stmt(stmt, function, calls);
        }
        if let Some(value) = block.value {
            self.self_calls(value, function, calls);
        }
    }

    /// Whether an argument of the call refers into this frame, which the
    /// jump is about to reuse. Reports it, and returns
    /// whether it did.
    fn borrows_frame(&mut self, call: ExprId) -> bool {
        let ExprKind::Call { args, .. } = self.state.body.exprs[call].kind.clone() else {
            unreachable!("a self-call is a call");
        };
        let mut found = None;
        for arg in args {
            self.frame_reference(arg, &mut found);
        }
        let Some((span, what)) = found else {
            return false;
        };
        let diagnostic = Diagnostic::error(
            codes::NOT_A_TAIL_CALL,
            format!("this argument refers into the frame the call reuses, so it cannot be a {what}"),
            span,
            "points into this call's frame",
        )
        .with_note(
            "a `@tailrec` function's call to itself is a jump, so this frame is the one the next round uses",
        )
        .with_help("pass the value itself, or a reference the function was given");
        self.report(diagnostic);
        true
    }

    /// The first reference into this frame inside `id`, if there is one.
    fn frame_reference(&self, id: ExprId, found: &mut Option<(Span, &'static str)>) {
        if found.is_some() {
            return;
        }
        let body = &self.state.body;
        let kind = &body.exprs[id].kind;
        match kind {
            // A closure lent for one call keeps its environment in this
            // frame; an owned one is on the heap.
            ExprKind::Closure { .. } => {
                *found = Some((body.exprs[id].span, "closure"));
                return;
            }
            ExprKind::Ref(inner) if self.roots_here(*inner) => {
                *found = Some((body.exprs[id].span, "reference"));
                return;
            }
            _ => {}
        }
        for child in kind.children() {
            self.frame_reference(child, found);
        }
    }

    /// Whether a place stands in this frame, rather than behind a reference
    /// the function was given.
    fn roots_here(&self, id: ExprId) -> bool {
        let body = &self.state.body;
        match &body.exprs[id].kind {
            ExprKind::Local(local) => !matches!(self.kind(body.locals[*local].ty), TyKind::Ref(..)),
            ExprKind::Field { base, .. }
            | ExprKind::Index { base, .. }
            | ExprKind::SubSlice { base, .. } => self.roots_here(*base),
            // A dereference reaches what the reference points to, which is
            // another frame's.
            ExprKind::Deref(_) => false,
            // A temporary of this call: it lives in this frame.
            _ => true,
        }
    }
}
