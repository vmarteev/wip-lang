//! Checking an expression against an expected type, and the coercions.

use super::*;

impl<'a> Lowerer<'a> {
    pub(super) fn check(&mut self, e: ast::ExprId, expected: Ty) -> ExprId {
        self.check_in(e, expected, None)
    }

    /// Checks `e` against `expected`; `context` labels where the expectation
    /// comes from, for the mismatch diagnostic.
    pub(super) fn check_in(
        &mut self,
        e: ast::ExprId,
        expected: Ty,
        context: Option<(Span, &str)>,
    ) -> ExprId {
        let id = self.infer(e, Some(expected));
        self.coerce(id, expected, context)
    }

    /// An operand of an operator or a condition: references and `own` are
    /// looked through.
    pub(super) fn operand(&mut self, e: ast::ExprId, hint: Option<Ty>) -> ExprId {
        let id = self.infer(e, hint);
        self.autoderef(id)
    }

    pub(super) fn check_operand(&mut self, e: ast::ExprId, expected: Ty) -> ExprId {
        let id = self.operand(e, Some(expected));
        self.coerce(id, expected, None)
    }

    pub(super) fn condition(&mut self, e: ast::ExprId) -> ExprId {
        self.check_operand(e, Types::BOOL)
    }

    pub(super) fn autoderef(&mut self, mut id: ExprId) -> ExprId {
        loop {
            let expr = &self.state.body.exprs[id];
            let (TyKind::Ref(inner, _) | TyKind::Own(inner)) = self.kind(expr.ty) else {
                return id;
            };
            let span = expr.span;
            id = self.alloc(ExprKind::Deref(id), inner, span);
        }
    }

    pub(super) fn coerce(
        &mut self,
        id: ExprId,
        expected: Ty,
        context: Option<(Span, &str)>,
    ) -> ExprId {
        self.coerce_to(id, expected, context, false)
    }

    /// Checks a call's argument against the parameter's type. An argument
    /// may be written `&place`, and may be lent where a
    /// reference is taken or a `str` where a `String` is given,
    /// as no other expression may.
    pub(super) fn check_argument(
        &mut self,
        e: ast::ExprId,
        expected: Ty,
        context: Option<(Span, &str)>,
    ) -> ExprId {
        // An index handed to `at` is any integer, as `x[i]` takes one.
        if expected == Types::I64 && self.state.index_operands.contains(&e) {
            return self.index_value(e);
        }
        let id = self.infer_argument(e, Some(expected));
        self.coerce_to(id, expected, context, true)
    }

    /// A call's argument, where what it is taken as is not known yet: an
    /// `&place` is the argument's own.
    pub(super) fn infer_argument(&mut self, e: ast::ExprId, hint: Option<Ty>) -> ExprId {
        let expr = &self.ast.exprs[e];
        if let ast::ExprKind::Unary {
            op: op @ (ast::UnaryOp::Ref | ast::UnaryOp::RefVar),
            operand,
            ..
        } = expr.kind
        {
            return self.reference(op, operand, hint, expr.span, true);
        }
        if let ast::ExprKind::Block(block) = &expr.kind {
            let (block, ty) = self.argument_block(block, hint);
            return self.alloc(ExprKind::Block(block), ty, expr.span);
        }
        self.infer(e, hint)
    }

    /// [`Self::coerce`], where `argument` says the expression is a call's
    /// argument itself, which may be lent as nothing else may.
    pub(super) fn coerce_to(
        &mut self,
        id: ExprId,
        expected: Ty,
        context: Option<(Span, &str)>,
        argument: bool,
    ) -> ExprId {
        let actual = self.state.body.exprs[id].ty;
        if actual == expected || self.is_poisoned(actual) || self.has_error(expected) {
            return id;
        }
        // Where nothing is expected, a value is discarded.
        if expected == Types::UNIT {
            return self.discard(id);
        }
        if let Some(coerced) = self.deref_coercion(id, expected) {
            return coerced;
        }
        if let Some(coerced) = self.string_argument(id, expected, argument) {
            return coerced;
        }
        if let Some(copied) = self.text_copied(id, expected) {
            return copied;
        }
        if let Some(coerced) = self.container_argument(id, expected, argument) {
            return coerced;
        }
        // A named function is lent as a closure.
        if let Some(closure) = self.fn_as_closure(id, expected) {
            return closure;
        }
        // A value with no place of its own is lent where it is used, so a
        // call takes it without an `&`.
        if let Some(lent) = self.lent_temporary(id, expected, argument) {
            return self.coerce_to(lent, expected, context, argument);
        }
        // A reference becomes a `&dyn Interface`, with the table of the
        // type's methods beside it.
        if let TyKind::Ref(target, kind) = self.kind(expected)
            && let TyKind::Dyn(interface, dyn_args) = self.kind(target)
            && let TyKind::Ref(mut pointee, actual_kind) = self.kind(actual)
            && kind == actual_kind
        {
            let span = self.state.body.exprs[id].span;
            // A reference to a `&Node` is read through to the node, whose
            // implementation the table holds.
            let mut id = id;
            while let Some(inner) = self.program.referred(pointee) {
                id = self.alloc(ExprKind::Deref(id), pointee, span);
                pointee = inner;
            }
            let wanted = crate::Constraint {
                interface,
                args: dyn_args,
            };
            if !self.implements_with(pointee, wanted) {
                let name = self.constraint_name(wanted, pointee);
                let diagnostic = Diagnostic::error(
                    codes::UNSATISFIED_CONSTRAINT,
                    format!("{} does not implement `{name}`", self.ty_name(pointee)),
                    span,
                    format!("`&dyn {name}` expected here"),
                )
                .with_note(format!(
                    "a type implements an interface by `extend …: {name}`"
                ));
                self.report(diagnostic);
                return self.error_expr(span);
            }
            return self.alloc(
                ExprKind::DynRef {
                    value: id,
                    interface,
                },
                expected,
                span,
            );
        }
        // An array on the heap becomes a buffer: its length moves from the
        // type into the value.
        if let TyKind::Own(target) = self.kind(expected)
            && let TyKind::Slice(elem) = self.kind(target)
            && let TyKind::Own(source) = self.kind(actual)
            && matches!(self.kind(source), TyKind::Array(e, _) if e == elem)
        {
            let span = self.state.body.exprs[id].span;
            return self.alloc(ExprKind::Unsize(id), expected, span);
        }
        self.mismatch(id, expected, context);
        id
    }

    /// A temporary passed where a `&` is expected is lent for the call: it
    /// has no place of its own to write an `&` in front of, and `move` is
    /// ruled the same way — writing it on a temporary is E0317.
    /// A closure keeps its `&`, which is what tells a lent one from an
    /// owned one.
    fn lent_temporary(&mut self, id: ExprId, expected: Ty, argument: bool) -> Option<ExprId> {
        let TyKind::Ref(pointee, crate::RefKind::Shared) = self.kind(expected) else {
            return None;
        };
        let actual = self.state.body.exprs[id].ty;
        // A `&Node` where a `&&Node` is taken is lent as any temporary is:
        // `refs.contains(&a)` of a `Vec<&Node>`.
        let lent_ref = pointee == actual
            && matches!(self.kind(actual), TyKind::Ref(inner, crate::RefKind::Shared)
                if !matches!(self.kind(inner), TyKind::Fn(..)));
        if !lent_ref
            && matches!(
                self.kind(actual),
                TyKind::Ref(..) | TyKind::Fn(..) | TyKind::Dyn(..)
            )
        {
            return None;
        }
        if let TyKind::Own(inner) = self.kind(actual)
            && matches!(self.kind(inner), TyKind::Fn(..))
        {
            return None;
        }
        // The expression is the argument itself, as the `String` rule asks.
        if !argument {
            return None;
        }
        let span = self.state.body.exprs[id].span;
        if !matches!(self.place_root(id), PlaceRoot::NotAPlace) {
            return None;
        }
        let ty = self.intern(TyKind::Ref(actual, crate::RefKind::Shared));
        Some(self.alloc(ExprKind::Ref(id), ty, span))
    }

    /// A `String` passed where a `str` is expected lends its bytes for the
    /// call: `f(text)` is `f(text.toStr())`. It is an
    /// argument's coercion alone, since that is where what is lent cannot
    /// outlive the lending — the rule references keep. A
    /// `String` stored in a `str` would outlive its bytes, and stays the
    /// mismatch it is. Where a `&str` is expected, the lent text is a
    /// temporary, lent for the call as any is.
    fn string_argument(&mut self, id: ExprId, expected: Ty, argument: bool) -> Option<ExprId> {
        // The expression is the argument itself, not something inside it.
        if !argument {
            return None;
        }
        if expected == Types::STR {
            return self.lend_string(id);
        }
        if let Some(lent) = self.text_as_string(id, expected) {
            return Some(lent);
        }
        if self.kind(expected) != TyKind::Ref(Types::STR, crate::RefKind::Shared)
            || !matches!(self.kind(self.ty_of(id)), TyKind::Struct(..))
        {
            return None;
        }
        let lent = self.lend_string(id)?;
        let span = self.state.body.exprs[id].span;
        Some(self.alloc(ExprKind::Ref(lent), expected, span))
    }

    /// Text where a `String` is expected is copied into one, by
    /// `String::of`, which allocates as `"\(x)"` does: `names.push(word)`
    /// is `names.push(String::of(word))`. Where the `String` is only read,
    /// a `&String`, the text is lent instead ([`Self::text_as_string`]).
    fn text_copied(&mut self, id: ExprId, expected: Ty) -> Option<ExprId> {
        if self.ty_of(id) != Types::STR || !self.program.is_string(expected) {
            return None;
        }
        let of = self.program.prelude_items.function(KnownFn::StringOf)?;
        let span = self.state.body.exprs[id].span;
        Some(self.alloc(
            ExprKind::Call {
                callee: of,
                args: vec![id],
                type_args: crate::TyList::EMPTY,
                order: vec![0],
            },
            expected,
            span,
        ))
    }

    /// Text given where a `&String` is taken is lent as a `String` whose
    /// bytes are its own: `f("the")` is `f(&String::over("the"))`, which
    /// allocates nothing and is never dropped. Behind a `&`, nothing can
    /// grow it or free it. Not where a `&var String` is taken, which may
    /// grow it, nor a `String` by value, which is the copy asked for.
    fn text_as_string(&mut self, id: ExprId, expected: Ty) -> Option<ExprId> {
        let TyKind::Ref(string, crate::RefKind::Shared) = self.kind(expected) else {
            return None;
        };
        if self.ty_of(id) != Types::STR || !self.program.is_string(string) {
            return None;
        }
        let over = self.program.prelude_items.function(KnownFn::StringOver)?;
        let span = self.state.body.exprs[id].span;
        let made = self.alloc(
            ExprKind::Call {
                callee: over,
                args: vec![id],
                type_args: crate::TyList::EMPTY,
                order: vec![0],
            },
            string,
            span,
        );
        let kept = self.alloc(ExprKind::Undropped(made), string, span);
        Some(self.alloc(ExprKind::Ref(kept), expected, span))
    }

    /// A container given where a call expects a slice is lent as the elements
    /// it lends: `f(&v)` is `f(&v.items())`, and `f(&var v)` lends them to be
    /// written, through the writing half of the pair. An argument's coercion
    /// alone, as a `String`'s is, since what is lent lives as long as the call.
    fn container_argument(&mut self, id: ExprId, expected: Ty, argument: bool) -> Option<ExprId> {
        if !argument {
            return None;
        }
        let TyKind::Ref(slice, kind) = self.kind(expected) else {
            return None;
        };
        let TyKind::Slice(elem) = self.kind(slice) else {
            return None;
        };
        // `&c`, or a reference parameter, which stands for what it refers
        // to where its own type is not what is expected.
        // And a temporary, which is lent where it is used, as any is.
        let (reference, container, given) = match self.kind(self.ty_of(id)) {
            TyKind::Ref(container, given) => (id, container, given),
            actual => match self.state.body.exprs[id].kind {
                ExprKind::Deref(r) => match self.kind(self.ty_of(r)) {
                    TyKind::Ref(container, given) => (r, container, given),
                    _ => return None,
                },
                _ if matches!(self.place_root(id), PlaceRoot::NotAPlace)
                    && !matches!(actual, TyKind::Ref(..) | TyKind::Fn(..)) =>
                {
                    (id, self.ty_of(id), crate::RefKind::Shared)
                }
                _ => return None,
            },
        };
        // A reference to read with is not lent as one to write with, and
        // an array or a slice has its own coercions.
        if (kind == crate::RefKind::Var && given != crate::RefKind::Var)
            || matches!(
                self.kind(container),
                TyKind::Array(..) | TyKind::Slice(_) | TyKind::Own(_)
            )
            || self.items_element(container) != Some(elem)
        {
            return None;
        }
        // What lends: the container `&c` was written of, or what a
        // reference refers to.
        let span = self.state.body.exprs[id].span;
        let receiver = match self.state.body.exprs[reference].kind {
            ExprKind::Ref(inner) => inner,
            _ if reference != id || container == self.ty_of(id) => id,
            _ => self.alloc(ExprKind::Deref(id), container, span),
        };
        let super::body::Walked::Items(place, _) = self.items_of(receiver, container) else {
            return None;
        };
        // A container that cannot be written was reported where `&var` was
        // written of it; it is lent all the same, so that it is reported
        // once.
        if kind == crate::RefKind::Var {
            self.writable_for_writing(place);
        }
        Some(self.alloc(ExprKind::Ref(place), expected, span))
    }

    /// Whether the type is the prelude's `String`, which lends its bytes as
    /// a `str`.
    pub(super) fn lends_text(&self, ty: Ty) -> bool {
        let Some(to_str) = self.program.prelude_items.function(KnownFn::StringToStr) else {
            return false;
        };
        let Some(owner) = self.program.owner_of_fn(to_str) else {
            return false;
        };
        self.owner_of(ty) == Some(owner)
    }

    /// The value, read as the text it holds, when it is a `String`:
    /// `text` becomes `text.toStr()`. What comes back
    /// borrows the `String`, so it is only ever put where the borrow cannot
    /// outlive the expression around it — an argument, a receiver, an index
    /// or a comparison.
    pub(super) fn lend_string(&mut self, id: ExprId) -> Option<ExprId> {
        let to_str = self.program.prelude_items.function(KnownFn::StringToStr)?;
        let owner = self.program.owner_of_fn(to_str)?;
        let ty = self.ty_of(id);
        if self.owner_of(ty) != Some(owner) {
            return None;
        }
        let span = self.state.body.exprs[id].span;
        let ref_ty = self.intern(TyKind::Ref(ty, crate::RefKind::Shared));
        let receiver = self.alloc(ExprKind::Ref(id), ref_ty, span);
        Some(self.alloc(
            ExprKind::Call {
                callee: to_str,
                args: vec![receiver],
                type_args: crate::TyList::EMPTY,
                order: vec![0],
            },
            Types::STR,
            span,
        ))
    }

    /// `&own<T>`, through any number of `own`s, coerces to `&T`, and
    /// `&var own<T>` to `&var T`.
    ///
    /// An array `[T; N]`, directly or through `own`s, coerces to the slice
    /// `[T]` of all its elements the same way.
    pub(super) fn deref_coercion(&mut self, id: ExprId, expected: Ty) -> Option<ExprId> {
        let TyKind::Ref(target, kind) = self.kind(expected) else {
            return None;
        };
        let TyKind::Ref(pointee, actual_kind) = self.kind(self.state.body.exprs[id].ty) else {
            return None;
        };
        if kind != actual_kind {
            return None;
        }
        let slice_elem = match self.kind(target) {
            TyKind::Slice(elem) => Some(elem),
            _ => None,
        };
        // How many `own`s to look through, and whether what is found there
        // is an array to view as a slice.
        let mut ty = pointee;
        let mut depth = 0;
        let unsize = loop {
            if matches!(self.kind(ty), TyKind::Array(elem, _) if Some(elem) == slice_elem) {
                break true;
            }
            if depth > 0 && ty == target {
                break false;
            }
            let TyKind::Own(inner) = self.kind(ty) else {
                return None;
            };
            ty = inner;
            depth += 1;
        };
        let span = self.state.body.exprs[id].span;
        let mut e = self.alloc(ExprKind::Deref(id), pointee, span);
        let mut ty = pointee;
        for _ in 0..depth {
            let TyKind::Own(inner) = self.kind(ty) else {
                unreachable!()
            };
            ty = inner;
            e = self.alloc(ExprKind::Deref(e), ty, span);
        }
        if unsize {
            let whole = ExprKind::SubSlice {
                base: e,
                lo: None,
                hi: None,
            };
            e = self.alloc(whole, target, span);
        }
        Some(self.alloc(ExprKind::Ref(e), expected, span))
    }

    pub(super) fn mismatch(&mut self, id: ExprId, expected: Ty, context: Option<(Span, &str)>) {
        let expr = &self.state.body.exprs[id];
        let (actual, span) = (expr.ty, expr.span);
        let mut diagnostic = Diagnostic::error(
            codes::MISMATCHED_TYPES,
            "mismatched types",
            span,
            format!(
                "expected {}, found {}",
                self.ty_name(expected),
                self.ty_name(actual)
            ),
        );
        if let Some((context_span, label)) = context {
            diagnostic = diagnostic.with_secondary(context_span, label);
        }
        if self.program.types.is_float(expected) && matches!(expr.kind, ExprKind::Int(_)) {
            diagnostic = diagnostic.with_fix("use a float literal", [Edit::insert(span.hi, ".0")]);
        } else if let TyKind::Ref(want, _) = self.kind(expected)
            && want == actual
            && matches!(self.kind(actual), TyKind::Fn(..))
        {
            // A function value where a closure is expected. There is no way to
            // write `&` on it: a closure is a pair, and a lambda makes one.
            diagnostic = diagnostic
                .with_note(
                    "a closure is an environment and its code; a function value is code alone",
                )
                .with_help("wrap it in a lambda that calls it, as in `(x) => f(x)`");
        } else if expected == Types::CSTRING && self.lends_text(actual) {
            // A `String` holds its bytes, and can write the NUL C wants
            // after them.
            diagnostic = diagnostic
                .with_note(
                    "a `cstring` is a pointer to bytes that end with a NUL, and a `String` holds bytes with nothing after them",
                )
                .with_fix(
                    "write the NUL with `toCstring()`",
                    [Edit::insert(span.hi, ".toCstring()")],
                );
        } else if expected == Types::CSTRING && actual == Types::STR {
            // Text that C can take must end with a NUL, and a `str` is a
            // pointer and a length with nothing after it.
            // Nothing converts one silently: it would have to allocate, and
            // C may keep the pointer.
            diagnostic = diagnostic
                .with_note(
                    "a `cstring` is a pointer to bytes that end with a NUL; a `str` is a pointer and a length, and has none",
                )
                .with_help(
                    "a literal is either, if its type is said: `val sql: cstring = \"select …\"`; text built at run time goes through `String::toCstring()`, which writes the NUL",
                );
        } else if (expected == Types::STR
            || self.kind(expected) == TyKind::Ref(Types::STR, crate::RefKind::Shared))
            && self
                .program
                .prelude_items
                .function(KnownFn::StringToStr)
                .is_some()
            && self.owner_of(actual)
                == self
                    .program
                    .prelude_items
                    .function(KnownFn::StringToStr)
                    .and_then(|f| self.program.owner_of_fn(f))
        {
            // A `String` where a `str` is expected, somewhere a call is not:
            // the bytes would outlive what holds them.
            diagnostic = diagnostic
                .with_note(
                    "a `String` owns its bytes and a `str` borrows them, so it is lent as a `str` for a call and no longer",
                )
                .with_help("call `toStr()` to say where the bytes are looked at, and keep the `String` alive for as long");
        } else if self.kind(expected) == TyKind::Ref(Types::STR, crate::RefKind::Var)
            && self.lends_text(actual)
        {
            // A `String` is lent as a `str` to read, never to write: the
            // `str` written would not be the `String`.
            diagnostic = diagnostic.with_note(
                "a `String` is lent as a `str` where its text is read; writing a `str` would not write the `String`",
            );
        } else if let TyKind::Ref(string, crate::RefKind::Var) = self.kind(expected)
            && self.program.is_string(string)
            && matches!(self.kind(actual), TyKind::Ref(text, _) if text == Types::STR)
        {
            // Text is copied where a `String` is kept and lent where one is
            // read; one that is written would be a copy the caller never sees.
            diagnostic = diagnostic.with_note(
                "text is copied into a `String` where one is kept, and lent as one where it is read; a `&var String` is written, and a copy would lose what is written",
            ).with_help("keep the text in a `String` to be written: `var text: String = …`");
        } else if let TyKind::Own(target) = self.kind(expected)
            && let TyKind::Slice(elem) = self.kind(target)
            && matches!(self.kind(actual), TyKind::Array(e, _) if e == elem)
            && matches!(expr.kind, ExprKind::Array(_))
        {
            // An array written where a list on the heap is expected, as
            // `Vec::of` takes one.
            diagnostic = diagnostic
                .with_fix("build it on the heap, with `own`", [Edit::insert(span.lo, "own ")])
                .with_note("an array is kept where it is written; `own [ … ]` builds the list on the heap, for what takes it to keep");
        } else if self.kind(expected) == TyKind::Ref(actual, crate::RefKind::Shared) {
            diagnostic = diagnostic.with_fix("borrow it with `&`", [Edit::insert(span.lo, "&")]);
        } else if self.kind(expected) == TyKind::Ref(actual, crate::RefKind::Var) {
            diagnostic =
                diagnostic.with_fix("pass a `&var` reference", [Edit::insert(span.lo, "&var ")]);
        } else if let TyKind::Ref(target, kind) = self.kind(expected)
            && let TyKind::Slice(elem) = self.kind(target)
            && matches!(self.kind(actual), TyKind::Array(e, _) if e == elem)
        {
            // An array where a slice of it is expected.
            let text = if kind == crate::RefKind::Var {
                "&var "
            } else {
                "&"
            };
            diagnostic = diagnostic.with_fix("pass it as a slice", [Edit::insert(span.lo, text)]);
        } else if let TyKind::Ref(target, kind) = self.kind(expected)
            && let TyKind::Slice(elem) = self.kind(target)
            && !matches!(self.kind(actual), TyKind::Ref(..))
            && self.items_element(actual) == Some(elem)
        {
            // A container where a slice of its elements is expected: a
            // call lends them, once it is borrowed.
            let text = if kind == crate::RefKind::Var {
                "&var "
            } else {
                "&"
            };
            diagnostic = diagnostic.with_fix(
                "borrow it, and its elements are lent",
                [Edit::insert(span.lo, text)],
            );
        } else if let TyKind::Ref(target, crate::RefKind::Var) = self.kind(expected)
            && let TyKind::Slice(elem) = self.kind(target)
            && let TyKind::Ref(have, crate::RefKind::Shared) = self.kind(actual)
            && self.items_element(have) == Some(elem)
            && let ExprKind::Ref(inner) = expr.kind
        {
            // `&c` where its elements are lent to be written.
            let between = Span::new(span.lo + 1, self.state.body.exprs[inner].span.lo);
            diagnostic =
                diagnostic.with_fix("pass a `&var` reference", [Edit::replace(between, "var ")]);
        } else if let (TyKind::Ref(want, want_kind), TyKind::Ref(have, have_kind)) =
            (self.kind(expected), self.kind(actual))
            && want == have
            && let ExprKind::Ref(inner) = expr.kind
        {
            // `&x` where `&var` is expected, or the other way round.
            let between = Span::new(span.lo + 1, self.state.body.exprs[inner].span.lo);
            let (label, text) = match (want_kind, have_kind) {
                (crate::RefKind::Var, _) => ("pass a `&var` reference", "var "),
                (_, _) => ("pass a `&` reference", ""),
            };
            diagnostic = diagnostic.with_fix(label, [Edit::replace(between, text)]);
        } else if let ExprKind::If {
            else_block: None, ..
        } = expr.kind
        {
            diagnostic = diagnostic
                .with_note("an `if` without `else` produces no value")
                .with_help(format!(
                    "add an `else` branch that produces {}",
                    self.ty_name(expected)
                ));
        } else if actual == Types::UNIT {
            diagnostic = diagnostic.with_note("this expression produces no value");
        }
        self.report(diagnostic);
    }

    pub(super) fn alloc(&mut self, kind: ExprKind, ty: Ty, span: Span) -> ExprId {
        self.state.body.exprs.alloc(Expr { kind, ty, span })
    }

    pub(super) fn error_expr(&mut self, span: Span) -> ExprId {
        self.alloc(ExprKind::Error, Types::ERROR, span)
    }

    /// Stands in for an expression that went wrong with an error expression,
    /// after checking the expressions inside it, so that their own mistakes
    /// are still reported.
    pub(super) fn give_up(
        &mut self,
        inside: impl IntoIterator<Item = ast::ExprId>,
        span: Span,
    ) -> ExprId {
        for e in inside {
            // Each is still the argument of a call, though the call is not
            // going to happen: a `&` in one is where it belongs.
            self.infer_argument(e, None);
        }
        self.error_expr(span)
    }

    pub(super) fn ty_of(&self, id: ExprId) -> Ty {
        self.state.body.exprs[id].ty
    }
}
