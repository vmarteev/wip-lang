//! A `String` copied where it could have been moved.
//!
//! Text given where a `String` is expected is copied into one, and so is
//! `String::of(name.toStr())`. Where the text is the whole of a `String`
//! the function owns, and nothing reads that variable after the copy on
//! some path, the variable itself could have been given, and moved: the
//! copy is a warning (E0448). A copy stays unread until a use of the
//! variable — a read, a borrow, a move — reads it; assigning it a new value
//! does not, since the old one could have been moved first.

use super::*;
use wip_hir::KnownFn;

impl Checker<'_> {
    /// Notes the call `id` where it copies the whole text of a `String`
    /// variable the function owns: `String::of(name.toStr())`.
    pub(super) fn record_copy(
        &mut self,
        id: ExprId,
        callee: FnId,
        args: &[ExprId],
        state: &mut State,
    ) {
        let known = &self.program.prelude_items;
        if Some(callee) != known.function(KnownFn::StringOf) {
            return;
        }
        let body = self.body;
        let [text] = args else {
            return;
        };
        let ExprKind::Call {
            callee: to_str,
            args: inner,
            ..
        } = &body.exprs[*text].kind
        else {
            return;
        };
        let [receiver] = inner[..] else {
            return;
        };
        if Some(*to_str) != known.function(KnownFn::StringToStr) {
            return;
        }
        let receiver = match body.exprs[receiver].kind {
            ExprKind::Ref(inner) => inner,
            _ => receiver,
        };
        let ExprKind::Local(local) = body.exprs[receiver].kind else {
            return;
        };
        let owned = matches!(
            body.locals[local].kind,
            LocalKind::Param | LocalKind::Let { .. } | LocalKind::Var
        );
        if !owned || !self.program.is_string(body.locals[local].ty) {
            return;
        }
        // Something kept borrows it, and would be left with nothing.
        if state
            .roots
            .values()
            .any(|paths| paths.iter().any(|path| path.local == local))
        {
            return;
        }
        let span = body.exprs[id].span;
        if !self.copies.contains(&(span, local)) {
            self.copies.push((span, local));
        }
        let spans = state.copied.entry(local).or_default();
        if !spans.contains(&span) {
            spans.push(span);
        }
    }

    /// `local` is used: its copies were not its last use. Where it is
    /// assigned, they were, and the old value could have been moved.
    pub(super) fn read_after_copy(&mut self, local: LocalId, assigned: bool, state: &mut State) {
        let Some(spans) = state.copied.remove(&local) else {
            return;
        };
        if !assigned {
            self.copies_read.extend(spans);
        }
    }

    /// Warns of each copy that nothing read its variable after.
    pub(super) fn report_copies(&mut self) {
        for &(span, local) in &self.copies {
            if self.copies_read.contains(&span) {
                continue;
            }
            let name = self.interner.resolve(self.body.locals[local].name);
            self.diagnostics.push(
                Diagnostic::warning(
                    codes::STRING_COPIED,
                    format!("`{name}` is copied here, and not used again"),
                    span,
                    format!("a copy of `{name}`'s text"),
                )
                .with_fix(
                    format!("give `{name}` itself, which moves it"),
                    [Edit::replace(span, format!("move {name}"))],
                )
                .with_note(
                    "a `String` given where one is kept is moved, and its text is copied only where the `String` is still wanted",
                ),
            );
        }
    }
}
