//! Calls and paths in expressions: functions of this module or another,
//! with or without type arguments.

use super::*;

impl<'a> Lowerer<'a> {
    pub(super) fn call_expr(
        &mut self,
        callee: ast::ExprId,
        args: &'a [ast::ExprId],
        names: &'a [Option<ast::Name>],
        rest: Option<ast::ExprId>,
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        let ast = self.ast;
        let callee_expr = &ast.exprs[callee];
        let used = ItemUse {
            names,
            ..ItemUse::plain(Some(args), hint, span)
        };
        // A struct, built as a variant is: by its name, or
        // `Self`, where no function or variable has that name.
        if let ast::ExprKind::Name(sym) = callee_expr.kind
            && self.names_struct(sym)
        {
            let path = [ast::Name {
                sym,
                span: callee_expr.span,
            }];
            let lit = StructLit {
                type_args: None,
                fields: aggregates::Written { args, names },
                rest,
                hint,
                span,
            };
            return self.struct_lit(&path, lit);
        }
        if let Some(rest) = rest
            && !matches!(callee_expr.kind, ast::ExprKind::Path { .. })
        {
            self.rest_without_struct(rest);
        }
        match callee_expr.kind {
            // A qualified name: a variant of an enum, or an item of another
            // module.
            ast::ExprKind::Path {
                leading_dot,
                ref segments,
                ref type_args,
            } => {
                let item = ItemUse {
                    type_args: type_args.as_ref(),
                    ..used
                };
                return self.path_call(leading_dot, segments, item, rest);
            }
            ast::ExprKind::Field { base, name } => {
                if let Some(variant) = self.variant_with_dot(base, name, used) {
                    return variant;
                }
                // A method of the receiver's type; otherwise a field of
                // function type, checked below.
                if let Some(call) = self.method_call(base, name, used) {
                    return call;
                }
            }
            // `panic(message)` does not return.
            ast::ExprKind::Name(sym) if self.lookup(sym).is_none() && self.text(sym) == "panic" => {
                return self.panic_call(args, names, span);
            }
            // A variable of the function around a lambda is called as the
            // value it is, below, whatever a function of the module is
            // called.
            ast::ExprKind::Name(sym) if self.lookup(sym).is_none() && !self.names_around(sym) => {
                if let Some(&f) = self.fns().get(&sym) {
                    return self.call(f, used);
                }
                // A function of another module, imported by name.
                let imported = self.imported(ast::Name {
                    sym,
                    span: callee_expr.span,
                });
                match imported {
                    Some(ImportedItem::Item(module, item))
                        if self.modules[module].fns.contains_key(&item.sym) =>
                    {
                        return self.module_fn(module, item, used);
                    }
                    Some(ImportedItem::Broken) => {
                        return self.give_up(args.iter().copied(), span);
                    }
                    _ => {}
                }
                if let Some(id) = self.prelude_fn(sym) {
                    return self.call(id, used);
                }
                // A closure a closure or a generator captured is called
                // as a value, below.
                if !self.types().contains_key(&sym) && imported.is_none() && !self.named_around(sym)
                {
                    let diagnostic = self.unknown_name(sym, callee_expr.span);
                    self.report(diagnostic);
                    return self.give_up(args.iter().copied(), span);
                }
            }
            _ => {}
        }
        let c = self.infer(callee, None);
        let ty = self.ty_of(c);
        // A closure is called like any other function value; its code takes
        // what it captured first.
        if let TyKind::Ref(inner, _) | TyKind::Own(inner) = self.kind(ty)
            && let TyKind::Fn(params, ret) = self.kind(inner)
        {
            if let Some(name) = names.iter().flatten().next() {
                let diagnostic = Diagnostic::error(
                    codes::NAMED_ARGUMENT_TO_VALUE,
                    "arguments to a closure cannot be named",
                    name.span,
                    "a named argument",
                )
                .with_note(
                    "the names in a function type document it, and are not part of the type",
                );
                self.report(diagnostic);
            }
            return self.call_closure(c, params, ret, args, span);
        }
        if let TyKind::Fn(params, ret) = self.kind(ty) {
            if let Some(name) = names.iter().flatten().next() {
                let diagnostic = Diagnostic::error(
                    codes::NAMED_ARGUMENT_TO_VALUE,
                    "arguments to a function value cannot be named",
                    name.span,
                    "a named argument",
                )
                .with_note("the names in a function type document it, and are not part of the type, so a value's names are not known")
                .with_help("pass the arguments by position");
                self.report(diagnostic);
            }
            return self.call_value(c, params, ret, args, span);
        }
        if !self.is_poisoned(ty) {
            let diagnostic = Diagnostic::error(
                codes::NOT_CALLABLE,
                format!("a value of type {} cannot be called", self.ty_name(ty)),
                callee_expr.span,
                "not a function",
            )
            .with_note(
                "functions, and values of a function type such as `(x: i64) => i64`, can be called",
            );
            self.report(diagnostic);
        }
        self.give_up(args.iter().copied(), span)
    }

    /// `panic(message)`: a call the checker knows, of type `never`, whose
    /// message is any text and whose place comes from its span.
    fn panic_call(
        &mut self,
        args: &'a [ast::ExprId],
        names: &'a [Option<ast::Name>],
        span: Span,
    ) -> ExprId {
        if let Some(name) = names.iter().flatten().next() {
            let diagnostic = Diagnostic::error(
                codes::PANIC_ARGUMENT,
                "the message of a panic is not named",
                name.span,
                "a named argument",
            )
            .with_note("`panic` takes one string: `panic(\"…\")`");
            self.report(diagnostic);
        }
        let message = match args {
            // A string written at the call is kept in the program; anything
            // else is built when the panic happens.
            [only] => self
                .message_text(*only, codes::PANIC_ARGUMENT, "a panic's message")
                .map(|value| match self.state.body.exprs[value].kind {
                    ExprKind::Str(sym) => ExprKind::Panic {
                        note: None,
                        message: Some(sym),
                    },
                    _ => ExprKind::Panic {
                        note: Some(value),
                        message: None,
                    },
                }),
            _ => {
                let diagnostic = Diagnostic::error(
                    codes::PANIC_ARGUMENT,
                    format!("`panic` takes one message, but {} were given", args.len()),
                    span,
                    "a panic takes one string",
                );
                self.report(diagnostic);
                for &arg in args {
                    self.infer(arg, None);
                }
                None
            }
        };
        match message {
            Some(panic) => self.alloc(panic, Types::NEVER, span),
            None => self.error_expr(span),
        }
    }

    /// A panic's message or an assert's note: a `str`, or a `String` lent
    /// as one. Anything else is reported with `code`.
    pub(super) fn message_text(
        &mut self,
        e: ast::ExprId,
        code: codes::Code,
        what: &str,
    ) -> Option<ExprId> {
        let value = self.infer_argument(e, Some(Types::STR));
        let ty = self.ty_of(value);
        if self.is_poisoned(ty) {
            return None;
        }
        if ty == Types::STR {
            return Some(value);
        }
        if let Some(lent) = self.lend_string(value) {
            return Some(lent);
        }
        let diagnostic = Diagnostic::error(
            code,
            format!("{what} is text"),
            self.state.body.exprs[value].span,
            format!("a value of type {}", self.ty_name(ty)),
        )
        .with_help("write it in a string: `\"\\(value)\"`")
        .with_note("a failure says what went wrong in text, built when it happens");
        self.report(diagnostic);
        None
    }

    /// A call of `callee`, with the type arguments written in its path, if
    /// any, and arguments given by position, by name or by
    /// default.
    pub(super) fn call(&mut self, callee: FnId, item: ItemUse<'a>) -> ExprId {
        // A file's contents, which the checker reads now: no call is made.
        if let Some(which @ (Intrinsic::EmbedBytes | Intrinsic::EmbedText)) =
            self.program.fns[callee].intrinsic
        {
            return self.embed(which == Intrinsic::EmbedText, item);
        }
        // A call to a C function that takes more than it declares is made
        // through C of the compiler's writing, with the types this call
        // passes.
        if self.program.fns[callee].is_variadic {
            return self.variadic_call(callee, item);
        }
        let args = item.args.expect("a call has arguments");
        let (hint, span) = (item.hint, item.span);
        let def = &self.program.fns[callee];
        let generics = def.generics.clone();
        let (mut ret, sig_span, name_sym) = (def.ret, def.span, def.name);
        let name = self.text(def.name);
        // A method's receiver is its first parameter, and is not written
        // among the arguments.
        let taken = usize::from(item.receiver.is_some());
        // Most calls give every parameter by position, and need no matching.
        let plain = item.names.iter().all(Option::is_none)
            && args.len() + taken == def.params.len()
            && def.params.iter().all(|p| p.default.is_none());
        let explicit =
            self.explicit_type_args(name_sym, &generics, item.type_args, item.name_segment);
        let mut inference = Inference::new(explicit);
        // A `static fn` reached through a type alias knows what the
        // type's own parameters are.
        if let Some(owner_ty) = item.owner_ty
            && let Some(owner) = self.program.owner_of_fn(callee)
        {
            let params: Vec<Ty> = generics
                .iter()
                .enumerate()
                .map(|(index, param)| {
                    self.intern(TyKind::Param(crate::ty::TyParam {
                        index: index as u32,
                        name: param.name,
                        copy: param.copy,
                    }))
                })
                .collect();
            let list = self.program.types.intern_list(&params);
            if let Some(declared) = self.program.type_of(owner, list) {
                self.unify(declared, owner_ty, &mut inference.tys);
            }
        }
        // A call through a constraint knows the interface's own types, which
        // the constraint carries: `C: At<i64, T>` says what `at` takes and what
        // it lends. They follow `Self`, which is the method's first parameter.
        if item.interface_args != crate::TyList::EMPTY {
            let args = self.program.types.list(item.interface_args).to_vec();
            for (n, arg) in args.into_iter().enumerate() {
                let Some(param) = generics.get(n + 1) else {
                    break;
                };
                let declared = self.intern(TyKind::Param(crate::ty::TyParam {
                    index: (n + 1) as u32,
                    name: param.name,
                    copy: param.copy,
                }));
                self.unify(declared, arg, &mut inference.tys);
            }
        }
        // A `static fn` called through a type parameter knows `Self`.
        if let Some(self_ty) = item.self_ty
            && let Some(param) = generics.first()
        {
            let declared = self.intern(TyKind::Param(crate::ty::TyParam {
                index: 0,
                name: param.name,
                copy: param.copy,
            }));
            self.unify(declared, self_ty, &mut inference.tys);
        }
        // What the receiver is decides the type's parameters: `T` of
        // `Option<T>` in `opt.unwrap_or(0)`.
        if let Some(receiver) = item.receiver {
            let declared = self.program.fns[callee].params[0].ty;
            let actual = self.ty_of(receiver);
            self.unify(declared, actual, &mut inference.tys);
        }
        // The result's type may decide what is still undecided, unless the
        // value is discarded or lent by a projection. It comes after the
        // receiver, which is what the value is: `Option<i64>`'s `unwrap`
        // answers an `i64` whatever is expected of it, and a `str`
        // expected is then a mismatch, not an `unwrap<str>` of an
        // `Option<i64>`.
        if let Some(hint) = hint
            && hint != Types::UNIT
            && !matches!(self.kind(ret), TyKind::Ref(..))
        {
            self.unify(ret, hint, &mut inference.tys);
        }
        // Defaults that are code, called with this call's type arguments
        // once they are known.
        let defaults = self.state.unsettled_defaults.len();
        let (mut hir_args, mut order) = if plain {
            let params = self.program.fns[callee].params[taken..].to_vec();
            let operands: Vec<GenericOperand<'_>> = args
                .iter()
                .zip(&params)
                .map(|(&expr, p)| GenericOperand {
                    expr,
                    declared: p.ty,
                    context: Some((p.span, "parameter declared here")),
                    is_arg: true,
                })
                .collect();
            (
                self.generic_operands(&operands, &mut inference.tys),
                Vec::new(),
            )
        } else {
            self.matched_call(callee, item, &mut inference, name, sig_span)
        };
        if let Some(receiver) = item.receiver {
            // The receiver is evaluated first, before the arguments.
            hir_args.insert(0, receiver);
            if !order.is_empty() {
                order = std::iter::once(0)
                    .chain(order.into_iter().map(|i| i + 1))
                    .collect();
            }
        }
        let mut type_list = crate::TyList::EMPTY;
        if !generics.is_empty() {
            let sliced = self.arrays_as_slices(&generics, &mut inference.tys);
            let written = format!("{name}<{}>(…)", vec!["…"; generics.len()].join(", "));
            let tys = self.finish_type_args(name, &written, &generics, inference, span);
            // An array passed as the slice it lends goes in converted to
            // one.
            if sliced {
                let params: Vec<Ty> = self.program.fns[callee]
                    .params
                    .iter()
                    .map(|p| p.ty)
                    .collect();
                for (arg, declared) in hir_args.iter_mut().zip(params) {
                    let declared = self.program.types.subst(declared, &tys);
                    if self.ty_of(*arg) != declared {
                        *arg = self.coerce(*arg, declared, None);
                    }
                }
            }
            type_list = self.program.types.intern_list(&tys);
            ret = self.program.types.subst(ret, &tys);
            self.settle_defaults(defaults, &tys);
        }
        let call = self.alloc(
            ExprKind::Call {
                callee,
                args: hir_args,
                type_args: type_list,
                order,
            },
            ret,
            span,
        );
        // A projection's call is a place, seen through like a reference
        // parameter. A function that answers a type
        // parameter answers a value, a reference included.
        let projects = matches!(self.kind(self.program.fns[callee].ret), TyKind::Ref(..));
        match self.kind(ret) {
            TyKind::Ref(pointee, _) if projects => self.alloc(ExprKind::Deref(call), pointee, span),
            _ => call,
        }
    }

    /// `printf(format, name, count)`: the declared parameters are checked
    /// as any others, and what follows them must already be what C expects,
    /// since nothing widens on the way. The call is pointed
    /// at a declaration of exactly these types, which the compiler writes a
    /// C wrapper for.
    fn variadic_call(&mut self, callee: FnId, item: ItemUse<'a>) -> ExprId {
        let args = item.args.expect("a call has arguments");
        let def = &self.program.fns[callee];
        let (declared, ret, name) = (def.params.clone(), def.ret, def.name);
        let sig_span = def.span;
        if let Some(name) = item.names.iter().flatten().next() {
            let diagnostic = Diagnostic::error(
                codes::NAMED_VARIADIC_ARGUMENT,
                "a call C reads in order cannot name its arguments",
                name.span,
                "a named argument",
            )
            .with_note("what follows the declared parameters is matched by position, so the rest of the call is too");
            self.report(diagnostic);
        }
        if args.len() < declared.len() {
            let text = self.text(name).to_string();
            let missing = declared.len() - args.len();
            let diagnostic = Diagnostic::error(
                codes::ARGUMENT_COUNT,
                format!(
                    "`{text}` takes {} and then as many as C reads",
                    plural(declared.len(), "argument", "arguments")
                ),
                item.span,
                format!("{missing} missing"),
            )
            .with_secondary(sig_span, "declared here");
            self.report(diagnostic);
        }
        let mut lowered = Vec::with_capacity(args.len());
        for (i, &arg) in args.iter().enumerate() {
            let value = match declared.get(i) {
                Some(param) => {
                    let context = Some((param.span, "parameter declared here"));
                    self.check_argument(arg, param.ty, context)
                }
                // A literal has no type of its own yet, and the only
                // string C reads here is a `cstring` — so a written one
                // becomes that, which widens nothing.
                None if matches!(self.ast.exprs[arg].kind, ast::ExprKind::Str(_)) => {
                    self.check_in(arg, Types::CSTRING, None)
                }
                None => {
                    let value = self.infer(arg, None);
                    let ty = self.ty_of(value);
                    self.check_variadic_arg(ty, self.ast.exprs[arg].span);
                    value
                }
            };
            lowered.push(value);
        }
        // The call keeps the declaration it names. Which C wrapper it goes
        // through is settled once every body is checked, since a worker
        // checking one body cannot declare a function the others would not
        // see.
        self.alloc(
            ExprKind::Call {
                callee,
                args: lowered,
                type_args: crate::TyList::EMPTY,
                order: Vec::new(),
            },
            ret,
            item.span,
        )
    }

    /// What may follow the parameters a C function declares: the scalars C
    /// reads with `va_arg`, already the width C expects.
    fn check_variadic_arg(&mut self, ty: Ty, span: Span) {
        let ok = match self.kind(ty) {
            TyKind::Str | TyKind::Fn(..) | TyKind::Struct(..) | TyKind::Ref(..) => false,
            _ => self.c_compatible(ty, false),
        };
        if ok || self.is_poisoned(ty) {
            return;
        }
        let diagnostic = Diagnostic::error(
            codes::NOT_C_COMPATIBLE,
            format!("{} cannot be read by a C function that takes more", self.ty_name(ty)),
            span,
            "not a value C reads there",
        )
        .with_note(
            "what follows the declared parameters is read one at a time, so it is the scalars C knows: integers of up to 64 bits, the floats, `bool`, `cstring` and `ptr<T>`",
        )
        .with_help("a `str` has no NUL: give C a `cstring`, or the two of `text.len()` and the bytes");
        self.report(diagnostic);
    }

    /// The arguments of a call that names some, leaves some to their
    /// defaults, or gives the wrong number: matched to the parameters, in
    /// their order, with the order they are evaluated in.
    fn matched_call(
        &mut self,
        callee: FnId,
        item: ItemUse<'a>,
        inference: &mut Inference,
        name: &str,
        sig_span: Span,
    ) -> (Vec<ExprId>, Vec<u32>) {
        let args = item.args.expect("a call has arguments");
        let names = item.arg_names();
        let taken = usize::from(item.receiver.is_some());
        let slots: Vec<Slot> = self.program.fns[callee].params[taken..]
            .iter()
            .map(|p| Slot {
                name: p.name,
                ty: p.ty,
                span: p.span,
                default: p.default.clone(),
            })
            .collect();
        let matched = self.match_arguments(&slots, args, &names, "parameter", name);
        // `f(a < b, c > (d))` reads as one argument, `a<b, c>(d)`; that is
        // reported, and the count would only repeat it.
        let misread = args.iter().any(|&a| self.misread_comparison(a));
        if !misread {
            self.report_argument_count(name, &slots, &matched, &names, sig_span, item.span);
        }
        // The matched arguments, as written.
        let mut written: Vec<(usize, usize)> = matched
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot, arg)| arg.map(|arg| (arg, slot)))
            .collect();
        written.sort_unstable();
        let operands: Vec<GenericOperand<'_>> = written
            .iter()
            .map(|&(arg, slot)| GenericOperand {
                expr: args[arg],
                declared: slots[slot].ty,
                context: Some((slots[slot].span, "parameter declared here")),
                is_arg: true,
            })
            .collect();
        let values = self.generic_operands(&operands, &mut inference.tys);
        let mut checked: Vec<Option<ExprId>> = vec![None; args.len()];
        for (&(arg, _), value) in written.iter().zip(values) {
            checked[arg] = Some(value);
        }
        for &arg in &matched.unmatched {
            self.infer_argument(args[arg], None);
        }
        self.arrange_arguments(&slots, &matched, &checked, item.span)
    }

    /// Reports arguments left over, and parameters given no value. A call
    /// that names nothing, of a function without defaults, is counted.
    /// `names` has the name of each argument given, if it has one.
    pub(super) fn report_argument_count(
        &mut self,
        name: &str,
        slots: &[Slot],
        matched: &Matched,
        names: &[Option<ast::Name>],
        sig_span: Span,
        span: Span,
    ) {
        let given = names.len();
        let missing = matched.missing(slots);
        if missing.is_empty() && matched.extra == 0 {
            return;
        }
        let plain = names.iter().all(Option::is_none) && slots.iter().all(|s| s.default.is_none());
        let diagnostic = if plain || matched.extra > 0 {
            Diagnostic::error(
                codes::ARGUMENT_COUNT,
                format!(
                    "`{name}` takes {} but {} given",
                    plural(slots.len(), "argument", "arguments"),
                    if given == 1 {
                        "1 was".to_string()
                    } else {
                        format!("{given} were")
                    }
                ),
                span,
                format!("expected {}", plural(slots.len(), "argument", "arguments")),
            )
        } else {
            let listed: Vec<String> = missing
                .iter()
                .map(|&i| format!("`{}`", self.text(slots[i].name)))
                .collect();
            Diagnostic::error(
                codes::ARGUMENT_COUNT,
                format!(
                    "`{name}` is missing {} {}",
                    if missing.len() == 1 {
                        "the argument"
                    } else {
                        "the arguments"
                    },
                    listed.join(", ")
                ),
                span,
                format!("missing {}", listed.join(", ")),
            )
        };
        self.report(diagnostic.with_secondary(sig_span, "declared here"));
    }

    /// A qualified name, with the arguments of the call around it if there
    /// was one.
    /// Whether a call of `sym` builds a struct: `Self`, or the name of a
    /// type here, imported or of the prelude, where no function or
    /// variable is called that. An enum's name is taken
    /// too, for the literal to say it is not a struct.
    fn names_struct(&self, sym: Symbol) -> bool {
        if self.lookup(sym).is_some()
            || self.names_around(sym)
            || self.fns().contains_key(&sym)
            || self.prelude_fn(sym).is_some()
        {
            return false;
        }
        let name = ast::Name {
            sym,
            span: Span::new(0, 0),
        };
        match self.imported(name) {
            Some(ImportedItem::Item(module, item)) => {
                !self.modules[module].fns.contains_key(&item.sym)
                    && (self.modules[module].types.contains_key(&item.sym)
                        || self.modules[module].aliases.contains_key(&item.sym))
            }
            _ => {
                self.text(sym) == "Self"
                    || self.types().contains_key(&sym)
                    || self.modules[self.current].aliases.contains_key(&sym)
                    || self.prelude_module(sym).is_some()
            }
        }
    }

    /// `..base` in a call that builds no struct.
    fn rest_without_struct(&mut self, rest: ast::ExprId) {
        let span = self.ast.exprs[rest].span;
        let diagnostic = Diagnostic::error(
            codes::REST_NOT_LAST,
            "`..` takes the fields of a struct the call builds, and this call builds none",
            span,
            "not a struct's fields",
        )
        .with_note(
            "`Name(field: value, ..base)` builds a struct whose fields not named are `base`'s",
        );
        self.report(diagnostic);
        self.infer(rest, None);
    }

    /// A call whose callee is a path: a struct built, where the path names
    /// one, and otherwise what `path_expr` finds.
    fn path_call(
        &mut self,
        leading_dot: bool,
        segments: &'a [ast::Name],
        item: ItemUse<'a>,
        rest: Option<ast::ExprId>,
    ) -> ExprId {
        if !leading_dot
            && let PathTarget::Item(module, name) = self.resolve_path_quietly(segments)
            && self.module_types_contain(module, name.sym)
        {
            let lit = StructLit {
                type_args: item.type_args,
                fields: aggregates::Written {
                    args: item.args.unwrap_or_default(),
                    names: item.names,
                },
                rest,
                hint: item.hint,
                span: item.span,
            };
            return self.struct_lit_of(module, name, segments.len() - 1, lit);
        }
        if let Some(rest) = rest {
            self.rest_without_struct(rest);
        }
        self.path_expr(leading_dot, segments, item)
    }

    pub(super) fn path_expr(
        &mut self,
        leading_dot: bool,
        segments: &'a [ast::Name],
        item: ItemUse<'a>,
    ) -> ExprId {
        let (args, span) = (item.args, item.span);
        if let Some(type_args) = item.type_args
            && let Some(diagnostic) = self.variable_with_type_args(segments, type_args)
        {
            self.report(diagnostic);
            return self.give_up(args.unwrap_or_default().iter().copied(), span);
        }
        if leading_dot {
            let [variant] = segments else {
                let diagnostic = Diagnostic::error(
                    codes::UNKNOWN_MODULE,
                    "a leading `.` names a variant, and takes no path",
                    segments[0].span,
                    "not a variant name",
                )
                .with_note(paths::MODULE_NOTE);
                self.report(diagnostic);
                return self.give_up(args.unwrap_or_default().iter().copied(), span);
            };
            let item = ItemUse {
                type_args: None,
                name_segment: 0,
                ..item
            };
            return self.variant_lit(None, *variant, item);
        }
        match self.resolve_path(segments, true, paths::MODULE_NOTE) {
            // A function, of this module or another.
            PathTarget::Item(module, name) => {
                let name_segment = segments.len() - 1;
                self.module_fn(
                    module,
                    name,
                    ItemUse {
                        name_segment,
                        ..item
                    },
                )
            }
            // `Enum::Variant`, in this module or in another.
            PathTarget::Variant(module, type_name, member) => {
                let name_segment = segments.len() - 2;
                let item = ItemUse {
                    name_segment,
                    ..item
                };
                // `T::name(…)` is a function of an interface the type
                // parameter requires.
                if module == self.current
                    && let Some(call) = self.constrained_static(type_name, member, item)
                {
                    return call;
                }
                // `Type::name(…)` is a static function, where the type has
                // one; otherwise a variant.
                if let Some(call) = self.static_fn(module, type_name, member, item) {
                    return call;
                }
                self.variant_lit(Some((module, type_name)), member, item)
            }
            PathTarget::Broken => self.give_up(args.unwrap_or_default().iter().copied(), span),
        }
    }

    /// Whether an argument is a variable followed by what reads as type
    /// arguments, which [`Lowerer::variable_with_type_args`] reports.
    fn misread_comparison(&self, arg: ast::ExprId) -> bool {
        let ast = self.ast;
        let mut e = arg;
        if let ast::ExprKind::Call { callee, .. } = ast.exprs[e].kind {
            e = callee;
        }
        match &ast.exprs[e].kind {
            ast::ExprKind::Path {
                segments,
                type_args: Some(_),
                ..
            } => matches!(segments[..], [name] if self.lookup(name.sym).is_some()),
            _ => false,
        }
    }

    /// A variable followed by what reads as type arguments: `x < y, z > (w)`
    /// in an argument list is `x<y, z>(w)` (grammar R7). Reports it with a
    /// fix that makes the first comparison one.
    pub(super) fn variable_with_type_args(
        &self,
        segments: &[ast::Name],
        type_args: &ast::TypeArgs,
    ) -> Option<Diagnostic> {
        let [name] = segments else {
            return None;
        };
        self.lookup(name.sym)?;
        let text = self.text(name.sym);
        let mut diagnostic = Diagnostic::error(
            codes::TYPE_ARGUMENT_COUNT,
            format!("`{text}` is a variable, and takes no type arguments"),
            type_args.span,
            "read as type arguments",
        )
        .with_note("after a name, `<` starts type arguments when `(`, `{` or `::` follows the matching `>`");
        if let Some(&first) = type_args.args.first() {
            let end = self.ast.types[first].span.hi;
            diagnostic = diagnostic.with_fix(
                "to compare, put the comparison in parentheses",
                [Edit::insert(name.span.lo, "("), Edit::insert(end, ")")],
            );
        }
        Some(diagnostic)
    }

    /// A function named through its module: `io::print(1)`.
    fn module_fn(&mut self, module: usize, item: ast::Name, used: ItemUse<'a>) -> ExprId {
        let (args, span) = (used.args, used.span);
        let name = self.text(item.sym).to_string();
        // A constant of that module, which is not called.
        if let Some(&id) = self.modules[module].consts.get(&item.sym)
            && self.program.consts[id].is_pub
        {
            let value = self.const_use(id, item.span);
            if let Some(args) = args {
                let diagnostic = Diagnostic::error(
                    codes::NOT_CALLABLE,
                    format!("`{name}` is a constant, not a function"),
                    span,
                    "called here",
                );
                self.report(diagnostic);
                return self.give_up(args.iter().copied(), span);
            }
            return self.referent(value, used.hint, item.span);
        }
        // A variable that module's C owns, read through the accessor the
        // compiler declared for it.
        if let Some(&id) = self.modules[module].globals.get(&item.sym)
            && self.program.globals[id].is_pub
        {
            let def = &self.program.globals[id];
            let (getter, ty) = (def.getter, def.ty);
            if let Some(args) = args {
                let diagnostic = Diagnostic::error(
                    codes::NOT_CALLABLE,
                    format!("`{name}` is a variable C owns, not a function"),
                    span,
                    "called here",
                );
                self.report(diagnostic);
                return self.give_up(args.iter().copied(), span);
            }
            return self.alloc(
                ExprKind::Call {
                    callee: getter,
                    args: Vec::new(),
                    type_args: crate::TyList::EMPTY,
                    order: Vec::new(),
                },
                ty,
                item.span,
            );
        }
        let own = self.modules[module].fns.get(&item.sym).copied();
        // A function of the prelude, where the path named this module.
        let found = own.or_else(|| {
            (module == self.current)
                .then(|| self.prelude_fn(item.sym))
                .flatten()
        });
        let Some(id) = found else {
            let path = self.modules[module].path.clone();
            // The module of `package::NAME` holds `VERSION` only where the
            // package says what its version is.
            if (path == "package" || path.ends_with("::package")) && name == "VERSION" {
                let diagnostic = Diagnostic::error(
                    codes::PACKAGE,
                    "this package says no version",
                    item.span,
                    "`package::VERSION` is its `@version`",
                )
                .with_help("write `@version(\"1.0.0\")` above `package` in its `package.wip`");
                self.report(diagnostic);
                return self.give_up(args.unwrap_or_default().iter().copied(), span);
            }
            let candidates: Vec<&str> = self.modules[module]
                .fns
                .keys()
                .chain(self.modules[module].consts.keys())
                .map(|&s| self.text(s))
                .collect();
            // A path that names no module is a name of this one: said as
            // an unknown name is, with what left the prelude.
            if module == self.current
                && let Some(help) = left_the_prelude(&name)
            {
                let diagnostic = Diagnostic::error(
                    codes::UNKNOWN_NAME,
                    format!("cannot find `{name}` in this scope"),
                    item.span,
                    "not found",
                )
                .with_help(help);
                self.report(diagnostic);
                return self.give_up(args.unwrap_or_default().iter().copied(), span);
            }
            let mut diagnostic = Diagnostic::error(
                codes::UNKNOWN_NAME,
                format!("cannot find `{name}` in module `{path}`"),
                item.span,
                "unknown name",
            );
            if let Some(similar) = suggest(&name, candidates) {
                diagnostic = diagnostic.with_fix(
                    format!("did you mean `{similar}`?"),
                    [Edit::replace(item.span, similar)],
                );
            }
            self.report(diagnostic);
            return self.give_up(args.unwrap_or_default().iter().copied(), span);
        };
        if !self.visible(module, self.program.fns[id].is_pub) {
            self.private_item(module, "fn", item);
        }
        match args {
            Some(_) => self.call(id, used),
            None => self.fn_value(id, used),
        }
    }

    /// A function named as a value: its address, of a function type.
    /// A generic function takes its type arguments from
    /// those written after its name, or from the function type expected.
    pub(super) fn fn_value(&mut self, id: FnId, item: ItemUse<'a>) -> ExprId {
        let def = &self.program.fns[id];
        let name = self.text(def.name);
        let (name_sym, ret, span) = (def.name, def.ret, item.span);
        // `@inline` promises that every call is spliced, and a call through
        // a value has no callee to splice.
        if def.is_inline {
            let declared = def.span;
            let diagnostic = Diagnostic::error(
                codes::CANNOT_INLINE,
                format!("`{name}` is `@inline`, so it cannot be a value"),
                span,
                "taken as a value here",
            )
            .with_secondary(declared, "declared `@inline` here")
            .with_note(
                "`@inline` promises that every call to it is spliced into its caller, and a call through a value has no callee to splice",
            )
            .with_help("write a lambda that calls it, or remove `@inline`");
            self.report(diagnostic);
            return self.error_expr(span);
        }
        if matches!(self.kind(ret), TyKind::Ref(..)) {
            let diagnostic = Diagnostic::error(
                codes::NOT_A_VALUE,
                format!("`{name}` is a projection, and cannot be a value yet"),
                span,
                "lends a place",
            )
            .with_note(
                "a projection lends a place to its caller; a function value returns a value",
            );
            self.report(diagnostic);
            return self.error_expr(span);
        }
        let params: Vec<Ty> = def.params.iter().map(|p| p.ty).collect();
        let generics = def.generics.clone();
        let params = self.program.types.intern_list(&params);
        let mut ty = self.intern(TyKind::Fn(params, ret));
        let explicit =
            self.explicit_type_args(name_sym, &generics, item.type_args, item.name_segment);
        let mut inference = Inference::new(explicit);
        if let Some(hint) = item.hint {
            self.unify(ty, hint, &mut inference.tys);
        }
        let mut type_args = crate::TyList::EMPTY;
        if !generics.is_empty() {
            let written = format!("{name}<{}>", vec!["…"; generics.len()].join(", "));
            let tys = self.finish_type_args(name, &written, &generics, inference, span);
            type_args = self.program.types.intern_list(&tys);
            ty = self.program.types.subst(ty, &tys);
        }
        self.alloc(ExprKind::FnRef { id, type_args }, ty, span)
    }

    /// A call through a value of the function type `(params) => ret`.
    fn call_value(
        &mut self,
        callee: ExprId,
        params: crate::TyList,
        ret: Ty,
        args: &'a [ast::ExprId],
        span: Span,
    ) -> ExprId {
        let params = self.program.types.list(params).to_vec();
        if args.len() != params.len() {
            let callee_ty = self.ty_of(callee);
            let diagnostic = Diagnostic::error(
                codes::ARGUMENT_COUNT,
                format!(
                    "the function takes {} but {} given",
                    plural(params.len(), "argument", "arguments"),
                    if args.len() == 1 {
                        "1 was".to_string()
                    } else {
                        format!("{} were", args.len())
                    }
                ),
                span,
                format!("expected {}", plural(params.len(), "argument", "arguments")),
            )
            .with_secondary(self.state.body.exprs[callee].span, self.ty_name(callee_ty));
            self.report(diagnostic);
        }
        let mut hir_args = Vec::new();
        for (i, &a) in args.iter().enumerate() {
            let arg = match params.get(i) {
                Some(&param) => self.check_argument(a, param, None),
                None => self.infer_argument(a, None),
            };
            hir_args.push(arg);
        }
        self.alloc(
            ExprKind::CallValue {
                callee,
                args: hir_args,
            },
            ret,
            span,
        )
    }

    /// A call through a closure: the arguments are checked as any other
    /// call's, and the code takes what was captured first.
    fn call_closure(
        &mut self,
        callee: ExprId,
        params: crate::TyList,
        ret: Ty,
        args: &'a [ast::ExprId],
        span: Span,
    ) -> ExprId {
        let value = self.call_value(callee, params, ret, args, span);
        let ExprKind::CallValue { args, .. } = self.state.body.exprs[value].kind.clone() else {
            unreachable!("a call through a value was just built");
        };
        // The code's own type: what it captured, then the arguments.
        let mut param_tys = vec![Types::PTR_U8];
        param_tys.extend(self.program.types.list(params).iter().copied());
        let list = self.program.types.intern_list(&param_tys);
        let code_ty = self.intern(TyKind::Fn(list, ret));
        self.state.body.exprs[value].kind = ExprKind::CallClosure {
            callee,
            args,
            code_ty,
        };
        value
    }
}
