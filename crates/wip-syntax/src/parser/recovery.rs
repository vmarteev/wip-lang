//! Reporting errors and recovering from them: expected tokens, skipping to
//! a synchronization point, unclosed braces, and misplaced operators.

use super::*;

impl<'a> Parser<'a> {
    pub(super) fn begin_region(&mut self) {
        self.region_start = self.prev_span().hi;
    }

    pub(super) fn report(&mut self, diagnostic: Diagnostic) {
        let first_gap = self.gaps.partition_point(|g| g.lo < self.region_start);
        if self
            .gaps
            .get(first_gap)
            .is_some_and(|g| g.lo <= self.span().hi)
        {
            return;
        }
        let at = diagnostic.primary.span.lo;
        if self.last_error_at == Some(at) {
            return;
        }
        self.last_error_at = Some(at);
        self.diagnostics.push(diagnostic);
    }

    /// Builds "expected X, found Y". When the unexpected token is on a later
    /// line, the error points just after the previous token — where the
    /// missing piece belongs — rather than at the start of the next line.
    pub(super) fn expected(&self, what: &str) -> Diagnostic {
        let span = if self.pos > 0 && self.on_later_line() {
            Span::at(self.prev_span().hi)
        } else {
            self.span()
        };
        let message = format!("expected {what}, found {}", self.found());
        let diagnostic =
            Diagnostic::error(codes::EXPECTED, message, span, format!("expected {what}"));
        // An annotation belongs before a declaration, and nowhere else.
        if self.peek() == T::At {
            return diagnostic.with_note(
                "an annotation is written before a function, a struct, an enum, an interface or an `extend` block",
            );
        }
        diagnostic
    }

    pub(super) fn expect(&mut self, kind: T) -> bool {
        if self.eat(kind) {
            return true;
        }
        let text = kind.text().expect("expect() takes a fixed token");
        let diagnostic = self.expected(&format!("`{text}`"));
        self.report(diagnostic);
        false
    }

    /// Expects the delimiter that closes `open`, pointing back at `open` if
    /// it is missing.
    pub(super) fn expect_closing(&mut self, close: T, open: Span) -> bool {
        if self.eat(close) {
            return true;
        }
        let text = close.text().expect("closing delimiters are fixed tokens");
        let diagnostic = self
            .expected(&format!("`{text}`"))
            .with_secondary(open, format!("to close this `{}`", self.text(open)))
            .with_fix(
                format!("add `{text}`"),
                [Edit::insert(self.prev_span().hi, text)],
            );
        self.report(diagnostic);
        false
    }

    /// Skips tokens, keeping `(`/`[`/`{` balanced, until `stop` accepts one
    /// outside any nested group; `stop` also learns whether a significant line
    /// break comes before it. Never skips an item keyword, the end of file,
    /// or a closing delimiter that belongs to an enclosing group.
    pub(super) fn skip_until(&mut self, stop: impl Fn(T, bool) -> bool) {
        let mut depth = 0usize;
        loop {
            let kind = self.peek();
            if kind == T::Eof
                || is_item_start(kind)
                || (depth == 0 && stop(kind, self.line_break()))
            {
                return;
            }
            match kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.bump();
        }
    }

    /// Skips the rest of a statement: up to the next `;`, line break or
    /// statement keyword.
    pub(super) fn skip_stmt(&mut self) {
        self.skip_until(|k, line_break| k == T::Semi || line_break || is_stmt_keyword(k));
    }

    /// Past a block whose head was rejected: the rest of the head, and then
    /// the braces with what is inside them, so that its methods are not read
    /// as items of the file.
    pub(super) fn skip_braced(&mut self) {
        while !matches!(self.peek(), T::LBrace | T::RBrace | T::Eof) {
            self.bump();
        }
        if !self.at(T::LBrace) {
            return;
        }
        let mut depth = 0usize;
        loop {
            match self.peek() {
                T::Eof => return,
                T::LBrace => depth += 1,
                T::RBrace => {
                    self.bump();
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                    continue;
                }
                _ => {}
            }
            self.bump();
        }
    }

    pub(super) fn skip_to_item(&mut self) {
        while !(self.at(T::Eof) || is_item_start(self.peek())) {
            self.bump();
        }
    }

    pub(super) fn line_of(&self, pos: u32) -> usize {
        self.src[..self.local(pos)].matches('\n').count() + 1
    }

    pub(super) fn line_start(&self, pos: u32) -> usize {
        self.src[..self.local(pos)].rfind('\n').map_or(0, |i| i + 1)
    }

    pub(super) fn indent_of(&self, pos: u32) -> usize {
        let start = self.line_start(pos);
        self.src[start..]
            .bytes()
            .take_while(|&b| b == b' ' || b == b'\t')
            .count()
    }

    pub(super) fn first_on_line(&self, pos: u32) -> bool {
        self.src[self.line_start(pos)..self.local(pos)]
            .bytes()
            .all(|b| b == b' ' || b == b'\t')
    }

    pub(super) fn unclosed_brace(&mut self, open: Span) {
        let mut diagnostic = Diagnostic::error(
            codes::UNCLOSED_BRACE,
            "unclosed `{`",
            open,
            "this `{` is never closed",
        );
        // Guess which brace is really missing: a later `}` that is indented
        // like `open`'s line, but closes a `{` opened on a line with different
        // indentation. That `}` was probably meant for `open`.
        let indent = self.indent_of(open.lo);
        let suspect = self.brace_pairs.iter().find(|(o, c)| {
            o.lo > open.lo
                && self.first_on_line(c.lo)
                && self.indent_of(c.lo) == indent
                && self.indent_of(o.lo) != indent
        });
        if let Some(&(inner_open, inner_close)) = suspect {
            let line = self.line_of(inner_open.lo);
            diagnostic = diagnostic
                .with_secondary(
                    inner_close,
                    format!(
                        "this `}}` is indented to close the block above, but it closes the `{{` on line {line}"
                    ),
                )
                .with_secondary(inner_open, "this `{` may be the one missing its `}`");
        }
        self.report(diagnostic);
    }

    /// A line that begins with a binary operator was meant to continue the
    /// line above, but a line break ends a complete expression (E0113).
    /// Reports it with a fix that joins the lines and skips the rest of the
    /// line. Returns whether there was one.
    pub(super) fn leading_operator(&mut self) -> bool {
        if !(self.pos > 0 && is_binary_only(self.peek()) && self.line_break()) {
            return false;
        }
        let op = self.span();
        let text = self.text(op);
        let between = Span::new(self.prev_span().hi, op.lo);
        let mut diagnostic = Diagnostic::error(
            codes::LEADING_OPERATOR,
            format!("a line cannot start with `{text}`"),
            op,
            "this line starts with an operator",
        )
        .with_note(
            "a line break ends an expression that is complete, so an operator at the start of the next line does not continue it",
        );
        // Joining the lines would delete a comment at the end of the line.
        if !self.text(between).contains("//") {
            diagnostic = diagnostic.with_fix(
                format!("move `{text}` to the end of the previous line"),
                [Edit::replace(between, " ")],
            );
        }
        self.report(diagnostic);
        self.bump();
        self.skip_until(|k, line_break| line_break || k == T::Semi || k == T::Comma);
        true
    }
}
