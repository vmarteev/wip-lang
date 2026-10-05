//! Types, and the integer literals they contain.

use super::*;

impl<'a> Parser<'a> {
    pub(super) fn ty(&mut self) -> TypeId {
        self.ty_with_args(true)
    }

    /// A type after `as`. `x as i64 < y` compares, so a `<` there does not
    /// start type arguments — except after `ptr`, which is the one generic
    /// name a cast's target may have: `p as ptr<Db>`.
    pub(super) fn cast_ty(&mut self) -> TypeId {
        self.ty_with_args(false)
    }

    fn ty_with_args(&mut self, args_allowed: bool) -> TypeId {
        let start = self.span();
        let kind = match self.peek() {
            T::Ident(sym) => {
                let first = Name {
                    sym,
                    span: self.bump(),
                };
                if self.at(T::ColonColon) {
                    // A type from another module.
                    let mut segments = vec![first];
                    while self.eat(T::ColonColon) {
                        match self.name("a name after `::`") {
                            Some(name) => segments.push(name),
                            None => break,
                        }
                    }
                    let args = self.type_args_in_type(args_allowed);
                    TypeKind::Path { segments, args }
                } else {
                    let pointer = self.text(first.span) == "ptr";
                    let args = self.type_args_in_type(args_allowed || pointer);
                    TypeKind::Named { name: sym, args }
                }
            }
            // `dyn Interface`, which only makes sense behind `&`;
            // the type checker reports it elsewhere.
            T::Dyn => {
                self.bump();
                match self.name("an interface name") {
                    Some(name) => {
                        let args = match self.at(T::Lt) {
                            true => self.type_arg_list().0,
                            false => Vec::new(),
                        };
                        TypeKind::Dyn(name, args)
                    }
                    None => TypeKind::Error,
                }
            }
            T::Own => {
                self.bump();
                if self.at(T::Lt) {
                    let open = self.bump();
                    let inner = self.in_type_args(|p| {
                        let inner = p.ty();
                        p.expect_closing(T::Gt, open);
                        inner
                    });
                    TypeKind::Own(inner)
                } else {
                    let diagnostic = self
                        .expected("`<`")
                        .with_help("an owned type is written `own<T>`");
                    self.report(diagnostic);
                    if can_start_type(self.peek()) && !self.on_later_line() {
                        TypeKind::Own(self.ty())
                    } else {
                        TypeKind::Error
                    }
                }
            }
            T::Amp => {
                self.bump();
                let var = self.eat(T::Var);
                TypeKind::Ref {
                    var,
                    inner: self.ty(),
                }
            }
            T::AmpAmp => {
                let span = self.bump();
                self.report_double_borrow(span);
                let var = self.eat(T::Var);
                let inner = self.ty();
                let inner_span = Span::new(span.lo + 1, self.ast.types[inner].span.hi);
                TypeKind::Ref {
                    var: false,
                    inner: self.alloc_type(TypeKind::Ref { var, inner }, inner_span),
                }
            }
            // `(a: A, b: B) => R`: a function type.
            // `(A, B)`: a tuple, the prelude's `TupleN`. A
            // function type is the other thing a `(` opens here: its `)` is
            // followed by `=>`, or its parameters are named.
            T::LParen
                if !(self.paren_before(T::FatArrow)
                    || (is_ident(self.nth(1)) && self.nth(2) == T::Colon)) =>
            {
                let open = self.bump();
                let (elems, _) =
                    self.list(open, T::RParen, "a type", can_start_type, |p| Some(p.ty()));
                let span = start.to(self.prev_span());
                match self.tuple_name(elems.len(), span, "type") {
                    Some(name) => TypeKind::Named { name, args: elems },
                    None => TypeKind::Error,
                }
            }
            T::LParen => {
                let open = self.bump();
                let starts = |kind: T| can_start_type(kind) || kind == T::Underscore;
                let (params, _) = self.list(open, T::RParen, "a parameter", starts, |p| {
                    if (is_ident(p.peek()) || p.at(T::Underscore)) && p.nth(1) == T::Colon {
                        let name = p.param_name("a parameter name")?;
                        return p.field_after(name);
                    }
                    // A parameter written without its name.
                    let ty = p.ty();
                    let span = p.ast.types[ty].span;
                    let diagnostic = Diagnostic::error(
                        codes::FN_TYPE_SYNTAX,
                        "a function type names its parameters",
                        span,
                        "a parameter without a name",
                    )
                    .with_help("write the name before the type, as in `(count: i64) => bool`")
                    .with_note("the names document what each argument is for");
                    p.report(diagnostic);
                    None
                });
                if !self.eat(T::FatArrow) {
                    let diagnostic = self
                        .expected("`=>` and the result type")
                        .with_help("a function type is written `(a: A, b: B) => R`, with `=> void` for a function that returns nothing");
                    self.report(diagnostic);
                    return self.alloc_type(TypeKind::Error, start.to(self.prev_span()));
                }
                let ret = self.ty();
                TypeKind::Fn { params, ret }
            }
            // The first spelling of a function type, `fn(A, B): R`.
            T::Fn => {
                let keyword = self.bump();
                if self.at(T::LParen) {
                    let open = self.bump();
                    self.list(open, T::RParen, "a parameter type", can_start_type, |p| {
                        if is_ident(p.peek()) && p.nth(1) == T::Colon {
                            p.bump();
                            p.bump();
                        }
                        Some(p.ty())
                    });
                    if self.eat(T::Colon) {
                        self.ty();
                    }
                }
                let span = keyword.to(self.prev_span());
                let diagnostic = Diagnostic::error(
                    codes::FN_TYPE_SYNTAX,
                    "a function type is written `(a: A, b: B) => R`",
                    span,
                    "the `fn(A, B): R` form",
                )
                .with_help("name each parameter, and write the result after `=>`: `(x: i64) => i64`, or `=> void` for none")
                .with_note("the type is written like a lambda, `(x) => body`");
                self.report(diagnostic);
                return self.alloc_type(TypeKind::Error, span);
            }
            T::LBracket => {
                let open = self.bump();
                let elem = self.ty();
                if self.at(T::RBracket) {
                    // `[T]`: a slice.
                    self.bump();
                    TypeKind::Slice(elem)
                } else {
                    let len = if self.eat(T::Semi) {
                        // A length is a literal, or a constant's name.
                        if is_ident(self.peek()) {
                            self.name("an array length").map(ArrayLen::Name)
                        } else {
                            self.int_literal("an array length").map(ArrayLen::Int)
                        }
                    } else {
                        let diagnostic = self.expected("`;` and a length").with_help(
                            "an array type is written `[T; N]`, for example `[i64; 4]`; a slice is `[T]`",
                        );
                        self.report(diagnostic);
                        // Resume at the `]`, so the rest of the type is not
                        // reported again.
                        self.skip_until(|kind, line_break| kind == T::RBracket || line_break);
                        None
                    };
                    self.expect_closing(T::RBracket, open);
                    TypeKind::Array {
                        elem,
                        len: len.unwrap_or(ArrayLen::Int(0)),
                    }
                }
            }
            _ => {
                let diagnostic = self.expected("a type");
                self.report(diagnostic);
                let span = Span::at(self.prev_span().hi);
                return self.alloc_type(TypeKind::Error, span);
            }
        };
        let span = start.to(self.prev_span());
        self.alloc_type(kind, span)
    }

    /// `<A, B>` after a type's name, if there is one.
    fn type_args_in_type(&mut self, allowed: bool) -> Vec<TypeId> {
        if !allowed || !self.at(T::Lt) {
            return Vec::new();
        }
        self.type_arg_list().0
    }

    /// `<A, B>`, at the `<`. Returns the types and the span of the whole list.
    pub(super) fn type_arg_list(&mut self) -> (Vec<TypeId>, Span) {
        let open = self.bump();
        let (args, close) =
            self.in_type_args(|p| p.list(open, T::Gt, "a type", can_start_type, |p| Some(p.ty())));
        (args, open.to(close))
    }

    /// `<T, U: copy>` after the name of a generic item, if there is one.
    /// `: copy`, `: Shape`, `: Shape + copy` after a type parameter,
    /// and nothing where there is no `:`.
    pub(super) fn bounds(&mut self) -> Option<Vec<Bound>> {
        let mut bounds = Vec::new();
        if self.eat(T::Colon) {
            loop {
                // `T: From<i64>`: an interface with the types it takes.
                let name = self.name("a constraint: `copy`, or an interface")?;
                let args = match self.at(T::Lt) {
                    true => self.type_arg_list().0,
                    false => Vec::new(),
                };
                bounds.push(Bound {
                    name,
                    args,
                    span: name.span.to(self.prev_span()),
                });
                if !self.eat(T::Plus) {
                    break;
                }
            }
        }
        Some(bounds)
    }

    pub(super) fn generic_params(&mut self) -> Vec<GenericParam> {
        if !self.at(T::Lt) {
            return Vec::new();
        }
        let open = self.bump();
        let (params, _) = self.in_type_args(|p| {
            p.list(open, T::Gt, "a type parameter", is_ident, |p| {
                let name = p.name("a type parameter")?;
                let bounds = p.bounds()?;
                // `K = T`: what a use that leaves it out means.
                let default = p.eat(T::Eq).then(|| p.ty());
                Some(GenericParam {
                    name,
                    bounds,
                    default,
                    span: name.span.to(p.prev_span()),
                })
            })
        });
        params
    }

    /// Parses an integer literal where only a literal is allowed (array
    /// lengths and repeat counts). Consumes a misplaced name, for recovery.
    pub(super) fn int_literal(&mut self, what: &str) -> Option<u64> {
        if self.at(T::Int) {
            let span = self.bump();
            return Some(u64::try_from(self.int_value(span)).unwrap_or(u64::MAX));
        }
        let diagnostic = self
            .expected(&format!("{what} (an integer literal)"))
            .with_note("Wip has no constants yet, so this must be written as a number");
        self.report(diagnostic);
        if is_ident(self.peek()) {
            self.bump();
        }
        None
    }

    /// The value of an integer literal. The type checker decides which type
    /// it has and whether it fits; here it only has to fit
    /// the widest one.
    pub(super) fn int_value(&mut self, span: Span) -> u128 {
        let text = self.text(span).replace('_', "");
        let (digits, radix) = match text.get(..2) {
            Some("0x") => (&text[2..], 16),
            Some("0o") => (&text[2..], 8),
            Some("0b") => (&text[2..], 2),
            _ => (text.as_str(), 10),
        };
        match u128::from_str_radix(digits, radix) {
            Ok(value) => value,
            // The lexer has reported every other malformed literal: a prefix
            // without digits, or digits its base does not allow.
            Err(err) if *err.kind() != std::num::IntErrorKind::PosOverflow => 0,
            Err(_) => {
                let diagnostic = Diagnostic::error(
                    codes::INTEGER_TOO_LARGE,
                    "integer literal is too large",
                    span,
                    "does not fit in 128 bits",
                );
                self.report(diagnostic);
                0
            }
        }
    }

    pub(super) fn report_double_borrow(&mut self, span: Span) {
        let diagnostic = Diagnostic::error(
            codes::DOUBLE_BORROW,
            "`&&` is the logical-and operator, not a double borrow",
            span,
            "logical and",
        )
        .with_fix(
            "for a reference to a reference, write `& &`",
            [Edit::replace(span, "& &")],
        );
        self.report(diagnostic);
    }
}
