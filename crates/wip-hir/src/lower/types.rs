//! Types, as a program writes them: what a name in a signature or a field
//! means, what an array's length is, and the two shapes a type may not
//! have — one that holds itself, and an `@inline` that calls itself.

use super::*;

impl<'a> Lowerer<'a> {
    /// The type `id` writes. Where it names one, the name is recorded with
    /// what it named, for an editor.
    pub(super) fn resolve_ty(&mut self, id: ast::TypeId) -> Ty {
        self.resolve_as(id, false)
    }

    /// The type `ptr<…>` points at, which may be an opaque C type: that
    /// type itself, and nothing inside it, since a value of an opaque type
    /// cannot be held anywhere.
    fn resolve_pointee(&mut self, id: ast::TypeId) -> Ty {
        self.resolve_as(id, true)
    }

    fn resolve_as(&mut self, id: ast::TypeId, pointee: bool) -> Ty {
        let resolved = self.resolve_written(id, pointee);
        let ty = &self.ast.types[id];
        let name = match ty.kind {
            ast::TypeKind::Named { name, .. } => {
                let len = self.text(name).len() as u32;
                Some(Span::new(ty.span.lo, ty.span.lo + len))
            }
            ast::TypeKind::Path { ref segments, .. } => segments.last().map(|n| n.span),
            _ => None,
        };
        if let Some(span) = name
            && resolved != Types::ERROR
            && !self.derived
        {
            self.program.names.push((span, Named::Type(resolved)));
        }
        resolved
    }

    fn resolve_written(&mut self, id: ast::TypeId, pointee: bool) -> Ty {
        let ast = self.ast;
        let ty = &ast.types[id];
        match ty.kind {
            // A type from another module.
            ast::TypeKind::Path {
                ref segments,
                ref args,
            } => match self.resolve_path(segments, false, paths::MODULE_NOTE) {
                PathTarget::Item(module, name) => {
                    self.named_type(module, name, args, ty.span, pointee)
                }
                _ => Types::ERROR,
            },
            // `dyn Interface`: a value of some type that implements it,
            // known only behind a reference.
            ast::TypeKind::Dyn(name, ref dyn_args) => {
                let Some(interface) = self.interface_named(name) else {
                    let text = self.text(name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::UNKNOWN_INTERFACE,
                        format!("cannot find interface `{text}`"),
                        name.span,
                        "unknown interface",
                    )
                    .with_note("`dyn` takes an interface: `&dyn Shape`");
                    self.report(diagnostic);
                    return Types::ERROR;
                };
                if let Some(diagnostic) = self.not_dispatchable(interface, name.span) {
                    self.report(diagnostic);
                    return Types::ERROR;
                }
                // `&dyn Items<Card>`: the types the interface takes, which
                // say which implementation the table holds.
                let generics = self.program.interfaces[interface].generics.clone();
                if dyn_args.len() != generics.len() {
                    let interface_name = self.program.interfaces[interface].name;
                    for &arg in dyn_args {
                        self.resolve_ty(arg);
                    }
                    self.type_arg_count(interface_name, &generics, dyn_args.len(), ty.span);
                    return Types::ERROR;
                }
                let mut args = Vec::new();
                for (&arg, param) in dyn_args.iter().zip(&generics) {
                    let arg_ty = self.type_arg(arg);
                    self.check_copy(arg_ty, param, self.ast.types[arg].span);
                    self.check_constraints(arg_ty, param, self.ast.types[arg].span);
                    args.push(arg_ty);
                }
                let args = self.program.types.intern_list(&args);
                self.intern(TyKind::Dyn(interface, args))
            }
            ast::TypeKind::Named {
                name: sym,
                ref args,
            } => {
                // `Self` is the type the method belongs to.
                let self_ty = (self.text(sym) == "Self").then_some(()).map(|()| {
                    self.self_ty.unwrap_or_else(|| {
                        let diagnostic = Diagnostic::error(
                            codes::SELF_OUTSIDE_METHOD,
                            "`Self` is only available in a method",
                            ty.span,
                            "no type here",
                        )
                        .with_note("`Self` is the type a method belongs to");
                        self.report(diagnostic);
                        Types::ERROR
                    })
                });
                // `ptr<T>`: a C pointer, which takes one type argument.
                if self.text(sym) == "ptr" {
                    return self.pointer_type(args, ty.span);
                }
                // `Slots<T>`: uninitialized storage, the standard library's
                // alone.
                if self.text(sym) == "Slots" {
                    return self.slots_type(args, ty.span);
                }
                // A type parameter, or a built-in type, takes no type
                // arguments.
                let simple = self_ty
                    .or_else(|| self.type_param(sym))
                    .or_else(|| self.builtin(sym));
                if let Some(simple) = simple {
                    if !args.is_empty() {
                        for &arg in args {
                            self.resolve_ty(arg);
                        }
                        self.type_arg_count(sym, &[], args.len(), ty.span);
                        return Types::ERROR;
                    }
                    return simple;
                }
                if let Some(&id) = self.opaques().get(&sym) {
                    if !pointee {
                        self.opaque_as_value(sym, ty.span);
                        return Types::ERROR;
                    }
                    if !args.is_empty() {
                        for &arg in args {
                            self.resolve_ty(arg);
                        }
                        self.type_arg_count(sym, &[], args.len(), ty.span);
                        return Types::ERROR;
                    }
                    return self.intern(TyKind::Opaque(id));
                }
                // `type Id = i64`: another name for a type, which stands
                // for what it names.
                if self.aliases().contains_key(&sym) {
                    return self
                        .alias_type(sym, args, ty.span)
                        .expect("just looked it up");
                }
                match self.types().get(&sym) {
                    Some(&(def, _)) => self.applied_type(def, args, ty.span, pointee),
                    None => {
                        // A type of another module, imported by name.
                        match self.imported(ast::Name { sym, span: ty.span }) {
                            Some(ImportedItem::Item(module, item)) => {
                                return self.named_type(module, item, args, ty.span, pointee);
                            }
                            Some(ImportedItem::Broken) => return Types::ERROR,
                            None => {}
                        }
                        // A type of the prelude, which every file sees.
                        if let Some(prelude) = self.prelude_type(sym) {
                            return self.applied_type(prelude, args, ty.span, pointee);
                        }
                        let text = self.text(sym);
                        let candidates = self
                            .types()
                            .keys()
                            .map(|&s| self.text(s))
                            .chain(BUILTIN_TYPES.iter().map(|(n, _)| *n));
                        let mut diagnostic = Diagnostic::error(
                            codes::UNKNOWN_TYPE,
                            format!("cannot find type `{text}`"),
                            ty.span,
                            "unknown type",
                        );
                        if let Some(help) = left_the_prelude(text) {
                            diagnostic = diagnostic.with_help(help);
                        } else if let Some(similar) = suggest(text, candidates) {
                            diagnostic = diagnostic.with_fix(
                                format!("a type with a similar name exists: `{similar}`"),
                                [Edit::replace(ty.span, similar)],
                            );
                        }
                        self.report(diagnostic);
                        Types::ERROR
                    }
                }
            }
            ast::TypeKind::Own(inner) => {
                let inner_ty = self.resolve_ty(inner);
                // `own` of a view is a view, kept where a view is, as a
                // `Vec` of one is: a tree of views holds one child in it.
                self.intern(TyKind::Own(inner_ty))
            }
            ast::TypeKind::Ref { var, inner } => {
                let inner = self.resolve_ty(inner);
                let kind = if var {
                    crate::RefKind::Var
                } else {
                    crate::RefKind::Shared
                };
                // `&(…) => R` and `&var (…) => R` are closures lent for one
                // call.
                self.intern(TyKind::Ref(inner, kind))
            }
            // `(a: A, b: B) => R`. Its parameters follow the
            // rules for a function's parameters; its result cannot lend a
            // place. The names only document it.
            ast::TypeKind::Fn { ref params, ret } => {
                let mut seen = FxHashMap::default();
                let mut tys = Vec::new();
                for param in params {
                    match seen.get(&param.name.sym) {
                        Some(&first) => self.duplicate(param.name, first),
                        None => {
                            seen.insert(param.name.sym, param.name.span);
                        }
                    }
                    let ty = self.resolve_ty(param.ty);
                    tys.push(self.param_ty(ty, param.ty));
                }
                let ret = {
                    let ty = self.resolve_ty(ret);
                    let message = "a function value's result cannot be a reference";
                    let help = "a projection lends a place, and cannot be a value yet";
                    let whole = matches!(self.kind(ty), TyKind::Ref(..));
                    if (whole && self.no_ref(ty, ret, message, Some(help)))
                        || self.no_var_ref(ty, ret, "a function value's result")
                        || self.bare_slice(ty, ret, false)
                    {
                        Types::ERROR
                    } else {
                        ty
                    }
                };
                if tys.iter().any(|&t| self.has_error(t)) || self.has_error(ret) {
                    return Types::ERROR;
                }
                let params = self.program.types.intern_list(&tys);
                self.intern(TyKind::Fn(params, ret))
            }
            ast::TypeKind::Array { elem, len } => {
                let ty = self.resolve_ty(elem);
                match self.array_len(len) {
                    Some(len) => self.intern(TyKind::Array(ty, len)),
                    None => Types::ERROR,
                }
            }
            ast::TypeKind::Slice(elem) => {
                let ty = self.resolve_ty(elem);
                self.intern(TyKind::Slice(ty))
            }
            ast::TypeKind::Error => Types::ERROR,
        }
    }

    /// A field that borrows, in a struct that is not a view, or a variant's.
    /// `payload` is for a variant's, which has no view of
    /// its own.
    pub(super) fn stored_field(&mut self, ty: Ty, type_id: ast::TypeId, payload: bool) -> bool {
        let span = self.ast.types[type_id].span;
        let help = if payload {
            "write `view enum`, for an enum whose variants borrow, or hold a `String`, which owns its bytes".to_string()
        } else {
            "write `view struct`, for a struct that borrows".to_string()
        };
        self.stored_view_at(ty, span, "a field", Some(help))
    }

    /// `&var` in a view: two views could then write one place.
    pub(super) fn var_in_view(&mut self, type_id: ast::TypeId) -> bool {
        let span = self.ast.types[type_id].span;
        let diagnostic = Diagnostic::error(
            codes::REF_OUTSIDE_PARAMETER,
            "a view holds `&`, not `&var`",
            span,
            "a `&var` reference",
        )
        .with_note("a view may be copied, and two copies of a `&var` would write one place; what a view changes, it is given as a parameter")
        .with_help("hold `&`, and take `&var` where the change is made");
        self.report(diagnostic);
        true
    }

    /// `Slots<T>`: a block of slots, none of them initialized. Only the
    /// standard library may name it, since nothing else may hold memory
    /// that holds nothing.
    fn slots_type(&mut self, args: &[ast::TypeId], span: Span) -> Ty {
        if !self.in_std() {
            let diagnostic = Diagnostic::error(
                codes::IMPL_TARGET,
                "`Slots` is the standard library's",
                span,
                "not a type this module may name",
            )
            .with_note(
                "a block of slots holds memory that holds nothing, which nothing else in Wip may; `Vec<T>` is what it is for",
            )
            .with_help("use `Vec<T>`");
            self.report(diagnostic);
            return Types::ERROR;
        }
        let [arg] = args else {
            let diagnostic = Diagnostic::error(
                codes::TYPE_ARGUMENT_COUNT,
                format!(
                    "`Slots` takes one type argument, and {} given",
                    plural(args.len(), "was", "were")
                ),
                span,
                "a block of slots",
            );
            self.report(diagnostic);
            return Types::ERROR;
        };
        let inner = self.resolve_ty(*arg);
        self.intern(TyKind::Slots(inner))
    }

    /// An opaque C type named where a value would have it: it says only
    /// that the type exists.
    pub(super) fn opaque_as_value(&mut self, sym: Symbol, span: Span) {
        let text = self.text(sym).to_string();
        let diagnostic = Diagnostic::error(
            codes::OPAQUE_VALUE,
            format!("`{text}` is a C type whose contents are not known"),
            span,
            "not a type a value can have",
        )
        .with_note(
            "an opaque C type says only that the type exists; how large one is, and what is in it, are C's business",
        )
        .with_help(format!("a pointer to one is written `ptr<{text}>`"));
        self.report(diagnostic);
    }

    /// `ptr<T>`: a C pointer. Its pointee may be anything
    /// with a size, an opaque C type, or `void` for `void *`.
    fn pointer_type(&mut self, args: &[ast::TypeId], span: Span) -> Ty {
        let [arg] = args else {
            let diagnostic = Diagnostic::error(
                codes::TYPE_ARGUMENT_COUNT,
                format!(
                    "`ptr` takes one type argument, and {} {} given",
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" }
                ),
                span,
                "a C pointer",
            )
            .with_note("a C pointer is written `ptr<T>`; `ptr<void>` is C's `void *`");
            self.report(diagnostic);
            // A C type may stand in any of them, wrong as their number is.
            for &arg in args {
                self.resolve_pointee(arg);
            }
            return Types::ERROR;
        };
        let inner = self.resolve_pointee(*arg);
        // A reference is second-class, so it cannot be behind a pointer that
        // is stored anywhere.
        if self.no_ref(inner, *arg, "a C pointer cannot point at a reference", None) {
            return Types::ERROR;
        }
        self.intern(TyKind::Ptr(inner))
    }

    /// An array's length: a literal, or a top-level `val` that stands for a
    /// whole number.
    /// The constant `name` stands for: this module's, one imported by
    /// name, or the prelude's.
    pub(super) fn const_named(&self, name: ast::Name) -> Option<ConstId> {
        if let Some(&id) = self.consts().get(&name.sym) {
            return Some(id);
        }
        if let Some(ImportedItem::Item(module, item)) = self.imported(name)
            && let Some(&id) = self.modules[module].consts.get(&item.sym)
            && self.program.consts[id].is_pub
        {
            return Some(id);
        }
        self.prelude_const(name.sym)
    }

    /// An array's length, where it is one; `None` where it is not, which
    /// was reported, so that what uses the array says nothing more of it.
    pub(super) fn array_len(&mut self, len: ast::ArrayLen) -> Option<u64> {
        let name = match len {
            ast::ArrayLen::Int(n) => return Some(n),
            ast::ArrayLen::Name(name) => name,
        };
        let text = self.text(name.sym).to_string();
        let Some(id) = self.const_named(name) else {
            let diagnostic = Diagnostic::error(
                codes::UNKNOWN_NAME,
                format!("cannot find the constant `{text}`"),
                name.span,
                "unknown name",
            )
            .with_note(
                "an array's length is a literal or a top-level `val`, which is known where it is written",
            );
            self.report(diagnostic);
            return None;
        };
        self.const_value(id);
        if !self.known_now(id, name.span, "an array's length") {
            return None;
        }
        let def = &self.program.consts[id];
        let (value, ty) = (def.value.clone(), def.ty);
        match value {
            Some(ConstValue::Int(n)) => {
                // A signed constant holds the low bits of its value, so the
                // sign comes from its own width.
                let written = match self.kind(ty) {
                    TyKind::Int(t) if t.signed() => consts::as_signed(n, t.bits()),
                    _ => n as i128,
                };
                match u64::try_from(written) {
                    Ok(length) => Some(length),
                    Err(_) if written < 0 => {
                        let diagnostic = Diagnostic::error(
                            codes::UNKNOWN_NAME,
                            format!("`{text}` is negative, so it is not a length"),
                            name.span,
                            "a negative length",
                        );
                        self.report(diagnostic);
                        None
                    }
                    Err(_) => {
                        let diagnostic = Diagnostic::error(
                            codes::UNKNOWN_NAME,
                            format!("`{text}` is too large for an array's length"),
                            name.span,
                            "a length that does not fit",
                        );
                        self.report(diagnostic);
                        None
                    }
                }
            }
            Some(_) => {
                let diagnostic = Diagnostic::error(
                    codes::UNKNOWN_NAME,
                    format!("`{text}` is {}, not a whole number", self.ty_name(ty)),
                    name.span,
                    "not a length",
                );
                self.report(diagnostic);
                None
            }
            // Its own error was reported where it was written.
            None => None,
        }
    }

    /// An `@inline` function that reaches itself, directly or through
    /// other `@inline` functions: splicing it would never end.
    /// A `@tailrec` function is a loop before inlining
    /// sees it, so its call to itself is not one of these.
    pub(super) fn check_inline_cycles(&mut self) {
        let inline: Vec<FnId> = self
            .program
            .fns
            .iter()
            .filter(|(_, def)| def.is_inline && def.body.is_some())
            .map(|(id, _)| id)
            .collect();
        // Who each one calls, keeping only the calls that would be spliced.
        let mut calls: FxHashMap<FnId, Vec<(FnId, Span)>> = FxHashMap::default();
        for &id in &inline {
            let def = &self.program.fns[id];
            let Some(body) = &def.body else { continue };
            let mut called = Vec::new();
            for (_, expr) in body.exprs.iter() {
                if let ExprKind::Call { callee, .. } = &expr.kind {
                    let callee = self.program.fns[*callee]
                        .instance_of
                        .map_or(*callee, |(generic, _)| generic);
                    if self.program.fns[callee].is_inline
                        && !(callee == id && self.program.fns[id].is_tailrec)
                    {
                        called.push((callee, expr.span));
                    }
                }
            }
            calls.insert(id, called);
        }
        // The first cycle each one is in, if it is in any.
        let mut reported: FxHashSet<FnId> = FxHashSet::default();
        for &start in &inline {
            if reported.contains(&start) {
                continue;
            }
            let mut stack = vec![(start, start, Span::at(0))];
            let mut seen: FxHashSet<FnId> = FxHashSet::default();
            while let Some((id, _, at)) = stack.pop() {
                if id == start && !seen.is_empty() {
                    let text = self.text(self.program.fns[start].name).to_string();
                    let declared = self.program.fns[start].span;
                    let diagnostic = Diagnostic::error(
                        codes::CANNOT_INLINE,
                        format!("`{text}` is `@inline` and calls itself"),
                        at,
                        "the call that comes back here",
                    )
                    .with_secondary(declared, "declared `@inline` here")
                    .with_note(
                        "splicing a body into itself never ends; `@tailrec` makes a function's calls to itself a loop, which inlining sees as one body",
                    );
                    self.report(diagnostic);
                    reported.insert(start);
                    break;
                }
                if !seen.insert(id) {
                    continue;
                }
                for &(callee, span) in calls.get(&id).into_iter().flatten() {
                    stack.push((callee, id, span));
                }
            }
        }
    }

    /// A struct or enum that contains itself by value (not behind `own` or
    /// `&`) would be infinitely large.
    pub(super) fn check_recursive_types(&mut self) {
        let mut defs: Vec<(TypeDef, Symbol, Span)> = Vec::new();
        for (id, s) in self.program.structs.iter() {
            defs.push((TypeDef::Struct(id), s.name, s.span));
        }
        for (id, e) in self.program.enums.iter() {
            defs.push((TypeDef::Enum(id), e.name, e.span));
        }
        for (def, name, span) in defs {
            // A generic type is checked with its own parameters as arguments.
            let generics = self.type_generics(def).to_vec();
            let params = self.param_tys(&generics);
            let list = self.program.types.intern_list(&params);
            let ty = match def {
                TypeDef::Struct(id) => self.intern(TyKind::Struct(id, list)),
                TypeDef::Enum(id) => self.intern(TyKind::Enum(id, list)),
                TypeDef::Builtin(_) => continue,
            };
            if self.contains_by_value(ty, ty, &mut FxHashSet::default(), 0) {
                let text = self.text(name);
                let diagnostic = Diagnostic::error(
                    codes::RECURSIVE_TYPE,
                    format!("recursive type `{text}` has infinite size"),
                    span,
                    "contains itself without indirection",
                )
                .with_help(format!(
                    "put the recursive field behind `own`: `own<{text}>`"
                ));
                self.report(diagnostic);
            }
        }
    }

    /// Whether `ty` holds `target` by value. A generic type that holds ever
    /// larger instances of itself, as `S<T>` holding `S<Pair<T, T>>`, has no
    /// size either: past a depth it is taken to hold itself.
    pub(super) fn contains_by_value(
        &mut self,
        ty: Ty,
        target: Ty,
        visited: &mut FxHashSet<Ty>,
        depth: u32,
    ) -> bool {
        if depth > crate::mono::MAX_TYPE_DEPTH {
            return true;
        }
        let children: Vec<Ty> = match self.kind(ty) {
            TyKind::Struct(..) | TyKind::Enum(..) => self.children(ty),
            TyKind::Array(elem, _) => vec![elem],
            _ => Vec::new(),
        };
        children.into_iter().any(|child| {
            child == target
                || (visited.insert(child)
                    && self.contains_by_value(child, target, visited, depth + 1))
        })
    }
}
