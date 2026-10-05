//! Comma-separated lists in parentheses and brackets, and brace lists
//! separated by line breaks or `,`.

use super::*;

impl<'a> Parser<'a> {
    /// Parses `elem ("," elem)* ","?` followed by `close`, after `open` has been
    /// consumed. Line breaks are whitespace inside. Returns the elements and
    /// the span of the closing delimiter (empty, where it should have been, if
    /// it is missing).
    pub(super) fn list<E>(
        &mut self,
        open: Span,
        close: T,
        what: &str,
        starts_elem: fn(T) -> bool,
        elem: impl FnMut(&mut Self) -> Option<E>,
    ) -> (Vec<E>, Span) {
        self.list_after(open, None, close, what, starts_elem, elem)
    }

    /// Like [`Parser::list`], when the first element has already been parsed.
    pub(super) fn list_after<E>(
        &mut self,
        open: Span,
        first: Option<E>,
        close: T,
        what: &str,
        starts_elem: fn(T) -> bool,
        mut elem: impl FnMut(&mut Self) -> Option<E>,
    ) -> (Vec<E>, Span) {
        let saved = std::mem::replace(&mut self.newlines, false);
        let errors_before = self.diagnostics.len();
        let mut need_elem = first.is_none();
        let mut elems: Vec<E> = first.into_iter().collect();
        loop {
            let kind = self.peek();
            if kind == close {
                break;
            }
            if need_elem {
                if !starts_elem(kind) {
                    if ends_list(kind) || matches!(kind, T::RParen | T::RBracket | T::RBrace) {
                        break;
                    }
                    let diagnostic = self.expected(what);
                    self.report(diagnostic);
                    self.skip_until(|k, _| k == T::Comma || k == close || ends_list(k));
                    if !self.eat(T::Comma) {
                        break;
                    }
                    continue;
                }
                let before = self.pos;
                elems.extend(elem(self));
                if self.pos == before {
                    self.bump();
                }
                need_elem = false;
                continue;
            }
            if self.eat(T::Comma) {
                need_elem = true;
                continue;
            }
            let close_text = close.text().unwrap_or_default();
            if starts_elem(kind) {
                let diagnostic = self
                    .expected(&format!("`,` or `{close_text}`"))
                    .with_fix("add `,`", [Edit::insert(self.prev_span().hi, ",")]);
                self.report(diagnostic);
                need_elem = true;
                continue;
            }
            let diagnostic = self.expected(&format!("`,` or `{close_text}`"));
            self.report(diagnostic);
            if ends_list(kind) {
                break;
            }
            self.skip_until(|k, _| k == T::Comma || k == close || ends_list(k));
            if !self.eat(T::Comma) {
                break;
            }
            need_elem = true;
        }
        self.newlines = saved;

        if self.at(close) {
            let close_span = self.bump();
            return (elems, close_span);
        }
        if self.diagnostics.len() == errors_before {
            self.expect_closing(close, open);
        }
        (elems, Span::at(self.prev_span().hi))
    }

    /// Parses the items of a brace list after its `{`, up to and including
    /// the `}`. Items are separated by `list.sep` or by line breaks (grammar
    /// §1.1). Returns the items and the span of the `}`.
    pub(super) fn brace_list<E>(
        &mut self,
        open: Span,
        list: BraceList,
        starts_elem: fn(T) -> bool,
        elem: impl FnMut(&mut Self) -> Option<E>,
    ) -> (Vec<E>, Span) {
        self.in_braces(|p| p.brace_list_items(open, list, starts_elem, elem))
    }

    pub(super) fn brace_list_items<E>(
        &mut self,
        open: Span,
        list: BraceList,
        starts_elem: fn(T) -> bool,
        mut elem: impl FnMut(&mut Self) -> Option<E>,
    ) -> (Vec<E>, Span) {
        let sep_text = list.sep.text().expect("separators are fixed tokens");
        let mut elems = Vec::new();
        loop {
            let kind = self.peek();
            if kind == T::RBrace {
                let close = self.bump();
                self.brace_pairs.push((open, close));
                return (elems, close);
            }
            if kind == T::Eof || is_item_start(kind) && !starts_elem(kind) {
                self.unclosed_brace(open);
                return (elems, Span::at(self.prev_span().hi));
            }
            self.begin_region();
            if self.leading_operator() {
                continue;
            }
            let before = self.pos;
            if starts_elem(kind) {
                elems.extend(elem(self));
            } else {
                let diagnostic = self.expected(list.what);
                self.report(diagnostic);
                self.skip_until(|k, line_break| k == list.sep || line_break);
            }
            if self.pos == before {
                self.bump();
                continue;
            }

            // The separator after an item.
            if self.eat(list.sep) || self.at(T::RBrace) || self.line_break() {
                continue;
            }
            if starts_elem(self.peek()) {
                let one = list.what.strip_prefix("a ").unwrap_or(list.what);
                let diagnostic = Diagnostic::error(
                    list.code,
                    format!("missing `{sep_text}` between {} on one line", list.items),
                    self.span(),
                    format!("starts another {one} on the same line"),
                )
                .with_note(format!(
                    "{} on one line are separated by `{sep_text}`",
                    list.items
                ))
                .with_fix(
                    format!("add `{sep_text}`"),
                    [Edit::insert(self.prev_span().hi, sep_text)],
                );
                self.report(diagnostic);
                continue;
            }
            let diagnostic = self.expected(&format!("`{sep_text}`, a line break or `}}`"));
            self.report(diagnostic);
            self.skip_until(|k, line_break| k == list.sep || line_break);
            self.eat(list.sep);
        }
    }
}
