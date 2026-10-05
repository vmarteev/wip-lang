//! `match` arms and patterns.

use super::*;

/// One element inside a variant pattern's parentheses.
enum BinderElem {
    Field(Binder),
    /// `..`: the fields the pattern does not name are ignored.
    Rest(Span),
}

fn starts_binder(kind: T) -> bool {
    matches!(kind, T::Ident(_) | T::DotDot | T::Underscore) || can_start_pattern(kind)
}

/// What a pattern may start with: a tuple's elements are patterns.
fn starts_pattern(kind: T) -> bool {
    can_start_pattern(kind)
}

impl<'a> Parser<'a> {
    pub(super) fn arm(&mut self) -> Arm {
        let pattern = self.pattern();
        // `if …` between the pattern and `=>`: a guard.
        // The condition holds no struct literal, since a `{` there would
        // read as the arm's body (grammar R1).
        let guard = self.eat(T::If).then(|| self.cond());
        if !self.eat(T::FatArrow) {
            let diagnostic = self.expected("`=>`");
            self.report(diagnostic);
            // `->` and `=` are the likely typos; treat them as `=>`.
            if matches!(self.peek(), T::Arrow | T::Eq) {
                self.bump();
            }
        }
        // `=> return x`, `=> break`, `=> continue`: the arm ends the
        // function or the loop, and is a block of that one statement.
        let body = if matches!(self.peek(), T::Return | T::Break | T::Continue) {
            let start = self.span();
            let stmts: Vec<_> = self.stmt().into_iter().collect();
            let span = start.to(self.prev_span());
            self.alloc_expr(ExprKind::Block(Block { stmts, span }), span)
        } else {
            self.expr()
        };
        Arm {
            span: pattern.span.to(self.expr_span(body)),
            pattern,
            guard,
            body,
        }
    }

    /// A pattern, or several separated by `|`: the value matches where it
    /// matches any one of them.
    pub(super) fn pattern(&mut self) -> Pattern {
        let first = self.one_pattern();
        if !self.at(T::Pipe) {
            return first;
        }
        let start = first.span;
        let mut alternatives = vec![first];
        while self.eat(T::Pipe) {
            alternatives.push(self.one_pattern());
        }
        Pattern {
            kind: PatternKind::Any(alternatives),
            span: start.to(self.prev_span()),
        }
    }

    fn one_pattern(&mut self) -> Pattern {
        let start = self.span();
        // `..0`, `'a'..='z'`, `MIN..`: a range.
        if matches!(self.peek(), T::DotDot | T::DotDotEq) || self.starts_range() {
            let lo = match self.peek() {
                T::DotDot | T::DotDotEq => None,
                _ => self.range_bound(),
            };
            let kind = self.range_rest(lo);
            return Pattern {
                kind,
                span: start.to(self.prev_span()),
            };
        }
        let kind = match self.peek() {
            // A number, a `bool` or text the value must equal.
            T::Int | T::Float | T::Minus => self.number_pattern(),
            T::True => {
                self.bump();
                PatternKind::Bool(true)
            }
            T::False => {
                self.bump();
                PatternKind::Bool(false)
            }
            T::Str(sym) => {
                self.bump();
                PatternKind::Str(sym)
            }
            // A character the value must equal.
            T::Char(value) => {
                self.bump();
                PatternKind::Char(value)
            }
            // A byte the value must equal.
            T::Byte(value) => {
                self.bump();
                PatternKind::Byte(value)
            }
            T::Underscore => {
                self.bump();
                PatternKind::Wildcard
            }
            // `.Variant`, whose enum is the type being matched.
            T::Dot => {
                self.bump();
                match self.name("a variant name") {
                    Some(variant) => self.variant_pattern(true, vec![variant]),
                    None => PatternKind::Error,
                }
            }
            T::Ident(sym) => match self.nth(1) {
                T::ColonColon => {
                    let mut segments = vec![Name {
                        sym,
                        span: self.bump(),
                    }];
                    let mut broken = false;
                    while self.eat(T::ColonColon) {
                        match self.name("a name after `::`") {
                            Some(name) => segments.push(name),
                            None => {
                                broken = true;
                                break;
                            }
                        }
                    }
                    if broken {
                        PatternKind::Error
                    } else {
                        self.variant_pattern(false, segments)
                    }
                }
                // `Point(x, y)`: a struct taken apart, or a variant whose
                // enum was left out — which one is a question for the
                // checker.
                T::LParen => {
                    let name = Name {
                        sym,
                        span: self.bump(),
                    };
                    self.variant_pattern(false, vec![name])
                }
                _ => {
                    self.bump();
                    PatternKind::Binding(sym)
                }
            },
            // `(x, y)`: a tuple taken apart, which is the prelude's
            // `Tuple2(_0: x, _1: y)`.
            T::LParen => {
                let open = self.bump();
                let (elems, _) = self.list(open, T::RParen, "a pattern", starts_pattern, |p| {
                    Some(p.pattern())
                });
                let span = start.to(self.prev_span());
                match self.tuple_name(elems.len(), span, "pattern") {
                    Some(name) => PatternKind::Variant {
                        leading_dot: false,
                        segments: vec![Name { sym: name, span }],
                        binders: Some(
                            elems
                                .into_iter()
                                .enumerate()
                                .map(|(i, pattern)| Binder {
                                    field: Name {
                                        sym: Symbol::tuple_field(i),
                                        span: pattern.span,
                                    },
                                    span: pattern.span,
                                    pattern: Some(pattern),
                                })
                                .collect(),
                        ),
                        rest: false,
                    },
                    None => PatternKind::Error,
                }
            }
            // `[first, ..rest]`: the elements of an array or a slice.
            T::LBracket => self.slice_pattern(),
            _ => {
                let diagnostic = self
                    .expected("a pattern")
                    .with_note("a pattern is `_`, a name, `(x, y)`, `[a, ..]`, or `.Variant(…)`");
                self.report(diagnostic);
                return Pattern {
                    kind: PatternKind::Error,
                    span: Span::at(self.prev_span().hi),
                };
            }
        };
        Pattern {
            kind,
            span: start.to(self.prev_span()),
        }
    }

    /// `[a, b]`, `[first, ..rest]`, `[.., last]`, at the `[`. `..` with a name
    /// after it binds the elements between; with a number, a character or a
    /// path after it, it is a range, as anywhere else. A name that is a
    /// constant is a range too, which the checker decides, as it decides any
    /// name in a pattern.
    fn slice_pattern(&mut self) -> PatternKind {
        let open = self.bump();
        let (elements, _) = self.list(
            open,
            T::RBracket,
            "a pattern or `..`",
            starts_pattern,
            |p| {
                let rest_here = p.at(T::DotDot)
                    && match p.nth(1) {
                        T::Ident(_) => p.nth(2) != T::ColonColon,
                        next => !can_start_bound(next),
                    };
                if !rest_here {
                    return Some(SliceElement::Pattern(p.pattern()));
                }
                let dots = p.bump();
                let name = match p.peek() {
                    T::Ident(sym) => Some(Name {
                        sym,
                        span: p.bump(),
                    }),
                    _ => None,
                };
                Some(SliceElement::Rest {
                    name,
                    span: dots.to(p.prev_span()),
                })
            },
        );
        PatternKind::Slice(elements)
    }

    /// Whether a range starts here: a bound, then `..` or `..=`.
    fn starts_range(&self) -> bool {
        let mut i = 0;
        match self.nth(i) {
            T::Minus => {
                if self.nth(1) != T::Int {
                    return false;
                }
                i = 2;
            }
            T::Int | T::Char(_) | T::Byte(_) => i = 1,
            T::Ident(_) => {
                i = 1;
                while self.nth(i) == T::ColonColon && is_ident(self.nth(i + 1)) {
                    i += 2;
                }
            }
            _ => return false,
        }
        matches!(self.nth(i), T::DotDot | T::DotDotEq)
    }

    /// One end of a range: a number, a character, a byte, or a constant.
    fn range_bound(&mut self) -> Option<RangeBound> {
        let start = self.span();
        let kind = match self.peek() {
            T::Int | T::Minus => match self.number_pattern() {
                PatternKind::Int {
                    negative,
                    magnitude,
                } => RangeBoundKind::Int {
                    negative,
                    magnitude,
                },
                _ => return None,
            },
            T::Char(value) => {
                self.bump();
                RangeBoundKind::Char(value)
            }
            T::Byte(value) => {
                self.bump();
                RangeBoundKind::Byte(value)
            }
            T::Ident(_) => {
                let mut path = vec![self.name("a constant")?];
                while self.eat(T::ColonColon) {
                    path.push(self.name("a name after `::`")?);
                }
                RangeBoundKind::Const(path)
            }
            _ => {
                let diagnostic = self.expected("a number, a character or a constant");
                self.report(diagnostic);
                return None;
            }
        };
        Some(RangeBound {
            kind,
            span: start.to(self.prev_span()),
        })
    }

    /// A range from `lo`, at its `..` or `..=`: the end after it, which
    /// `..=` must have and `..` may leave off, as `..` must where `lo` is
    /// off too.
    fn range_rest(&mut self, lo: Option<RangeBound>) -> PatternKind {
        let inclusive = self.at(T::DotDotEq);
        self.bump();
        let hi = if can_start_bound(self.peek()) {
            self.range_bound()
        } else {
            None
        };
        if hi.is_none() && (inclusive || lo.is_none()) {
            let diagnostic = self
                .expected("the end of the range")
                .with_note("`lo..=hi` takes its end and must have one; `lo..` has none, and runs to the type's greatest value");
            self.report(diagnostic);
            return PatternKind::Error;
        }
        PatternKind::Range { lo, hi, inclusive }
    }

    /// A number in a pattern, with the `-` before it where it has one.
    /// A float is reported: two of them may be equal and
    /// have different bytes, and a pattern is an equality.
    fn number_pattern(&mut self) -> PatternKind {
        let negative = self.at(T::Minus);
        let minus = negative.then(|| self.bump());
        let span = self.span();
        if self.at(T::Float) {
            let text = self.text(span).to_string();
            self.bump();
            let diagnostic = Diagnostic::error(
                codes::FLOAT_PATTERN,
                "a pattern cannot be a float",
                minus.map_or(span, |m| m.to(span)),
                "not a pattern",
            )
            .with_help(format!("test it with `==` or a range: `if x == {text}`"))
            .with_note(
                "two floats may be equal and hold different bytes, and a pattern asks whether a value is one thing",
            );
            self.report(diagnostic);
            return PatternKind::Error;
        }
        match self.int_literal("a number") {
            Some(magnitude) => PatternKind::Int {
                negative,
                magnitude: u128::from(magnitude),
            },
            None => PatternKind::Error,
        }
    }

    /// A variant pattern after its name: its binders, if it has parentheses.
    fn variant_pattern(&mut self, leading_dot: bool, segments: Vec<Name>) -> PatternKind {
        let (binders, rest) = if self.at(T::LParen) {
            let open = self.bump();
            let (elems, _) = self.list(
                open,
                T::RParen,
                "a field name or `..`",
                starts_binder,
                Self::binder_elem,
            );
            let mut binders = Vec::new();
            let mut rest: Option<Span> = None;
            let mut misplaced = false;
            for elem in elems {
                match elem {
                    BinderElem::Field(binder) => {
                        if let Some(dots) = rest
                            && !misplaced
                        {
                            misplaced = true;
                            let diagnostic = Diagnostic::error(
                                codes::BINDER_AFTER_REST,
                                "`..` must come last in a pattern",
                                binder.span,
                                "named after `..`",
                            )
                            .with_secondary(dots, "the remaining fields")
                            .with_help("`..` stands for every field the pattern does not name");
                            self.report(diagnostic);
                        }
                        binders.push(binder);
                    }
                    BinderElem::Rest(span) => rest = rest.or(Some(span)),
                }
            }
            (Some(binders), rest.is_some())
        } else {
            (None, false)
        };
        PatternKind::Variant {
            leading_dot,
            segments,
            binders,
            rest,
        }
    }

    /// A field name, `field: name`, or `..`.
    fn binder_elem(&mut self) -> Option<BinderElem> {
        match self.peek() {
            T::DotDot if !can_start_bound(self.nth(1)) => Some(BinderElem::Rest(self.bump())),
            // `_` alone stands for the one field, as any pattern there does;
            // the checker refuses it where the variant has more.
            T::Underscore if matches!(self.nth(1), T::Comma | T::RParen) => {
                let span = self.bump();
                Some(BinderElem::Field(Binder {
                    field: Name {
                        sym: Symbol::positional(),
                        span,
                    },
                    span,
                    pattern: Some(Pattern {
                        kind: PatternKind::Wildcard,
                        span,
                    }),
                }))
            }
            T::Underscore => {
                let span = self.bump();
                let diagnostic = Diagnostic::error(
                    codes::UNDERSCORE_BINDER,
                    "`_` is not a binder",
                    span,
                    "a field name is expected here",
                )
                .with_help(
                    "binders name their fields: leave a field out, and end the pattern with `..`",
                );
                self.report(diagnostic);
                None
            }
            T::Ident(sym) => {
                let field = Name {
                    sym,
                    span: self.bump(),
                };
                // `field: name` binds it under that name, and a bare name
                // in a pattern is a binding, so one rule covers both.
                let pattern = self.eat(T::Colon).then(|| self.pattern());
                Some(BinderElem::Field(Binder {
                    field,
                    pattern,
                    span: field.span.to(self.prev_span()),
                }))
            }
            // A pattern where a field name could stand: the one field the
            // variant has is the one it tests, as a bare name binds it.
            kind if can_start_pattern(kind) => {
                let start = self.span();
                let pattern = self.pattern();
                Some(BinderElem::Field(Binder {
                    field: Name {
                        sym: Symbol::positional(),
                        span: start,
                    },
                    span: pattern.span,
                    pattern: Some(pattern),
                }))
            }
            _ => {
                let diagnostic = self.expected("a field name or `..`");
                self.report(diagnostic);
                None
            }
        }
    }
}
