//! Expressions whose type is synthesized: literals, names, operators and
//! casts. Calls are in `calls`, and `lend` in `projections`.

use super::*;

/// The implementation an arithmetic operator calls on a type of a
/// program's own, and what it takes and answers.
pub(super) struct OperatorImpl {
    pub method: FnId,
    pub type_args: crate::TyList,
    pub rhs: Ty,
    pub out: Ty,
}

impl<'a> Lowerer<'a> {
    pub(super) fn infer(&mut self, id: ast::ExprId, hint: Option<Ty>) -> ExprId {
        let ast = self.ast;
        let expr = &ast.exprs[id];
        let span = expr.span;
        match &expr.kind {
            ast::ExprKind::Int(v) => {
                let ty = self.int_literal_type(hint);
                let value = self.int_literal(false, *v, ty, span);
                self.alloc(ExprKind::Int(value), ty, span)
            }
            ast::ExprKind::Block(block) => {
                let (block, ty) = self.check_block(block, hint);
                self.alloc(ExprKind::Block(block), ty, span)
            }
            ast::ExprKind::If {
                cond,
                then_block,
                else_branch,
            } => self.if_expr(*cond, then_block, *else_branch, hint, span),
            ast::ExprKind::Try(value) => self.try_expr(*value, span),
            ast::ExprKind::Assert {
                cond,
                note,
                message,
            } => self.assert(*cond, *note, *message, span),
            ast::ExprKind::Lambda { .. } => self.lambda(id, hint, false, span),
            ast::ExprKind::Cast { expr, ty } => self.cast(*expr, *ty, span),
            // Literals take the type expected of them.
            ast::ExprKind::Float(v) => {
                let ty = if hint == Some(Types::F32) {
                    Types::F32
                } else {
                    Types::F64
                };
                self.alloc(ExprKind::Float(*v), ty, span)
            }
            ast::ExprKind::Str(s) => {
                let ty = if hint == Some(Types::CSTRING) {
                    Types::CSTRING
                } else {
                    Types::STR
                };
                self.alloc(ExprKind::Str(*s), ty, span)
            }
            ast::ExprKind::Bool(b) => self.alloc(ExprKind::Bool(*b), Types::BOOL, span),
            // A character is its number, in 32 bits.
            ast::ExprKind::Char(c) => self.alloc(ExprKind::Int(*c as u128), Types::CHAR, span),
            // A byte is the number it is, a `u8`.
            ast::ExprKind::Byte(b) => self.alloc(ExprKind::Int(u128::from(*b)), Types::U8, span),
            // `null`: a C pointer that points at nothing. Which pointer it is
            // comes from what is expected.
            ast::ExprKind::Null => {
                let ty = match hint.map(|h| (h, self.kind(h))) {
                    Some((h, TyKind::Ptr(_))) => h,
                    _ => {
                        let diagnostic = Diagnostic::error(
                            codes::NULL_WITHOUT_TYPE,
                            "the type of this `null` is not known here",
                            span,
                            "no pointer type",
                        )
                        .with_note(
                            "`null` is a C pointer that points at nothing, so what it points at comes from what is expected",
                        )
                        .with_help("write the type, as in `val handle: ptr<sqlite3> = null`");
                        self.report(diagnostic);
                        return self.error_expr(span);
                    }
                };
                self.alloc(ExprKind::Null, ty, span)
            }
            ast::ExprKind::Name(sym) => self.name(*sym, hint, span),
            ast::ExprKind::SelfRef => self.self_expr(span),
            ast::ExprKind::Paren(inner) => self.infer(*inner, hint),
            ast::ExprKind::Path {
                leading_dot,
                segments,
                type_args,
            } => {
                let item = ItemUse {
                    type_args: type_args.as_ref(),
                    ..ItemUse::plain(None, hint, span)
                };
                self.path_expr(*leading_dot, segments, item)
            }
            ast::ExprKind::Array(elems) => self.array_lit(elems, hint, span),
            ast::ExprKind::ArrayRepeat {
                elem,
                count: ast::RepeatCount::Literal(count),
            } => self.array_repeat(*elem, *count, hint, span),
            // `[0; SIZE]`: a count that names a constant is known where it
            // is written, as a type's length is.
            ast::ExprKind::ArrayRepeat {
                elem,
                count: ast::RepeatCount::Expr(count),
            } if self.constant_count(*count).is_some() => {
                let name = self.constant_count(*count).expect("just checked");
                match self.array_len(ast::ArrayLen::Name(name)) {
                    Some(count) => self.array_repeat(*elem, count, hint, span),
                    None => {
                        self.infer(*elem, None);
                        self.error_expr(span)
                    }
                }
            }
            // `own [x; n]` handles its own count; anywhere else a count must
            // be a literal.
            ast::ExprKind::ArrayRepeat {
                elem,
                count: ast::RepeatCount::Expr(count),
            } => {
                self.infer(*elem, None);
                self.check_operand(*count, Types::I64);
                let diagnostic = Diagnostic::error(
                    codes::RUNTIME_LENGTH,
                    "a length known only at run time needs `own`",
                    span,
                    "the length is not a literal",
                )
                .with_note("an array's length is part of its type; only a buffer on the heap keeps its length with it")
                .with_fix("allocate it on the heap", [Edit::insert(span.lo, "own ")]);
                self.report(diagnostic);
                self.error_expr(span)
            }
            ast::ExprKind::Match { scrutinee, arms } => {
                self.match_expr(*scrutinee, arms, hint, span)
            }
            ast::ExprKind::Is {
                scrutinee,
                pattern,
                negated: false,
            } => self.is_expr(*scrutinee, pattern, false, span),
            ast::ExprKind::Is {
                scrutinee,
                pattern,
                negated: true,
            } => self.negated_is(*scrutinee, pattern, span),
            ast::ExprKind::Unary { op, operand, .. } => self.unary(*op, *operand, hint, span),
            // A chain whose tests bind, outside a condition.
            ast::ExprKind::Binary {
                op: ast::BinaryOp::And,
                ..
            } if self.chain_binds(id) => self.bound_chain(id, span),
            ast::ExprKind::Binary {
                op,
                op_span,
                lhs,
                rhs,
                wrapping,
            } => self.binary(*op, *op_span, (*lhs, *rhs), *wrapping, hint, span),
            // `f(style = x)` was reported as the named argument it was
            // meant to be, and is checked as its value.
            ast::ExprKind::Assign { value, .. } if self.state.named_by_equals.contains(&id) => {
                self.infer(*value, hint)
            }
            ast::ExprKind::Assign {
                target,
                op,
                wrapping,
                value,
            } => self.assign(*target, *op, *wrapping, *value, span),
            ast::ExprKind::Field { base, name } => self.field(*base, *name, span),
            ast::ExprKind::Lend(value) => self.lend_expr(*value, span),
            ast::ExprKind::Call {
                callee,
                args,
                names,
                rest,
            } => self.call_expr(*callee, args, names, *rest, hint, span),
            ast::ExprKind::Index { base, index } => self.index(*base, index, span),
            ast::ExprKind::SubSlice {
                base,
                lo,
                hi,
                inclusive,
            } => self.sub_slice(*base, *lo, *hi, *inclusive, span, false),
            ast::ExprKind::Error => self.error_expr(span),
            // A loop where a value stands, which no `own` collects: a
            // generator, whose loop runs as it is asked.
            ast::ExprKind::ForElement {
                binding,
                source,
                body,
            } => self.loop_generator(id, binding, *source, body, span),
        }
    }

    /// `own [elem; count]` with a count known only at run time: a buffer of
    /// type `own<[T]>`.
    fn own_repeat(
        &mut self,
        elem: ast::ExprId,
        count: ast::ExprId,
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        let elem_hint = match hint.map(|h| self.kind(h)) {
            Some(TyKind::Own(inner)) => match self.kind(inner) {
                TyKind::Slice(elem) => Some(elem),
                _ => None,
            },
            _ => None,
        };
        let elem = match elem_hint {
            Some(ty) => self.check(elem, ty),
            None => self.infer(elem, None),
        };
        let count = self.check_operand(count, Types::I64);
        let elem_ty = self.ty_of(elem);
        if self.is_poisoned(elem_ty) || self.stored_view_at(elem_ty, span, "a buffer", None) {
            return self.error_expr(span);
        }
        let slice = self.intern(TyKind::Slice(elem_ty));
        let ty = self.intern(TyKind::Own(slice));
        self.alloc(ExprKind::OwnRepeat { elem, count }, ty, span)
    }

    /// The type of an integer literal: the integer type expected of it, or
    /// `i64`.
    pub(super) fn int_literal_type(&self, hint: Option<Ty>) -> Ty {
        match hint {
            Some(ty) if self.program.types.is_integer(ty) => ty,
            _ => Types::I64,
        }
    }

    /// The bits of an integer literal of type `ty`, `-magnitude` when
    /// `negative`, reporting it if it is out of range.
    pub(super) fn int_literal(
        &mut self,
        negative: bool,
        magnitude: u128,
        ty: Ty,
        span: Span,
    ) -> u128 {
        let t = self
            .program
            .types
            .int_ty(ty)
            .expect("integer literals have an integer type");
        let fits = if negative {
            magnitude <= t.min_magnitude()
        } else {
            magnitude <= t.max()
        };
        if fits {
            let bits = if negative {
                magnitude.wrapping_neg()
            } else {
                magnitude
            };
            return bits & t.mask();
        }
        let name = t.name();
        let text = if negative {
            format!("-{magnitude}")
        } else {
            magnitude.to_string()
        };
        let diagnostic = Diagnostic::error(
            codes::INTEGER_OUT_OF_RANGE,
            format!("integer literal `{text}` is out of range for `{name}`"),
            span,
            format!("does not fit in `{name}`"),
        );
        let note = if negative && !t.signed() {
            format!("`{name}` is unsigned, so it has no negative values")
        } else if t.signed() {
            format!("`{name}` holds -{} to {}", t.min_magnitude(), t.max())
        } else {
            format!("`{name}` holds 0 to {}", t.max())
        };
        self.report(diagnostic.with_note(note));
        0
    }

    /// What a variable or a constant holding a reference stands for: the
    /// place it refers to, and through a reference to a
    /// reference what the inner one refers to, except where
    /// that `&` reference is what is expected: `&var at` of a `var at:
    /// &Node` lends the variable, and `val b: &Node = a` copies `a`. A
    /// closure is the reference itself: it carries what it captured beside
    /// its code.
    pub(super) fn referent(&mut self, mut e: ExprId, hint: Option<Ty>, span: Span) -> ExprId {
        while let TyKind::Ref(inner, kind) = self.kind(self.ty_of(e))
            && !matches!(self.kind(inner), TyKind::Fn(..))
            && !(kind == crate::RefKind::Shared && hint == Some(self.ty_of(e)))
        {
            e = self.alloc(ExprKind::Deref(e), inner, span);
        }
        e
    }

    /// The reference a slice was read through, where what is kept is the
    /// slice: `val all = bytes` and `[bytes, other]` of a `bytes: &[u8]`
    /// hold the reference, since there is nothing of fixed size to copy.
    /// Anything else is not a slice read so.
    pub(super) fn kept_reference(&self, e: ExprId) -> Option<ExprId> {
        let ExprKind::Deref(inner) = self.state.body.exprs[e].kind else {
            return None;
        };
        let TyKind::Ref(referent, crate::RefKind::Shared) = self.kind(self.ty_of(inner)) else {
            return None;
        };
        matches!(self.kind(referent), TyKind::Slice(_)).then_some(inner)
    }

    /// The amount of a shift of `shifted`, made the shifted value's type
    /// where both are integers of different types; anything else as it is.
    pub(super) fn shift_amount(&mut self, op: BinaryOp, shifted: ExprId, amount: ExprId) -> ExprId {
        let (st, at) = (self.ty_of(shifted), self.ty_of(amount));
        let integers =
            matches!(self.kind(st), TyKind::Int(_)) && matches!(self.kind(at), TyKind::Int(_));
        if !matches!(op, BinaryOp::Shl | BinaryOp::Shr) || st == at || !integers {
            return amount;
        }
        let span = self.state.body.exprs[amount].span;
        self.alloc(ExprKind::Cast(amount), st, span)
    }

    /// An operand of `op`. A comparison reads its operands, as a method
    /// reads its receiver, so a range of elements may be one, lent where it
    /// is written: `window[0..4] == magic`.
    fn compared_operand(&mut self, op: BinaryOp, e: ast::ExprId, hint: Option<Ty>) -> ExprId {
        use BinaryOp::*;
        if matches!(op, Eq | Ne | Lt | Le | Gt | Ge) {
            let id = self.infer_borrowed(e, hint);
            return self.autoderef(id);
        }
        self.operand(e, hint)
    }

    /// What a comparison reads of an array, a slice, a buffer or a
    /// reference to a slice: its elements as a slice place, and their type.
    /// Nothing for anything else.
    fn as_elements(&mut self, e: ExprId) -> Option<(ExprId, Ty)> {
        let ty = self.ty_of(e);
        let span = self.state.body.exprs[e].span;
        match self.kind(ty) {
            TyKind::Array(elem, _) => {
                let slice = self.intern(TyKind::Slice(elem));
                let whole = ExprKind::SubSlice {
                    base: e,
                    lo: None,
                    hi: None,
                };
                Some((self.alloc(whole, slice, span), elem))
            }
            TyKind::Slice(elem) => Some((e, elem)),
            TyKind::Own(inner) | TyKind::Ref(inner, _) => match self.kind(inner) {
                TyKind::Slice(elem) => Some((self.alloc(ExprKind::Deref(e), inner, span), elem)),
                _ => None,
            },
            _ => None,
        }
    }

    /// `+%` or `+%=` on a type of a program's own, which answers only the plain
    /// operators: the wrapping forms are the machine's, for its integers.
    /// `suffix` is `=` for the compound form.
    pub(super) fn report_wrapping_own_type(
        &mut self,
        op: BinaryOp,
        op_span: Span,
        ty: Ty,
        suffix: &str,
    ) {
        let text = op.text();
        let diagnostic = Diagnostic::error(
            codes::INVALID_OPERANDS,
            format!("`{text}%{suffix}` wraps an integer, and {} is not one", self.ty_name(ty)),
            op_span,
            "the wrapping form",
        )
        .with_note(
            "the wrapping operators say what an integer does when it overflows; a type of a program's own answers only the plain ones",
        )
        .with_help(format!(
            "write `{text}{suffix}`, which the type answers where it implements its interface"
        ));
        self.report(diagnostic);
    }

    pub(super) fn name(&mut self, sym: Symbol, hint: Option<Ty>, span: Span) -> ExprId {
        if let Some(local) = self.lookup(sym) {
            let ty = self.state.body.locals[local].ty;
            let e = self.alloc(ExprKind::Local(local), ty, span);
            return self.referent(e, hint, span);
        }
        // A variable of the function around a lambda is captured, before
        // anything the module or the prelude calls by its name.
        if self.names_around(sym)
            && let Some(captured) = self.captured_name(sym, span)
        {
            return captured;
        }
        // What else the name could be, here or in the module it was imported
        // from.
        let (module, item) = match self.imported(ast::Name { sym, span }) {
            Some(ImportedItem::Broken) => return self.error_expr(span),
            Some(ImportedItem::Item(module, item)) => (module, item.sym),
            None => (self.current, sym),
        };
        // A top-level `val`: the value it was worked out to.
        if let Some(&id) = self.modules[module].consts.get(&item) {
            let exported = self.program.consts[id].is_pub || module == self.current;
            if exported {
                let value = self.const_use(id, span);
                return self.referent(value, hint, span);
            }
        }
        // A variable C owns: reading it is a call to the accessor the
        // compiler declared for it.
        if let Some(&id) = self.modules[module].globals.get(&item) {
            let def = &self.program.globals[id];
            if def.is_pub || module == self.current {
                let (getter, ty) = (def.getter, def.ty);
                return self.alloc(
                    ExprKind::Call {
                        callee: getter,
                        args: Vec::new(),
                        type_args: crate::TyList::EMPTY,
                        order: Vec::new(),
                    },
                    ty,
                    span,
                );
            }
        }
        // A function named as a value.
        if let Some(&id) = self.modules[module].fns.get(&item) {
            return self.fn_value(id, ItemUse::plain(None, hint, span), None);
        }
        if let Some(id) = self.prelude_const(item) {
            let value = self.const_use(id, span);
            return self.referent(value, hint, span);
        }
        if let Some(id) = self.prelude_fn(item) {
            return self.fn_value(id, ItemUse::plain(None, hint, span), None);
        }
        // A name of the function around a lambda is captured.
        if let Some(captured) = self.captured_name(sym, span) {
            return captured;
        }
        let text = self.text(sym);
        let diagnostic = if let Some(&(def, _)) = self.modules[module].types.get(&item) {
            let diagnostic = Diagnostic::error(
                codes::NOT_A_VALUE,
                format!("`{text}` is a type, not a value"),
                span,
                "type used as a value",
            );
            match def {
                TypeDef::Builtin(_) => diagnostic,
                TypeDef::Enum(_) => {
                    diagnostic.with_help(format!("enum variants are written `{text}::Variant`"))
                }
                TypeDef::Struct(_) => {
                    diagnostic.with_help(format!("to construct one, write `{text} {{ … }}`"))
                }
            }
        } else {
            self.unknown_name(sym, span)
        };
        self.report(diagnostic);
        self.error_expr(span)
    }

    pub(super) fn unknown_name(&self, sym: Symbol, span: Span) -> Diagnostic {
        let text = self.text(sym);
        let locals = self.state.scopes.iter().flat_map(|scope| scope.keys());
        let candidates = locals.chain(self.fns().keys()).map(|&s| self.text(s));
        let diagnostic = Diagnostic::error(
            codes::UNKNOWN_NAME,
            format!("cannot find `{text}` in this scope"),
            span,
            "not found",
        );
        if let Some(help) = left_the_prelude(text) {
            return diagnostic.with_help(help);
        }
        match suggest(text, candidates) {
            Some(similar) => diagnostic.with_fix(
                format!("a name with a similar spelling exists: `{similar}`"),
                [Edit::replace(span, similar)],
            ),
            None => diagnostic,
        }
    }

    pub(super) fn array_lit(
        &mut self,
        elems: &'a [ast::ExprId],
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        // An array's length is part of its type, known where it is
        // written; a list built as it runs is on the heap.
        if let Some(&yields) = elems.iter().find(|&&e| lists::yields(self.ast, e)) {
            return self.needs_own(
                self.ast.exprs[yields].span,
                span,
                "a list whose elements are yielded",
            );
        }
        // `[a, if c { b }]`: an element that is no value.
        if let Some(&nothing) = elems.iter().find(|&&e| {
            matches!(self.ast.exprs[e].kind, ast::ExprKind::If { .. })
                && body::lacks_else(self.ast, e)
        }) {
            self.adds_nothing(self.ast.exprs[nothing].span);
            return self.error_expr(span);
        }
        // An array is expected, or a slice it becomes, as `own<[View]>`
        // takes `own [.Text(…), .Space()]`.
        let elem_hint = match hint.map(|h| self.kind(h)) {
            Some(TyKind::Array(t, _) | TyKind::Slice(t)) => Some(t),
            _ => None,
        };
        let Some((&first, rest)) = elems.split_first() else {
            let Some(elem) = elem_hint else {
                let diagnostic = Diagnostic::error(
                    codes::CANNOT_INFER,
                    "cannot infer the element type of an empty array",
                    span,
                    "type unknown",
                )
                .with_help("annotate the variable, for example `val xs: [i64; 0] = []`");
                self.report(diagnostic);
                return self.error_expr(span);
            };
            let ty = self.intern(TyKind::Array(elem, 0));
            return self.alloc(ExprKind::Array(Vec::new()), ty, span);
        };
        let first = match elem_hint {
            Some(t) => self.check(first, t),
            None => {
                let first = self.infer(first, None);
                self.kept_reference(first).unwrap_or(first)
            }
        };
        let elem = elem_hint.unwrap_or(self.ty_of(first));
        let mut hir_elems = vec![first];
        for &e in rest {
            hir_elems.push(self.check(e, elem));
        }
        let ty = self.intern(TyKind::Array(elem, elems.len() as u64));
        self.alloc(ExprKind::Array(hir_elems), ty, span)
    }

    /// `[elem; count]` with its count known.
    fn array_repeat(
        &mut self,
        elem: ast::ExprId,
        count: u64,
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        let elem = match hint.map(|h| self.kind(h)) {
            Some(TyKind::Array(t, _)) => self.check(elem, t),
            _ => self.infer(elem, None),
        };
        let ty = self.intern(TyKind::Array(self.ty_of(elem), count));
        self.alloc(ExprKind::ArrayRepeat { elem, count }, ty, span)
    }

    /// The name of a repeat count that is a constant, not a variable.
    fn constant_count(&self, count: ast::ExprId) -> Option<ast::Name> {
        let expr = &self.ast.exprs[count];
        let ast::ExprKind::Name(sym) = expr.kind else {
            return None;
        };
        if self.lookup(sym).is_some() {
            return None;
        }
        let name = ast::Name {
            sym,
            span: expr.span,
        };
        self.const_named(name).map(|_| name)
    }

    pub(super) fn unary(
        &mut self,
        op: ast::UnaryOp,
        operand: ast::ExprId,
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        let ast = self.ast;
        match op {
            ast::UnaryOp::Neg => {
                // `-9223372036854775808` is a literal, not the negation of an
                // out-of-range one.
                if let ast::ExprKind::Int(v) = ast.exprs[operand].kind {
                    let ty = self.int_literal_type(hint);
                    let value = self.int_literal(true, v, ty, span);
                    return self.alloc(ExprKind::Int(value), ty, span);
                }
                let e = self.operand(operand, hint);
                let ty = self.ty_of(e);
                if let Some(t) = self.program.types.int_ty(ty)
                    && !t.signed()
                {
                    let diagnostic = Diagnostic::error(
                        codes::INVALID_OPERANDS,
                        format!("cannot negate a value of type `{}`", t.name()),
                        span,
                        "`-` needs a signed number",
                    )
                    .with_note(format!(
                        "`{}` is unsigned, so it has no negative values",
                        t.name()
                    ))
                    .with_help("convert it to a signed type first, for example `-(x as i64)`");
                    self.report(diagnostic);
                    return self.error_expr(span);
                }
                if self.program.types.is_numeric(ty) {
                    return self.alloc(
                        ExprKind::Unary {
                            op: UnaryOp::Neg,
                            operand: e,
                        },
                        ty,
                        span,
                    );
                }
                // `-x` on a type that implements `Negate` is `x.negate()`.
                if let Some(negate) = self.interface_method(KnownInterface::Negate, ty) {
                    return self.unary_call(negate, ty, e, span);
                }
                if !self.is_poisoned(ty) {
                    let mut diagnostic = Diagnostic::error(
                        codes::INVALID_OPERANDS,
                        format!("cannot negate a value of type {}", self.ty_name(ty)),
                        span,
                        "`-` needs a number",
                    );
                    diagnostic = self.operator_hint(diagnostic, ty, "Negate", "negates");
                    self.report(diagnostic);
                }
                self.error_expr(span)
            }
            // `!` negates a `bool` and flips every bit of an integer.
            ast::UnaryOp::Not => {
                let e = self.operand(operand, hint);
                let ty = self.ty_of(e);
                if ty == Types::BOOL || self.program.types.is_integer(ty) {
                    return self.alloc(
                        ExprKind::Unary {
                            op: UnaryOp::Not,
                            operand: e,
                        },
                        ty,
                        span,
                    );
                }
                // `!x` on a type that implements `Not` is `x.not()`.
                if let Some(not) = self.interface_method(KnownInterface::Not, ty) {
                    return self.unary_call(not, ty, e, span);
                }
                if !self.is_poisoned(ty) {
                    let mut diagnostic = Diagnostic::error(
                        codes::INVALID_OPERANDS,
                        format!("cannot apply `!` to {}", self.ty_name(ty)),
                        span,
                        "`!` needs a `bool` or an integer",
                    );
                    diagnostic = self.operator_hint(diagnostic, ty, "Not", "answers `!`");
                    self.report(diagnostic);
                }
                self.error_expr(span)
            }
            ast::UnaryOp::Ref | ast::UnaryOp::RefVar => {
                self.reference(op, operand, hint, span, false)
            }
            ast::UnaryOp::Move => {
                let e = self.infer(operand, hint);
                let ty = self.ty_of(e);
                if !self.state.body.is_place(e) && !self.is_poisoned(ty) {
                    let diagnostic = Diagnostic::error(
                        codes::MOVE_NOT_A_PLACE,
                        "`move` needs a variable, field or element",
                        self.state.body.exprs[e].span,
                        "a temporary value",
                    )
                    .with_help("a temporary is already moved where it is used; remove `move`");
                    self.report(diagnostic);
                }
                self.no_move_out_of_drop(e, span);
                self.alloc(ExprKind::Move(e), ty, span)
            }
            ast::UnaryOp::Own => {
                // `own [a, for x in xs { yield x }]` and `own for …`: a list
                // whose `yield`s decide how long it is.
                if let ast::ExprKind::Array(elems) = &ast.exprs[operand].kind
                    && elems.iter().any(|&e| lists::yields(ast, e))
                {
                    return self.built_list(elems, hint, span);
                }
                if let ast::ExprKind::ForElement { .. } = ast.exprs[operand].kind {
                    return self.built_list(&[operand], hint, span);
                }
                // `own [x; n]` with a length known only at run time.
                if let ast::ExprKind::ArrayRepeat {
                    elem,
                    count: ast::RepeatCount::Expr(count),
                } = ast.exprs[operand].kind
                {
                    return self.own_repeat(elem, count, hint, span);
                }
                // `own (x) => …`: an owned closure, whose environment holds
                // what it captured on the heap.
                if let ast::ExprKind::Lambda { .. } = ast.exprs[operand].kind {
                    return self.lambda(operand, hint, true, span);
                }
                let inner_hint = match hint.map(|h| self.kind(h)) {
                    Some(TyKind::Own(t)) => Some(t),
                    _ => None,
                };
                let e = self.infer(operand, inner_hint);
                let inner = self.ty_of(e);
                if self.is_poisoned(inner) {
                    return self.error_expr(span);
                }
                let ty = self.intern(TyKind::Own(inner));
                self.alloc(ExprKind::Own(e), ty, span)
            }
        }
    }

    /// Whether what is borrowed is a closure, which is written `&(…) => R`
    /// wherever it is lent, so that a lent one is told from an owned one.
    fn lends_a_closure(&self, inner: Ty) -> bool {
        match self.kind(inner) {
            TyKind::Fn(..) => true,
            TyKind::Own(code) => matches!(self.kind(code), TyKind::Fn(..)),
            _ => false,
        }
    }

    /// Whether an expression is a number written down, which takes the
    /// type expected of it rather than one of its own.
    pub(super) fn number_literal(&self, id: ast::ExprId) -> bool {
        match self.ast.exprs[id].kind {
            ast::ExprKind::Int(_) | ast::ExprKind::Float(_) => true,
            ast::ExprKind::Unary {
                op: ast::UnaryOp::Neg,
                operand,
                ..
            } => matches!(
                self.ast.exprs[operand].kind,
                ast::ExprKind::Int(_) | ast::ExprKind::Float(_)
            ),
            _ => false,
        }
    }

    pub(super) fn binary(
        &mut self,
        op: BinaryOp,
        op_span: Span,
        (lhs, rhs): (ast::ExprId, ast::ExprId),
        wrapping: bool,
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        use BinaryOp::*;
        if matches!(op, And | Or) {
            let lhs = self.check_operand(lhs, Types::BOOL);
            let rhs = self.check_operand(rhs, Types::BOOL);
            return self.alloc(
                ExprKind::Binary {
                    op,
                    lhs,
                    rhs,
                    wrapping,
                },
                Types::BOOL,
                span,
            );
        }
        // A literal takes the type of the other operand, whichever side it
        // is on: `byte >= 48` and `48 <= byte` mean the same thing.
        // Arithmetic answers its operands' type, so a number expected of it
        // is expected of them: `val step: f32 = 1.0 / 60.0`.
        let arithmetic = matches!(
            op,
            Add | Sub | Mul | Div | Rem | BitAnd | BitOr | BitXor | Shl | Shr
        );
        let hint = hint.filter(|&ty| arithmetic && self.program.types.is_numeric(ty));
        // So does `null`, which is a pointer of the other side's type.
        let takes_other = |lowerer: &Self, id: ast::ExprId| {
            lowerer.number_literal(id) || matches!(lowerer.ast.exprs[id].kind, ast::ExprKind::Null)
        };
        // Beside a type of a program's own, a number is what an
        // implementation of the operator takes it as: on the right, the
        // `Rhs` of the left type's one implementation that is not of its
        // own type, `v * 0.5`; on the left, the built-in type whose
        // implementation takes the right side, `2.0 * v`.
        let interface = KnownInterface::of_binary(op);
        let only = |tys: Vec<Ty>| match tys.as_slice() {
            [one] => Some(*one),
            _ => None,
        };
        // Not a shift's amount, though: it may be of any type, and says
        // nothing of what is shifted, whose type is the answer's — `1 << n`
        // of a `u32` amount is the `1` expected, or an `i64`.
        let shift = matches!(op, Shl | Shr);
        let (l, r) = if takes_other(self, lhs) && !takes_other(self, rhs) && !shift {
            let r = self.compared_operand(op, rhs, hint);
            let rt = self.ty_of(r);
            let left = match interface {
                Some(which) if !self.program.types.is_numeric(rt) => {
                    only(self.operator_lhs_builtins(which, rt))
                }
                _ => None,
            };
            let l = self.compared_operand(op, lhs, Some(left.unwrap_or(rt)));
            (l, r)
        } else {
            let l = self.compared_operand(op, lhs, hint);
            let lt = self.ty_of(l);
            let right = match interface {
                Some(which) if !self.operator_applies(op, lt) => {
                    let mut rhs = self.operator_rhs_tys(which, lt);
                    rhs.retain(|&rhs| rhs != lt);
                    only(rhs)
                }
                _ => None,
            };
            let r = self.compared_operand(op, rhs, Some(right.unwrap_or(lt)));
            (l, r)
        };
        let lt = self.ty_of(l);
        let rt = self.ty_of(r);
        let result = match op {
            Add | Sub | Mul | Div | Rem | BitAnd | BitOr | BitXor | Shl | Shr => lt,
            _ => Types::BOOL,
        };
        if self.is_poisoned(lt) || self.is_poisoned(rt) {
            return self.alloc(
                ExprKind::Binary {
                    op,
                    lhs: l,
                    rhs: r,
                    wrapping,
                },
                result,
                span,
            );
        }
        // Comparing reads the text, so a `String` lends its bytes for it,
        // as it does for an argument.
        let text = |ty| ty == Types::STR;
        let (l, r) = if matches!(op, Eq | Ne | Lt | Le | Gt | Ge)
            && (self.lends_text(lt) || self.lends_text(rt))
            && (self.lends_text(lt) || text(lt))
            && (self.lends_text(rt) || text(rt))
        {
            let l = self.lend_string(l).unwrap_or(l);
            let r = self.lend_string(r).unwrap_or(r);
            (l, r)
        } else {
            (l, r)
        };
        // A shift's amount may be of any integer type: it is taken modulo
        // the width of what is shifted, which it keeps when it is made that
        // type first, since every width is a power of two.
        let r = self.shift_amount(op, l, r);
        let (lt, rt) = (self.ty_of(l), self.ty_of(r));
        // Two strings are compared by their bytes, not by where they are:
        // the same word is the same word wherever it was written.
        if lt == Types::STR && rt == Types::STR && matches!(op, Eq | Ne | Lt | Le | Gt | Ge) {
            let order = self.alloc(ExprKind::StrCmp { lhs: l, rhs: r }, Types::I64, span);
            let zero = self.alloc(ExprKind::Int(0), Types::I64, span);
            return self.alloc(
                ExprKind::Binary {
                    op,
                    lhs: order,
                    rhs: zero,
                    wrapping: false,
                },
                Types::BOOL,
                span,
            );
        }
        // An array is compared as the slice of all of it, which is what
        // implements `Eq` and `Ord`; and an array, a slice and a buffer of one
        // element type are compared with each other, either way round, as the
        // slices of their elements. A mistake is reported with the operands as
        // written.
        let written = (l, r);
        let compared = matches!(op, Eq | Ne | Lt | Le | Gt | Ge)
            && (lt != rt || matches!(self.kind(lt), TyKind::Array(..)));
        let (l, r, lt, rt) = match (compared, self.as_elements(l), self.as_elements(r)) {
            (true, Some((l, le)), Some((r, re))) if le == re => {
                let slice = self.intern(TyKind::Slice(le));
                (l, r, slice, slice)
            }
            _ => (l, r, lt, rt),
        };
        // `a < b` on a type that implements `Ord` asks it.
        if matches!(op, Lt | Le | Gt | Ge)
            && lt == rt
            && !self.operator_applies(op, lt)
            && let Some(compare) = self.interface_method(KnownInterface::Ord, lt)
        {
            return self.ordering(op, compare, l, r, span);
        }
        // `a == b` on a type that implements `Eq` is `a.equals(&b)`.
        if matches!(op, Eq | Ne)
            && lt == rt
            && !self.operator_applies(op, lt)
            && let Some(equals) = self.interface_method(KnownInterface::Eq, lt)
        {
            return self.equality(op, equals, l, r, span);
        }
        // `a + b` on a type that implements `Add` is `a.add(&b)`, and so
        // for the rest of the arithmetic: the left side's
        // implementation whose `Rhs` is the right side's type, answering its
        // `Out`. The wrapping forms are the machine's, for
        // its integers.
        if matches!(op, Add | Sub | Mul | Div | Rem)
            && !wrapping
            && !(lt == rt && self.operator_applies(op, lt))
            && let Some(interface) = KnownInterface::of_binary(op)
            && let Some(found) = self.operator_impl(interface, lt, rt)
        {
            return self.operator_call(found, l, r, span);
        }
        // The wrapping forms are the machine's, for its integers: a type of
        // a program's own answers `+`, never `+%`.
        if wrapping && matches!(self.kind(lt), TyKind::Struct(..) | TyKind::Enum(..)) {
            self.report_wrapping_own_type(op, op_span, lt, "");
            return self.error_expr(span);
        }
        if !(lt == rt && self.operator_applies(op, lt)) {
            self.report_operands(op, op_span, op.text(), written.0, written.1);
            // With a type of a program's own on either side, what the
            // answer would have been is unknown, and nothing more is said
            // of it.
            let declared = |lowerer: &Self, ty: Ty| {
                matches!(lowerer.kind(ty), TyKind::Struct(..) | TyKind::Enum(..))
            };
            if lt != rt && (declared(self, lt) || declared(self, rt)) {
                return self.error_expr(span);
            }
        }
        self.alloc(
            ExprKind::Binary {
                op,
                lhs: l,
                rhs: r,
                wrapping,
            },
            result,
            span,
        )
    }

    /// `&place` or `&var place`. A call's argument may be either, which
    /// `argument` says. Anywhere else a `&place` may stand
    /// where a `&` reference is expected — a variant's payload, a local
    /// declared as one, an assignment to one — since a `&` is kept where a
    /// view is; a `&var` may not. What is refused is
    /// reported, and checked as if it were an argument, so that what is
    /// inside is checked too.
    pub(super) fn reference(
        &mut self,
        op: ast::UnaryOp,
        operand: ast::ExprId,
        hint: Option<Ty>,
        span: Span,
        argument: bool,
    ) -> ExprId {
        let kind = if op == ast::UnaryOp::RefVar {
            crate::RefKind::Var
        } else {
            crate::RefKind::Shared
        };
        // A `val`'s whole value: this `&`, and no `&` inside it.
        let local = std::mem::take(&mut self.state.local_reference);
        let inner_hint = match hint.map(|h| self.kind(h)) {
            Some(TyKind::Ref(t, _)) => Some(t),
            _ => None,
        };
        // The operand may be a range of elements.
        let e = self.infer_borrowed(operand, inner_hint);
        let inner = self.ty_of(e);
        if self.is_poisoned(inner) {
            return self.error_expr(span);
        }
        let expected = kind == crate::RefKind::Shared
            && (local
                || matches!(
                    hint.map(|h| self.kind(h)),
                    Some(TyKind::Ref(_, crate::RefKind::Shared))
                ));
        if !argument && !expected {
            let (message, note, help) = if kind == crate::RefKind::Var {
                (
                    "`&var` can only be passed as an argument",
                    "a `&var` exists only while a call runs, so that two writers of one place are only ever two arguments of one call",
                    "write the place directly, or pass it to a function that takes a `&var`",
                )
            } else {
                (
                    "`&` can only be written where a reference is expected",
                    "a `&` reference is an argument, a view's field, a variant's payload, a local's value or an assignment to one",
                    "use the value directly, or keep it in a local: `val r = &place`",
                )
            };
            let diagnostic = Diagnostic::error(
                codes::REF_OUTSIDE_ARGUMENT,
                message,
                span,
                format!("`{}` where no reference is expected", op.text()),
            )
            .with_note(note)
            .with_help(help);
            self.report(diagnostic);
        }
        if kind == crate::RefKind::Var {
            self.require_writable(e, Writing::Borrow);
        }
        // A value with no place of its own is lent where it is
        // used, as it is moved where it is used: `move` on one is
        // E0317, and this is the same rule for `&`.
        // Elsewhere, where a reference is expected, `&` is the only way to
        // lend one, and the move checker refuses it where it is kept past
        // its statement.
        if kind == crate::RefKind::Shared
            && argument
            && !self.lends_a_closure(inner)
            && matches!(self.place_root(e), PlaceRoot::NotAPlace)
        {
            let amp = Span::new(span.lo, span.lo + 1);
            let diagnostic = Diagnostic::error(
                codes::REF_NOT_A_PLACE,
                "`&` needs a variable, field or element",
                self.state.body.exprs[e].span,
                "a temporary value",
            )
            .with_fix("remove the `&`", [Edit::replace(amp, "")])
            .with_note("a temporary is already lent where it is used, as it is already moved");
            self.report(diagnostic);
        }
        // An owned closure lends itself, as an `own<T>` lends a `&T`:
        // the same pair, seen as one lent for a call.
        if let TyKind::Own(code) = self.kind(inner)
            && matches!(self.kind(code), TyKind::Fn(..))
        {
            let ty = self.intern(TyKind::Ref(code, kind));
            return self.alloc(ExprKind::LendClosure(e), ty, span);
        }
        let ty = self.intern(TyKind::Ref(inner, kind));
        self.alloc(ExprKind::Ref(e), ty, span)
    }

    /// `-x` or `!x` as the call it stands for, with the operand lent.
    fn unary_call(&mut self, method: FnId, ty: Ty, operand: ExprId, span: Span) -> ExprId {
        let lent = self.intern(TyKind::Ref(ty, crate::RefKind::Shared));
        let receiver = self.alloc(ExprKind::Ref(operand), lent, span);
        let type_args = match self.program.fns[method].interface {
            Some(_) => self.program.types.intern_list(&[ty]),
            None => {
                let args = self.owner_args(ty);
                self.program.types.intern_list(&args)
            }
        };
        self.alloc(
            ExprKind::Call {
                callee: method,
                args: vec![receiver],
                type_args,
                order: vec![0],
            },
            ty,
            span,
        )
    }

    /// The help for an operator a type of a program's own does not answer:
    /// the block that would make it.
    pub(super) fn operator_hint(
        &self,
        diagnostic: Diagnostic,
        ty: Ty,
        interface: &str,
        what: &str,
    ) -> Diagnostic {
        if !matches!(self.kind(ty), TyKind::Struct(..) | TyKind::Enum(..)) {
            return diagnostic;
        }
        let name = self.ty_name(ty).trim_matches('`').to_string();
        diagnostic.with_help(format!("a type {what} by `extend {name}: {interface}`"))
    }

    /// The method of one of the prelude's one-method interfaces for a type that
    /// implements it, if it does: the implementation's, or for a type parameter
    /// constrained by the interface, the interface's own, which its instance
    /// replaces. It is what `==`, `<`, `+`, `-x` and the rest call on a type of
    /// a program's own.
    pub(super) fn interface_method(&mut self, which: KnownInterface, ty: Ty) -> Option<FnId> {
        let interface = self.program.prelude_items.interface(which)?;
        if let TyKind::Param(param) = self.kind(ty) {
            let constrained = self
                .type_params
                .get(param.index as usize)
                .is_some_and(|p| p.interfaces.iter().any(|c| c.interface == interface));
            return constrained.then(|| self.program.interfaces[interface].methods[0].id);
        }
        self.method_of_impl(ty, interface)
    }

    /// The implementation `a op b` calls, where `a` is of type `lt` and `b`
    /// of `rt`: the left type's implementation of the operator's interface
    /// whose `Rhs` is `rt`, or, for a type parameter, a constraint that
    /// says it has one. Nothing is looked for on
    /// the right: `2.0 * v` is an implementation for `f32`.
    pub(super) fn operator_impl(
        &mut self,
        which: KnownInterface,
        lt: Ty,
        rt: Ty,
    ) -> Option<OperatorImpl> {
        let interface = self.program.prelude_items.interface(which)?;
        if let TyKind::Param(param) = self.kind(lt) {
            let constraint = self
                .type_params
                .get(param.index as usize)?
                .interfaces
                .iter()
                .find(|c| {
                    c.interface == interface && self.program.types.list(c.args).first() == Some(&rt)
                })
                .copied()?;
            let args = self.program.types.list(constraint.args).to_vec();
            let mut type_args = vec![lt];
            type_args.extend(&args);
            return Some(OperatorImpl {
                method: self.program.interfaces[interface].methods[0].id,
                type_args: self.program.types.intern_list(&type_args),
                rhs: rt,
                out: args.get(1).copied().unwrap_or(lt),
            });
        }
        let owner = self.owner_of(lt)?;
        let owner_args = self.owner_args(lt);
        for at in 0..self.program.impls.len() {
            let found = &self.program.impls[at];
            if found.interface != interface || found.ty != owner {
                continue;
            }
            let args = self.program.types.list(found.args).to_vec();
            let (Some(&rhs), Some(&out)) = (args.first(), args.get(1)) else {
                continue;
            };
            let rhs = self.program.types.subst(rhs, &owner_args);
            if rhs != rt || self.unmet_condition(at, &owner_args).is_some() {
                continue;
            }
            let method = *self.program.impls[at].methods.first()?;
            let out = self.program.types.subst(out, &owner_args);
            return Some(OperatorImpl {
                method,
                type_args: self.program.types.intern_list(&owner_args),
                rhs,
                out,
            });
        }
        None
    }

    /// The right sides the left type's implementations of an operator take,
    /// which say what a number written there is.
    pub(super) fn operator_rhs_tys(&mut self, which: KnownInterface, lt: Ty) -> Vec<Ty> {
        let Some(interface) = self.program.prelude_items.interface(which) else {
            return Vec::new();
        };
        if let TyKind::Param(param) = self.kind(lt) {
            return self
                .type_params
                .get(param.index as usize)
                .map(|p| {
                    p.interfaces
                        .iter()
                        .filter(|c| c.interface == interface)
                        .filter_map(|c| self.program.types.list(c.args).first().copied())
                        .collect()
                })
                .unwrap_or_default();
        }
        let Some(owner) = self.owner_of(lt) else {
            return Vec::new();
        };
        let owner_args = self.owner_args(lt);
        let mut found = Vec::new();
        for i in &self.program.impls {
            if i.interface == interface
                && i.ty == owner
                && let Some(&rhs) = self.program.types.list(i.args).first()
            {
                found.push(self.program.types.subst(rhs, &owner_args));
            }
        }
        found
    }

    /// The left sides whose implementations of an operator take `rt` on
    /// the right, of the built-in types: what a number written on the left
    /// of a type of a program's own is, `2.0 * v`.
    pub(super) fn operator_lhs_builtins(&mut self, which: KnownInterface, rt: Ty) -> Vec<Ty> {
        let Some(interface) = self.program.prelude_items.interface(which) else {
            return Vec::new();
        };
        let owners: Vec<TypeDef> = self
            .program
            .impls
            .iter()
            .filter(|i| {
                i.interface == interface
                    && matches!(i.ty, TypeDef::Builtin(_))
                    && self.program.types.list(i.args).first() == Some(&rt)
            })
            .map(|i| i.ty)
            .collect();
        owners
            .into_iter()
            .map(|owner| self.self_ty(owner))
            .collect()
    }

    /// `a op b` as the call of the implementation `operator_impl` found,
    /// with both sides lent.
    pub(super) fn operator_call(
        &mut self,
        found: OperatorImpl,
        l: ExprId,
        r: ExprId,
        span: Span,
    ) -> ExprId {
        let (lt, rt) = (self.ty_of(l), found.rhs);
        let left = self.intern(TyKind::Ref(lt, crate::RefKind::Shared));
        let right = self.intern(TyKind::Ref(rt, crate::RefKind::Shared));
        let receiver = self.alloc(ExprKind::Ref(l), left, span);
        let other = self.alloc(ExprKind::Ref(r), right, span);
        self.alloc(
            ExprKind::Call {
                callee: found.method,
                args: vec![receiver, other],
                type_args: found.type_args,
                order: vec![0, 1],
            },
            found.out,
            span,
        )
    }

    /// `a < b` as what it asks: `a.compare(&b)`, and then which answer that
    /// operator wants.
    fn ordering(
        &mut self,
        op: BinaryOp,
        compare: FnId,
        l: ExprId,
        r: ExprId,
        span: Span,
    ) -> ExprId {
        let Some(ordering) = self.program.prelude_items.enumeration(KnownEnum::Ordering) else {
            return self.error_expr(span);
        };
        let ty = self.ty_of(l);
        let answer_ty = self.intern(TyKind::Enum(ordering, crate::TyList::EMPTY));
        let call = self.compare_call(compare, ty, answer_ty, l, r, span);
        // `.Less` is the first variant of `Ordering`, `.Same` the second
        // and `.More` the third.
        let (variant, negated) = match op {
            BinaryOp::Lt => (0, false),
            BinaryOp::Ge => (0, true),
            BinaryOp::Gt => (2, false),
            _ => (2, true),
        };
        let test = self.alloc(
            ExprKind::Is {
                scrutinee: call,
                pattern: Pattern::Variant {
                    variant,
                    binders: Vec::new(),
                },
            },
            Types::BOOL,
            span,
        );
        if negated {
            return self.alloc(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    operand: test,
                },
                Types::BOOL,
                span,
            );
        }
        test
    }

    /// The call of a generated or written `compare`, with both sides lent.
    pub(super) fn compare_call(
        &mut self,
        compare: FnId,
        ty: Ty,
        answer: Ty,
        l: ExprId,
        r: ExprId,
        span: Span,
    ) -> ExprId {
        let other = self.intern(TyKind::Ref(ty, crate::RefKind::Shared));
        let receiver = self.alloc(ExprKind::Ref(l), other, span);
        let borrowed = self.alloc(ExprKind::Ref(r), other, span);
        let type_args = match self.program.fns[compare].interface {
            Some(_) => self.program.types.intern_list(&[ty]),
            None => {
                let args = self.owner_args(ty);
                self.program.types.intern_list(&args)
            }
        };
        self.alloc(
            ExprKind::Call {
                callee: compare,
                args: vec![receiver, borrowed],
                type_args,
                order: vec![0, 1],
            },
            answer,
            span,
        )
    }

    /// `a == b` as the call it stands for, and `a != b` as its negation.
    fn equality(&mut self, op: BinaryOp, equals: FnId, l: ExprId, r: ExprId, span: Span) -> ExprId {
        let ty = self.ty_of(l);
        let other = self.intern(TyKind::Ref(ty, crate::RefKind::Shared));
        // Both sides are lent for the call, as `a.equals(&b)` lends them.
        let receiver = self.alloc(ExprKind::Ref(l), other, span);
        let borrowed = self.alloc(ExprKind::Ref(r), other, span);
        // An interface's method is generic in `Self`, which is this type;
        // a method of a generic type is generic in the type's own
        // parameters.
        let type_args = match self.program.fns[equals].interface {
            Some(_) => self.program.types.intern_list(&[ty]),
            None => {
                let args = self.owner_args(ty);
                self.program.types.intern_list(&args)
            }
        };
        let call = self.alloc(
            ExprKind::Call {
                callee: equals,
                args: vec![receiver, borrowed],
                type_args,
                order: vec![0, 1],
            },
            Types::BOOL,
            span,
        );
        match op {
            BinaryOp::Ne => self.alloc(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    operand: call,
                },
                Types::BOOL,
                span,
            ),
            _ => call,
        }
    }

    /// Reports operands of `op`, written `text`, that it does not apply to:
    /// of different types, or of a type it does not take.
    pub(super) fn report_operands(
        &mut self,
        op: BinaryOp,
        op_span: Span,
        text: &str,
        l: ExprId,
        r: ExprId,
    ) {
        use BinaryOp::*;
        let (lt, rt) = (self.ty_of(l), self.ty_of(r));
        let allowed = self.operator_applies(op, lt);
        let (lspan, rspan) = (self.state.body.exprs[l].span, self.state.body.exprs[r].span);
        let declared = |lowerer: &Self, ty: Ty| {
            matches!(lowerer.kind(ty), TyKind::Struct(..) | TyKind::Enum(..))
        };
        let mut diagnostic = if lt != rt && (declared(self, lt) || declared(self, rt)) {
            // A type of a program's own on either side: no implementation
            // takes the two.
            Diagnostic::error(
                codes::INVALID_OPERANDS,
                format!(
                    "cannot apply `{text}` to {} and {}",
                    self.ty_name(lt),
                    self.ty_name(rt)
                ),
                op_span,
                "no implementation takes these two",
            )
        } else if lt != rt && allowed {
            let diagnostic = Diagnostic::error(
                codes::INVALID_OPERANDS,
                format!(
                    "cannot apply `{text}` to {} and {}",
                    self.ty_name(lt),
                    self.ty_name(rt)
                ),
                op_span,
                "operands of different types",
            )
            .with_note("Wip has no implicit numeric conversions");
            // An integer literal next to a float was almost certainly meant
            // as a float literal.
            let is_int_literal =
                |id: ExprId| matches!(self.state.body.exprs[id].kind, ExprKind::Int(_));
            let types = &self.program.types;
            if types.is_float(lt) && is_int_literal(r) {
                diagnostic.with_fix("use a float literal", [Edit::insert(rspan.hi, ".0")])
            } else if types.is_float(rt) && is_int_literal(l) {
                diagnostic.with_fix("use a float literal", [Edit::insert(lspan.hi, ".0")])
            } else {
                diagnostic
            }
        } else {
            let what = match op {
                Rem => "`%` needs integer operands",
                BitAnd | BitOr | BitXor | Shl | Shr => "bitwise operators need integer operands",
                Eq | Ne => "`==` and `!=` compare numbers or `bool`",
                _ => "arithmetic and ordering need numbers",
            };
            Diagnostic::error(
                codes::INVALID_OPERANDS,
                format!("cannot apply `{text}` to {}", self.ty_name(lt)),
                op_span,
                what,
            )
        };
        diagnostic = diagnostic
            .with_secondary(lspan, self.ty_name(lt))
            .with_secondary(rspan, self.ty_name(rt));
        // A type of a program's own answers an arithmetic operator by
        // implementing its interface, with the type on the
        // other side its argument where it is another.
        if let Some(interface) = KnownInterface::of_binary(op) {
            let verb = match op {
                Add => "adds",
                Sub => "subtracts",
                Mul => "multiplies",
                Div => "divides",
                _ => "answers `%`",
            };
            let declared = |lowerer: &Self, ty: Ty| {
                matches!(lowerer.kind(ty), TyKind::Struct(..) | TyKind::Enum(..))
            };
            if lt == rt {
                diagnostic = self.operator_hint(diagnostic, lt, interface.name(), verb);
            } else if declared(self, lt) {
                let other = self.ty_name(rt).trim_matches('`').to_string();
                let written = format!("{}<{other}>", interface.name());
                diagnostic = self.operator_hint(diagnostic, lt, &written, verb);
            } else if declared(self, rt) {
                let (number, other) = (
                    self.ty_name(lt).trim_matches('`').to_string(),
                    self.ty_name(rt).trim_matches('`').to_string(),
                );
                diagnostic = diagnostic.with_help(format!(
                    "a number {verb} a `{other}` by `extend {number}: {}<{other}, …>`, written in the module of `{other}`",
                    interface.name()
                ));
            }
        }
        // On `bool`, `&` and `|` were almost certainly meant as `&&` and `||`.
        if lt == Types::BOOL && rt == Types::BOOL && text == op.text() {
            diagnostic = match op {
                BitAnd => diagnostic.with_fix(
                    "for logical and, write `&&`",
                    [Edit::replace(op_span, "&&")],
                ),
                BitOr => diagnostic
                    .with_fix("for logical or, write `||`", [Edit::replace(op_span, "||")]),
                BitXor => diagnostic.with_help("for `bool`, `!=` is exclusive or"),
                _ => diagnostic,
            };
        }
        // A type that does implement the interface, but only for type
        // arguments that do themselves, is told which argument does not:
        // `Option<T: Eq>` compares where `T` does.
        let wanted = match op {
            Eq | Ne => self.program.prelude_items.interface(KnownInterface::Eq),
            Lt | Le | Gt | Ge => self.program.prelude_items.interface(KnownInterface::Ord),
            _ => None,
        };
        let unmet = wanted.filter(|_| lt == rt).and_then(|interface| {
            let wanted = crate::Constraint {
                interface,
                args: crate::TyList::EMPTY,
            };
            let chain = self.unmet_chain(lt, wanted);
            (!chain.is_empty()).then_some((wanted, chain))
        });
        // A type says it compares with `@derive(Eq)` and that it is ordered
        // with `@derive(Ord)`, and the compiler writes both.
        let declared = matches!(self.kind(lt), TyKind::Struct(..) | TyKind::Enum(..));
        if let Some((wanted, chain)) = unmet {
            diagnostic = diagnostic.with_help(self.unmet_help(lt, wanted, &chain));
            // The type that lacks it is the innermost, which is where it is
            // written, if the program may write it.
            let (arg, condition) = chain[chain.len() - 1];
            let name = self.constraint_name(condition, arg);
            let inside = self.ty_name(arg).trim_matches('`').to_string();
            if let Some(module) = self.std_module_of(arg) {
                let module = if module == crate::PRELUDE {
                    "the prelude".to_string()
                } else {
                    format!("`{module}`")
                };
                diagnostic = diagnostic.with_help(format!(
                    "`{inside}` is declared in {module}, which does not implement `{name}` for it"
                ));
            } else if matches!(self.kind(arg), TyKind::Struct(..) | TyKind::Enum(..)) {
                diagnostic = diagnostic.with_help(format!(
                    "write `@derive({name})` on `{inside}`, and the compiler writes it field by field"
                ));
            }
            diagnostic = diagnostic.with_note(
                "an implementation may hold only for some type arguments: `extend Option<T: Eq>: Eq`",
            );
        } else if matches!(op, Eq | Ne) && declared {
            let name = self.ty_name(lt).trim_matches('`').to_string();
            diagnostic = diagnostic
                .with_help(format!(
                    "write `@derive(Eq)` on `{name}`, and the compiler writes the comparison field by field"
                ))
                .with_note(
                    "a type whose equality is not its fields writes `extend …: Eq` itself instead",
                );
        } else if matches!(op, Lt | Le | Gt | Ge) && declared && lt == rt {
            let name = self.ty_name(lt).trim_matches('`').to_string();
            diagnostic = diagnostic
                .with_help(format!(
                    "write `@derive(Eq, Ord)` on `{name}`, and the compiler writes the order field by field"
                ))
                .with_note(
                    "a type whose order is not its fields writes `extend …: Ord` itself instead",
                );
        } else if matches!(op, Eq | Ne) && matches!(self.kind(lt), TyKind::Array(..)) {
            diagnostic = diagnostic.with_help("compare the elements one by one");
        }
        self.report(diagnostic);
    }

    /// Whether `op` applies to operands of type `ty`.
    pub(super) fn operator_applies(&self, op: BinaryOp, ty: Ty) -> bool {
        use BinaryOp::*;
        let types = &self.program.types;
        match op {
            Rem | BitAnd | BitOr | BitXor | Shl | Shr => types.is_integer(ty),
            Add | Sub | Mul | Div => types.is_numeric(ty),
            // Characters compare as the numbers they are.
            Lt | Le | Gt | Ge => types.is_numeric(ty) || ty == Types::CHAR,
            // Two pointers are equal where they point at the same place.
            Eq | Ne => {
                types.is_numeric(ty)
                    || ty == Types::BOOL
                    || ty == Types::CHAR
                    || matches!(types.kind(ty), TyKind::Ptr(_))
            }
            And | Or => false,
        }
    }

    /// `expr as T`: a conversion between `i64`, `i32` and `f64`.
    /// A field of a type that cleans up after itself may not be moved out:
    /// its drop is about to run over the hole.
    fn no_move_out_of_drop(&mut self, place: ExprId, span: Span) {
        let mut at = place;
        loop {
            let base = match self.state.body.exprs[at].kind {
                ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => base,
                ExprKind::Deref(inner) => inner,
                _ => return,
            };
            let ty = self.under_refs(self.ty_of(base));
            if self.program.has_drop(ty) {
                let name = self.ty_name(ty);
                let diagnostic = Diagnostic::error(
                    codes::DROP_RULES,
                    format!("cannot move out of {name}, which cleans up after itself"),
                    span,
                    "a part of it",
                )
                .with_note(
                    "its `destroy` runs over the whole value, so a part that was taken out would be ended twice or not at all",
                )
                .with_help("move the whole value, or give the type a method that hands out what it holds");
                self.report(diagnostic);
                return;
            }
            at = base;
        }
    }

    pub(super) fn cast(&mut self, expr: ast::ExprId, ty: ast::TypeId, span: Span) -> ExprId {
        let target = self.resolve_ty(ty);
        // `null as ptr<T>` is a pointer of the type it is cast to.
        let hint = (matches!(self.ast.exprs[expr].kind, ast::ExprKind::Null)
            && matches!(self.kind(target), TyKind::Ptr(_)))
        .then_some(target);
        let e = self.operand(expr, hint);
        let from = self.ty_of(e);
        if self.is_poisoned(from) || self.has_error(target) {
            return self.error_expr(span);
        }
        let types = &self.program.types;
        if types.is_numeric(from) && types.is_numeric(target) {
            return self.alloc(ExprKind::Cast(e), target, span);
        }
        // A character is its number, which fits any integer type that holds
        // it; of the numbers, only a byte is always a character.
        if from == Types::CHAR && types.is_integer(target) {
            return self.alloc(ExprKind::Cast(e), target, span);
        }
        if target == Types::CHAR && from == Types::U8 {
            return self.alloc(ExprKind::Cast(e), target, span);
        }
        if target == Types::CHAR && types.is_integer(from) {
            let diagnostic = Diagnostic::error(
                codes::INVALID_CAST,
                format!("cannot convert {} to `char`", self.ty_name(from)),
                span,
                "not every number is a character",
            )
            .with_note("a character is a Unicode scalar value: up to 10FFFF, and not a surrogate; only a `u8` is always one")
            .with_help("`char::fromInt(n)` answers the character, or nothing where there is none");
            self.report(diagnostic);
            return self.error_expr(span);
        }
        // An enum whose variants carry nothing is a number: which variant
        // it holds.
        if let TyKind::Enum(id, _) = self.kind(from)
            && types.is_integer(target)
        {
            if self.program.enums[id]
                .variants
                .iter()
                .all(|v| v.fields.is_empty())
            {
                return self.alloc(ExprKind::VariantIndex(e), target, span);
            }
            let name = self.ty_name(from).to_string();
            let diagnostic = Diagnostic::error(
                codes::INVALID_CAST,
                format!("{name} carries values, so it is not a number"),
                span,
                "not an enum of names alone",
            )
            .with_note(
                "an enum whose variants carry nothing is which one it holds, and one that carries values is that and what it holds",
            );
            self.report(diagnostic);
            return self.error_expr(span);
        }
        // One C pointer as another: what C's casts between pointer types do.
        // The bits are the same, so nothing is converted.
        if matches!(self.kind(from), TyKind::Ptr(_)) && matches!(self.kind(target), TyKind::Ptr(_))
        {
            return self.alloc(ExprKind::Cast(e), target, span);
        }
        // A pointer as the integer that is its address, and back: what C's
        // handles are passed as, and compared or hashed by. Only an integer
        // as wide as a pointer holds one.
        let pointer = |ty: Ty| matches!(self.kind(ty), TyKind::Ptr(_));
        let address = |ty: Ty| [Types::USIZE, Types::ISIZE, Types::U64, Types::I64].contains(&ty);
        if (pointer(from) && address(target)) || (address(from) && pointer(target)) {
            return self.alloc(ExprKind::Cast(e), target, span);
        }
        let mut diagnostic = Diagnostic::error(
            codes::INVALID_CAST,
            format!(
                "cannot convert {} to {}",
                self.ty_name(from),
                self.ty_name(target)
            ),
            span,
            "not a numeric conversion",
        )
        .with_note(
            "`as` converts between the integer types, `f32` and `f64`, between C pointer types, and between a pointer and an integer as wide as one, `usize`, `isize`, `u64` or `i64`",
        );
        // A number is not a variant on its own: an enum has the numbers of
        // its own variants and no others.
        if let TyKind::Enum(id, _) = self.kind(target)
            && self.program.types.is_integer(from)
            && self.program.enums[id]
                .variants
                .iter()
                .all(|v| v.fields.is_empty())
        {
            let name = self.type_name(TypeDef::Enum(id)).to_string();
            diagnostic = diagnostic.with_help(format!(
                "`{name}::fromIndex(n)` answers the variant with that number, or nothing where `{name}` has no such variant"
            ));
        }
        self.report(diagnostic);
        self.error_expr(span)
    }
}
