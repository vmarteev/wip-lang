//! Expressions: precedence climbing, prefix and postfix operators, and the
//! primary forms.

use super::*;

/// One argument of a call: a value, with its name if it has one, or
/// `..base`.
enum CallArg {
    Plain(Option<Name>, ExprId),
    Rest(ExprId, Span),
}

/// One element inside the braces of a struct literal written with braces,
/// read only to say how it is written.
enum FieldElem {
    Init(BraceField),
    /// `..value`: where the fields the literal does not name come from.
    Rest(ExprId),
}

/// `name: value`, or `name` alone, which was `name: name`.
struct BraceField {
    name: Name,
    value: ExprId,
    span: Span,
    shorthand: bool,
}

impl<'a> Parser<'a> {
    pub(super) fn expr(&mut self) -> ExprId {
        // `lend` hands a place to the caller. It
        // takes the whole expression after it, like `return`.
        if self.at(T::Lend) {
            let start = self.bump();
            let value = self.expr();
            let span = start.to(self.expr_span(value));
            return self.alloc_expr(ExprKind::Lend(value), span);
        }
        // `yield` is a statement, which hands a value to the list being
        // built. Where a value is expected, it is
        // most likely meant as a projection's `lend`.
        if self.at(T::Yield) {
            let span = self.span();
            let diagnostic = Diagnostic::error(
                codes::YIELD_KEYWORD,
                "`yield` is a statement, not a value",
                span,
                "where a value is expected",
            )
            .with_fix("to lend a place from a projection, write `lend`", [Edit::replace(span, "lend")])
            .with_note(
                "`yield` is a statement that hands a value to the list being built, `own [for x in xs { yield x }]`, or to whoever asks a generator for it; in a `match` arm, it is written in braces, `{ yield x }`. A projection lends a place and stops, with `lend`",
            );
            self.report(diagnostic);
            self.bump();
            let value = self.expr();
            let span = span.to(self.expr_span(value));
            return self.alloc_expr(ExprKind::Lend(value), span);
        }
        self.expr_bp(0)
    }

    /// Precedence climbing over [`infix_binding_power`]. A complete expression
    /// stops at a significant line break.
    pub(super) fn expr_bp(&mut self, min_bp: u8) -> ExprId {
        let mut lhs = self.prefix();
        loop {
            if self.line_break() {
                break;
            }
            let op = self.peek();
            // `value is pattern`, as tight as a comparison,
            // and `value !is pattern`, its negation, `!` and `is` written
            // together as `!=` is.
            let negated = op == T::Bang && self.nth(1) == T::Is;
            if op == T::Is || negated {
                if IS_BINDING_POWER < min_bp {
                    break;
                }
                let first = self.bump();
                if negated {
                    let is = self.bump();
                    if is.lo != first.hi {
                        let between = Span::new(first.hi, is.lo);
                        let diagnostic = Diagnostic::error(
                            codes::EXPECTED,
                            "`!is` is written together",
                            first.to(is),
                            "`!` and `is` apart",
                        )
                        .with_fix("write `!is`", [Edit::replace(between, "")])
                        .with_note("`x !is P` is `!(x is P)`, one operator, as `!=` is");
                        self.report(diagnostic);
                    }
                }
                let pattern = self.pattern();
                let span = self.expr_span(lhs).to(pattern.span);
                lhs = self.alloc_expr(
                    ExprKind::Is {
                        scrutinee: lhs,
                        pattern,
                        negated,
                    },
                    span,
                );
                continue;
            }
            if op == T::As {
                if CAST_BINDING_POWER < min_bp {
                    break;
                }
                self.bump();
                let ty = self.cast_ty();
                let span = self.expr_span(lhs).to(self.ast.types[ty].span);
                lhs = self.alloc_expr(ExprKind::Cast { expr: lhs, ty }, span);
                continue;
            }
            // `place op= value`, as loose as `=`.
            if let Some((assign_op, tokens, wrapping)) = self.compound_assign() {
                if ASSIGN_BINDING_POWER.0 < min_bp {
                    break;
                }
                let mut op_span = self.bump();
                if tokens == 2 {
                    op_span = op_span.to(self.bump());
                }
                let value = self.expr_bp(ASSIGN_BINDING_POWER.1);
                let span = self.expr_span(lhs).to(self.expr_span(value));
                lhs = self.alloc_expr(
                    ExprKind::Assign {
                        target: lhs,
                        op: Some((assign_op, op_span)),
                        wrapping,
                        value,
                    },
                    span,
                );
                continue;
            }
            let shift = self.shift();
            let binding_power = match shift {
                Some(_) => Some(SHIFT_BINDING_POWER),
                None => infix_binding_power(op),
            };
            let Some((left_bp, right_bp)) = binding_power else {
                break;
            };
            if left_bp < min_bp {
                break;
            }
            let mut op_span = self.bump();
            if shift.is_some() {
                op_span = op_span.to(self.bump());
            }
            let rhs = self.expr_bp(right_bp);
            let span = self.expr_span(lhs).to(self.expr_span(rhs));
            let wrapping = matches!(op, T::PlusPercent | T::MinusPercent | T::StarPercent);
            let kind = match (op, shift) {
                (_, Some(shift)) => ExprKind::Binary {
                    op: shift,
                    op_span,
                    lhs,
                    rhs,
                    wrapping: false,
                },
                (T::Eq, None) => ExprKind::Assign {
                    target: lhs,
                    op: None,
                    wrapping: false,
                    value: rhs,
                },
                _ => ExprKind::Binary {
                    op: binary_op(op),
                    op_span,
                    lhs,
                    rhs,
                    wrapping,
                },
            };
            lhs = self.alloc_expr(kind, span);
        }
        lhs
    }

    /// `<<` or `>>` at the current token: two `<` or `>` with nothing between
    /// them. They are not one token, so that `own<own<i64>>` closes two type
    /// argument lists.
    fn shift(&self) -> Option<BinaryOp> {
        let op = match self.peek() {
            T::Lt => BinaryOp::Shl,
            T::Gt => BinaryOp::Shr,
            _ => return None,
        };
        let first = self.tokens.get(self.pos)?;
        let second = self.tokens.get(self.pos + 1)?;
        (second.kind == first.kind && second.span.lo == first.span.hi).then_some(op)
    }

    /// A compound assignment at the current token, how many tokens it
    /// takes, and whether it wraps: `<<=` is `<` and `<=` with nothing
    /// between them, and `>>=` is `>` and `>=`; `+%=` is
    /// `+%` and `=` the same way, and wraps.
    fn compound_assign(&self) -> Option<(BinaryOp, usize, bool)> {
        let op = match self.peek() {
            T::PlusPercent | T::MinusPercent | T::StarPercent => {
                let first = self.tokens.get(self.pos)?;
                let second = self.tokens.get(self.pos + 1)?;
                let op = match first.kind {
                    T::PlusPercent => BinaryOp::Add,
                    T::MinusPercent => BinaryOp::Sub,
                    _ => BinaryOp::Mul,
                };
                let adjacent = second.kind == T::Eq && second.span.lo == first.span.hi;
                return adjacent.then_some((op, 2, true));
            }
            T::PlusEq => BinaryOp::Add,
            T::MinusEq => BinaryOp::Sub,
            T::StarEq => BinaryOp::Mul,
            T::SlashEq => BinaryOp::Div,
            T::PercentEq => BinaryOp::Rem,
            T::AmpEq => BinaryOp::BitAnd,
            T::PipeEq => BinaryOp::BitOr,
            T::CaretEq => BinaryOp::BitXor,
            T::Lt | T::Gt => {
                let first = self.tokens.get(self.pos)?;
                let second = self.tokens.get(self.pos + 1)?;
                let (op, next) = match first.kind {
                    T::Lt => (BinaryOp::Shl, T::LtEq),
                    _ => (BinaryOp::Shr, T::GtEq),
                };
                let adjacent = second.kind == next && second.span.lo == first.span.hi;
                return adjacent.then_some((op, 2, false));
            }
            _ => return None,
        };
        Some((op, 1, false))
    }

    pub(super) fn prefix(&mut self) -> ExprId {
        let op = match self.peek() {
            T::Minus => UnaryOp::Neg,
            T::Bang => UnaryOp::Not,
            T::Amp => UnaryOp::Ref,
            T::Move => UnaryOp::Move,
            T::Own => UnaryOp::Own,
            T::AmpAmp => {
                let span = self.bump();
                self.report_double_borrow(span);
                let operand = self.expr_bp(PREFIX_BINDING_POWER);
                let end = self.expr_span(operand).hi;
                let inner_op = Span::new(span.lo + 1, span.hi);
                let inner = self.alloc_expr(
                    ExprKind::Unary {
                        op: UnaryOp::Ref,
                        op_span: inner_op,
                        operand,
                    },
                    Span::new(inner_op.lo, end),
                );
                let outer_op = Span::new(span.lo, span.lo + 1);
                return self.alloc_expr(
                    ExprKind::Unary {
                        op: UnaryOp::Ref,
                        op_span: outer_op,
                        operand: inner,
                    },
                    Span::new(span.lo, end),
                );
            }
            _ => return self.postfix(),
        };
        let mut op_span = self.bump();
        let op = if op == UnaryOp::Ref && self.at(T::Var) {
            op_span = op_span.to(self.bump());
            UnaryOp::RefVar
        } else {
            op
        };
        // `own for x in xs { … }`: a list of what the loop yields.
        let operand = if op == UnaryOp::Own && self.at(T::For) {
            self.element()
        } else {
            self.expr_bp(PREFIX_BINDING_POWER)
        };
        let span = op_span.to(self.expr_span(operand));
        self.alloc_expr(
            ExprKind::Unary {
                op,
                op_span,
                operand,
            },
            span,
        )
    }

    pub(super) fn postfix(&mut self) -> ExprId {
        let mut e = self.primary();
        loop {
            let kind = match self.peek() {
                // A `.` that starts a line begins a variant instead,
                // so field access stays on its value's line,
                // or inside brackets, where line breaks do not separate.
                // `expr?`: the value, or an early return of the other
                // variant.
                T::Question if !self.line_break() => {
                    self.bump();
                    ExprKind::Try(e)
                }
                T::Dot if !self.line_break() => {
                    self.bump();
                    // `pair.0`: a tuple's element, which lies in the field
                    // the prelude calls `_0`.
                    if matches!(self.peek(), T::Int | T::Float) {
                        match self.tuple_index(e) {
                            Some(kind) => kind,
                            None => break,
                        }
                    } else {
                        let Some(name) = self.name("a field name") else {
                            break;
                        };
                        ExprKind::Field { base: e, name }
                    }
                }
                T::LParen if !self.line_break() => {
                    let open = self.bump();
                    let starts = |kind: T| can_start_expr(kind) || kind == T::DotDot;
                    let (args, _) = self.in_parens(|p| {
                        p.list(open, T::RParen, "an argument", starts, |p| {
                            // `..base`: the fields a struct built by the call
                            // does not name.
                            if p.at(T::DotDot) {
                                let dots = p.bump();
                                let value = p.expr();
                                return Some(CallArg::Rest(value, dots));
                            }
                            // `name: value`.
                            let name = (is_ident(p.peek()) && p.nth(1) == T::Colon)
                                .then(|| p.name("a parameter name"))
                                .flatten();
                            if name.is_some() {
                                p.bump();
                            }
                            Some(CallArg::Plain(name, p.expr()))
                        })
                    });
                    let (args, rest) = self.rest_last(args);
                    self.positional_after_named(&args);
                    let (names, args) = args.into_iter().unzip();
                    ExprKind::Call {
                        callee: e,
                        args,
                        names,
                        rest,
                    }
                }
                T::LBracket if !self.line_break() => {
                    let open = self.bump();
                    // `xs[i]`, or a range of elements `xs[lo..hi]` with
                    // either bound optional.
                    let kind = self.in_parens(|p| {
                        let lo = (!p.at(T::DotDot) && !p.at(T::DotDotEq)).then(|| p.expr());
                        if p.eat(T::DotDot) {
                            let hi = (!p.at(T::RBracket)).then(|| p.expr());
                            ExprKind::SubSlice {
                                base: e,
                                lo,
                                hi,
                                inclusive: false,
                            }
                        } else if p.eat(T::DotDotEq) {
                            // `..=` takes its end too, so it needs one.
                            let hi = p.expr();
                            ExprKind::SubSlice {
                                base: e,
                                lo,
                                hi: Some(hi),
                                inclusive: true,
                            }
                        } else {
                            let index = lo.expect("an index was parsed");
                            ExprKind::Index { base: e, index }
                        }
                    });
                    self.expect_closing(T::RBracket, open);
                    kind
                }
                _ => break,
            };
            let span = self.expr_span(e).to(self.prev_span());
            e = self.alloc_expr(kind, span);
        }
        e
    }

    /// An interpolated literal, `"read \(rows) rows"`, rewritten here into
    /// the block a program would write itself:
    ///
    /// ```text
    /// {
    ///     var text# = String::withCapacity(16)
    ///     text#.push("read ")
    ///     rows.appendTo(&var text#)
    ///     text#.push(" rows")
    ///     text#
    /// }
    /// ```
    ///
    /// Nothing after the parser knows the literal was one: the value is a
    /// `String`, the errors are the errors of those calls, and a literal
    /// with no `\(` in it stays a `str` that allocates nothing.
    fn interpolation(&mut self, first: Symbol) -> ExprId {
        let start = self.span();
        self.bump();
        let mut stmts = Vec::new();
        // How many bytes to ask for: the text as it is written, and a guess
        // for each value. Space at the start and end of a line is left out,
        // so that a text block asks for the same however it is indented.
        let mut capacity = 0;
        let mut piece = (first, start, 3);
        loop {
            capacity += self.push_text(&mut stmts, piece);
            let (value, options) = self.in_parens(|p| {
                let value = p.expr();
                (value, p.piece_options())
            });
            capacity += 8;
            // A number written to a precision or in a radix is written as
            // the value `withPrecision` or `inRadix` makes of it.
            let value = match options.written {
                Some(written) => self.written_as(value, written),
                None => value,
            };
            match options.fitting {
                Some(fitting) => self.push_fitted(&mut stmts, value, fitting),
                None => self.push_append(&mut stmts, value),
            }
            match self.peek() {
                T::StrMid(sym) => {
                    let span = self.span();
                    self.bump();
                    piece = (sym, span, 3);
                }
                T::StrEnd(sym) => {
                    let span = self.span();
                    self.bump();
                    capacity += self.push_text(&mut stmts, (sym, span, 2));
                    break;
                }
                // The literal was not closed, which the lexer has reported.
                _ => break,
            }
        }
        let span = start.to(self.prev_span());
        let init = self.string_init(capacity, span);
        stmts.insert(0, init);
        // The block's value is the `String` it built, which it hands over:
        // a value that owns memory is moved, never copied.
        let value = self.alloc_expr(ExprKind::Name(Symbol::text()), span);
        let value = self.alloc_expr(
            ExprKind::Unary {
                op: UnaryOp::Move,
                op_span: span,
                operand: value,
            },
            span,
        );
        let last = self.alloc_stmt(StmtKind::Expr(value), span);
        stmts.push(last);
        self.alloc_expr(ExprKind::Block(Block { stmts, span }), span)
    }

    /// `var text# = String::withCapacity(capacity)`.
    fn string_init(&mut self, capacity: u32, span: Span) -> StmtId {
        let segments = vec![
            Name {
                sym: Symbol::string_type(),
                span,
            },
            Name {
                sym: Symbol::with_capacity(),
                span,
            },
        ];
        let callee = self.alloc_expr(
            ExprKind::Path {
                leading_dot: false,
                segments,
                type_args: None,
            },
            span,
        );
        let count = self.alloc_expr(ExprKind::Int(u128::from(capacity)), span);
        let init = self.alloc_expr(
            ExprKind::Call {
                callee,
                args: vec![count],
                names: vec![None],
                rest: None,
            },
            span,
        );
        self.alloc_stmt(
            StmtKind::Let {
                mutable: true,
                name: Name {
                    sym: Symbol::text(),
                    span,
                },
                ty: None,
                init,
            },
            span,
        )
    }

    /// `text#.push("…")`, for a piece of the literal's own text. The piece is
    /// `(contents, span, how many bytes of that span are delimiters)`, and
    /// what comes back is how much room the text asks for. An empty piece
    /// pushes nothing.
    fn push_text(&mut self, stmts: &mut Vec<StmtId>, piece: (Symbol, Span, u32)) -> u32 {
        let (sym, span, delimiters) = piece;
        let written = &self.src[(span.lo - self.base) as usize..(span.hi - self.base) as usize];
        let length: usize = written
            .split('\n')
            .map(|line| line.trim_matches([' ', '\t', '\r']).len() + 1)
            .sum();
        let bytes = (length as u32 - 1).saturating_sub(delimiters);
        if bytes == 0 {
            return 0;
        }
        let text = self.alloc_expr(ExprKind::Str(sym), span);
        let call = self.method_call(Symbol::push(), text, span);
        stmts.push(self.alloc_stmt(StmtKind::Expr(call), span));
        bytes
    }

    /// What a piece of an interpolation says after a comma: its width,
    /// `\(name, width: 16)`, `\(n, width: 4, fill: '0')`, `align: .Center`;
    /// and how a number is written, `\(seconds, precision: 3)`,
    /// `\(byte, radix: 16)`. Nothing where it says nothing.
    fn piece_options(&mut self) -> PieceOptions {
        let mut options = PieceOptions {
            fitting: None,
            written: None,
        };
        if !self.at(T::Comma) {
            return options;
        }
        let start = self.span();
        let [width, fill, align] = Symbol::fitting_options();
        let [precision, radix, with_precision, in_radix] = Symbol::written();
        let upper = Symbol::upper();
        let names = [width, fill, align, precision, radix, upper];
        let mut given: [Option<(Span, ExprId)>; 6] = [None; 6];
        while self.eat(T::Comma) {
            let Some(name) =
                self.name("an option: `width`, `fill`, `align`, `precision`, `radix` or `upper`")
            else {
                break;
            };
            self.expect(T::Colon);
            let value = self.expr();
            let Some(which) = names.iter().position(|&n| n == name.sym) else {
                let diagnostic = Diagnostic::error(
                    codes::INTERPOLATION_OPTION,
                    "a piece of an interpolation takes `width`, `fill`, `align`, `precision`, `radix` and `upper`",
                    name.span,
                    "not one of them",
                )
                .with_note("`\\(value, width: 4, fill: '0', align: .Right)` pads the text to a width; `precision: 3` writes a float with three digits after its point, `radix: 16` an integer in hexadecimal, and `upper: true` its digits above 9 in upper case");
                self.report(diagnostic);
                continue;
            };
            if let Some((first, _)) = given[which] {
                let diagnostic = Diagnostic::error(
                    codes::INTERPOLATION_OPTION,
                    "an option is given once",
                    name.span,
                    "given again",
                )
                .with_secondary(first, "first given here");
                self.report(diagnostic);
                continue;
            }
            given[which] = Some((name.span, value));
        }
        let [width, fill, align, precision, radix, upper] = given;
        // Upper-case digits are a radix's.
        if let (Some((span, _)), None) = (upper, radix) {
            let diagnostic = Diagnostic::error(
                codes::INTERPOLATION_OPTION,
                "upper-case digits are a radix's",
                span,
                "no `radix:`",
            )
            .with_note("`\\(byte, radix: 16, upper: true)` writes an integer in hexadecimal with `A` to `F`");
            self.report(diagnostic);
        }
        // A precision is a float's and a radix an integer's, so no value
        // takes both.
        if let (Some((first, _)), Some((second, _))) = (precision, radix) {
            let diagnostic = Diagnostic::error(
                codes::INTERPOLATION_OPTION,
                "a piece is written to a precision or in a radix, not both",
                second,
                "and a radix",
            )
            .with_secondary(first, "a precision")
            .with_note("a precision is the digits after a float's point, and a radix the base an integer is written in");
            self.report(diagnostic);
        }
        options.written = match (precision, radix) {
            (Some((span, value)), _) => Some(Written {
                method: with_precision,
                span,
                value,
                upper: None,
            }),
            (None, Some((span, value))) => Some(Written {
                method: in_radix,
                span,
                value,
                upper,
            }),
            (None, None) => None,
        };
        match width {
            Some((_, width)) => {
                options.fitting = Some(Fitting {
                    width,
                    fill: fill.map(|(_, value)| value),
                    align: align.map(|(_, value)| value),
                });
            }
            None if fill.is_some() || align.is_some() => {
                let diagnostic = Diagnostic::error(
                    codes::INTERPOLATION_OPTION,
                    "a fill or an alignment needs a width",
                    start.to(self.prev_span()),
                    "no `width:`",
                )
                .with_note("the fill makes up the text to the width, and the alignment says where");
                self.report(diagnostic);
            }
            None => {}
        }
        options
    }

    /// `value.withPrecision(digits)` or `value.inRadix(radix)`: what a piece
    /// written to a precision or in a radix writes, named where the option
    /// is, so that a value of the wrong type is reported there.
    fn written_as(&mut self, value: ExprId, written: Written) -> ExprId {
        let span = self.expr_span(value);
        let callee = self.alloc_expr(
            ExprKind::Field {
                base: value,
                name: Name {
                    sym: written.method,
                    span: written.span,
                },
            },
            span,
        );
        let mut args = vec![written.value];
        let mut names = vec![None];
        if let Some((name_span, upper)) = written.upper {
            args.push(upper);
            names.push(Some(Name {
                sym: Symbol::upper(),
                span: name_span,
            }));
        }
        self.alloc_expr(
            ExprKind::Call {
                callee,
                args,
                names,
                rest: None,
            },
            span,
        )
    }

    /// `value.appendFitted(&var text#, width: …, fill: …, align: .Some(…))`:
    /// a piece of an interpolation padded to its width.
    fn push_fitted(&mut self, stmts: &mut Vec<StmtId>, value: ExprId, fitting: Fitting) {
        let span = self.expr_span(value);
        let out = self.alloc_expr(ExprKind::Name(Symbol::text()), span);
        let reference = self.alloc_expr(
            ExprKind::Unary {
                op: UnaryOp::RefVar,
                op_span: span,
                operand: out,
            },
            span,
        );
        let callee = self.alloc_expr(
            ExprKind::Field {
                base: value,
                name: Name {
                    sym: Symbol::append_fitted(),
                    span,
                },
            },
            span,
        );
        let [width, fill, align] = Symbol::fitting_options();
        let named = |sym| Some(Name { sym, span });
        let mut args = vec![reference, fitting.width];
        let mut names = vec![None, named(width)];
        if let Some(value) = fitting.fill {
            args.push(value);
            names.push(named(fill));
        }
        if let Some(value) = fitting.align {
            // The alignment said, where the parameter's `.None` lets the
            // value choose.
            let at = self.expr_span(value);
            let some = self.alloc_expr(
                ExprKind::Path {
                    leading_dot: true,
                    segments: vec![Name {
                        sym: Symbol::some(),
                        span: at,
                    }],
                    type_args: None,
                },
                at,
            );
            let given = self.alloc_expr(
                ExprKind::Call {
                    callee: some,
                    args: vec![value],
                    names: vec![None],
                    rest: None,
                },
                at,
            );
            args.push(given);
            names.push(named(align));
        }
        let call = self.alloc_expr(
            ExprKind::Call {
                callee,
                args,
                names,
                rest: None,
            },
            span,
        );
        stmts.push(self.alloc_stmt(StmtKind::Expr(call), span));
    }

    fn push_append(&mut self, stmts: &mut Vec<StmtId>, value: ExprId) {
        let span = self.expr_span(value);
        let out = self.alloc_expr(ExprKind::Name(Symbol::text()), span);
        let reference = self.alloc_expr(
            ExprKind::Unary {
                op: UnaryOp::RefVar,
                op_span: span,
                operand: out,
            },
            span,
        );
        let callee = self.alloc_expr(
            ExprKind::Field {
                base: value,
                name: Name {
                    sym: Symbol::append_to(),
                    span,
                },
            },
            span,
        );
        let call = self.alloc_expr(
            ExprKind::Call {
                callee,
                args: vec![reference],
                names: vec![None],
                rest: None,
            },
            span,
        );
        stmts.push(self.alloc_stmt(StmtKind::Expr(call), span));
    }

    /// `text#.name(arg)`.
    fn method_call(&mut self, name: Symbol, arg: ExprId, span: Span) -> ExprId {
        let receiver = self.alloc_expr(ExprKind::Name(Symbol::text()), span);
        let callee = self.alloc_expr(
            ExprKind::Field {
                base: receiver,
                name: Name { sym: name, span },
            },
            span,
        );
        self.alloc_expr(
            ExprKind::Call {
                callee,
                args: vec![arg],
                names: vec![None],
                rest: None,
            },
            span,
        )
    }

    pub(super) fn primary(&mut self) -> ExprId {
        let start = self.span();
        let kind = match self.peek() {
            T::Int => {
                let span = self.bump();
                ExprKind::Int(self.int_value(span))
            }
            T::Float => {
                let span = self.bump();
                // The lexer has already validated the form.
                ExprKind::Float(self.text(span).replace('_', "").parse().unwrap_or(0.0))
            }
            T::Str(sym) => {
                self.bump();
                ExprKind::Str(sym)
            }
            // `'x'`.
            T::Char(value) => {
                self.bump();
                ExprKind::Char(value)
            }
            // `b'x'`.
            T::Byte(value) => {
                self.bump();
                ExprKind::Byte(value)
            }
            // `"a\(x)b"`: a `String` built by pushes.
            T::StrStart(sym) => return self.interpolation(sym),
            T::Null => {
                self.bump();
                ExprKind::Null
            }
            T::True => {
                self.bump();
                ExprKind::Bool(true)
            }
            T::False => {
                self.bump();
                ExprKind::Bool(false)
            }
            // A leading `.` names a variant of the expected enum.
            T::Dot => return self.dot_path_expr(),
            T::Ident(sym) => return self.ident_expr(sym),
            // `self` is a method's receiver.
            T::SelfKw => {
                self.bump();
                ExprKind::SelfRef
            }
            T::LBracket => return self.array_lit(),
            // A loop where a value stands: the checker says it is
            // collected with `own for`.
            T::For => return self.element(),
            T::LBrace => ExprKind::Block(self.block()),
            T::If => return self.if_expr(),
            T::Assert(message) => return self.assert_expr(message),
            T::Match => return self.match_expr(),
            T::LParen if self.looks_like_lambda() => return self.lambda(),
            T::LParen => {
                let open = self.bump();
                // `(a, b)` is a tuple, and `(a)` is parentheses around
                // one expression.
                let (elems, comma) = self.in_parens(|p| {
                    if p.at(T::RParen) {
                        return (Vec::new(), false);
                    }
                    let mut elems = vec![p.expr()];
                    let mut comma = false;
                    while p.eat(T::Comma) {
                        comma = true;
                        if p.at(T::RParen) {
                            break;
                        }
                        elems.push(p.expr());
                    }
                    (elems, comma)
                });
                self.expect_closing(T::RParen, open);
                match elems[..] {
                    [inner] if !comma => ExprKind::Paren(inner),
                    _ => {
                        let span = start.to(self.prev_span());
                        match self.tuple_name(elems.len(), span, "expression") {
                            Some(name) => self.tuple_lit(name, elems, span),
                            None => ExprKind::Error,
                        }
                    }
                }
            }
            _ => {
                let diagnostic = self.expected("an expression");
                self.report(diagnostic);
                return self.error_expr();
            }
        };
        let span = start.to(self.prev_span());
        self.alloc_expr(kind, span)
    }

    /// Whether the `<` `n` tokens ahead starts type arguments rather than a
    /// comparison (grammar R7): the tokens up to its matching
    /// `>` can be types, and the token after that is `(`, `::`, a `{` that
    /// starts a struct literal, or something no operand can start, as after
    /// a function named as a value: `)`, `,`, `]`, `}`, `;` or the end of
    /// the line. No correct program reads differently, since `bool` values
    /// cannot be ordered, neither functions nor types can be compared, and a
    /// comparison needs a right operand.
    pub(super) fn type_args_at(&self, n: usize) -> bool {
        if self.nth(n) != T::Lt {
            return false;
        }
        // `<<` is a shift, whatever follows.
        let open = self.tokens.get(self.pos + n);
        let next = self.tokens.get(self.pos + n + 1);
        if let (Some(open), Some(next)) = (open, next)
            && next.kind == T::Lt
            && next.span.lo == open.span.hi
        {
            return false;
        }
        // Each token must be able to follow the one before it in a type, so
        // that `a < xs[i], c > (d)` or `a < (b), c > (d)` stay comparisons.
        // A `(` starts a function type, whose `)` must be followed by `=>`.
        let starts_type = |prev: T| {
            matches!(
                prev,
                T::Lt
                    | T::Comma
                    | T::Amp
                    | T::Var
                    | T::LParen
                    | T::Colon
                    | T::LBracket
                    | T::FatArrow
            )
        };
        let ends_type = |prev: T| matches!(prev, T::Ident(_) | T::Gt | T::RBracket);
        let (mut angles, mut brackets) = (0usize, 0usize);
        // The parentheses open here, and whether each holds a comma, which
        // is what tells a tuple type from a function type's parameters.
        // `tuple` is set while the `)` just read closed
        // one, since that `)` ends a type where a function type's does not.
        let mut parens: Vec<bool> = Vec::new();
        let mut tuple = false;
        let mut prev = T::Eof;
        let mut i = n;
        loop {
            let kind = self.nth(i);
            let ends = |prev: T| ends_type(prev) || tuple;
            let fits = match kind {
                T::Lt if i == n => true,
                T::Lt => matches!(prev, T::Ident(_) | T::Own),
                T::Gt => ends(prev),
                // A type, or a parameter's name.
                T::Ident(_) => starts_type(prev) || prev == T::ColonColon,
                T::ColonColon => matches!(prev, T::Ident(_)),
                T::Comma => ends(prev),
                T::Amp | T::Own | T::LBracket | T::LParen => starts_type(prev),
                T::Var => prev == T::Amp,
                T::RParen => !parens.is_empty() && (prev == T::LParen || ends(prev)),
                T::Colon => !parens.is_empty() && matches!(prev, T::Ident(_)),
                T::FatArrow => prev == T::RParen,
                T::Semi => brackets > 0 && ends_type(prev),
                T::Int => prev == T::Semi,
                T::RBracket => brackets > 0 && (prev == T::Int || ends_type(prev)),
                _ => false,
            };
            // A function type's `)` is followed by `=>`; a tuple's ends the
            // type, so `Vec<(A, B)>` and `Map<(A, B), C>` read as types
            // while `a < (b), c > (d)` stays a comparison.
            let fits = fits && (prev != T::RParen || kind == T::FatArrow || tuple);
            if !fits {
                return false;
            }
            tuple = false;
            match kind {
                T::Lt => angles += 1,
                T::Gt => {
                    angles -= 1;
                    if angles == 0 {
                        break;
                    }
                }
                T::LBracket => brackets += 1,
                T::RBracket => brackets -= 1,
                T::LParen => parens.push(false),
                T::Comma => {
                    if let Some(inner) = parens.last_mut() {
                        *inner = true;
                    }
                }
                T::RParen => tuple = parens.pop().unwrap_or(false),
                _ => {}
            }
            prev = kind;
            i += 1;
        }
        match self.nth(i + 1) {
            T::LParen | T::ColonColon => true,
            T::LBrace => !self.no_struct && !self.line_break_at(i + 1),
            T::RParen | T::Comma | T::RBracket | T::RBrace | T::Semi | T::Eof => true,
            _ => self.line_break_at(i + 1),
        }
    }

    /// A call's arguments, and its `..base`, which comes last and once.
    fn rest_last(&mut self, args: Vec<CallArg>) -> (Vec<(Option<Name>, ExprId)>, Option<ExprId>) {
        let mut plain = Vec::new();
        let mut rest: Option<(ExprId, Span)> = None;
        for arg in args {
            match arg {
                CallArg::Plain(name, value) => {
                    if let Some((_, dots)) = rest {
                        let diagnostic = Diagnostic::error(
                            codes::REST_NOT_LAST,
                            "`..` must come last in a call",
                            self.expr_span(value),
                            "after `..`",
                        )
                        .with_secondary(dots, "the remaining fields")
                        .with_help("`..base` stands for every field the call does not name");
                        self.report(diagnostic);
                    }
                    plain.push((name, value));
                }
                CallArg::Rest(value, dots) => match rest {
                    Some((_, first)) => {
                        let diagnostic = Diagnostic::error(
                            codes::REST_NOT_LAST,
                            "a call takes one `..`",
                            dots,
                            "a second `..`",
                        )
                        .with_secondary(first, "the first one");
                        self.report(diagnostic);
                    }
                    None => rest = Some((value, dots)),
                },
            }
        }
        (plain, rest.map(|(value, _)| value))
    }

    /// Reports a positional argument after a named one (E0122).
    fn positional_after_named(&mut self, args: &[(Option<Name>, ExprId)]) {
        let Some(first_named) = args.iter().position(|(name, _)| name.is_some()) else {
            return;
        };
        if let Some((_, value)) = args[first_named..].iter().find(|(name, _)| name.is_none()) {
            let named = args[first_named].0.expect("found above").span;
            let diagnostic = Diagnostic::error(
                codes::POSITIONAL_AFTER_NAMED,
                "a positional argument follows a named one",
                self.expr_span(*value),
                "positional",
            )
            .with_secondary(named, "named")
            .with_help("name this argument too, or move it before the named ones")
            .with_note("positional arguments come first, then named ones in any order");
            self.report(diagnostic);
        }
    }

    /// A `(` that opens a lambda's parameters rather than a parenthesized
    /// expression: what follows its `)` is `=>`, or `:` and a type and then
    /// `=>`. In a guard, the `=>` after a `)` is the arm's.
    pub(super) fn looks_like_lambda(&self) -> bool {
        // `(x): i64 => …`, a result type between the two.
        !self.guard && (self.paren_before(T::FatArrow) || self.paren_before(T::Colon))
    }

    /// Whether the token after the `)` that closes the `(` the parser is at
    /// is `token`. It is what tells a lambda from a parenthesized
    /// expression, and a function type from a tuple.
    pub(super) fn paren_before(&self, token: T) -> bool {
        let mut depth = 0usize;
        let mut n = 0usize;
        loop {
            match self.nth(n) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                T::Eof => return false,
                _ => {}
            }
            n += 1;
        }
        self.nth(n + 1) == token
    }

    /// `pair.0`, and `pair.0.1` — which the lexer reads as one float,
    /// since `0.1` is a number wherever else it appears.
    fn tuple_index(&mut self, base: ExprId) -> Option<ExprKind> {
        let span = self.span();
        let text = self.text(span);
        let float = self.at(T::Float);
        self.bump();
        let mut base = base;
        let mut kind = None;
        for (n, digits) in text.split('.').enumerate() {
            // `pair.0.` is a field access with its name left out, and
            // `pair.1e3` is not an index at all.
            let index = digits.parse::<usize>().ok().filter(|_| n < 2);
            let Some(index) = index.filter(|&i| i < MAX_TUPLE) else {
                let mut diagnostic = Diagnostic::error(
                    codes::TUPLE_ARITY,
                    format!("`{text}` is not a tuple's element"),
                    span,
                    "not an element",
                )
                .with_note("a tuple's elements are reached by position, `pair.0` to `pair.3`");
                if float && n == 0 {
                    diagnostic = diagnostic
                        .with_help("write the two positions in parentheses: `(pair.0).1`");
                }
                self.report(diagnostic);
                return None;
            };
            if let Some(kind) = kind.take() {
                let span = self.expr_span(base).to(span);
                base = self.alloc_expr(kind, span);
            }
            kind = Some(ExprKind::Field {
                base,
                name: Name {
                    sym: Symbol::tuple_field(index),
                    span,
                },
            });
        }
        kind
    }

    /// `(a, b)`, which is the prelude's `Tuple2(a, b)`. The sugar is written
    /// out here, so that a tuple is checked, moved, dropped and generated as
    /// the struct it is.
    fn tuple_lit(&mut self, name: Symbol, elems: Vec<ExprId>, span: Span) -> ExprKind {
        let callee = self.alloc_expr(ExprKind::Name(name), span);
        let names = vec![None; elems.len()];
        ExprKind::Call {
            callee,
            args: elems,
            names,
            rest: None,
        }
    }

    /// The prelude's `TupleN` for a tuple of `n` elements, or a report that
    /// there are too few or too many.
    pub(super) fn tuple_name(&mut self, n: usize, span: Span, what: &str) -> Option<Symbol> {
        if (MIN_TUPLE..=MAX_TUPLE).contains(&n) {
            return Some(Symbol::tuple(n));
        }
        let (label, help) = if n < MIN_TUPLE {
            let help = match what {
                // A pattern is not written in parentheses: a name of its
                // own binds the whole value.
                "pattern" => "a name on its own binds the whole value".to_string(),
                what => {
                    format!("`(x)` is one {what} in parentheses, and nothing at all is `void`")
                }
            };
            ("a tuple of one element or none", help)
        } else {
            (
                "too many elements",
                format!(
                    "more than {MAX_TUPLE} of them is a struct, whose fields have names; this one has {n}"
                ),
            )
        };
        let diagnostic = Diagnostic::error(
            codes::TUPLE_ARITY,
            format!("a tuple has {MIN_TUPLE} to {MAX_TUPLE} elements"),
            span,
            label,
        )
        .with_help(help)
        .with_note("the prelude declares the tuples, and `(a, b)` is their sugar");
        self.report(diagnostic);
        None
    }

    /// `(a, b: B): R => body`. A parameter's type, and the
    /// result's, may be left out where the expected type gives them.
    pub(super) fn lambda(&mut self) -> ExprId {
        let open = self.bump();
        let starts = |kind: T| is_ident(kind) || kind == T::Underscore;
        let (params, _) = self.list(open, T::RParen, "a parameter", starts, |p| {
            let name = p.param_name("a parameter name")?;
            let ty = p.eat(T::Colon).then(|| p.ty());
            Some(LambdaParam {
                name,
                ty,
                span: name.span.to(p.prev_span()),
            })
        });
        let ret = self.eat(T::Colon).then(|| self.ty());
        let body = if self.expect(T::FatArrow) {
            self.expr()
        } else {
            self.error_expr()
        };
        let span = open.to(self.expr_span(body));
        self.alloc_expr(ExprKind::Lambda { params, ret, body }, span)
    }

    /// An expression starting with an identifier: a name, a struct literal or
    /// a variant literal, decided by the next token.
    pub(super) fn ident_expr(&mut self, sym: Symbol) -> ExprId {
        let name = Name {
            sym,
            span: self.span(),
        };
        // A `{` on the next line starts a block, not a struct literal.
        let brace = self.nth(1) == T::LBrace && !self.line_break_at(1);
        match self.nth(1) {
            T::ColonColon => self.path_expr(name),
            // `name<A, B>`, followed by a call or `::`.
            T::Lt if self.type_args_at(1) => self.path_expr(name),
            // `Row { … }`, a struct written with braces.
            // In a condition, `IDENT {` is the name and the block, but a
            // block does not start with `IDENT :`, so that one was meant as
            // a struct too.
            _ if brace
                && (!self.no_struct || (is_ident(self.nth(2)) && self.nth(3) == T::Colon)) =>
            {
                self.bump();
                self.brace_literal(vec![name], None)
            }
            _ => {
                self.bump();
                self.alloc_expr(ExprKind::Name(sym), name.span)
            }
        }
    }

    /// `Row { name: x, size }`: a struct written with braces, which is
    /// refused (E0133), with a fix that writes the call.
    /// The call it stands for is what the parser goes on with, so what is
    /// inside it is checked as well.
    pub(super) fn brace_literal(&mut self, path: Vec<Name>, type_args: Option<TypeArgs>) -> ExprId {
        let open = self.bump();
        let list = BraceList {
            sep: T::Comma,
            what: "a field",
            items: "fields",
            code: codes::SEPARATOR,
        };
        let starts_field = |kind: T| is_ident(kind) || kind == T::DotDot;
        let (elems, close) = self.brace_list(open, list, starts_field, Self::field_elem);
        let mut fields = Vec::new();
        let mut rest = None;
        for elem in elems {
            match elem {
                FieldElem::Init(field) => fields.push(field),
                FieldElem::Rest(value) => rest = rest.or(Some(value)),
            }
        }
        let start = path.first().expect("a path has segments").span;
        let span = start.to(close);
        // The call, written from what was written.
        let head_end = type_args
            .as_ref()
            .map_or(path.last().expect("a path has segments").span, |args| {
                args.span
            });
        let mut parts: Vec<String> = fields
            .iter()
            .map(|field| match field.shorthand {
                true => format!("{0}: {0}", self.text(field.name.span)),
                false => self.text(field.span).to_string(),
            })
            .collect();
        if let Some(rest) = rest {
            parts.push(format!("..{}", self.text(self.expr_span(rest))));
        }
        let call = format!("{}({})", self.text(start.to(head_end)), parts.join(", "));
        let diagnostic = Diagnostic::error(
            codes::BRACE_LITERAL,
            "a struct is built with a call",
            span,
            "written with braces",
        )
        .with_note("a struct is built as a variant is: its name, and its fields as a call's arguments, by position and then by name")
        .with_fix(format!("write `{call}`"), [Edit::replace(span, call.clone())]);
        self.report(diagnostic);
        let callee = match (&path[..], type_args) {
            ([only], None) => self.alloc_expr(ExprKind::Name(only.sym), only.span),
            (_, type_args) => self.alloc_expr(
                ExprKind::Path {
                    leading_dot: false,
                    segments: path,
                    type_args,
                },
                start.to(head_end),
            ),
        };
        let names = fields.iter().map(|field| Some(field.name)).collect();
        let args = fields.iter().map(|field| field.value).collect();
        self.alloc_expr(
            ExprKind::Call {
                callee,
                args,
                names,
                rest,
            },
            span,
        )
    }

    /// One element inside a struct literal's braces: a field, or `..value`.
    fn field_elem(&mut self) -> Option<FieldElem> {
        if self.at(T::DotDot) {
            self.bump();
            return Some(FieldElem::Rest(self.expr()));
        }
        Some(FieldElem::Init(self.field_init()?))
    }

    fn field_init(&mut self) -> Option<BraceField> {
        let name = self.name("a field name")?;
        // `Point { x, y }`: a field with nothing after it takes the value of
        // the variable with its name.
        let shorthand = matches!(self.peek(), T::Comma | T::RBrace) || self.line_break();
        let value = if self.eat(T::Colon) {
            self.expr()
        } else {
            if !shorthand {
                let diagnostic = self.expected("`:` and a value");
                self.report(diagnostic);
            }
            self.alloc_expr(ExprKind::Name(name.sym), name.span)
        };
        Some(BraceField {
            name,
            value,
            span: name.span.to(self.expr_span(value)),
            shorthand,
        })
    }

    /// `a::b::c`, a qualified name whose meaning is decided when it is
    /// resolved. Any arguments belong to the call
    /// around it.
    pub(super) fn path_expr(&mut self, first: Name) -> ExprId {
        let start = self.bump();
        let mut segments = vec![first];
        let mut type_args: Option<TypeArgs> = None;
        loop {
            // `<A, B>`: type arguments for the segment before it.
            if self.type_args_at(0) {
                self.path_type_args(&mut type_args, segments.len() - 1);
            }
            if !self.eat(T::ColonColon) {
                break;
            }
            // `::<A, B>`, as Rust writes it.
            if self.at(T::Lt) {
                let colons = self.prev_span();
                let diagnostic = Diagnostic::error(
                    codes::TYPE_ARGS_COLONS,
                    "type arguments are written without `::`",
                    colons,
                    "not needed",
                )
                .with_note("after a name, `<` starts type arguments when `(`, `{` or `::` follows the matching `>`")
                .with_fix("remove the `::`", [Edit::replace(colons, "")]);
                self.report(diagnostic);
                self.path_type_args(&mut type_args, segments.len() - 1);
                if !self.eat(T::ColonColon) {
                    break;
                }
            }
            match self.name("a name after `::`") {
                Some(name) => segments.push(name),
                None => return self.alloc_expr(ExprKind::Error, start.to(self.prev_span())),
            }
        }
        // `pkg::Point { … }`, as an unqualified one.
        if self.at(T::LBrace) && !self.no_struct && !self.line_break() {
            return self.brace_literal(segments, type_args);
        }
        let span = start.to(self.prev_span());
        self.alloc_expr(
            ExprKind::Path {
                leading_dot: false,
                segments,
                type_args,
            },
            span,
        )
    }

    /// The type arguments at `<` in a path, for the segment `after`. A path
    /// has one list of them.
    fn path_type_args(&mut self, type_args: &mut Option<TypeArgs>, after: usize) {
        let (args, span) = self.type_arg_list();
        if let Some(earlier) = type_args {
            let diagnostic = Diagnostic::error(
                codes::EXPECTED,
                "a path has one list of type arguments",
                span,
                "a second list",
            )
            .with_secondary(earlier.span, "the first list");
            self.report(diagnostic);
        } else {
            *type_args = Some(TypeArgs { after, args, span });
        }
    }

    /// `.Variant`, whose enum comes from the expected type.
    pub(super) fn dot_path_expr(&mut self) -> ExprId {
        let start = self.bump();
        let Some(name) = self.name("a variant name") else {
            return self.alloc_expr(ExprKind::Error, start.to(self.prev_span()));
        };
        let span = start.to(self.prev_span());
        self.alloc_expr(
            ExprKind::Path {
                leading_dot: true,
                segments: vec![name],
                type_args: None,
            },
            span,
        )
    }

    pub(super) fn array_lit(&mut self) -> ExprId {
        let open = self.bump();
        let kind = self.in_parens(|p| {
            if p.eat(T::RBracket) {
                return ExprKind::Array(Vec::new());
            }
            let first = p.element();
            if p.eat(T::Semi) {
                // A literal count is part of the array's type; any other is
                // a length known only at run time.
                let count = if p.at(T::Int) && p.nth(1) == T::RBracket {
                    RepeatCount::Literal(p.int_literal("a repeat count").unwrap_or(0))
                } else {
                    RepeatCount::Expr(p.expr())
                };
                p.expect_closing(T::RBracket, open);
                return ExprKind::ArrayRepeat { elem: first, count };
            }
            let (elems, _) = p.list_after(
                open,
                Some(first),
                T::RBracket,
                "an element",
                |t| t == T::For || can_start_expr(t),
                |p| Some(p.element()),
            );
            ExprKind::Array(elems)
        });
        let span = open.to(self.prev_span());
        self.alloc_expr(kind, span)
    }

    /// An element of a list literal: a value, or `for x in xs { … }`,
    /// whose `yield`s are its elements.
    fn element(&mut self) -> ExprId {
        if !self.at(T::For) {
            return self.expr();
        }
        let start = self.bump();
        let Some((binding, source)) = self.for_header() else {
            return self.alloc_expr(ExprKind::Error, start);
        };
        let body = self.block();
        let span = start.to(self.prev_span());
        self.alloc_expr(
            ExprKind::ForElement {
                binding,
                source,
                body,
            },
            span,
        )
    }

    /// `assert(condition)`, or `assert(condition, "note")`.
    /// The message a failure prints comes with the keyword's token, since
    /// the lexer is what holds the interner.
    pub(super) fn assert_expr(&mut self, message: Option<Symbol>) -> ExprId {
        let start = self.bump();
        if !self.at(T::LParen) {
            let diagnostic = self
                .expected("`(`")
                .with_note("an assert is written as a call: `assert(condition)`");
            self.report(diagnostic);
            return self.error_expr();
        }
        let open = self.bump();
        let (args, _) = self.in_parens(|p| {
            p.list(open, T::RParen, "the condition", can_start_expr, |p| {
                Some(p.expr())
            })
        });
        let span = start.to(self.prev_span());
        let (cond, note) = match args[..] {
            [cond] => (cond, None),
            [cond, note] => (cond, Some(note)),
            _ => {
                let diagnostic = Diagnostic::error(
                    codes::ASSERT_ARGUMENTS,
                    format!(
                        "`assert` takes a condition and a note, but {} were given",
                        args.len()
                    ),
                    span,
                    "not a condition and a note",
                )
                .with_note(
                    "an assert is written `assert(condition)` or `assert(condition, \"note\")`",
                );
                self.report(diagnostic);
                return self.error_expr();
            }
        };
        self.alloc_expr(
            ExprKind::Assert {
                cond,
                note,
                message,
            },
            span,
        )
    }

    pub(super) fn if_expr(&mut self) -> ExprId {
        self.if_in_form(None)
    }

    /// `if c { … } else { … }`, or `if c then a else b`, whose branches are
    /// one expression each. An `else if` chain takes one
    /// form throughout: `chain` is the form of the `if` it continues.
    fn if_in_form(&mut self, chain: Option<bool>) -> ExprId {
        let start = self.bump();
        let cond = self.cond();
        let then_at = self.at(T::Then).then(|| self.span());
        let then_form = then_at.is_some();
        if let Some(outer) = chain
            && outer != then_form
        {
            self.mixed_chain(start, outer);
        }
        let then_block = match then_at {
            Some(then_span) => {
                self.bump();
                self.one_expression_branch(then_span)
            }
            None => self.block(),
        };
        // `else` continues across a line break: it cannot begin a statement.
        let else_branch = if self.at(T::Else) {
            let else_span = self.bump();
            if self.at(T::If) {
                Some(self.if_in_form(Some(then_form)))
            } else if then_form && self.at(T::LBrace) {
                let diagnostic = Diagnostic::error(
                    codes::EXPECTED,
                    "a `then` form's `else` is one expression, not a block",
                    self.span(),
                    "a block",
                )
                .with_help("where a branch is more than one expression, write the whole `if` with braces: `if c { … } else { … }`")
                .with_note("one `if` takes one form: `then` and expressions, or braces and blocks");
                self.report(diagnostic);
                let block = self.block();
                let span = block.span;
                Some(self.alloc_expr(ExprKind::Block(block), span))
            } else if then_form {
                let block = self.one_expression_branch(else_span);
                let span = block.span;
                Some(self.alloc_expr(ExprKind::Block(block), span))
            } else {
                if !self.at(T::LBrace) && can_start_expr(self.peek()) {
                    let diagnostic = Diagnostic::error(
                        codes::EXPECTED,
                        "a braced `if`'s `else` is a block",
                        self.span(),
                        "an expression without braces",
                    )
                    .with_help(
                        "put the branch in braces, or write the whole `if` as `if c then a else b`",
                    )
                    .with_note(
                        "one `if` takes one form: `then` and expressions, or braces and blocks",
                    );
                    self.report(diagnostic);
                    let block = self.one_expression_branch(else_span);
                    let span = block.span;
                    Some(self.alloc_expr(ExprKind::Block(block), span))
                } else {
                    let block = self.block();
                    let span = block.span;
                    Some(self.alloc_expr(ExprKind::Block(block), span))
                }
            }
        } else {
            None
        };
        let span = start.to(self.prev_span());
        self.alloc_expr(
            ExprKind::If {
                cond,
                then_block,
                else_branch,
            },
            span,
        )
    }

    /// A branch of a `then` form: one expression, or a jump — `return`,
    /// `break`, `continue` — held as a block of that one statement, which
    /// is what the braced form's branch would be. `after`
    /// is the `then` or `else` before it, which a block after it is told
    /// to drop.
    fn one_expression_branch(&mut self, after: Span) -> Block {
        let start = self.span();
        if self.at(T::LBrace) {
            let diagnostic = Diagnostic::error(
                codes::EXPECTED,
                "`then` is followed by one expression, not a block",
                start,
                "a block",
            )
            .with_fix(
                "drop `then`, for the braced form",
                [Edit::replace(after, "")],
            )
            .with_note("one `if` takes one form: `then` and expressions, or braces and blocks");
            self.report(diagnostic);
            return self.block();
        }
        let stmt = match self.peek() {
            T::Return | T::Break | T::Continue => self.stmt(),
            kind if can_start_expr(kind) => {
                let value = self.expr();
                let span = self.expr_span(value);
                Some(self.alloc_stmt(StmtKind::Expr(value), span))
            }
            _ => {
                let diagnostic = self.expected("an expression after `then`");
                self.report(diagnostic);
                None
            }
        };
        Block {
            stmts: stmt.into_iter().collect(),
            span: start.to(self.prev_span()),
        }
    }

    /// An `else if` whose form is not the chain's: a chain is one `if`
    /// with several branches, and takes one form.
    fn mixed_chain(&mut self, at: Span, outer_then: bool) {
        let (this, that) = match outer_then {
            true => ("braces", "`then`"),
            false => ("`then`", "braces"),
        };
        let diagnostic = Diagnostic::error(
            codes::EXPECTED,
            format!("this `else if` is written with {this}, and the `if` it continues with {that}"),
            at,
            "the other form",
        )
        .with_help("write every branch of the chain one way")
        .with_note(
            "an `else if` chain is one `if` with several branches, and takes one form throughout",
        );
        self.report(diagnostic);
    }

    pub(super) fn match_expr(&mut self) -> ExprId {
        let start = self.bump();
        let scrutinee = self.cond();
        let mut arms = Vec::new();
        if self.at(T::LBrace) {
            let open = self.bump();
            let list = BraceList {
                sep: T::Comma,
                what: "a match arm",
                items: "match arms",
                code: codes::MATCH_ARM_SEPARATOR,
            };
            arms = self
                .brace_list(open, list, can_start_pattern, |p| Some(p.arm()))
                .0;
        } else {
            let diagnostic = self.expected("`{`");
            self.report(diagnostic);
        }
        let span = start.to(self.prev_span());
        self.alloc_expr(ExprKind::Match { scrutinee, arms }, span)
    }
}

/// What a piece of an interpolation says of its width.
struct Fitting {
    width: ExprId,
    fill: Option<ExprId>,
    align: Option<ExprId>,
}

/// How a piece of an interpolation is written: the method that makes the
/// value it writes, `withPrecision` or `inRadix`, where its option was
/// written, and what the option says.
struct Written {
    method: Symbol,
    span: Span,
    value: ExprId,
    /// `upper: …`, with a radix, and where it was written.
    upper: Option<(Span, ExprId)>,
}

/// What a piece of an interpolation says after its value.
struct PieceOptions {
    fitting: Option<Fitting>,
    written: Option<Written>,
}
