//! Diagnostics for using a moved value, and assignment that refills one.

use super::*;

impl Checker<'_> {
    /// Checks that `path` holds a value where it is used, and reports the
    /// move that emptied it otherwise.
    pub(super) fn check_live(
        &mut self,
        path: &Path,
        name: &str,
        span: Span,
        state: &State,
    ) -> bool {
        let Some((moved, m)) = state.conflict(path) else {
            return true;
        };
        let first = m.spans[0];
        let by_defer = m.by_defer;
        if self.quiet > 0 || !self.reported.insert(first) {
            return false;
        }
        let partial = moved != path && moved.within(path);
        let root = self.interner.resolve(self.body.locals[path.local].name);
        // A move inside the enclosing loop that is not before this use in
        // the same iteration happened in the previous one.
        let previous_iteration = self.loops.last().and_then(|body| {
            m.spans.iter().copied().find(|s| {
                body.lo <= s.lo
                    && s.hi <= body.hi
                    && (s.lo >= span.lo || (s.lo <= span.lo && span.hi <= s.hi))
            })
        });
        let diagnostic = if let Some(move_span) = previous_iteration
            && move_span.lo <= span.lo
            && span.hi <= move_span.hi
        {
            // The use is the move itself: `run(move job)` in a loop.
            Diagnostic::error(
                codes::MOVED_IN_PREVIOUS_ITERATION,
                format!("`{}` is moved in every iteration of the loop", m.name),
                move_span,
                "moved here, but only the first iteration has a value to move",
            )
            .with_help(format!("assign `{root}` a new value before the next iteration, or declare it inside the loop"))
        } else if let Some(move_span) = previous_iteration {
            Diagnostic::error(
                codes::MOVED_IN_PREVIOUS_ITERATION,
                format!("`{}` is used after it was moved in the previous iteration of the loop", m.name),
                span,
                "used here in the next iteration",
            )
            .with_secondary(move_span, "moved here in the previous iteration")
            .with_help(format!("assign `{root}` a new value before the next iteration, or declare it inside the loop"))
        } else if partial {
            let code = if m.definite {
                codes::USE_OF_MOVED_VALUE
            } else {
                codes::USE_OF_POSSIBLY_MOVED_VALUE
            };
            Diagnostic::error(
                code,
                format!("use of partially moved value `{name}`"),
                span,
                format!("`{name}` used as a whole here"),
            )
            .with_secondary(first, format!("`{}` moved here", m.name))
            .with_note(format!(
                "the other fields of `{name}` can still be used, but not `{name}` as a whole"
            ))
        } else if m.definite {
            Diagnostic::error(
                codes::USE_OF_MOVED_VALUE,
                format!("use of moved value `{}`", m.name),
                span,
                "used here after the move",
            )
            .with_secondary(first, format!("`{}` moved here", m.name))
            .with_note(format!(
                "after `move`, `{}` holds no value until it is assigned again",
                m.name
            ))
        } else {
            let mut diagnostic = Diagnostic::error(
                codes::USE_OF_POSSIBLY_MOVED_VALUE,
                format!("use of possibly moved value `{}`", m.name),
                span,
                "used here",
            );
            for &s in m.spans.iter().take(3) {
                diagnostic = diagnostic.with_secondary(s, format!("`{}` moved here", m.name));
            }
            diagnostic
                .with_note(format!(
                    "`{}` is moved on some paths to this point, so it may hold no value",
                    m.name
                ))
                .with_help("move it on every path, or assign it a new value after the move")
        };
        let diagnostic = if let Some((exit, label)) = self.exit {
            diagnostic.with_secondary(exit, label).with_note(
                "a deferred expression runs where its block exits, and uses its variables as they are there",
            )
        } else if by_defer {
            diagnostic.with_note("a `defer` makes its moves when it runs, where its block exits")
        } else {
            diagnostic
        };
        self.report(diagnostic);
        false
    }

    pub(super) fn assign(&mut self, path: Path, name: &str, span: Span, state: &mut State) {
        // Assigning a field of a struct that was moved as a whole would
        // write into nothing.
        let around = state
            .moved
            .iter()
            .find(|(moved, _)| path.within(moved) && **moved != path);
        if let Some((_, m)) = around {
            let (moved_name, first) = (m.name.clone(), m.spans[0]);
            let diagnostic = Diagnostic::error(
                codes::ASSIGN_TO_PART_OF_MOVED,
                format!("cannot assign to `{name}`: `{moved_name}` was moved"),
                span,
                "assigned here",
            )
            .with_secondary(first, format!("`{moved_name}` moved here"))
            .with_help(format!(
                "assign all of `{moved_name}` instead: `{moved_name} = …`"
            ));
            self.report(diagnostic);
            return;
        }
        state.reinit(&path);
    }
}
