//! Generic items: type parameters, type arguments, and inferring them where
//! a generic function, struct or enum is used.

use super::*;

/// An operand of a generic call or literal: the expression, the declared
/// type it is checked against, which may mention the item's type parameters,
/// and where that type was declared.
pub(super) struct GenericOperand<'a> {
    pub expr: ast::ExprId,
    pub declared: Ty,
    pub context: Option<(Span, &'a str)>,
    /// A call argument, which may be a reference.
    pub is_arg: bool,
}

/// The type arguments of one use of a generic item, as they become known.
pub(super) struct Inference {
    pub tys: Vec<Option<Ty>>,
    /// Which were known at the start: written, or taken from a type that was
    /// checked where it was made. They are not checked again.
    given: Vec<bool>,
}

impl Inference {
    pub fn new(tys: Vec<Option<Ty>>) -> Inference {
        let given = tys.iter().map(Option::is_some).collect();
        Inference { tys, given }
    }
}

impl<'a> Lowerer<'a> {
    /// The type parameters an item other than a struct or an enum declares,
    /// which take no defaults.
    pub(super) fn generic_params(&mut self, params: &[ast::GenericParam]) -> Vec<GenericParamDef> {
        for param in params {
            if let Some(default) = param.default {
                let span = self.ast.types[default].span;
                let diagnostic = Diagnostic::error(
                    codes::TYPE_PARAMETER_DEFAULT,
                    "only a struct's or an enum's type parameter has a default",
                    param.name.span.to(span),
                    "a default",
                )
                .with_note("a default says what a type is where it is written with fewer type arguments; a function's are inferred where it is called, and an `extend` block names every one");
                self.report(diagnostic);
            }
        }
        self.params_of(params, false)
    }

    /// The type parameters an interface declares, which may have defaults
    /// as a struct's do, and whose defaults may name `Self`, the type that
    /// implements it.
    pub(super) fn interface_generic_params(
        &mut self,
        params: &[ast::GenericParam],
    ) -> Vec<GenericParamDef> {
        self.type_generic_params(params)
    }

    /// The type parameters a struct or an enum declares. A default is read
    /// once every type has a name; here it is only noted,
    /// and the ones after it must have one too.
    pub(super) fn type_generic_params(
        &mut self,
        params: &[ast::GenericParam],
    ) -> Vec<GenericParamDef> {
        let mut defaulted: Option<Span> = None;
        for param in params {
            match (param.default, defaulted) {
                (Some(default), None) => {
                    defaulted = Some(param.name.span.to(self.ast.types[default].span));
                }
                (None, Some(first)) => {
                    let text = self.text(param.name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::TYPE_PARAMETER_DEFAULT,
                        format!("`{text}` has no default, and follows a parameter that has one"),
                        param.name.span,
                        "needs a default",
                    )
                    .with_secondary(first, "a default here")
                    .with_note("a use leaves out type arguments from the end, so the parameters with defaults come last");
                    self.report(diagnostic);
                }
                _ => {}
            }
        }
        self.params_of(params, true)
    }

    fn params_of(&mut self, params: &[ast::GenericParam], defaults: bool) -> Vec<GenericParamDef> {
        let mut seen: FxHashMap<Symbol, Span> = FxHashMap::default();
        let mut defs = Vec::new();
        for param in params {
            match seen.get(&param.name.sym) {
                Some(&first) => self.duplicate(param.name, first),
                None => {
                    seen.insert(param.name.sym, param.name.span);
                }
            }
            if self.builtin(param.name.sym).is_some() {
                let diagnostic = Diagnostic::error(
                    codes::BUILTIN_REDEFINED,
                    format!(
                        "`{}` is a built-in type and cannot name a type parameter",
                        self.text(param.name.sym)
                    ),
                    param.name.span,
                    "built-in type name",
                );
                self.report(diagnostic);
            }
            // `copy` is read here; the interfaces wait until every
            // parameter has a name, since a constraint may use one.
            let copy = param
                .bounds
                .iter()
                .any(|bound| self.text(bound.name.sym) == "copy");
            defs.push(GenericParamDef {
                name: param.name.sym,
                written: param.name.sym,
                interfaces: Vec::new(),
                copy,
                default: match param.default {
                    Some(_) if defaults => ParamDefault::Pending,
                    _ => ParamDefault::None,
                },
                span: param.name.span,
            });
        }
        // With the names in scope, `C: Items<T>` can be read.
        let outer = self.type_params.len();
        self.type_params.extend(defs.iter().cloned());
        for (i, param) in params.iter().enumerate() {
            let mut interfaces = Vec::new();
            for bound in &param.bounds {
                if self.text(bound.name.sym) == "copy" {
                    if !bound.args.is_empty() {
                        let diagnostic = Diagnostic::error(
                            codes::UNKNOWN_CONSTRAINT,
                            "`copy` takes no types",
                            bound.span,
                            "not an interface",
                        )
                        .with_note("`copy` says a type owns no memory");
                        self.report(diagnostic);
                    }
                    continue;
                }
                match self.interface_named(bound.name) {
                    Some(id) => {
                        // `T: From<i64>`: the types the interface takes
                        // here, which say which implementation a call means.
                        let constrained = self.intern(TyKind::Param(crate::TyParam {
                            index: (outer + i) as u32,
                            name: defs[i].name,
                            copy: defs[i].copy,
                        }));
                        let args = self.constraint_args(id, bound, constrained);
                        interfaces.push(crate::Constraint {
                            interface: id,
                            args,
                        });
                    }
                    None => {
                        let diagnostic = Diagnostic::error(
                            codes::UNKNOWN_CONSTRAINT,
                            format!("unknown constraint `{}`", self.text(bound.name.sym)),
                            bound.span,
                            "not a constraint",
                        )
                        .with_note("a constraint is `copy` or an interface");
                        self.report(diagnostic);
                    }
                }
            }
            defs[i].interfaces = interfaces;
        }
        self.type_params.truncate(outer);
        defs
    }

    /// The types a constraint's interface takes, checked against what it
    /// declares: an interface that takes types must be given them, but for
    /// those with defaults, which `constrained` — the type the constraint
    /// is on — stands in for `Self` in.
    fn constraint_args(
        &mut self,
        id: crate::InterfaceId,
        bound: &ast::Bound,
        constrained: Ty,
    ) -> crate::TyList {
        let generics = self.program.interfaces[id].generics.clone();
        if bound.args.len() < required_type_args(&generics) || bound.args.len() > generics.len() {
            let name = self.program.interfaces[id].name;
            for &arg in &bound.args {
                self.resolve_ty(arg);
            }
            self.type_arg_count(name, &generics, bound.args.len(), bound.span);
            return crate::TyList::EMPTY;
        }
        let mut args = Vec::new();
        for (&arg, param) in bound.args.iter().zip(&generics) {
            let ty = self.type_arg(arg);
            self.check_copy(ty, param, self.ast.types[arg].span);
            self.check_constraints(ty, param, self.ast.types[arg].span);
            args.push(ty);
        }
        let args = self.interface_defaults(id, args, constrained);
        self.program.types.intern_list(&args)
    }

    /// An interface's arguments as written, and those left out from the
    /// end, as their defaults say, with `implementing` in place of `Self`.
    /// A default is written in the terms of an interface's methods: `Self`
    /// first, then the interface's own parameters.
    pub(super) fn interface_defaults(
        &mut self,
        id: crate::InterfaceId,
        written: Vec<Ty>,
        implementing: Ty,
    ) -> Vec<Ty> {
        let generics = self.program.interfaces[id].generics.clone();
        let mut args = written;
        while args.len() < generics.len() {
            let ty = match generics[args.len()].default {
                ParamDefault::Ty(default) => {
                    let mut known = vec![implementing];
                    known.extend(args.iter().copied());
                    self.program.types.subst(default, &known)
                }
                ParamDefault::Pending | ParamDefault::None => Types::ERROR,
            };
            args.push(ty);
        }
        args
    }

    /// The type parameters of an item, as types.
    pub(super) fn param_tys(&mut self, generics: &[GenericParamDef]) -> Vec<Ty> {
        generics
            .iter()
            .enumerate()
            .map(|(index, param)| {
                self.intern(TyKind::Param(crate::TyParam {
                    index: index as u32,
                    name: param.name,
                    copy: param.copy,
                }))
            })
            .collect()
    }

    /// The type parameter in scope under `sym`, if there is one.
    pub(super) fn type_param(&mut self, sym: Symbol) -> Option<Ty> {
        let index = self.type_params.iter().position(|p| p.written == sym)?;
        let param = &self.type_params[index];
        let kind = TyKind::Param(crate::TyParam {
            index: index as u32,
            name: param.name,
            copy: param.copy,
        });
        Some(self.intern(kind))
    }

    /// The type parameters of a struct or enum.
    pub(super) fn type_generics(&self, def: TypeDef) -> &[GenericParamDef] {
        match def {
            TypeDef::Struct(id) => &self.program.structs[id].generics,
            TypeDef::Enum(id) => &self.program.enums[id].generics,
            // The element of `extend [T]`, and nothing for the rest.
            TypeDef::Builtin(owner) => self
                .program
                .builtins
                .get(&owner)
                .map_or(&[][..], |built| &built.generics),
        }
    }

    /// A struct or enum named in a type, with the type arguments written
    /// after it; `pointee` where it is what a `ptr` points at.
    pub(super) fn applied_type(
        &mut self,
        def: TypeDef,
        args: &[ast::TypeId],
        span: Span,
        pointee: bool,
    ) -> Ty {
        // `@opaque`: C knows how large one is and Wip does not, so a value
        // may not have this type; a pointer to one may.
        if let TypeDef::Struct(id) = def
            && self.program.structs[id].is_opaque
            && !pointee
        {
            let text = self.text(self.program.structs[id].name).to_string();
            let declared = self.program.structs[id].span;
            let diagnostic = Diagnostic::error(
                codes::OPAQUE_VALUE,
                format!("`{text}` is laid out by C, so a value cannot have its type"),
                span,
                "not a type a value can have",
            )
            .with_secondary(declared, "declared `@opaque` here")
            .with_note(
                "`@opaque` says where the fields are is C's business, so how large one is, is too",
            )
            .with_help(format!(
                "a pointer to one is written `ptr<{text}>`, and its fields are read through it"
            ));
            self.report(diagnostic);
            return Types::ERROR;
        }
        let generics = self.type_generics(def).to_vec();
        let name = match def {
            TypeDef::Struct(id) => self.program.structs[id].name,
            TypeDef::Enum(id) => self.program.enums[id].name,
            // A built-in type is never named through this path: it is
            // resolved by its own name.
            TypeDef::Builtin(_) => return Types::ERROR,
        };
        if args.len() < required_type_args(&generics) || args.len() > generics.len() {
            for &arg in args {
                self.resolve_ty(arg);
            }
            self.type_arg_count(name, &generics, args.len(), span);
            return Types::ERROR;
        }
        let mut tys = Vec::new();
        for (&arg, param) in args.iter().zip(&generics) {
            let ty = self.type_arg(arg);
            self.check_copy(ty, param, self.ast.types[arg].span);
            self.check_constraints(ty, param, self.ast.types[arg].span);
            tys.push(ty);
        }
        let tys = self.with_defaults(&generics, tys, span);
        if tys.iter().any(|&t| self.has_error(t)) {
            return Types::ERROR;
        }
        let list = self.program.types.intern_list(&tys);
        match def {
            TypeDef::Struct(id) => self.intern(TyKind::Struct(id, list)),
            TypeDef::Enum(id) => self.intern(TyKind::Enum(id, list)),
            TypeDef::Builtin(_) => Types::ERROR,
        }
    }

    /// A type argument written in the source. Like a field's type, it cannot
    /// be a bare slice. It may be a `str`, a view or a `&`
    /// reference, which makes the type given it a view, but not a `&var`.
    pub(super) fn type_arg(&mut self, id: ast::TypeId) -> Ty {
        let ty = self.resolve_ty(id);
        if self.no_var_ref(ty, id, "a type argument") || self.bare_slice(ty, id, false) {
            return Types::ERROR;
        }
        ty
    }

    /// Reports the wrong number of type arguments for `name`.
    pub(super) fn type_arg_count(
        &mut self,
        name: Symbol,
        generics: &[GenericParamDef],
        given: usize,
        span: Span,
    ) {
        let text = self.text(name);
        // Those with defaults may be left out, from the end.
        let required = required_type_args(generics);
        let count = match generics.len() - required {
            0 => plural(generics.len(), "type argument", "type arguments"),
            1 => format!("{required} or {} type arguments", generics.len()),
            _ => format!("{required} to {} type arguments", generics.len()),
        };
        let message = if generics.is_empty() {
            format!("`{text}` is not generic, and takes no type arguments")
        } else {
            let names: Vec<&str> = generics.iter().map(|p| self.text(p.name)).collect();
            format!(
                "`{text}` takes {count} (`{}`), but {} given",
                names.join("`, `"),
                if given == 1 {
                    "1 was".to_string()
                } else {
                    format!("{given} were")
                }
            )
        };
        let label = format!("expected {count}");
        let diagnostic = Diagnostic::error(codes::TYPE_ARGUMENT_COUNT, message, span, label);
        self.report(diagnostic);
    }

    /// Reports a type argument that owns memory where `param` requires
    /// `copy`. While type bodies are still being resolved, whether a type
    /// owns memory is not known yet, so the check waits until they are.
    pub(super) fn check_copy(&mut self, ty: Ty, param: &GenericParamDef, span: Span) {
        if !param.copy || self.is_poisoned(ty) {
            return;
        }
        if self.phase < Phase::TypesResolved {
            self.pending_copy.push((ty, param.name, span));
            return;
        }
        if self.owns(ty) {
            let diagnostic = Diagnostic::error(
                codes::NOT_COPY,
                format!(
                    "{} owns memory, but `{}` must be `copy`",
                    self.ty_name(ty),
                    self.text(param.name)
                ),
                span,
                "owns memory",
            )
            .with_note("a `copy` type is plain data, read without `move`: it holds no `own`, directly or in its fields");
            self.report(diagnostic);
        }
    }

    /// Runs the `copy` checks that waited for every type body.
    pub(super) fn flush_copy_checks(&mut self) {
        self.phase = Phase::TypesResolved;
        for (ty, name, span) in std::mem::take(&mut self.pending_copy) {
            let param = GenericParamDef {
                name,
                written: name,
                copy: true,
                interfaces: Vec::new(),
                default: ParamDefault::None,
                span,
            };
            self.check_copy(ty, &param, span);
        }
    }

    /// Whether a value of type `ty` owns memory. The field types of the
    /// instances inside it are interned first, since the program only looks
    /// them up. A type whose instances never end is reported elsewhere, and
    /// taken to own nothing.
    pub(super) fn owns(&mut self, ty: Ty) -> bool {
        crate::mono::complete_type(&mut self.program, ty) && self.program.owns_memory(ty)
    }

    /// The types of every field of a struct or enum type, with its type
    /// arguments substituted.
    pub(super) fn children(&mut self, ty: Ty) -> Vec<Ty> {
        crate::mono::children(&mut self.program, ty)
    }

    /// The type of field `index` of a struct type.
    pub(super) fn struct_field_ty(&mut self, ty: Ty, index: usize) -> Ty {
        let TyKind::Struct(id, args) = self.kind(ty) else {
            return Types::ERROR;
        };
        let declared = self.program.structs[id].fields[index].ty;
        let args = self.program.types.list(args).to_vec();
        self.program.types.subst(declared, &args)
    }

    /// The type of a field of a variant of an enum type.
    pub(super) fn variant_field_ty(&mut self, ty: Ty, variant: usize, field: usize) -> Ty {
        let TyKind::Enum(id, args) = self.kind(ty) else {
            return Types::ERROR;
        };
        let declared = self.program.enums[id].variants[variant].fields[field].ty;
        let args = self.program.types.list(args).to_vec();
        self.program.types.subst(declared, &args)
    }

    /// Type arguments written after `::` in an expression, for an item whose
    /// name is segment `expected_after` of the path. `None` in each slot the
    /// caller must infer.
    pub(super) fn explicit_type_args(
        &mut self,
        name: Symbol,
        generics: &[GenericParamDef],
        type_args: Option<&'a ast::TypeArgs>,
        expected_after: usize,
    ) -> Vec<Option<Ty>> {
        let Some(type_args) = type_args else {
            return vec![None; generics.len()];
        };
        if type_args.after != expected_after {
            for &arg in &type_args.args {
                self.resolve_ty(arg);
            }
            let diagnostic = Diagnostic::error(
                codes::TYPE_ARGUMENT_COUNT,
                format!(
                    "type arguments belong after the name of `{}`",
                    self.text(name)
                ),
                type_args.span,
                "not after the generic item",
            );
            self.report(diagnostic);
            return vec![Some(Types::ERROR); generics.len()];
        }
        let given = type_args.args.len();
        if given < required_type_args(generics) || given > generics.len() {
            for &arg in &type_args.args {
                self.resolve_ty(arg);
            }
            self.type_arg_count(name, generics, given, type_args.span);
            return vec![Some(Types::ERROR); generics.len()];
        }
        let written: Vec<Ty> = type_args
            .args
            .iter()
            .zip(generics)
            .map(|(&arg, param)| {
                let ty = self.type_arg(arg);
                self.check_copy(ty, param, self.ast.types[arg].span);
                self.check_constraints(ty, param, self.ast.types[arg].span);
                ty
            })
            .collect();
        self.with_defaults(generics, written, type_args.span)
            .into_iter()
            .map(Some)
            .collect()
    }

    /// The type arguments a use writes, followed by the defaults of the
    /// ones it leaves out, each read in the arguments before it and checked
    /// as a written one is. A default not read yet, while
    /// the defaults are read, is waited for.
    fn with_defaults(
        &mut self,
        generics: &[GenericParamDef],
        mut tys: Vec<Ty>,
        span: Span,
    ) -> Vec<Ty> {
        let written = tys.len().min(generics.len());
        for param in &generics[written..] {
            let ty = match param.default {
                ParamDefault::Ty(default) => self.program.types.subst(default, &tys),
                ParamDefault::Pending => {
                    self.default_waits = true;
                    Types::ERROR
                }
                ParamDefault::None => Types::ERROR,
            };
            tys.push(ty);
        }
        // Checked with every argument known, since a constraint may name a
        // parameter after its own.
        for (&ty, param) in tys[written..].iter().zip(&generics[written..]) {
            self.check_copy(ty, param, span);
            self.check_constraints_in(ty, param, span, &tys);
        }
        tys
    }

    /// Learns type arguments by matching a declared type, which mentions the
    /// item's type parameters, against the type found. Only unknown
    /// arguments are set: a conflict is reported later, as a mismatch.
    pub(super) fn unify(&self, declared: Ty, found: Ty, solution: &mut [Option<Ty>]) {
        if !self.program.types.is_generic(declared) || self.is_poisoned(found) {
            return;
        }
        let types = &self.program.types;
        match (types.kind(declared), types.kind(found)) {
            (TyKind::Param(param), _) => {
                let slot = &mut solution[param.index as usize];
                if slot.is_none() {
                    *slot = Some(found);
                }
            }
            // A slice is matched by the elements of an array or buffer, as
            // the coercions allow.
            (TyKind::Own(d), TyKind::Own(f)) => {
                if let TyKind::Slice(elem) = types.kind(d)
                    && let TyKind::Array(found_elem, _) = types.kind(f)
                {
                    self.unify(elem, found_elem, solution);
                    return;
                }
                self.unify(d, f, solution);
            }
            // An array where a list on the heap is declared is reported
            // where it is checked, with `own` to write; its elements say
            // what the list's are.
            (TyKind::Own(d), TyKind::Array(found_elem, _))
                if matches!(types.kind(d), TyKind::Slice(_)) =>
            {
                if let TyKind::Slice(elem) = types.kind(d) {
                    self.unify(elem, found_elem, solution);
                }
            }
            // A reference to an `own` stands for a reference to what it owns
            // (the `&own<T>` to `&T` coercion), and an array or buffer for a
            // slice of its elements.
            (TyKind::Ref(d, _), TyKind::Ref(f, _)) => {
                let mut under = f;
                loop {
                    let declared_own = matches!(types.kind(d), TyKind::Own(_) | TyKind::Param(_));
                    match types.kind(under) {
                        TyKind::Own(inner) if !declared_own => under = inner,
                        _ => break,
                    }
                }
                if let TyKind::Slice(elem) = types.kind(d)
                    && let TyKind::Array(found_elem, _) | TyKind::Slice(found_elem) =
                        types.kind(under)
                {
                    self.unify(elem, found_elem, solution);
                    return;
                }
                // A container is lent as the elements it lends where a
                // slice is declared.
                if let TyKind::Slice(elem) = types.kind(d)
                    && let Some(found_elem) = self.items_element(under)
                {
                    self.unify(elem, found_elem, solution);
                    return;
                }
                self.unify(d, under, solution);
            }
            // Elements given without the `&` a slice is declared with say
            // what the elements are, so that what is reported is the borrow
            // left out; and so does a container a reference parameter
            // stands for, which is lent as them.
            (TyKind::Ref(d, _), TyKind::Slice(found_elem) | TyKind::Array(found_elem, _))
                if matches!(types.kind(d), TyKind::Slice(_)) =>
            {
                if let TyKind::Slice(elem) = types.kind(d) {
                    self.unify(elem, found_elem, solution);
                }
            }
            (TyKind::Ref(d, _), _) if matches!(types.kind(d), TyKind::Slice(_)) => {
                if let TyKind::Slice(elem) = types.kind(d)
                    && let Some(found_elem) = self.items_element(found)
                {
                    self.unify(elem, found_elem, solution);
                }
            }
            (TyKind::Array(d, _), TyKind::Array(f, _))
            | (TyKind::Slice(d), TyKind::Slice(f))
            // A temporary lent where `&[T]` is declared is matched against
            // `[T]`, which an array's elements answer.
            | (TyKind::Slice(d), TyKind::Array(f, _))
            | (TyKind::Ptr(d), TyKind::Ptr(f))
            | (TyKind::Slots(d), TyKind::Slots(f)) => {
                self.unify(d, f, solution);
            }
            (TyKind::Struct(a, d), TyKind::Struct(b, f)) if a == b => {
                self.unify_lists(d, f, solution);
            }
            // A temporary container lent where `&[T]` is declared is matched
            // against `[T]`, which the elements it lends answer.
            (TyKind::Slice(d), _) => {
                if let Some(found_elem) = self.items_element(found) {
                    self.unify(d, found_elem, solution);
                }
            }
            (TyKind::Enum(a, d), TyKind::Enum(b, f)) if a == b => {
                self.unify_lists(d, f, solution);
            }
            // A function value converts to a closure lent for a call, so a
            // lent closure's types are learnt from one.
            (TyKind::Ref(d, _), TyKind::Fn(..)) if matches!(types.kind(d), TyKind::Fn(..)) => {
                self.unify(d, found, solution);
            }
            (TyKind::Fn(d, d_ret), TyKind::Fn(f, f_ret)) => {
                self.unify_lists(d, f, solution);
                self.unify(d_ret, f_ret, solution);
            }
            _ => {}
        }
    }

    fn unify_lists(
        &self,
        declared: crate::TyList,
        found: crate::TyList,
        solution: &mut [Option<Ty>],
    ) {
        let types = &self.program.types;
        for (&d, &f) in types.list(declared).iter().zip(types.list(found)) {
            self.unify(d, f, solution);
        }
    }

    /// Whether every type parameter `ty` mentions has a type argument.
    fn resolved(&self, ty: Ty, solution: &[Option<Ty>]) -> bool {
        !self.program.types.any(ty, &|kind| {
            matches!(kind, TyKind::Param(param) if solution[param.index as usize].is_none())
        })
    }

    /// Whether an expression's type is decided by what is expected of it:
    /// a literal number, a `.Variant`, or an array of them. Such operands
    /// are checked after the others, which may decide the type arguments.
    /// A lambda, lent or owned.
    fn is_lambda(&self, e: ast::ExprId) -> bool {
        match &self.ast.exprs[e].kind {
            ast::ExprKind::Lambda { .. } => true,
            ast::ExprKind::Unary {
                op: ast::UnaryOp::Own,
                operand,
                ..
            } => matches!(self.ast.exprs[*operand].kind, ast::ExprKind::Lambda { .. }),
            _ => false,
        }
    }

    fn takes_expected_type(&self, e: ast::ExprId) -> bool {
        let ast = self.ast;
        match &ast.exprs[e].kind {
            ast::ExprKind::Int(_) | ast::ExprKind::Float(_) => true,
            ast::ExprKind::Unary {
                op: ast::UnaryOp::Neg,
                operand,
                ..
            }
            | ast::ExprKind::Paren(operand) => self.takes_expected_type(*operand),
            ast::ExprKind::Path { leading_dot, .. } => *leading_dot,
            ast::ExprKind::Call { callee, .. } => matches!(
                ast.exprs[*callee].kind,
                ast::ExprKind::Path {
                    leading_dot: true,
                    ..
                }
            ),
            ast::ExprKind::Array(elems) => elems.iter().all(|&e| self.takes_expected_type(e)),
            _ => false,
        }
    }

    /// Checks the operands of a use of a generic item, learning its type
    /// arguments from them. Operands whose declared type is already known
    /// are checked against it; the others are inferred first, and converted
    /// once the type arguments are known. Returns the operands in order.
    pub(super) fn generic_operands(
        &mut self,
        operands: &[GenericOperand<'_>],
        solution: &mut [Option<Ty>],
    ) -> Vec<ExprId> {
        let mut done: Vec<Option<ExprId>> = vec![None; operands.len()];
        // Inferred without knowing their declared type: converted at the end.
        let mut unconverted = Vec::new();
        for pass in 0..2 {
            for (i, operand) in operands.iter().enumerate() {
                if done[i].is_some() {
                    continue;
                }
                // A literal waits for what the others say its type is; so
                // does a lambda, whose parameters take their types from it:
                // in `fold(0, (sum, n) => …)` the `0` says what `sum` is.
                let deferred = pass == 0
                    && (self.takes_expected_type(operand.expr) || self.is_lambda(operand.expr));
                let known = self.resolved(operand.declared, solution);
                if deferred && !known {
                    continue;
                }
                // An argument may be written `&place`, and lent where
                // nothing else may be.
                let id = if known {
                    let args: Vec<Ty> =
                        solution.iter().map(|t| t.unwrap_or(Types::ERROR)).collect();
                    let declared = self.program.types.subst(operand.declared, &args);
                    if operand.is_arg {
                        self.check_argument(operand.expr, declared, operand.context)
                    } else {
                        self.check_in(operand.expr, declared, operand.context)
                    }
                } else {
                    // A lambda takes its parameters' types from what is
                    // expected, so it is given what is known of the declared
                    // type; what is not known is inferred from its body.
                    let is_lambda = match &self.ast.exprs[operand.expr].kind {
                        ast::ExprKind::Lambda { .. } => true,
                        // `own (x) => …`, an owned closure.
                        ast::ExprKind::Unary {
                            op: ast::UnaryOp::Own,
                            operand,
                            ..
                        } => matches!(self.ast.exprs[*operand].kind, ast::ExprKind::Lambda { .. }),
                        _ => false,
                    };
                    let hint = is_lambda.then(|| {
                        let args: Vec<Ty> =
                            solution.iter().map(|t| t.unwrap_or(Types::ERROR)).collect();
                        self.program.types.subst(operand.declared, &args)
                    });
                    let id = if operand.is_arg {
                        self.infer_argument(operand.expr, hint)
                    } else {
                        self.infer(operand.expr, hint)
                    };
                    // A value with no place of its own is lent where it is
                    // used, so a `&T` parameter is answered by a `T`.
                    let mut declared = operand.declared;
                    if operand.is_arg
                        && let TyKind::Ref(inner, crate::RefKind::Shared) = self.kind(declared)
                        && !matches!(self.kind(self.ty_of(id)), TyKind::Ref(..))
                        && matches!(self.place_root(id), PlaceRoot::NotAPlace)
                    {
                        declared = inner;
                    }
                    self.unify(declared, self.ty_of(id), solution);
                    unconverted.push(i);
                    id
                };
                done[i] = Some(id);
            }
        }
        let args: Vec<Ty> = solution.iter().map(|t| t.unwrap_or(Types::ERROR)).collect();
        for i in unconverted {
            let declared = self.program.types.subst(operands[i].declared, &args);
            let id = done[i].expect("every operand was checked");
            // Still the argument, though its type was worked out after it
            // was lowered: a `&` and a `String` need to know.
            let context = operands[i].context;
            done[i] = Some(self.coerce_to(id, declared, context, operands[i].is_arg));
        }
        done.into_iter()
            .map(|id| id.expect("every operand was checked"))
            .collect()
    }

    /// An array inferred for a parameter whose interfaces its slice implements
    /// and it does not: the slice it lends, as an array's methods are its
    /// slice's. `total(&numbers)` with `C: Sequence<i64>` passes the array as
    /// `&[i64]`. Whether anything changed, so the caller converts its arguments
    /// to match.
    pub(super) fn arrays_as_slices(
        &mut self,
        generics: &[GenericParamDef],
        solution: &mut [Option<Ty>],
    ) -> bool {
        let mut changed = false;
        for (i, param) in generics.iter().enumerate() {
            if param.interfaces.is_empty() {
                continue;
            }
            let Some(Some(ty)) = solution.get(i).copied() else {
                continue;
            };
            let TyKind::Array(elem, _) = self.kind(ty) else {
                continue;
            };
            // The same question the constraint check asks, with the
            // arguments known so far; one that is not known yet asks only
            // whether the type implements the interface at all.
            let args: Vec<Ty> = solution.iter().map(|t| t.unwrap_or(Types::ERROR)).collect();
            let constraints: Vec<(crate::Constraint, bool)> = param
                .interfaces
                .iter()
                .map(|c| {
                    let args = self.program.types.subst_list(c.args, &args);
                    let known = self
                        .program
                        .types
                        .list(args)
                        .iter()
                        .all(|&a| !self.is_poisoned(a));
                    (
                        crate::Constraint {
                            interface: c.interface,
                            args,
                        },
                        known,
                    )
                })
                .collect();
            let holds = |this: &Self, t: Ty| {
                constraints.iter().all(|&(c, known)| match known {
                    true => this.implements_with(t, c),
                    false => this.implements_any(t, c.interface),
                })
            };
            if holds(self, ty) {
                continue;
            }
            let slice = self.intern(TyKind::Slice(elem));
            if holds(self, slice) {
                solution[i] = Some(slice);
                changed = true;
            }
        }
        changed
    }

    /// The type arguments of a use of `name`, once its operands have been
    /// checked: reports those that could not be inferred, and checks the
    /// inferred ones. `written` shows where type arguments are written.
    pub(super) fn finish_type_args(
        &mut self,
        name: &str,
        written: &str,
        generics: &[GenericParamDef],
        inference: Inference,
        span: Span,
    ) -> Vec<Ty> {
        let Inference {
            tys: mut solution,
            given,
        } = inference;
        // A constraint's argument may be a parameter of its own, and the
        // implementation the type has says what it is: `C = Vec<i64>` with
        // `C: Items<T>` makes `T` an `i64`.
        self.solve_through_constraints(generics, &mut solution, span);
        // A parameter nothing decided is its default, where it has one and
        // the parameters the default names are known: `Arena()` is an
        // `Arena<T, T>`.
        for (i, param) in generics.iter().enumerate() {
            if solution[i].is_some() {
                continue;
            }
            if let ParamDefault::Ty(default) = param.default
                && self.resolved(default, &solution)
            {
                let args: Vec<Ty> = solution.iter().map(|t| t.unwrap_or(Types::ERROR)).collect();
                solution[i] = Some(self.program.types.subst(default, &args));
            }
        }
        let solution = solution;
        let missing: Vec<&str> = generics
            .iter()
            .zip(&solution)
            .filter(|(_, t)| t.is_none())
            .map(|(p, _)| self.text(p.name))
            .collect();
        // An error inside the use, in an argument, is why nothing could be
        // inferred from it, and has been said.
        let explained = self
            .diagnostics
            .iter()
            .any(|d| d.is_error() && d.primary.span.lo >= span.lo && d.primary.span.hi <= span.hi);
        if !missing.is_empty() && !explained {
            let names = missing.join("`, `");

            let diagnostic = Diagnostic::error(
                codes::CANNOT_INFER,
                format!(
                    "cannot infer the type {} `{names}` of `{name}`",
                    if missing.len() == 1 {
                        "argument"
                    } else {
                        "arguments"
                    }
                ),
                span,
                "type arguments needed",
            )
            .with_help(format!("write them: `{written}`"));
            self.report(diagnostic);
        }
        let tys: Vec<Ty> = solution
            .into_iter()
            .map(|t| t.unwrap_or(Types::ERROR))
            .collect();
        for ((&ty, param), &given) in tys.iter().zip(generics).zip(&given) {
            if given {
                continue;
            }
            {
                if self.holds_var_ref(ty) {
                    let diagnostic = Diagnostic::error(
                        codes::REF_OUTSIDE_PARAMETER,
                        format!(
                            "`{}` would be {}, but a type argument cannot hold a `&var` reference",
                            self.text(param.name),
                            self.ty_name(ty)
                        ),
                        span,
                        "infers a `&var` reference",
                    )
                    .with_note("a `&` reference may stand for a type parameter, as a view may, but a `&var` is a parameter's alone");
                    self.report(diagnostic);
                } else {
                    self.check_copy(ty, param, span);
                    self.check_constraints_in(ty, param, span, &tys);
                }
            }
        }
        tys
    }

    /// What a constraint's arguments say about the parameters they name.
    /// `C: Items<T>` with `C` known looks for the implementation `C` has,
    /// reads its arguments in `C`'s own, and that is `T`.
    fn solve_through_constraints(
        &mut self,
        generics: &[GenericParamDef],
        solution: &mut [Option<Ty>],
        span: Span,
    ) {
        for (i, param) in generics.iter().enumerate() {
            let Some(ty) = solution[i] else { continue };
            if self.is_poisoned(ty) {
                continue;
            }
            for constraint in param.interfaces.clone() {
                let declared = self.program.types.list(constraint.args).to_vec();
                // Only where something is still unknown.
                let wanted: Vec<usize> = declared
                    .iter()
                    .filter_map(|&arg| match self.kind(arg) {
                        TyKind::Param(p) => Some(p.index as usize),
                        _ => None,
                    })
                    .filter(|&index| solution.get(index).is_some_and(Option::is_none))
                    .collect();
                if wanted.is_empty() {
                    continue;
                }
                let Some(args) = self.implemented_args(ty, constraint.interface, span) else {
                    continue;
                };
                for (&arg, found) in declared.iter().zip(args) {
                    if let TyKind::Param(p) = self.kind(arg)
                        && let Some(slot) = solution.get_mut(p.index as usize)
                        && slot.is_none()
                    {
                        *slot = Some(found);
                    }
                }
            }
        }
    }

    /// The types a concrete type implements an interface with, read in its
    /// own parameters; `None` where it implements it once for none of them,
    /// or more than once.
    fn implemented_args(
        &mut self,
        ty: Ty,
        interface: crate::InterfaceId,
        span: Span,
    ) -> Option<Vec<Ty>> {
        let owner = self.owner_of(ty)?;
        let own_args: Vec<Ty> = match self.kind(ty) {
            TyKind::Struct(_, list) | TyKind::Enum(_, list) => {
                self.program.types.list(list).to_vec()
            }
            TyKind::Slice(elem) => vec![elem],
            _ => Vec::new(),
        };
        let found: Vec<crate::TyList> = self
            .program
            .impls
            .iter()
            .filter(|i| i.interface == interface && i.ty == owner)
            .map(|i| i.args)
            .collect();
        if found.len() > 1 {
            let kinds: Vec<String> = found
                .iter()
                .map(|&args| self.constraint_name(crate::Constraint { interface, args }, ty))
                .collect();
            let type_name = self.ty_name(ty);
            let diagnostic = Diagnostic::error(
                codes::CANNOT_INFER,
                format!("{type_name} implements `{}` more than once", {
                    let name = self.program.interfaces[interface].name;
                    self.text(name).to_string()
                }),
                span,
                format!("{} both apply", kinds.join(" and ")),
            )
            .with_help("write the type arguments, or constrain it with the ones you mean")
            .with_note("a constraint's arguments say which implementation a call means");
            self.report(diagnostic);
            return None;
        }
        let args = self.program.types.list(*found.first()?).to_vec();
        Some(
            args.iter()
                .map(|&arg| self.program.types.subst(arg, &own_args))
                .collect(),
        )
    }

    /// The type arguments a generic struct or enum takes from the type
    /// expected where it is written.
    pub(super) fn expected_args(&self, def: TypeDef, expected: Option<Ty>) -> Vec<Option<Ty>> {
        let count = self.type_generics(def).len();
        let Some(expected) = expected else {
            return vec![None; count];
        };
        let list = match (def, self.kind(expected)) {
            (TypeDef::Struct(a), TyKind::Struct(b, list)) if a == b => list,
            (TypeDef::Enum(a), TyKind::Enum(b, list)) if a == b => list,
            _ => return vec![None; count],
        };
        self.program
            .types
            .list(list)
            .iter()
            .map(|&t| Some(t))
            .collect()
    }

    /// Interns the field types of every instance interned since `from`, so
    /// that later phases, which share the types read-only, can look them up.
    pub(super) fn complete_types(&mut self, from: usize) {
        let mut diagnostics = Vec::new();
        crate::mono::complete_types(&mut self.program, from, self.interner, &mut diagnostics);
        self.diagnostics.extend(diagnostics);
    }
}

/// How many type arguments a use must write: up to the last parameter with
/// no default. A `static fn`'s own parameters follow its type's, so a
/// default of the type's is left out only where the function has none of
/// its own.
pub(super) fn required_type_args(generics: &[GenericParamDef]) -> usize {
    generics
        .iter()
        .rposition(|p| p.default == ParamDefault::None)
        .map_or(0, |last| last + 1)
}
