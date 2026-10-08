//! Items: type definitions, function signatures, and the types allowed
//! across the C boundary.

use super::*;

/// What a type was declared by: an item of a file, or a `struct` written
/// inside an `extern "C"` block, which is the same declaration without
/// the word `extern`.
#[derive(Clone, Copy)]
pub(crate) enum Declared<'a> {
    Item(&'a ast::Item),
    Struct(&'a ast::StructDecl),
}

impl<'a> Declared<'a> {
    /// The annotations written before it, which a `@derive` is read from.
    pub(super) fn annotations(self) -> &'a [ast::Annotation] {
        match self {
            Declared::Struct(s) => &s.annotations,
            Declared::Item(ast::Item::Struct(s)) => &s.annotations,
            Declared::Item(ast::Item::Enum(e)) => &e.annotations,
            Declared::Item(_) => &[],
        }
    }

    /// The struct it declares, where it declares one.
    pub(super) fn struct_decl(self) -> Option<&'a ast::StructDecl> {
        match self {
            Declared::Struct(s) | Declared::Item(ast::Item::Struct(s)) => Some(s),
            Declared::Item(_) => None,
        }
    }

    /// The enum it declares, where it declares one.
    pub(super) fn enum_decl(self) -> Option<&'a ast::EnumDecl> {
        match self {
            Declared::Item(ast::Item::Enum(e)) => Some(e),
            _ => None,
        }
    }

    /// Whether a field of it, or of one of its variants, has a default.
    pub(super) fn has_field_defaults(self) -> bool {
        if let Some(s) = self.struct_decl() {
            return s.fields.iter().any(|f| f.default.is_some());
        }
        self.enum_decl().is_some_and(|e| {
            e.variants
                .iter()
                .any(|v| v.fields.iter().any(|f| f.default.is_some()))
        })
    }
}

impl<'a> Lowerer<'a> {
    pub(super) fn collect_types(&mut self) -> Vec<(Declared<'a>, TypeDef)> {
        let ast = self.ast;
        let mut types = Vec::new();
        for item in &ast.items {
            let (name, def) = match item {
                // `type name` in an extern block: a C type whose contents
                // Wip does not know.
                ast::Item::Extern(e) => {
                    // The block's `@header` is what its declarations
                    // share, and a struct written inside it takes that one.
                    // It is read rather than checked
                    // here: the block's own annotations are checked where
                    // its functions are declared, and checking them twice
                    // would report each mistake twice.
                    let block = self.written_header(&e.annotations);
                    for declared in &e.structs {
                        if let Some((name, def)) = self.collect_struct(declared, block) {
                            self.name_type(name, def);
                            types.push((Declared::Struct(declared), def));
                        }
                    }
                    for declared in &e.types {
                        self.annotations(&declared.annotations, annotations::Target::ExternType);
                        let name = declared.name;
                        let id = self.program.opaques.alloc(OpaqueDef {
                            name: name.sym,
                            module: self.current as u32,
                            is_pub: declared.is_pub,
                            span: name.span,
                        });
                        if let Some(&first) = self.opaques().get(&name.sym) {
                            let previous = self.program.opaques[first].span;
                            self.duplicate(name, previous);
                        } else if let Some(&(_, previous)) = self.types().get(&name.sym) {
                            self.duplicate(name, previous);
                        } else {
                            self.opaques_mut().insert(name.sym, id);
                        }
                    }
                    continue;
                }
                ast::Item::Struct(s) => match self.collect_struct(s, None) {
                    Some(pair) => pair,
                    None => continue,
                },
                ast::Item::Enum(e) => {
                    self.annotations(&e.annotations, annotations::Target::Enum);
                    let generics = self.type_generic_params(&e.generics);
                    let def = EnumDef {
                        name: e.name.sym,
                        generics,
                        variants: Vec::new(),
                        methods: Vec::new(),
                        module: self.current as u32,
                        is_pub: e.is_pub,
                        is_view: e.is_view,
                        span: e.name.span,
                    };
                    let id = self.program.enums.alloc(def);
                    // The enums of the prelude's that the compiler makes
                    // values of, each for what `KnownEnum` says.
                    if self.prelude == Some(self.current)
                        && let Some(known) = KnownEnum::named(self.text(e.name.sym))
                    {
                        self.program.prelude_items.set_enumeration(known, id);
                    }
                    (e.name, TypeDef::Enum(id))
                }
                _ => continue,
            };
            self.name_type(name, def);
            types.push((Declared::Item(item), def));
        }
        types
    }

    /// The paths an annotation names, each of which must be inside the
    /// module: where a file is on this machine is the command line's
    /// business, not a module's.
    fn module_paths(&mut self, written: &[(Symbol, Span)], what: &str) -> Vec<(String, Span)> {
        let mut paths = Vec::new();
        for &(text, span) in written {
            let path = self.text(text).to_string();
            let outside = path.starts_with(['/', '\\'])
                || path.get(1..2) == Some(":")
                || path.split(['/', '\\']).any(|piece| piece == "..");
            if outside {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    format!("`{path}` is not in the module"),
                    span,
                    "a path that leaves it",
                )
                .with_note(format!(
                    "`{what}` names a file of the module, so that a module carries its own C"
                ));
                self.report(diagnostic);
                continue;
            }
            paths.push((path, span));
        }
        paths
    }

    /// The settings `@define` names, each `NAME` or `NAME=value`, where
    /// `NAME` is what C could have written after `#define`. A value is
    /// what follows the `=`, and reaches the C compiler as one argument, so
    /// nothing in it is read by a shell.
    fn c_defines(&mut self, written: &[(Symbol, Span)]) -> Vec<String> {
        let mut defines = Vec::new();
        for &(text, span) in written {
            let define = self.text(text).to_string();
            let name = define
                .split_once('=')
                .map_or(define.as_str(), |(name, _)| name);
            let is_name = name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            let problem = if !is_name {
                Some("not a name C could define")
            } else if define.contains(['\n', '\r']) {
                Some("a value on more than one line")
            } else {
                None
            };
            if let Some(label) = problem {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    format!("`{define}` is not a setting `@define` can pass"),
                    span,
                    label,
                )
                .with_note(
                    "`@define` takes `NAME` or `NAME=value`, which each C file of the module is compiled with as `-D` passes it; a setting that needs more than that belongs in the header `@prefix` names",
                );
                self.report(diagnostic);
                continue;
            }
            defines.push(define);
        }
        defines
    }

    /// The header an `extern "C"` block names, read without checking the
    /// block's annotations — that happens where its declarations are.
    fn written_header(&self, written: &[ast::Annotation]) -> Option<(Symbol, Span)> {
        for annotation in written {
            if self.text(annotation.name.sym) != "header" {
                continue;
            }
            if let [arg] = &annotation.args[..]
                && let ast::AnnotationValue::Str(text) = arg.value
            {
                return Some((text, annotation.span));
            }
        }
        None
    }

    /// Puts a type's name in this module's scope, where it is free: the
    /// prelude's names are reserved, a built-in's cannot
    /// be taken, and a name declared twice is reported.
    fn name_type(&mut self, name: ast::Name, def: TypeDef) {
        if self.prelude_name(name, "a type named") {
            // Reported; it keeps its own place in this module's scope.
        } else if self.builtin(name.sym).is_some() {
            let diagnostic = Diagnostic::error(
                codes::BUILTIN_REDEFINED,
                format!(
                    "`{}` is a built-in type and cannot be redefined",
                    self.text(name.sym)
                ),
                name.span,
                "built-in type name",
            );
            self.report(diagnostic);
        } else if let Some(&(_, first)) = self.types().get(&name.sym) {
            self.duplicate(name, first);
        } else {
            self.types_mut().insert(name.sym, (def, name.span));
        }
    }

    /// Declares a struct or a union, whether it was written at the top
    /// level or inside an `extern "C"` block, where `block` is that
    /// block's `@header` — the one its declarations share.
    fn collect_struct(
        &mut self,
        s: &'a ast::StructDecl,
        block: Option<(Symbol, Span)>,
    ) -> Option<(ast::Name, TypeDef)> {
        let mut annotations = self.annotations(&s.annotations, annotations::Target::Struct);
        // A struct in a block takes the block's header where it says none
        // of its own.
        if annotations.header.is_none() {
            annotations.header = block;
        }
        // `@opaque` is C's promise that it knows the layout, so
        // it needs the header that says so, and a Wip struct is
        // laid out by Wip.
        let opaque = match (annotations.opaque, s.is_extern, annotations.header) {
            (None, _, _) => false,
            (Some(span), false, _) => {
                let diagnostic = Diagnostic::error(
                            codes::ANNOTATION,
                            "`@opaque` belongs on an `extern struct`",
                            span,
                            "not a C type",
                        )
                        .with_note(
                            "it says C knows where the fields are, which is only true of a struct C declares",
                        );
                self.report(diagnostic);
                false
            }
            (Some(span), true, None) => {
                let diagnostic = Diagnostic::error(
                            codes::ANNOTATION,
                            "`@opaque` needs the header that declares the struct",
                            span,
                            "no header to read it from",
                        )
                        .with_help("write `@header(\"config.h\")` beside it")
                        .with_note(
                            "the C that reaches its fields is compiled against that header, since Wip does not know where they are",
                        );
                self.report(diagnostic);
                false
            }
            (Some(_), true, Some(_)) => true,
        };
        // A type the compiler knows is the standard library's to declare.
        if annotations.intrinsic && !self.in_std() {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                "`@intrinsic` on a struct is the standard library's",
                s.name.span,
                "a type the compiler would have to know",
            )
            .with_note(
                "it marks a type whose meaning the compiler knows, as `std::sync::Atomic`, reached only through what the compiler writes",
            );
            self.report(diagnostic);
        }
        let generics = self.type_generic_params(&s.generics);
        let def = StructDef {
            name: s.name.sym,
            is_extern: s.is_extern,
            is_union: s.is_union,
            is_opaque: opaque,
            is_intrinsic: annotations.intrinsic && self.in_std(),
            header: annotations.header.map(|(h, _)| h),
            accessors: Vec::new(),
            generics,
            fields: Vec::new(),
            methods: Vec::new(),
            is_env: false,
            is_view: s.is_view,
            // The prelude's tuples, which `(A, B)` is written
            // into.
            is_tuple: self.prelude == Some(self.current)
                && (MIN_TUPLE..=MAX_TUPLE).any(|n| s.name.sym == Symbol::tuple(n)),
            generator: None,
            module: self.current as u32,
            is_pub: s.is_pub,
            span: s.name.span,
        };
        let id = self.program.structs.alloc(def);
        // The structs of the prelude's that the compiler makes values of,
        // each for what `KnownStruct` says.
        if self.prelude == Some(self.current)
            && let Some(known) = KnownStruct::named(self.text(s.name.sym))
        {
            self.program.prelude_items.set_structure(known, id);
        }
        Some((s.name, TypeDef::Struct(id)))
    }

    /// The names this file's type aliases declare. The
    /// bodies wait until every name is known, since one alias may name
    /// another.
    pub(super) fn collect_aliases(&mut self) {
        for item in &self.ast.items {
            let ast::Item::Type(alias) = item else {
                continue;
            };
            let name = alias.name;
            if self.prelude_name(name, "a type named") {
                continue;
            }
            if self.builtin(name.sym).is_some() {
                let text = self.text(name.sym).to_string();
                let diagnostic = Diagnostic::error(
                    codes::BUILTIN_REDEFINED,
                    format!("`{text}` is a built-in type"),
                    name.span,
                    "a name of the language",
                )
                .with_note("the built-in types are the language's, and a program may not take one of their names");
                self.report(diagnostic);
                continue;
            }
            if let Some(&(_, first)) = self.types().get(&name.sym) {
                self.duplicate(name, first);
                continue;
            }
            if let Some(first) = self.aliases().get(&name.sym).map(|a| a.span) {
                self.duplicate(name, first);
                continue;
            }
            let generics = self.generic_params(&alias.generics);
            // An alias is another name for a type, and the type it names
            // carries whatever its own parameters ask of theirs.
            for (param, written) in generics.iter().zip(&alias.generics) {
                if param.interfaces.is_empty() && !param.copy {
                    continue;
                }
                let diagnostic = Diagnostic::error(
                    codes::ALIAS,
                    "an alias's parameters take no constraints",
                    written.span,
                    "a constraint",
                )
                .with_note(
                    "an alias is another name for a type, and what that type asks of its arguments is written where it is declared",
                );
                self.report(diagnostic);
            }
            let def = crate::lower::AliasDef {
                ty: None,
                generics,
                is_pub: alias.is_pub,
                span: name.span,
            };
            self.aliases_mut().insert(name.sym, def);
        }
    }

    /// Where each of this file's aliases is written, so that its body can
    /// be resolved there when something first names it.
    pub(super) fn record_alias_sources(&mut self) {
        let ast = self.ast;
        for item in &ast.items {
            let ast::Item::Type(decl) = item else {
                continue;
            };
            let source = AliasSource {
                decl,
                ast,
                imports: self.imports.clone(),
            };
            self.alias_sources
                .insert((self.current, decl.name.sym), source);
        }
    }

    /// The bodies of this file's aliases, each resolved where it was not
    /// already, when an alias before it named it.
    pub(super) fn resolve_aliases(&mut self) {
        for item in &self.ast.items {
            if let ast::Item::Type(decl) = item {
                self.alias_body(self.current, decl.name.sym);
            }
        }
    }

    /// The type `module`'s alias `sym` stands for, resolved in the file
    /// that wrote it the first time it is asked for, as a constant's value
    /// is. One that is being resolved already names itself,
    /// through however many others, and each alias of the cycle says so.
    fn alias_body(&mut self, module: usize, sym: Symbol) -> Option<Ty> {
        let alias = self.modules[module].aliases.get(&sym)?;
        if let Some(ty) = alias.ty {
            return Some(ty);
        }
        if let Some(at) = self.alias_stack.iter().position(|&on| on == (module, sym)) {
            let cycle = self.alias_stack[at..].to_vec();
            for (module, name) in cycle {
                self.alias_names_itself(module, name);
            }
            return Some(Types::ERROR);
        }
        let source = self.alias_sources.get(&(module, sym))?.clone();
        let generics = alias.generics.clone();
        // Resolved in its own file, with its own parameters, from wherever
        // it was asked for: a type being resolved elsewhere, which is put
        // back as it was.
        let here = (self.ast, self.current, self.imports.clone());
        let type_params = std::mem::replace(&mut self.type_params, generics);
        let self_ty = self.self_ty.take();
        self.alias_stack.push((module, sym));
        self.enter(module, source.ast, source.imports);
        let ty = self.resolve_ty(source.decl.ty);
        self.enter(here.1, here.0, here.2);
        self.alias_stack.pop();
        self.type_params = type_params;
        self.self_ty = self_ty;
        // An alias a default type argument names, which leaves out another
        // type's argument whose default is not read yet, is read again with
        // that default.
        if self.default_waits {
            return Some(Types::ERROR);
        }
        let alias = Arc::make_mut(&mut self.modules)[module]
            .aliases
            .get_mut(&sym)
            .expect("it was found above");
        // A cycle through it has said so, and it stands for nothing.
        Some(*alias.ty.get_or_insert(ty))
    }

    /// An alias that names itself, through however many others.
    fn alias_names_itself(&mut self, module: usize, name: Symbol) {
        let Some(alias) = self.modules[module].aliases.get(&name) else {
            return;
        };
        if alias.ty.is_some() {
            return;
        }
        let span = alias.span;
        let text = self.text(name).to_string();
        let diagnostic = Diagnostic::error(
            codes::ALIAS,
            format!("`{text}` is a type alias that names itself"),
            span,
            "an endless name",
        )
        .with_note(
            "an alias stands for the type it names, so one that names itself stands for nothing",
        );
        self.report(diagnostic);
        if let Some(alias) = Arc::make_mut(&mut self.modules)[module]
            .aliases
            .get_mut(&name)
        {
            alias.ty = Some(Types::ERROR);
        }
    }

    /// A use of a type alias: the type it names, with the arguments it was
    /// given in place of its parameters.
    pub(super) fn alias_type(
        &mut self,
        sym: Symbol,
        args: &[ast::TypeId],
        span: Span,
    ) -> Option<Ty> {
        self.alias_type_of(self.current, sym, args, span)
    }

    /// The same, for an alias of another module.
    pub(super) fn alias_type_of(
        &mut self,
        module: usize,
        sym: Symbol,
        args: &[ast::TypeId],
        span: Span,
    ) -> Option<Ty> {
        let alias = self.modules[module].aliases.get(&sym)?.clone();
        if module != self.current && !alias.is_pub {
            let name = ast::Name { sym, span };
            self.private_item(module, "type alias", name);
        }
        // Resolved here if nothing has asked for it before.
        let body = self.alias_body(module, sym).unwrap_or(Types::ERROR);
        if args.len() != alias.generics.len() {
            for &arg in args {
                self.resolve_ty(arg);
            }
            self.type_arg_count(sym, &alias.generics, args.len(), span);
            return Some(Types::ERROR);
        }
        if alias.generics.is_empty() {
            return Some(body);
        }
        let mut tys = Vec::new();
        for &arg in args {
            tys.push(self.type_arg(arg));
        }
        Some(self.program.types.subst(body, &tys))
    }

    /// Reads the defaults of this file's structs' and enums' type
    /// parameters that are still to be read. A default that
    /// leaves out an argument of a type whose own default is not read yet
    /// waits for the next round; in the `last`, what still waits can only
    /// rest on itself, and says so. Whether any was read.
    pub(super) fn resolve_defaults(
        &mut self,
        types: &[(Declared<'a>, TypeDef)],
        last: bool,
    ) -> bool {
        let mut read = false;
        for &(item, def) in types {
            let params: &[ast::GenericParam] = match (item.struct_decl(), item.enum_decl()) {
                (Some(s), _) => &s.generics,
                (_, Some(e)) => &e.generics,
                _ => continue,
            };
            for (index, param) in params.iter().enumerate() {
                let Some(written) = param.default else {
                    continue;
                };
                if self.type_generics(def)[index].default != ParamDefault::Pending {
                    continue;
                }
                self.type_params = self.type_generics(def).to_vec();
                let mark = self.diagnostics.len();
                self.default_waits = false;
                let mut ty = self.type_arg(written);
                let waits = std::mem::take(&mut self.default_waits);
                self.type_params.clear();
                let span = self.ast.types[written].span;
                if waits {
                    self.diagnostics.truncate(mark);
                    if !last {
                        continue;
                    }
                    let text = self.text(param.name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::TYPE_PARAMETER_DEFAULT,
                        format!("the default of `{text}` rests on itself"),
                        span,
                        "leaves out a type argument whose default is this one",
                    )
                    .with_note("a default is read in the parameters before it, and so is the default of a type it leaves an argument out of");
                    self.report(diagnostic);
                    ty = Types::ERROR;
                }
                // `K = T` reads the parameters before it, which a use has
                // given by then; one after it is not known yet.
                let later = self.program.types.any(
                    ty,
                    &|kind| matches!(kind, TyKind::Param(p) if p.index as usize >= index),
                );
                if later {
                    let text = self.text(param.name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::TYPE_PARAMETER_DEFAULT,
                        format!("the default of `{text}` names a parameter that is not before it"),
                        span,
                        "names a later parameter",
                    )
                    .with_note("a use leaves out type arguments from the end, so a default is worked out from the ones before it");
                    self.report(diagnostic);
                    ty = Types::ERROR;
                }
                // A default that names no parameter is one type, checked
                // against what the parameter asks here; one that names
                // some is checked where a use gives them.
                if !self.program.types.is_generic(ty) {
                    let declared = self.type_generics(def)[index].clone();
                    self.check_copy(ty, &declared, span);
                    self.check_constraints(ty, &declared, span);
                }
                match def {
                    TypeDef::Struct(id) => {
                        self.program.structs[id].generics[index].default = ParamDefault::Ty(ty)
                    }
                    TypeDef::Enum(id) => {
                        self.program.enums[id].generics[index].default = ParamDefault::Ty(ty)
                    }
                    TypeDef::Builtin(_) => {}
                }
                read = true;
            }
        }
        read
    }

    pub(super) fn resolve_type_bodies(&mut self, types: &[(Declared<'a>, TypeDef)]) {
        for &(item, def) in types {
            self.type_params = self.type_generics(def).to_vec();
            match (item, def) {
                (declared, TypeDef::Struct(id)) if declared.struct_decl().is_some() => {
                    let s = declared.struct_decl().expect("just asked");
                    let fields = self.fields(&s.fields, s.is_extern, s.is_view);
                    // C promises a layout for the types it can write, and
                    // knows nothing of a type parameter.
                    // A union belongs to C, and holds what C can write.
                    if s.is_union && fields.is_empty() {
                        let diagnostic = Diagnostic::error(
                            codes::NOT_C_COMPATIBLE,
                            "a union needs a field",
                            s.name.span,
                            "no fields",
                        )
                        .with_note(
                            "a union is one of its fields at a time, and there is nothing to be one of",
                        );
                        self.report(diagnostic);
                    }
                    if s.is_extern {
                        if let Some(first) = s.generics.first() {
                            let diagnostic = Diagnostic::error(
                                codes::CANNOT_BE_GENERIC,
                                if s.is_union {
                                    "an `extern union` cannot be generic"
                                } else {
                                    "an `extern struct` cannot be generic"
                                },
                                first.span,
                                "a type parameter",
                            )
                            .with_note("it is a layout C writes too, and C has no type parameters");
                            self.report(diagnostic);
                        }
                        for (field, declared) in fields.iter().zip(&s.fields) {
                            let span = self.ast.types[declared.ty].span;
                            self.check_c_field(field.ty, span, s.is_union);
                        }
                    }
                    self.program.structs[id].fields = fields;
                    // `@opaque`: C knows where the fields are, so each one
                    // is read and written through C.
                    if self.program.structs[id].is_opaque {
                        self.declare_field_accessors(id);
                    }
                }
                (declared, TypeDef::Enum(id)) if declared.enum_decl().is_some() => {
                    let e = declared.enum_decl().expect("just asked");
                    let mut seen = FxHashMap::default();
                    let mut variants = Vec::new();
                    for v in &e.variants {
                        match seen.get(&v.name.sym) {
                            Some(&first) => self.duplicate(v.name, first),
                            None => {
                                seen.insert(v.name.sym, v.name.span);
                            }
                        }
                        // A `view enum`'s variants borrow as a view's
                        // fields do.
                        let fields = self.fields(&v.fields, true, e.is_view);
                        variants.push(VariantDef {
                            name: v.name.sym,
                            fields,
                            span: v.name.span,
                        });
                    }
                    self.program.enums[id].variants = variants;
                }
                _ => unreachable!("collect_types pairs items with their kind"),
            }
        }
        self.type_params.clear();
    }

    /// The fields of a struct or a variant. `exported` is for the ones
    /// that are public whatever they say: an `extern struct` is C's
    /// layout, and a variant's fields are exported with its enum.
    /// A view's fields may borrow: hold a `str`,
    /// a view, or a `&` reference.
    pub(super) fn fields(
        &mut self,
        fields: &[ast::Field],
        exported: bool,
        view: bool,
    ) -> Vec<FieldDef> {
        let mut seen = FxHashMap::default();
        let mut defs = Vec::new();
        for f in fields {
            match seen.get(&f.name.sym) {
                Some(&first) => self.duplicate(f.name, first),
                None => {
                    seen.insert(f.name.sym, f.name.span);
                }
            }
            let mut ty = self.resolve_ty(f.ty);
            let lent = view && matches!(self.kind(ty), TyKind::Ref(_, crate::RefKind::Shared));
            let refused = if lent {
                // What the reference refers to is checked as a parameter's
                // would be.
                self.no_never(ty, f.ty, "a field")
            } else if view && matches!(self.kind(ty), TyKind::Ref(_, crate::RefKind::Var)) {
                self.var_in_view(f.ty)
            } else {
                self.no_var_ref(ty, f.ty, "a field")
                    || self.no_never(ty, f.ty, "a field")
                    || self.bare_dyn(ty, f.ty)
                    || self.bare_slice(ty, f.ty, false)
                    || (!view && self.stored_field(ty, f.ty, exported))
            };
            if refused {
                ty = Types::ERROR;
            }
            defs.push(FieldDef {
                // `pub` exports a field; an `extern struct` is C's layout,
                // and C hides nothing.
                is_pub: f.is_pub || exported,
                // C writes what it lays out, and a variant's fields are
                // the enum's.
                is_var: f.is_var || exported,
                name: f.name.sym,
                ty,
                span: f.span,
                // Checked once every type's fields are known.
                default: None,
            });
        }
        defs
    }

    /// A field's default, checked once every type's fields are known: a
    /// constant of the field's type, which each literal that leaves the
    /// field out copies.
    /// The defaults of these functions' parameters, once every function is
    /// declared: each with the function's type parameters in scope, since
    /// one that is code is generic as the function is.
    pub(super) fn check_param_defaults(&mut self, defaults: &[(FnId, usize, ast::ExprId)]) {
        for &(id, index, value) in defaults {
            let ty = self.program.fns[id].params[index].ty;
            if self.has_error(ty) {
                continue;
            }
            self.type_params = self.program.fns[id].generics.clone();
            let default = self.default_of(value, ty);
            self.program.fns[id].params[index].default = default;
        }
        self.type_params.clear();
    }

    /// The defaults of the fields of these types, once every function is
    /// declared, since one may be code that calls one.
    /// The defaults of `def`'s fields, checked now where they are not yet:
    /// in the file that declares them, from wherever they were asked for,
    /// which is put back as it was. A literal asks before it reads them,
    /// so one in another type's default finds them however the two are
    /// ordered.
    pub(super) fn ensure_defaults(&mut self, def: TypeDef) {
        let Some(source) = self.default_sources.remove(&def) else {
            return;
        };
        let here = (self.ast, self.current, self.imports.clone());
        let files = (
            std::mem::replace(&mut self.lambdas, source.lambdas),
            std::mem::replace(&mut self.lambda_envs, source.lambda_envs),
            std::mem::replace(&mut self.generators, source.generators),
        );
        let type_params = std::mem::take(&mut self.type_params);
        let self_ty = self.self_ty.take();
        self.enter(source.module, source.ast, source.imports);
        self.check_field_defaults(&[(source.item, def)]);
        self.enter(here.1, here.0, here.2);
        (self.lambdas, self.lambda_envs, self.generators) = files;
        self.type_params = type_params;
        self.self_ty = self_ty;
    }

    pub(super) fn check_field_defaults(&mut self, types: &[(Declared<'a>, TypeDef)]) {
        for &(item, def) in types {
            if let (Some(e), TypeDef::Enum(id)) = (item.enum_decl(), def) {
                self.check_variant_defaults(e, id);
                continue;
            }
            let (Some(s), TypeDef::Struct(id)) = (item.struct_decl(), def) else {
                continue;
            };
            if !s.fields.iter().any(|f| f.default.is_some()) {
                continue;
            }
            self.type_params = self.type_generics(def).to_vec();
            for (index, f) in s.fields.iter().enumerate() {
                let Some(value) = f.default else {
                    continue;
                };
                let ty = self.program.structs[id].fields[index].ty;
                let refusal = if self.program.structs[id].is_union {
                    Some("a union's field has no default")
                } else if self.program.structs[id].is_opaque {
                    Some("an `@opaque` struct's field has no default")
                } else {
                    None
                };
                if let Some(message) = refusal {
                    let span = self.ast.exprs[value].span;
                    let diagnostic =
                        Diagnostic::error(codes::INVALID_DEFAULT, message, span, "a default")
                            .with_secondary(f.name.span, "for this field");
                    self.report(diagnostic);
                    continue;
                }
                if self.has_error(ty) {
                    continue;
                }
                let default = self.default_of(value, ty);
                self.program.structs[id].fields[index].default = default;
            }
            self.type_params.clear();
        }
    }

    /// The defaults of an enum's variant fields, which each variant built
    /// without the field copies, or calls.
    fn check_variant_defaults(&mut self, e: &'a ast::EnumDecl, id: EnumId) {
        if !e
            .variants
            .iter()
            .any(|v| v.fields.iter().any(|f| f.default.is_some()))
        {
            return;
        }
        self.type_params = self.type_generics(TypeDef::Enum(id)).to_vec();
        for (v, variant) in e.variants.iter().enumerate() {
            for (index, f) in variant.fields.iter().enumerate() {
                let Some(value) = f.default else {
                    continue;
                };
                let ty = self.program.enums[id].variants[v].fields[index].ty;
                if self.has_error(ty) {
                    continue;
                }
                let default = self.default_of(value, ty);
                self.program.enums[id].variants[v].fields[index].default = default;
            }
        }
        self.type_params.clear();
    }

    /// A type this file imported by name.
    pub(super) fn imported_type(&mut self, name: ast::Name) -> Option<TypeDef> {
        let ImportedItem::Item(module, item) = self.imported(name)? else {
            return None;
        };
        self.modules[module]
            .types
            .get(&item.sym)
            .map(|&(def, _)| def)
    }

    /// Whether a field may be named here: the module that declares the
    /// struct sees all of them, another only what is `pub`.
    pub(super) fn field_visible(&self, id: StructId, index: usize) -> bool {
        let def = &self.program.structs[id];
        def.module as usize == self.current || def.fields[index].is_pub
    }

    /// Whether a field may be written here: the module that declares the
    /// struct writes all of them, another only what is `pub var`.
    pub(super) fn field_settable(&self, id: StructId, index: usize) -> bool {
        let def = &self.program.structs[id];
        def.module as usize == self.current || def.fields[index].is_var
    }

    /// Reports a field that another module may read and not write.
    pub(super) fn readonly_field(&mut self, id: StructId, index: usize, at: Span, what: &str) {
        let type_name = self.text(self.program.structs[id].name).to_string();
        let field = self
            .text(self.program.structs[id].fields[index].name)
            .to_string();
        let declared = self.program.structs[id].fields[index].span;
        let diagnostic = Diagnostic::error(
            codes::PRIVATE_ITEM,
            format!("`{field}` of `{type_name}` is read here, not written"),
            at,
            what,
        )
        .with_secondary(declared, "declared `pub`, which exports the reading")
        .with_help(format!(
            "write `pub var {field}: …` where it is declared, or ask the type to change it"
        ))
        .with_note(
            "a `pub` field is read by any module and written by the one that declares it; `pub var` is the one anyone writes",
        );
        self.report(diagnostic);
    }

    /// Reports a field that another module may not name.
    pub(super) fn private_field(&mut self, id: StructId, index: usize, at: Span) {
        let type_name = self.text(self.program.structs[id].name).to_string();
        let field = self
            .text(self.program.structs[id].fields[index].name)
            .to_string();
        let declared = self.program.structs[id].fields[index].span;
        let diagnostic = Diagnostic::error(
            codes::PRIVATE_ITEM,
            format!("`{field}` of `{type_name}` is not exported"),
            at,
            "not `pub`",
        )
        .with_secondary(declared, "declared here")
        .with_help(format!("write `pub {field}: …` where it is declared, or reach it through a method"))
        .with_note(
            "a struct's fields belong to the module that declares it unless they say `pub`, so a type can keep what it promises",
        );
        self.report(diagnostic);
    }

    /// A type named through its module: `list::Node`.
    pub(super) fn named_type(
        &mut self,
        module: usize,
        name: ast::Name,
        args: &[ast::TypeId],
        span: Span,
        pointee: bool,
    ) -> Ty {
        match self.modules[module].types.get(&name.sym).copied() {
            Some((TypeDef::Builtin(_), _)) => Types::ERROR,
            Some((def @ TypeDef::Struct(s), _)) => {
                if !self.visible(module, self.program.structs[s].is_pub) {
                    self.private_item(module, "struct", name);
                }
                self.applied_type(def, args, span, pointee)
            }
            Some((def @ TypeDef::Enum(e), _)) => {
                if !self.visible(module, self.program.enums[e].is_pub) {
                    self.private_item(module, "enum", name);
                }
                self.applied_type(def, args, span, pointee)
            }
            None => {
                // A type alias another module declared: `disk::Seen`.
                if let Some(ty) = self.alias_type_of(module, name.sym, args, span) {
                    return ty;
                }
                // A C type another module declared, which is only ever seen
                // through `ptr`.
                if let Some(&id) = self.modules[module].opaques.get(&name.sym) {
                    if !self.visible(module, self.program.opaques[id].is_pub) {
                        self.private_item(module, "C type", name);
                    }
                    if !args.is_empty() {
                        for &arg in args {
                            self.resolve_ty(arg);
                        }
                        self.type_arg_count(name.sym, &[], args.len(), span);
                        return Types::ERROR;
                    }
                    if !pointee {
                        self.opaque_as_value(name.sym, span);
                        return Types::ERROR;
                    }
                    return self.intern(TyKind::Opaque(id));
                }
                let text = self.text(name.sym).to_string();
                let path = self.modules[module].path.clone();
                let candidates: Vec<&str> = self.modules[module]
                    .types
                    .keys()
                    .map(|&s| self.text(s))
                    .chain(self.modules[module].opaques.keys().map(|&s| self.text(s)))
                    .collect();
                let mut diagnostic = Diagnostic::error(
                    codes::UNKNOWN_TYPE,
                    format!("cannot find type `{text}` in module `{path}`"),
                    name.span,
                    "unknown type",
                );
                if let Some(similar) = suggest(&text, candidates) {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `{similar}`?"),
                        [Edit::replace(name.span, similar)],
                    );
                }
                self.report(diagnostic);
                Types::ERROR
            }
        }
    }

    pub(super) fn collect_fns(&mut self) -> Vec<BodyWork> {
        let ast = self.ast;
        let mut bodies = Vec::new();
        for item in &ast.items {
            match item {
                ast::Item::Fn(f) => {
                    let id = self.declare_fn(&f.sig, &f.annotations, false, f.is_pub, None);
                    // An `@intrinsic` has no body to check: the compiler
                    // writes it.
                    if let Some(body) = f.body {
                        bodies.push(BodyWork {
                            body,
                            id,
                            span: f.span,
                        });
                    }
                }
                ast::Item::Extern(e) => {
                    // `@link("sqlite3", …)` names the libraries its
                    // declarations come from, in the order the linker is
                    // given them.
                    let annotations = self.annotations(&e.annotations, annotations::Target::Extern);
                    for &(library, span) in &annotations.link {
                        let name = self.text(library).to_string();
                        // It becomes `-lname` on the linker's command line, so
                        // it is a name, not a path.
                        if name.contains(['/', '\\', ' ']) {
                            let diagnostic = Diagnostic::error(
                                codes::ANNOTATION,
                                format!("`{name}` is not a library name"),
                                span,
                                "a path, not a name",
                            )
                            .with_note(
                                "`@link(\"sqlite3\")` links `-lsqlite3`; a library somewhere else is found with the linker's own search paths",
                            );
                            self.report(diagnostic);
                        } else if !self.program.libraries.contains(&name) {
                            self.program.libraries.push(name);
                        }
                    }
                    // How the module's C is built: where its headers are,
                    // and what an Apple target links.
                    for &(framework, _) in &annotations.framework {
                        let name = self.text(framework).to_string();
                        if !self.program.frameworks.contains(&name) {
                            self.program.frameworks.push(name);
                        }
                    }
                    for &(dir, span) in &annotations.include {
                        let path = self.text(dir).to_string();
                        // Where the module is on this machine is the
                        // command line's to say.
                        if path.starts_with(['/', '\\']) || path.get(1..2) == Some(":") {
                            let diagnostic = Diagnostic::error(
                                codes::ANNOTATION,
                                format!("`{path}` is not a directory in the module"),
                                span,
                                "an absolute path",
                            )
                            .with_note(
                                "`@include` names a directory relative to the module's own; where something is on this machine is given with `-I`",
                            );
                            self.report(diagnostic);
                        } else {
                            let entry = (self.current, path);
                            if !self.program.c_includes.contains(&entry) {
                                self.program.c_includes.push(entry);
                            }
                        }
                    }
                    // `@source("src/rcore.c")`: a C file of the module,
                    // below its own directory, compiled with the program;
                    // `@prefix("build.h")` is put before each of the
                    // module's C files.
                    for (path, _) in self.module_paths(&annotations.source, "@source") {
                        let entry = (self.current, path, annotations.source_language);
                        if !self.program.c_sources.contains(&entry) {
                            self.program.c_sources.push(entry);
                        }
                    }
                    for (path, _) in self.module_paths(
                        &annotations.prefix.into_iter().collect::<Vec<_>>(),
                        "@prefix",
                    ) {
                        let entry = (self.current, path);
                        if !self.program.c_prefix.contains(&entry) {
                            self.program.c_prefix.push(entry);
                        }
                    }
                    // `@define("PLATFORM_DESKTOP")`: what the module's C is
                    // compiled with defined.
                    for define in self.c_defines(&annotations.define) {
                        let entry = (self.current, define);
                        if !self.program.c_defines.contains(&entry) {
                            self.program.c_defines.push(entry);
                        }
                    }
                    for declared in &e.globals {
                        self.declare_global(declared);
                    }
                    for declared in &e.fns {
                        // An extern declaration names a C symbol, which every
                        // module shares. `pub` exports the declaration, so
                        // that a module of bindings is the unit people
                        // share.
                        self.declare_fn(
                            &declared.sig,
                            &declared.annotations,
                            true,
                            declared.is_pub,
                            None,
                        );
                    }
                }
                // A type's methods belong to it, not to the module.
                ast::Item::Struct(ast::StructDecl { name, methods, .. })
                | ast::Item::Enum(ast::EnumDecl { name, methods, .. }) => {
                    let Some(&(owner, _)) = self.types().get(&name.sym) else {
                        continue;
                    };
                    let type_params = self.type_generics(owner).to_vec();
                    self.declare_methods(owner, &type_params, methods, &mut bodies);
                }
                // The same methods, written outside the declaration, and
                // the implementation of an interface.
                ast::Item::Extend(block) => {
                    self.annotations(&block.annotations, annotations::Target::Extend);
                    // `extend Iterator<T: Ord> { … }`: methods every
                    // implementer has where its types meet the condition.
                    if let Some(interface) = self.extended_interface(block) {
                        self.declare_extension(block, interface, &mut bodies);
                        continue;
                    }
                    self.derived = block.derived.is_some();
                    if let Some((owner, type_params)) = self.impl_target(block) {
                        match block.interface {
                            None => self.declare_methods(
                                owner,
                                &type_params,
                                &block.methods,
                                &mut bodies,
                            ),
                            Some(interface) => self.declare_impl(
                                block,
                                interface,
                                owner,
                                &type_params,
                                &mut bodies,
                            ),
                        }
                    }
                    self.derived = false;
                }
                _ => {}
            }
        }
        bodies
    }

    /// The methods of a type, declared with the type's parameters under the
    /// names this declaration gives them.
    pub(super) fn declare_methods(
        &mut self,
        owner: TypeDef,
        type_params: &[GenericParamDef],
        methods: &'a [ast::FnDecl],
        bodies: &mut Vec<BodyWork>,
    ) {
        self.declare_methods_of(owner, type_params, methods, bodies, false, None);
    }

    /// The same, for the methods of an `extend` of an interface that takes
    /// types, where one name may be declared once per implementation.
    pub(super) fn declare_methods_of(
        &mut self,
        owner: TypeDef,
        type_params: &[GenericParamDef],
        methods: &'a [ast::FnDecl],
        bodies: &mut Vec<BodyWork>,
        overloaded: bool,
        // A conformance's methods are as visible as the interface, whatever
        // they say; `None` leaves each method its own word.
        visible: Option<bool>,
    ) {
        for method in methods {
            let (receiver, keyword) = method.receiver.expect("a method has a receiver");
            // `lend fn` is a lending pair written once: the reading half as
            // written, and the writing half from the same body, checked
            // again with `self` a `&var Self`.
            let halves: &[Receiver] = match receiver {
                Receiver::Lend => &[Receiver::Read, Receiver::Var],
                Receiver::Read => &[Receiver::Read],
                Receiver::Var => &[Receiver::Var],
                Receiver::Move => &[Receiver::Move],
                Receiver::Static => &[Receiver::Static],
            };
            for &half in halves {
                let member = Member {
                    receiver: half,
                    keyword,
                    owner: MemberOwner::Type(owner),
                    type_params: type_params.to_vec(),
                    overloaded,
                    lent: receiver == Receiver::Lend && half == Receiver::Var,
                };
                let id = self.declare_fn(
                    &method.sig,
                    &method.annotations,
                    false,
                    visible.unwrap_or(method.is_pub),
                    Some(member),
                );
                let both = receiver != Receiver::Lend || self.lent_both_ways(id, half, method);
                if receiver == Receiver::Lend && half == Receiver::Var {
                    self.program.lent_halves.insert(id);
                }
                if let Some(body) = method.body {
                    bodies.push(BodyWork {
                        body,
                        id,
                        span: method.span,
                    });
                }
                if !both {
                    break;
                }
            }
        }
    }

    /// One half of a `lend fn`. Its result is a reference,
    /// `&T` as written, which the writing half lends as `&var T`; a result
    /// that is not one lends nothing, and is reported once, on the reading
    /// half, which is all that is then declared.
    fn lent_both_ways(&mut self, id: FnId, half: Receiver, method: &ast::FnDecl) -> bool {
        let ret = self.program.fns[id].ret;
        match self.kind(ret) {
            TyKind::Ref(_, kind) if (kind == crate::RefKind::Var) == (half == Receiver::Var) => {
                true
            }
            _ if self.is_poisoned(ret) => true,
            kind => {
                let span = method
                    .sig
                    .ret
                    .map_or(method.sig.name.span, |t| self.ast.types[t].span);
                let written_var = matches!(kind, TyKind::Ref(_, crate::RefKind::Var));
                let diagnostic = Diagnostic::error(
                    codes::IMPL_METHODS,
                    "a `lend fn` lends a place, and its result is `&T`",
                    span,
                    match written_var {
                        true => "written `&var`",
                        false => "not a `&` reference",
                    },
                )
                .with_note(
                    "`lend fn` declares a projection's reading half and its writing half from one body: the result is `&T` where the call reads and `&var T` where it writes",
                );
                let diagnostic = match written_var {
                    true => diagnostic
                        .with_help("write `&`: the writing half lends it as `&var` by itself"),
                    false => diagnostic,
                };
                self.report(diagnostic);
                // Reported once: a result written `&var` is read as the `&`
                // it should be, so the body is checked as the reading half
                // it is.
                if let TyKind::Ref(inner, crate::RefKind::Var) = kind {
                    self.program.fns[id].ret =
                        self.intern(TyKind::Ref(inner, crate::RefKind::Shared));
                }
                false
            }
        }
    }

    /// The type an `extend` block is for, and its parameters under the names
    /// the block writes.
    fn impl_target(
        &mut self,
        block: &'a ast::ExtendBlock,
    ) -> Option<(TypeDef, Vec<GenericParamDef>)> {
        let name = *block.path.last().expect("a path has segments");
        let text = self.text(name.sym).to_string();
        // What `@derive` wrote is for the type that asked, whatever its
        // name finds here; a declaration that was refused, and reported,
        // asked for nothing.
        if let Some(at) = block.derived {
            let owner = *self.derive_owners.get(&at)?;
            return self.target_params(block, owner, name, &text);
        }
        // A built-in type: `extend [T]`, `extend str`, `extend i64` — the
        // prelude's to give methods to.
        if let Some(elem) = block.slice_of {
            // The element, with what the block asks of it.
            let param = block
                .generics
                .first()
                .cloned()
                .unwrap_or_else(|| ast::GenericParam {
                    decided: None,
                    name: elem,
                    bounds: Vec::new(),
                    default: None,
                    span: elem.span,
                });
            let generics = self.generic_params(&[param]);
            return self
                .builtin_owner(block, BuiltinOwner::Slice, generics.clone(), elem.span)
                .map(|owner| (owner, generics));
        }
        // `extend Slots<T>`: the prelude's storage, which takes its element as
        // a parameter.
        if block.path.len() == 1 && self.text(name.sym) == "Slots" {
            let generics = self.generic_params(&block.generics);
            return self
                .builtin_owner(block, BuiltinOwner::Slots, generics.clone(), name.span)
                .map(|owner| (owner, generics));
        }
        if block.path.len() == 1
            && let Some(ty) = self.builtin(name.sym)
            && let Some(builtin) = BuiltinOwner::of(self.kind(ty))
        {
            if let Some(first) = block.generics.first() {
                let diagnostic = Diagnostic::error(
                    codes::IMPL_TARGET,
                    format!("`{text}` has no type parameters"),
                    first.span,
                    "type parameters",
                );
                self.report(diagnostic);
                return None;
            }
            return self
                .builtin_owner(block, builtin, Vec::new(), name.span)
                .map(|owner| (owner, Vec::new()));
        }
        let elsewhere = |span: Span| {
            Diagnostic::error(
                codes::IMPL_TARGET,
                format!("`{text}` is declared in another module"),
                span,
                "not a type of this module",
            )
            .with_note("only the module that declares a type may add methods to it")
        };
        // A path names another module's type.
        if let [first, ..] = *block.path.as_slice()
            && block.path.len() > 1
        {
            let diagnostic = elsewhere(first.span.to(name.span));
            self.report(diagnostic);
            return None;
        }
        let Some(&(owner, _)) = self.types().get(&name.sym) else {
            // `extend Id { … }`, where `Id` is another name for a type:
            // the methods belong to the type itself.
            if self.aliases().contains_key(&name.sym) {
                let diagnostic = Diagnostic::error(
                    codes::IMPL_TARGET,
                    format!("`{text}` is another name for a type, not a type"),
                    name.span,
                    "an alias",
                )
                .with_help("write the block for the type the alias names")
                .with_note(
                    "a method belongs to the type it is declared in, and an alias declares nothing",
                );
                self.report(diagnostic);
                return None;
            }
            let diagnostic = if self.imported(name).is_some() {
                elsewhere(name.span)
            } else {
                self.unknown_name(name.sym, name.span)
            };
            self.report(diagnostic);
            return None;
        };
        self.program.names.push((name.span, Named::Owner(owner)));
        self.target_params(block, owner, name, &text)
    }

    /// The parameters of `owner`, a type of this module, under the names an
    /// `extend` block of it writes.
    fn target_params(
        &mut self,
        block: &'a ast::ExtendBlock,
        owner: TypeDef,
        name: ast::Name,
        text: &str,
    ) -> Option<(TypeDef, Vec<GenericParamDef>)> {
        let declared = self.type_generics(owner).to_vec();
        let written = &block.generics;
        if written.len() != declared.len() {
            let names: Vec<String> = declared
                .iter()
                .map(|p| self.text(p.name).to_string())
                .collect();
            let expected = if declared.is_empty() {
                format!("`{text}` has no type parameters")
            } else {
                format!(
                    "`{text}` has {}: `<{}>`",
                    plural(declared.len(), "type parameter", "type parameters"),
                    names.join(", ")
                )
            };
            let written_span = written
                .last()
                .map_or(name.span, |last| name.span.to(last.span));
            let diagnostic = Diagnostic::error(
                codes::IMPL_TARGET,
                format!("`extend` of `{text}` must name its type parameters"),
                written_span,
                expected,
            )
            .with_note("an `extend` block binds the type's parameters under the names it writes, one for each");
            // A default is for a use of the type, which may leave it out.
            let diagnostic = match declared.iter().any(|p| p.default != ParamDefault::None) {
                true => diagnostic.with_note("a default type argument is for a use of the type; the block names the parameter it stands for too"),
                false => diagnostic,
            };
            self.report(diagnostic);
            return None;
        }
        // The names are binders for the type's own parameters, and a
        // constraint on one says when the block applies.
        let written_params = self.generic_params(written);
        // A conformance's condition is an interface: whether a type owns
        // memory is worked out as types are completed, and an
        // implementation is chosen before that. A block of
        // methods chooses nothing, and `copy` on a method's parameters is
        // checked where it is called, so it may ask for it.
        for (param, condition) in written.iter().zip(&written_params) {
            if condition.copy
                && block.interface.is_some()
                && !self.type_generics(owner)[0..1].is_empty()
            {
                let copy = param
                    .bounds
                    .iter()
                    .find(|b| self.text(b.name.sym) == "copy")
                    .map(|b| b.span);
                if let Some(span) = copy {
                    let diagnostic = Diagnostic::error(
                        codes::IMPL_TARGET,
                        "`copy` is not a condition an implementation of an interface may ask for",
                        span,
                        "not a condition",
                    )
                    .with_note(
                        "an implementation's condition is an interface the argument implements; `copy` is worked out as types are completed, and which implementation applies is decided before that. A block of methods may ask for it",
                    );
                    self.report(diagnostic);
                }
            }
        }
        let mut params = Vec::new();
        for ((param, declared), mut condition) in written.iter().zip(&declared).zip(written_params)
        {
            // The type's own constraints hold wherever it is written, so
            // they are part of what the block may rely on.
            for &interface in &declared.interfaces {
                if !condition.interfaces.contains(&interface) {
                    condition.interfaces.push(interface);
                }
            }
            params.push(GenericParamDef {
                decided: false,
                // The type's parameter, under the name the block writes.
                name: declared.name,
                written: param.name.sym,
                interfaces: condition.interfaces,
                copy: condition.copy || declared.copy,
                // The type's default, which `Pool<i64>::of()` leaves out
                // as `Pool<i64>` does; the block itself names every one.
                default: declared.default,
                span: param.name.span,
            });
        }
        Some((owner, params))
    }

    /// The pair of functions that read and write each field of an
    /// `@opaque` struct, whose bodies the compiler writes in C because only
    /// C knows where the fields are.
    fn declare_field_accessors(&mut self, id: StructId) {
        let def = &self.program.structs[id];
        let (fields, header, is_pub) = (def.fields.clone(), def.header, def.is_pub);
        let owner_ty = self.intern(TyKind::Struct(id, crate::TyList::EMPTY));
        let pointer = self.intern(TyKind::Ptr(owner_ty));
        let mut accessors = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            let owner = ParamDef {
                name: field.name,
                name_span: field.span,
                ty: pointer,
                span: field.span,
                default: None,
            };
            let accessor = |lowerer: &mut Self, params: Vec<ParamDef>, ret: Ty| {
                lowerer.program.fns.alloc(FnDef {
                    name: field.name,
                    name_span: field.span,
                    symbol: None,
                    receiver: None,
                    owner: None,
                    interface: None,
                    generics: Vec::new(),
                    instance_of: None,
                    params,
                    ret,
                    ret_span: None,
                    is_extern: true,
                    exports_c: false,
                    header,
                    accesses: Some(Access::Field(id, index as u32)),
                    is_variadic: false,
                    variadic_of: None,
                    lends_from: None,
                    is_lambda: false,
                    generator: None,
                    is_tailrec: false,
                    is_test: false,
                    is_inline: false,
                    generated: None,
                    intrinsic: None,
                    body: None,
                    projects: None,
                    compile_time: false,
                    module: lowerer.current as u32,
                    is_pub,
                    span: field.span,
                })
            };
            let get = accessor(self, vec![owner.clone()], field.ty);
            let value = ParamDef {
                ty: field.ty,
                ..owner.clone()
            };
            let set = accessor(self, vec![owner, value], Types::UNIT);
            accessors.push((get, set));
        }
        self.program.structs[id].accessors = accessors;
    }

    /// `val optind: c_int` or `var errno: c_int` in an extern block: a
    /// variable C owns. Wip reads and writes it
    /// through two functions the compiler declares for it, whose bodies it
    /// writes in C — because the name may be a macro rather than a symbol,
    /// as `errno` is, and only C knows which.
    fn declare_global(&mut self, declared: &'a ast::ExternGlobal) {
        let annotations = self.annotations(&declared.annotations, annotations::Target::ExternVar);
        // Its own `@header` says C has to reach it — `errno` is a macro —
        // so it is read and written through C the compiler writes. Without
        // one it is a symbol, which the compiler loads and stores itself.
        let header = annotations.header.map(|(h, _)| h);
        let symbol = self.c_symbol(annotations.symbol);
        let name = declared.name;
        let span = self.ast.types[declared.ty].span;
        let mut ty = self.resolve_ty(declared.ty);
        // C hands over a copy of what it holds, so the type must be one
        // that travels: a scalar, a `cstring` or a pointer.
        if !self.c_compatible(ty, false) || matches!(self.kind(ty), TyKind::Str) {
            if !self.is_poisoned(ty) {
                let diagnostic = Diagnostic::error(
                    codes::NOT_C_COMPATIBLE,
                    format!("{} cannot be a variable Wip reads from C", self.ty_name(ty)),
                    span,
                    "not a value C hands over",
                )
                .with_note(
                    "a variable is read as a copy of what C holds, so it is one of the scalars, a `cstring` or a `ptr<T>`",
                );
                self.report(diagnostic);
            }
            ty = Types::ERROR;
        }
        let accessor = |lowerer: &mut Self, params: Vec<ParamDef>, ret: Ty| {
            lowerer.program.fns.alloc(FnDef {
                name: name.sym,
                name_span: name.span,
                symbol,
                receiver: None,
                owner: None,
                interface: None,
                generics: Vec::new(),
                instance_of: None,
                params,
                ret,
                ret_span: None,
                is_extern: true,
                exports_c: false,
                header,
                accesses: None,
                is_variadic: false,
                variadic_of: None,
                lends_from: None,
                is_lambda: false,
                generator: None,
                is_tailrec: false,
                is_test: false,
                is_inline: false,
                generated: None,
                intrinsic: None,
                body: None,
                projects: None,
                compile_time: false,
                module: lowerer.current as u32,
                is_pub: declared.is_pub,
                span: declared.span,
            })
        };
        let getter = accessor(self, Vec::new(), ty);
        let setter = declared.is_mut.then(|| {
            let param = ParamDef {
                name: name.sym,
                name_span: name.span,
                ty,
                span: declared.span,
                default: None,
            };
            accessor(self, vec![param], Types::UNIT)
        });
        let id = self.program.globals.alloc(GlobalDef {
            name: name.sym,
            symbol,
            ty,
            is_mut: declared.is_mut,
            header,
            getter,
            setter,
            module: self.current as u32,
            is_pub: declared.is_pub,
            span: name.span,
        });
        self.program.fns[getter].accesses = Some(Access::Global(id));
        if let Some(setter) = setter {
            self.program.fns[setter].accesses = Some(Access::Global(id));
        }
        // It takes a name in the module, as a constant does.
        if self.prelude_name(name, "a variable named") {
            // Reported; it keeps its own place in this module's scope.
        } else if let Some(&first) = self.globals().get(&name.sym) {
            let previous = self.program.globals[first].span;
            self.duplicate(name, previous);
        } else if let Some(&first) = self.consts().get(&name.sym) {
            let previous = self.program.consts[first].span;
            self.duplicate(name, previous);
        } else {
            self.globals_mut().insert(name.sym, id);
        }
    }

    /// `@symbol("sqlite3_open")`: the name C knows, which must be one C
    /// could have written.
    fn c_symbol(&mut self, written: Option<(Symbol, Span)>) -> Option<Symbol> {
        let (symbol, span) = written?;
        let text = self.text(symbol).to_string();
        let identifier = !text.is_empty()
            && !text.starts_with(|c: char| c.is_ascii_digit())
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
        if !identifier {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                format!("`{text}` is not a name C could have written"),
                span,
                "not a C name",
            )
            .with_note(
                "the compiler writes this name into the C it generates, so it is letters, digits and `_`",
            );
            self.report(diagnostic);
            return None;
        }
        Some(symbol)
    }

    /// Declares a function or a method from its signature: its type
    /// parameters, its parameters and result read with them in scope, what
    /// its annotations say, and its name where the module or its type keeps
    /// it. Its body is checked later, once every function is declared.
    pub(super) fn declare_fn(
        &mut self,
        sig: &ast::FnSig,
        annotations: &[ast::Annotation],
        is_extern: bool,
        is_pub: bool,
        member: Option<Member>,
    ) -> FnId {
        // A C declaration takes `@symbol`, a Wip function does not, and
        // the other way about.
        let target = if is_extern {
            annotations::Target::ExternFn
        } else {
            annotations::Target::Fn
        };
        let annotations = self.annotations(annotations, target);
        let ast = self.ast;
        let generics = self.fn_generics(sig, member.as_ref(), is_extern);
        // The signature is read with the function's type parameters, and a
        // method's `Self`, in scope.
        self.type_params = generics.clone();
        self.self_ty = member.as_ref().map(|member| self.member_self_ty(member));
        let pins = self.decided_pins(&generics);
        self.program.types.set_pins(pins);
        let (params, deferred) = self.fn_params(sig, member.as_ref(), is_extern);
        let (ret, yields, generator_args) = self.fn_result(sig, member.as_ref(), is_extern);
        self.program.types.set_pins(Vec::new());
        self.type_params.clear();
        self.self_ty = None;
        let intrinsic = if annotations.intrinsic {
            self.fn_intrinsic(sig, member.as_ref())
        } else {
            None
        };
        self.check_variadic(sig, is_extern);
        let lends_from = self.lends_from(sig, &params);
        let exported = self.fn_exported(&annotations, &params, sig, ret);
        let def = FnDef {
            name: sig.name.sym,
            name_span: sig.name.span,
            symbol: self.c_symbol(annotations.symbol),
            receiver: member.as_ref().map(|m| m.receiver),
            owner: member.as_ref().and_then(|m| match m.owner {
                MemberOwner::Type(owner) => Some(owner),
                MemberOwner::Interface(_) => None,
            }),
            interface: member.as_ref().and_then(|m| match m.owner {
                MemberOwner::Interface(id) => Some(id),
                MemberOwner::Type(_) => None,
            }),
            generics,
            instance_of: None,
            params,
            ret,
            ret_span: sig.ret.map(|t| ast.types[t].span),
            is_extern,
            exports_c: exported,
            // A declaration's own `@header` says the header defines it — a
            // macro or a `static inline`, which no symbol answers to — so the
            // call goes through C that includes that header. A block's
            // `@header` says where its types are, and does not make a call
            // one C has to write.
            header: if is_extern {
                annotations.header.map(|(h, _)| h)
            } else {
                None
            },
            accesses: None,
            is_variadic: sig.variadic.is_some(),
            variadic_of: None,
            lends_from,
            is_lambda: false,
            generator: None,
            is_tailrec: annotations.tailrec,
            is_test: annotations.test.is_some(),
            is_inline: annotations.inline.is_some(),
            generated: None,
            intrinsic,
            body: None,
            // An intrinsic projection has no body to read what it lends
            // from: `slots.slot(index)` lends a slot of its receiver.
            projects: match intrinsic {
                Some(Intrinsic::SlotsSlot | Intrinsic::OptionAssumed) => Some(Lent::Param(0)),
                _ => None,
            },
            compile_time: false,
            module: self.current as u32,
            is_pub,
            span: sig.span,
        };
        let id = self.program.fns.alloc(def);
        self.check_test_shape(id, annotations.test);
        self.param_defaults.extend(
            deferred
                .into_iter()
                .map(|(index, value)| (id, index, value)),
        );
        if let Some(elem) = yields {
            self.declare_fn_generator(
                id,
                elem,
                generator_args,
                member.as_ref().map(|m| m.receiver),
            );
        }
        self.check_builtin_method(sig, member.as_ref());
        self.name_fn(id, sig, member.as_ref(), is_extern);
        id
    }

    /// A function's type parameters: a method's type's, then its own, so
    /// its body may use both. A C function has none.
    fn fn_generics(
        &mut self,
        sig: &ast::FnSig,
        member: Option<&Member>,
        is_extern: bool,
    ) -> Vec<GenericParamDef> {
        let generics = match member {
            None => self.generic_params(&sig.generics),
            Some(member) => {
                let mut generics = member.type_params.clone();
                // The type's come first, so a constraint on one of the
                // method's own — `J: Iterator<V>` — names the others where
                // they will be.
                self.type_params = generics.clone();
                for param in self.generic_params(&sig.generics) {
                    if let Some(shadowed) = generics.iter().find(|p| p.written == param.written) {
                        let name = self.text(param.written).to_string();
                        let diagnostic = Diagnostic::error(
                            codes::SHADOWED_TYPE_PARAMETER,
                            format!("`{name}` is already a type parameter of the type"),
                            param.span,
                            "a type parameter of the method",
                        )
                        .with_secondary(shadowed.span, format!("the type's `{name}`"))
                        .with_note("a method sees its type's parameters, so one of its own cannot have the same name");
                        self.report(diagnostic);
                    }
                    generics.push(param);
                }
                generics
            }
        };
        if is_extern && let Some(first) = generics.first() {
            let diagnostic = Diagnostic::error(
                codes::CANNOT_BE_GENERIC,
                "a C function cannot be generic",
                first.span,
                "a type parameter",
            )
            .with_note("C has no generics; declare the function once for each type it takes");
            self.report(diagnostic);
        }
        generics
    }

    /// `Self` in a method: the type it belongs to.
    fn member_self_ty(&mut self, member: &Member) -> Ty {
        match member.owner {
            // Built from the block's own parameters, which say what the
            // block asks of them: `extend [T: copy]` is a slice of a `T`
            // that is `copy`.
            MemberOwner::Type(owner) => self.self_ty_of(owner, &member.type_params),
            // In an interface, `Self` is the method's type parameter, which
            // an implementation substitutes.
            MemberOwner::Interface(_) => self.intern(TyKind::Param(crate::ty::TyParam {
                index: 0,
                name: self.interner.self_type_symbol(),
                copy: false,
            })),
        }
    }

    /// A function's parameters, a method's receiver first, and the defaults
    /// to check once every function is declared, by the parameter's place.
    /// What `from` names the result as borrowing: a parameter named alone,
    /// with its place and what it borrows; and what a parameter borrows,
    /// where a `&` or view field of it is named, `self.ast`.
    fn lends_from(&mut self, sig: &ast::FnSig, params: &[ParamDef]) -> Option<Vec<LendFrom>> {
        if sig.lends_from.is_empty() {
            return None;
        }
        let note = "`from` names what the result borrows: a parameter that is a `str`, a reference or a view, `self`, or a `&` or view field reached from one";
        let mut lends = Vec::new();
        for path in &sig.lends_from {
            let name = path
                .root
                .map_or(self.interner.self_symbol(), |name| name.sym);
            let written = self.text(name).to_string();
            let Some(index) = params.iter().position(|p| p.name == name) else {
                let diagnostic = Diagnostic::error(
                    codes::LENDS_FROM,
                    format!(
                        "`{written}` is not a parameter of `{}`",
                        self.text(sig.name.sym)
                    ),
                    path.span,
                    "not a parameter",
                )
                .with_note(note);
                self.report(diagnostic);
                continue;
            };
            if path.fields.is_empty() {
                let ty = params[index].ty;
                if !self.program.holds_view(ty) {
                    let diagnostic = Diagnostic::error(
                        codes::LENDS_FROM,
                        format!(
                            "`{written}` is {}, which a result cannot borrow from",
                            self.ty_name(ty)
                        ),
                        path.span,
                        "borrows nothing",
                    )
                    .with_note(note);
                    self.report(diagnostic);
                    continue;
                }
                lends.push(LendFrom::Param(index as u32));
                continue;
            }
            // Each field is a `&` or a view, which points outside what holds
            // it; a field held as its own is part of the view.
            let mut ty = params[index].ty;
            let mut reached = written.clone();
            let mut lent = true;
            for field in &path.fields {
                let owner = match self.kind(ty) {
                    TyKind::Ref(inner, _) => inner,
                    _ => ty,
                };
                let found = match self.kind(owner) {
                    TyKind::Struct(id, _) => self.program.structs[id]
                        .fields
                        .iter()
                        .position(|f| f.name == field.sym),
                    _ => None,
                };
                let Some(at) = found else {
                    let diagnostic = Diagnostic::error(
                        codes::LENDS_FROM,
                        format!("`{reached}` has no field `{}`", self.text(field.sym)),
                        field.span,
                        "no such field",
                    )
                    .with_note(note);
                    self.report(diagnostic);
                    lent = false;
                    break;
                };
                ty = self.program.field_ty(owner, at as u32);
                reached = format!("{reached}.{}", self.text(field.sym));
                let points_out = matches!(self.kind(ty), TyKind::Ref(..))
                    || matches!(self.kind(ty), TyKind::Struct(id, _) if self.program.structs[id].is_view);
                if !points_out {
                    let diagnostic = Diagnostic::error(
                        codes::LENDS_FROM,
                        format!("`{reached}` is held by `{written}`, not borrowed by it"),
                        path.span,
                        format!("{} of its own", self.ty_name(ty)),
                    )
                    .with_help(format!(
                        "name `{written}`, which the result then borrows whole"
                    ))
                    .with_note("a field names what the result borrows where it is a `&`, or a view, which points outside what holds it");
                    self.report(diagnostic);
                    lent = false;
                    break;
                }
            }
            if lent {
                lends.push(LendFrom::Borrowed(index as u32));
            }
        }
        Some(lends)
    }

    fn fn_params(
        &mut self,
        sig: &ast::FnSig,
        member: Option<&Member>,
        is_extern: bool,
    ) -> (Vec<ParamDef>, Vec<(usize, ast::ExprId)>) {
        let ast = self.ast;
        let mut seen = FxHashMap::default();
        let mut params = Vec::new();
        // The receiver is the first parameter, written as a word before `fn`.
        if let Some(member) = member
            && member.receiver != Receiver::Static
        {
            let self_ty = self.self_ty.expect("a method knows its type");
            let ty = match member.receiver {
                Receiver::Read => self.intern(TyKind::Ref(self_ty, crate::RefKind::Shared)),
                Receiver::Var => self.intern(TyKind::Ref(self_ty, crate::RefKind::Var)),
                _ => self_ty,
            };
            params.push(ParamDef {
                name: self.interner.self_symbol(),
                name_span: member.keyword,
                ty,
                span: member.keyword,
                default: None,
            });
        }
        let mut deferred: Vec<(usize, ast::ExprId)> = Vec::new();
        for p in &sig.params {
            match seen.get(&p.name.sym) {
                Some(&first) => self.duplicate(p.name, first),
                None => {
                    seen.insert(p.name.sym, p.name.span);
                }
            }
            let mut ty = self.resolve_ty(p.ty);
            if is_extern {
                self.check_c_compatible(ty, ast.types[p.ty].span, false);
            } else {
                ty = self.param_ty(ty, p.ty);
            }
            // Checked once every function is declared, since it may be code
            // that calls one.
            if let Some(value) = p.default
                && self.default_allowed(value, ty, p, is_extern)
            {
                deferred.push((params.len(), value));
            }
            params.push(ParamDef {
                name: p.name.sym,
                name_span: p.name.span,
                ty,
                span: p.span,
                default: None,
            });
        }
        (params, deferred)
    }

    /// A function's result; for one that answers `Iterator<T>`, the `T` it
    /// yields, and the type arguments its generator is declared with.
    fn fn_result(
        &mut self,
        sig: &ast::FnSig,
        member: Option<&Member>,
        is_extern: bool,
    ) -> (Ty, Option<Ty>, crate::TyList) {
        let ast = self.ast;
        // `Iterator<T>` as the result: a function that yields, whose
        // generator is declared once the function is.
        let yields = match sig.ret {
            Some(t) if !is_extern => self.generator_elem(t),
            _ => None,
        };
        let generator_args = self.own_type_args();
        let ret = match sig.ret {
            Some(_) if yields.is_some() => Types::ERROR,
            Some(t) => {
                let mut ty = self.resolve_ty(t);
                if is_extern {
                    self.check_c_compatible(ty, ast.types[t].span, true);
                } else {
                    // A reference result declares a projection, which lends a
                    // place instead of returning a value.
                    // Deeper in the type a `&` is a view, as in
                    // `Option<&V>`, and a `&var` is refused.
                    let inner = match self.kind(ty) {
                        TyKind::Ref(inner, _) => inner,
                        _ => ty,
                    };
                    if self.no_var_ref(inner, t, "a result")
                        || self.bare_dyn(ty, t)
                        || self.bare_slice(ty, t, false)
                    {
                        ty = Types::ERROR;
                    }
                }
                // The writing half of a `lend fn` lends what the reading half
                // does, for writing.
                if member.is_some_and(|m| m.lent)
                    && let TyKind::Ref(inner, crate::RefKind::Shared) = self.kind(ty)
                {
                    ty = self.intern(TyKind::Ref(inner, crate::RefKind::Var));
                }
                ty
            }
            None => Types::UNIT,
        };
        (ret, yields, generator_args)
    }

    /// The body the compiler writes for an `@intrinsic`, which must be one
    /// it knows.
    fn fn_intrinsic(&mut self, sig: &ast::FnSig, member: Option<&Member>) -> Option<Intrinsic> {
        let owner = member.map(|m| m.owner);
        let name = self.text(sig.name.sym).to_string();
        let known = match owner {
            Some(MemberOwner::Type(TypeDef::Builtin(BuiltinOwner::Slots))) => match name.as_str() {
                "alloc" => Some(Intrinsic::SlotsAlloc),
                "moveIn" => Some(Intrinsic::SlotsMoveIn),
                "moveOut" => Some(Intrinsic::SlotsMoveOut),
                "slot" => Some(Intrinsic::SlotsSlot),
                "takeBuffer" => Some(Intrinsic::SlotsTakeBuffer),
                "takeFrom" => Some(Intrinsic::SlotsTakeFrom),
                "over" => Some(Intrinsic::SlotsOver),
                "release" => Some(Intrinsic::SlotsRelease),
                _ => None,
            },
            Some(MemberOwner::Type(TypeDef::Builtin(BuiltinOwner::Str))) => match name.as_str() {
                "fromBytes" => Some(Intrinsic::StrFromBytes),
                "items" => Some(Intrinsic::StrBytes),
                "len" => Some(Intrinsic::Len),
                _ => None,
            },
            // What a float is made of, and what the machine does to one
            // exactly.
            Some(MemberOwner::Type(TypeDef::Builtin(BuiltinOwner::Float(_)))) => {
                match name.as_str() {
                    "toBits" => Some(Intrinsic::FloatToBits),
                    "fromBits" => Some(Intrinsic::FloatFromBits),
                    "sqrt" => Some(Intrinsic::FloatSqrt),
                    "floor" => Some(Intrinsic::FloatFloor),
                    "ceil" => Some(Intrinsic::FloatCeil),
                    "trunc" => Some(Intrinsic::FloatTrunc),
                    "mulAdd" => Some(Intrinsic::FloatMulAdd),
                    _ => None,
                }
            }
            // An integer's bits, as the machine counts and turns them.
            Some(MemberOwner::Type(TypeDef::Builtin(BuiltinOwner::Int(_)))) => {
                match name.as_str() {
                    "countOnes" => Some(Intrinsic::IntCountOnes),
                    "addOverflows" => Some(Intrinsic::IntAddOverflows),
                    "subOverflows" => Some(Intrinsic::IntSubOverflows),
                    "mulOverflows" => Some(Intrinsic::IntMulOverflows),
                    "leadingZeros" => Some(Intrinsic::IntLeadingZeros),
                    "trailingZeros" => Some(Intrinsic::IntTrailingZeros),
                    "swapBytes" => Some(Intrinsic::IntSwapBytes),
                    "reverseBits" => Some(Intrinsic::IntReverseBits),
                    "rotateLeft" => Some(Intrinsic::IntRotateLeft),
                    "rotateRight" => Some(Intrinsic::IntRotateRight),
                    _ => None,
                }
            }
            Some(MemberOwner::Type(TypeDef::Builtin(BuiltinOwner::Slice))) => match name.as_str() {
                "swap" => Some(Intrinsic::SliceSwap),
                "len" => Some(Intrinsic::Len),
                _ => None,
            },
            Some(MemberOwner::Type(TypeDef::Builtin(BuiltinOwner::Cstring))) => {
                match name.as_str() {
                    "fromBytes" => Some(Intrinsic::CstringFromBytes),
                    _ => None,
                }
            }
            Some(MemberOwner::Type(TypeDef::Builtin(BuiltinOwner::Char))) => match name.as_str() {
                "fromScalar" => Some(Intrinsic::CharFromScalar),
                _ => None,
            },
            // An `own` as the address C can hold, and back: how a
            // value crosses to another thread, for the standard
            // library alone.
            None if self.in_std() => match name.as_str() {
                "intoAddress" => Some(Intrinsic::OwnIntoAddress),
                // An option's value, where the library knows it holds one.
                "assumed" | "assumedVar" => Some(Intrinsic::OptionAssumed),
                "fromAddress" => Some(Intrinsic::OwnFromAddress),
                "unbox" => Some(Intrinsic::OwnUnbox),
                // A value lent to C as a callback's `void *`, and had back.
                "addressOf" => Some(Intrinsic::ReferenceAddress),
                "referenceAt" => Some(Intrinsic::AddressReference),
                // C's `sizeof` and `_Alignof`.
                "sizeOf" => Some(Intrinsic::SizeOf),
                // A slice's address, for C to read.
                "pointerTo"
                    if matches!(
                        self.modules[self.current].path.as_str(),
                        "std::c" | "std::prelude"
                    ) =>
                {
                    Some(Intrinsic::SlicePointer)
                }
                "alignOf" => Some(Intrinsic::AlignOf),
                // What the runtime written in Wip needs of memory C
                // gave it.
                "atomicAdd" => Some(Intrinsic::AtomicAdd),
                "atomicLoad" => Some(Intrinsic::AtomicLoad),
                "atomicStore" => Some(Intrinsic::AtomicStore),
                // A slice lent in parts, and a closure lent to a thread.
                "sliceAt" => Some(Intrinsic::SliceAt),
                "boxAddress" => Some(Intrinsic::BoxAddress),
                "closureCode" => Some(Intrinsic::ClosureCode),
                "closureCaptures" => Some(Intrinsic::ClosureCaptures),
                // What `std::sync::Atomic` does.
                "atomicSubtract" => Some(Intrinsic::AtomicSubtract),
                "atomicSwap" => Some(Intrinsic::AtomicSwap),
                "atomicCompareSwap" => Some(Intrinsic::AtomicCompareSwap),
                "offsetBytes" => Some(Intrinsic::OffsetBytes),
                "runtimeWords" => Some(Intrinsic::RuntimeWords),
                // What a panic reads to say the calls that led to it.
                "frameTables" => Some(Intrinsic::FrameTables),
                // A file's contents, read when the program is compiled.
                "bytes" if self.modules[self.current].path == "std::embed" => {
                    Some(Intrinsic::EmbedBytes)
                }
                "text" if self.modules[self.current].path == "std::embed" => {
                    Some(Intrinsic::EmbedText)
                }
                _ => None,
            },
            _ => None,
        };
        if known.is_none() {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                format!("the compiler has no body for `{name}`"),
                sig.name.span,
                "not an intrinsic it knows",
            )
            .with_note(
                "`@intrinsic` says the compiler writes the body, so it must be one of the few it knows",
            );
            self.report(diagnostic);
        }
        known
    }

    /// Whether C calls the function by a plain symbol, `@export("C")`, in
    /// which case its signature must be one C can write.
    fn fn_exported(
        &mut self,
        annotations: &annotations::Annotations,
        params: &[ParamDef],
        sig: &ast::FnSig,
        ret: Ty,
    ) -> bool {
        let ast = self.ast;
        let exported = match annotations.export {
            Some((abi, span)) => {
                let text = self.text(abi).to_string();
                if text != "C" {
                    let diagnostic = Diagnostic::error(
                        codes::ANNOTATION,
                        format!("`@export(\"{text}\")` names no ABI Wip knows"),
                        span,
                        "not an ABI",
                    )
                    .with_help("the only one is `@export(\"C\")`");
                    self.report(diagnostic);
                }
                true
            }
            None => false,
        };
        if exported {
            for (param, declared) in params.iter().zip(&sig.params) {
                self.check_c_compatible(param.ty, ast.types[declared.ty].span, false);
            }
            if let Some(span) = sig.ret.map(|t| ast.types[t].span) {
                self.check_c_compatible(ret, span, true);
            }
        }
        exported
    }

    /// A method of a built-in type may not take a name the checker answers
    /// itself, and a type with no value form has no `move fn`.
    fn check_builtin_method(&mut self, sig: &ast::FnSig, member: Option<&Member>) {
        if let Some(MemberOwner::Type(TypeDef::Builtin(builtin))) = member.map(|m| m.owner) {
            let receiver = member.map(|m| m.receiver);
            if builtin == BuiltinOwner::Slice && receiver == Some(Receiver::Move) {
                let diagnostic = Diagnostic::error(
                    codes::IMPL_TARGET,
                    "a slice has no value to move",
                    sig.name.span,
                    "a `move fn`",
                )
                .with_note(
                    "`[T]` is only ever behind a reference, so a method of it takes `fn` or `var fn`",
                );
                self.report(diagnostic);
            }
        }
    }

    /// Keeps a function's name where it is found: a method among its
    /// type's members, a free function among its module's, and the ones of
    /// the prelude's the compiler calls in its table.
    fn name_fn(&mut self, id: FnId, sig: &ast::FnSig, member: Option<&Member>, is_extern: bool) {
        match member.map(|m| m.owner) {
            // A type's members share one set of names. An
            // interface's methods are collected by its own pass.
            // An `extend X: From<A>` and an `extend X: From<B>` each
            // declare `from`, and the argument says which.
            Some(MemberOwner::Type(owner)) => match self
                .member_named(owner, sig.name.sym)
                .filter(|_| !member.is_some_and(|m| m.overloaded))
                // A lending pair is one name for the accessor that reads
                // and the one that writes.
                .filter(|_| {
                    !self
                        .method_of(owner, sig.name.sym)
                        .is_some_and(|first| self.is_lending_pair(first, id))
                }) {
                Some(first) => self.duplicate(sig.name, first),
                None => match owner {
                    TypeDef::Struct(owner) => {
                        // The methods of the prelude's that the compiler
                        // calls, each for what `KnownFn` says.
                        if self.prelude == Some(self.current)
                            && let Some(known) = KnownFn::named(
                                KnownStruct::named(self.text(self.program.structs[owner].name)),
                                self.text(sig.name.sym),
                            )
                            && known.owner().is_some()
                        {
                            self.program.prelude_items.set_function(known, id);
                        }
                        self.program.structs[owner].methods.push(id);
                    }
                    TypeDef::Enum(owner) => self.program.enums[owner].methods.push(id),
                    TypeDef::Builtin(owner) => self
                        .program
                        .builtins
                        .entry(owner)
                        .or_default()
                        .methods
                        .push(id),
                },
            },
            Some(MemberOwner::Interface(_)) => {}
            None if !is_extern && self.prelude_name(sig.name, "a function named") => {}
            None => match self.fns().get(&sig.name.sym) {
                Some(&first) => {
                    let first = self.program.fns[first].name_span;
                    self.duplicate(sig.name, first);
                }
                // `Row(…)` builds a struct, or what an alias names, so a
                // function may not take its name.
                None if let Some(first) = self.struct_named(sig.name.sym) => {
                    let text = self.text(sig.name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::DUPLICATE_DEFINITION,
                        format!("`{text}` is a type of this module and a function"),
                        sig.name.span,
                        "a function of that name",
                    )
                    .with_secondary(first, "the type")
                    .with_note(format!("`{text}(…)` builds the struct, as a variant is built, so it cannot also call a function"))
                    .with_help("give the function a name of its own, or make it a `static fn` of the type");
                    self.report(diagnostic);
                }
                None => {
                    // The free functions of the prelude's that the
                    // compiler calls, each for what `KnownFn` says.
                    if self.prelude == Some(self.current)
                        && let Some(known) = KnownFn::named(None, self.text(sig.name.sym))
                    {
                        self.program.prelude_items.set_function(known, id);
                    }
                    self.fns_mut().insert(sig.name.sym, id);
                }
            },
        }
    }

    /// Where this module declares a struct, or an alias, called `sym`,
    /// which `sym(…)` would build.
    fn struct_named(&self, sym: Symbol) -> Option<Span> {
        if let Some(&(TypeDef::Struct(_), span)) = self.types().get(&sym) {
            return Some(span);
        }
        self.aliases().get(&sym).map(|alias| alias.span)
    }

    /// What `...` may not do: stand in a Wip function,
    /// have anything written after it, or be the whole list, since C reads
    /// the rest by what the first arguments said.
    fn check_variadic(&mut self, sig: &ast::FnSig, is_extern: bool) {
        // Only C takes more arguments than it declares: a Wip function
        // that wanted to would take a slice.
        if let Some(span) = sig.variadic
            && !is_extern
        {
            let diagnostic = Diagnostic::error(
                codes::VARIADIC,
                "only a C declaration may be variadic",
                span,
                "`...` outside an extern block",
            )
            .with_note(
                "a Wip function that takes however many values takes a slice, which needs no C ABI",
            );
            self.report(diagnostic);
        }
        // `...` is where the declared parameters end, so nothing may be
        // written after it.
        if let Some(span) = sig.variadic
            && let Some(after) = sig.params.iter().find(|p| p.span.lo > span.hi)
        {
            let diagnostic = Diagnostic::error(
                codes::VARIADIC,
                "nothing may follow `...`",
                after.span,
                "a parameter after it",
            )
            .with_secondary(span, "the declaration ends here")
            .with_note(
                "what a call passes after the declared parameters is read one at a time, so `...` is last",
            );
            self.report(diagnostic);
        }
        // C reads the rest by what the first ones said, so there must be a
        // first one.
        if let Some(span) = sig.variadic
            && is_extern
            && sig.params.is_empty()
        {
            let diagnostic = Diagnostic::error(
                codes::VARIADIC,
                "`...` needs a parameter before it",
                span,
                "nothing declared before it",
            )
            .with_note(
                "C reads what follows by what the declared arguments said, as `printf` reads its format",
            );
            self.report(diagnostic);
        }
    }

    /// A function marked `@test` at `test` takes nothing and answers
    /// nothing, since the runner `wip test` writes calls it by itself.
    fn check_test_shape(&mut self, id: FnId, test: Option<Span>) {
        if let Some(at) = test {
            let def = &self.program.fns[id];
            let wrong = if def.receiver.is_some() {
                Some("a method")
            } else if !def.params.is_empty() {
                Some("parameters")
            } else if !def.generics.is_empty() {
                Some("type parameters")
            } else if def.ret != Types::UNIT {
                Some("a result")
            } else {
                None
            };
            if let Some(wrong) = wrong {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    "a test takes nothing and answers nothing",
                    def.span,
                    format!("has {wrong}"),
                )
                .with_secondary(at, "marked a test here")
                .with_note(
                    "`wip test` calls each test by itself: `@test fn theBoardStartsEmpty() = { … }`",
                );
                self.report(diagnostic);
            }
        }
    }

    /// Where a type already has a member of this name: a field, a variant,
    /// a method or a static function.
    fn member_named(&self, owner: TypeDef, name: Symbol) -> Option<Span> {
        let (fields, methods) = match owner {
            TypeDef::Struct(id) => {
                let def = &self.program.structs[id];
                (
                    def.fields.iter().map(|f| (f.name, f.span)).collect(),
                    &def.methods,
                )
            }
            TypeDef::Enum(id) => {
                let def = &self.program.enums[id];
                let variants: Vec<(Symbol, Span)> =
                    def.variants.iter().map(|v| (v.name, v.span)).collect();
                (variants, &def.methods)
            }
            // A built-in type has no fields a program declares; the ones the
            // checker knows are refused where a method is declared.
            TypeDef::Builtin(owner) => {
                let built = self.program.builtins.get(&owner)?;
                (Vec::new(), &built.methods)
            }
        };
        let fields: Vec<(Symbol, Span)> = fields;
        if let Some(&(_, span)) = fields.iter().find(|&&(n, _)| n == name) {
            return Some(span);
        }
        methods
            .iter()
            .map(|&id| &self.program.fns[id])
            .find(|def| def.name == name)
            .map(|def| def.name_span)
    }

    /// A built-in type an `extend` block names: only the prelude may give one
    /// methods, since a method is found by the type of its receiver and would
    /// otherwise appear in files that never asked for it.
    fn builtin_owner(
        &mut self,
        block: &'a ast::ExtendBlock,
        builtin: BuiltinOwner,
        generics: Vec<GenericParamDef>,
        span: Span,
    ) -> Option<TypeDef> {
        // A module may implement its own interface for a built-in type: the
        // interface is its own, so no two modules can disagree about what
        // the method means. Methods of its own are
        // the prelude's.
        let own_interface = block
            .interface
            .and_then(|name| self.interface_named(name))
            .is_some_and(|id| self.program.interfaces[id].module == self.current as u32);
        // And another module's interface where one of its arguments is a
        // type of this module, which teaches the built-in type only about
        // this module's types: `extend f32: Multiply<Vec2, Vec2>`.
        let names_own_type = block.interface.is_some()
            && block.interface_args.as_ref().is_some_and(|args| {
                args.args.iter().any(|&arg| {
                    matches!(self.ast.types[arg].kind, ast::TypeKind::Named { name, .. }
                        if self.types().contains_key(&name))
                })
            });
        if !own_interface && !names_own_type && self.prelude != Some(self.current) {
            let diagnostic = Diagnostic::error(
                codes::IMPL_TARGET,
                format!("`{}` is a built-in type", builtin.text()),
                span,
                "not a type of this module",
            )
            .with_note(
                "a method is found by the type of its receiver, so a method on a built-in type would appear in every file; only the prelude declares them",
            )
            .with_help(match block.interface {
                Some(_) => "implement an interface of this module, or one whose arguments name a type of this module: `extend f32: Multiply<Vec2, Vec2>` in the module of `Vec2`",
                None => "write a function that takes it as a parameter, or an interface of this module and implement that",
            });
            self.report(diagnostic);
            return None;
        }
        let entry = self.program.builtins.entry(builtin).or_default();
        if entry.generics.is_empty() {
            entry.generics = generics;
        }
        Some(TypeDef::Builtin(builtin))
    }

    /// The type a method belongs to, with its own type parameters as
    /// arguments: `Option<T>` inside `enum Option<T>`.
    pub(super) fn self_ty(&mut self, owner: TypeDef) -> Ty {
        let generics = self.type_generics(owner).to_vec();
        self.self_ty_of(owner, &generics)
    }

    /// [`Self::self_ty`], with the parameters as a block names them.
    pub(super) fn self_ty_of(&mut self, owner: TypeDef, generics: &[GenericParamDef]) -> Ty {
        let generics = generics.to_vec();
        let args: Vec<Ty> = generics
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
        let list = self.program.types.intern_list(&args);
        match owner {
            TypeDef::Struct(id) => self.intern(TyKind::Struct(id, list)),
            TypeDef::Enum(id) => self.intern(TyKind::Enum(id, list)),
            // `Self` inside `extend [T]` is `[T]`, and inside `extend str` is
            // `str`.
            TypeDef::Builtin(owner) => match owner {
                BuiltinOwner::Int(t) => self.intern(TyKind::Int(t)),
                BuiltinOwner::Float(t) => self.intern(TyKind::Float(t)),
                BuiltinOwner::Bool => Types::BOOL,
                BuiltinOwner::Char => Types::CHAR,
                BuiltinOwner::Str => Types::STR,
                BuiltinOwner::Cstring => Types::CSTRING,
                BuiltinOwner::Slice => {
                    let elem = args.first().copied().unwrap_or(Types::ERROR);
                    self.intern(TyKind::Slice(elem))
                }
                BuiltinOwner::Slots => {
                    let elem = args.first().copied().unwrap_or(Types::ERROR);
                    self.intern(TyKind::Slots(elem))
                }
                BuiltinOwner::Void => Types::UNIT,
            },
        }
    }

    /// The type of a parameter, of a function or of a function type: a
    /// `&var` can be its whole type, and a `&` its whole type or a view's
    /// part, `&Option<&Node>`.
    pub(super) fn param_ty(&mut self, ty: Ty, type_id: ast::TypeId) -> Ty {
        let inner = match self.kind(ty) {
            TyKind::Ref(inner, _) => inner,
            _ => ty,
        };
        if self.no_var_ref(inner, type_id, "a parameter")
            || self.no_never(ty, type_id, "a parameter")
            || self.bare_dyn(ty, type_id)
            || self.bare_slice(ty, type_id, true)
        {
            return Types::ERROR;
        }
        ty
    }

    /// Whether `ty` may cross into C. A result may also be
    /// `never`, for a C function that does not return — `exit`, `abort` —
    /// which a parameter may not be, since no value of that type exists.
    /// Whether a type may cross into C. 128-bit
    /// integers have no standard C type, and `isize` and `usize` are Wip's
    /// own; `str` is a pointer and a length.
    pub(super) fn c_compatible(&self, ty: Ty, is_result: bool) -> bool {
        match self.kind(ty) {
            TyKind::Int(t) => {
                t.bits() <= 64 && !matches!(t, crate::IntTy::Isize | crate::IntTy::Usize)
            }
            TyKind::Never => is_result,
            // A C pointer is what C calls a pointer.
            TyKind::Ptr(_) => true,
            // `&T` and `&var T` pass `const T *` and `T *`, which is how C
            // takes what it reads and what it writes through, and `&[T]`
            // passes the pointer and the length.
            // An `extern struct` by value crosses through a shim the
            // compiler writes in C, which is what item 7 is for: `cc` then
            // decides how the fields travel, and Wip never has to.
            // Any other struct is Wip's own layout.
            TyKind::Struct(id, _) => self.program.structs[id].is_extern,
            TyKind::Ref(inner, _) if !is_result => match self.kind(inner) {
                // A pointer to a C layout is a C pointer (item 4).
                TyKind::Struct(id, _) if self.program.structs[id].is_extern => true,
                // A slice goes to C as a pointer and a length, two
                // arguments, as a `str` does.
                TyKind::Slice(elem) => self.c_compatible(elem, false),
                // An array is passed where it is, as C's `T x[N]`
                // parameter is: the address of its first element.
                TyKind::Array(..) => self.c_element(inner),
                TyKind::Dyn(..) => false,
                _ => self.c_compatible(inner, false),
            },
            // A `str` goes to C as a pointer and a length, two arguments.
            // A result cannot: C returns one value.
            TyKind::Str => !is_result,
            // A C function pointer: what it points at must be a function C
            // could have written, so its own types cross too. A `str` and a
            // slice do not, since they would take two parameters of C's
            // where the pointer's type says one.
            // A struct by value does not either, nor a function pointer
            // that may be null, which Wip holds as an `Option`: C passes
            // either in registers, and a Wip function C calls through a
            // pointer takes its address.
            TyKind::Fn(params, ret) => {
                let params = self.program.types.list(params).to_vec();
                let by_value = |ty: Ty| {
                    matches!(self.kind(ty), TyKind::Struct(..))
                        || self.program.nullable_function(ty)
                };
                params.iter().all(|&p| {
                    !matches!(self.kind(p), TyKind::Str)
                        && !by_value(p)
                        && self.c_compatible(p, false)
                }) && (ret == Types::UNIT || (!by_value(ret) && self.c_compatible(ret, true)))
            }
            TyKind::Float(_) | TyKind::Bool | TyKind::Cstring | TyKind::Error => true,
            // A C function pointer that may be null: the pointer, with
            // `.None` as null.
            TyKind::Enum(_, args) if self.program.nullable_function(ty) => {
                let function = self.program.types.list(args)[0];
                self.c_compatible(function, false)
            }
            _ => false,
        }
    }

    /// What C can hold as the elements of an array it is lent: what crosses
    /// by itself, an `extern struct`, or an array of those.
    fn c_element(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Array(elem, _) => self.c_element(elem),
            TyKind::Struct(id, _) => self.program.structs[id].is_extern,
            TyKind::Str | TyKind::Fn(..) | TyKind::Ref(..) => false,
            _ => self.c_compatible(ty, false),
        }
    }

    /// A field of an `extern struct`: what C can hold in one. A fixed
    /// array is C's array, and another `extern struct` is C's struct.
    fn check_c_field(&mut self, ty: Ty, span: Span, in_union: bool) {
        let what = if in_union {
            "an `extern union`"
        } else {
            "an `extern struct`"
        };
        let ok = match self.kind(ty) {
            TyKind::Array(elem, _) => {
                self.check_c_field(elem, span, in_union);
                return;
            }
            TyKind::Struct(id, _) => self.program.structs[id].is_extern,
            _ => {
                self.c_compatible(ty, false)
                    && !matches!(self.kind(ty), TyKind::Str | TyKind::Fn(..))
            }
        };
        // A function pointer is a field as an `Option`, which may be null
        // as C's is; `c_compatible` has taken it above.
        if ok {
            return;
        }
        let diagnostic = Diagnostic::error(
            codes::NOT_C_COMPATIBLE,
            format!("{} cannot be a field of {what}", self.ty_name(ty)),
            span,
            "not a type C can hold",
        )
        .with_note(format!(
            "{what} promises C's layout, so its fields are what C writes: integers of up to 64 bits, `f32`, `f64`, `bool`, `cstring`, `ptr<T>`, fixed arrays of those, other `extern struct`s, and a C function pointer as `Option<(…) => R>`, which may be null as C's is"
        ));
        self.report(diagnostic);
    }

    pub(super) fn check_c_compatible(&mut self, ty: Ty, span: Span, is_result: bool) {
        let ok = self.c_compatible(ty, is_result);
        if !ok {
            let mut diagnostic = Diagnostic::error(
                codes::NOT_C_COMPATIBLE,
                format!("{} cannot be passed to or from C", self.ty_name(ty)),
                span,
                "not a C-compatible type",
            )
            .with_note("extern functions can take and return only integers of up to 64 bits, `f32`, `f64`, `bool`, `cstring`, `ptr<T>` and C's structs; a parameter may also be a `str` or a `&[T]`, which pass as a pointer and a length, an array by reference, which passes as a pointer to its first element, or a function that takes and answers no struct by value, and no function that may be null, and whose own types cross, which passes as a C function pointer; a result may be `never`, for a C function that does not return");
            // An `extern struct` is C's layout; what it cannot do yet is
            // cross by value.
            if ty == Types::STR {
                diagnostic = diagnostic.with_help(
                    "C takes strings as `cstring`; a string literal can be passed as either",
                );
            }
            // C's pointer-sized integers are built in under C's names.
            if matches!(
                self.kind(ty),
                TyKind::Int(crate::IntTy::Isize | crate::IntTy::Usize)
            ) {
                diagnostic = diagnostic.with_help(
                    "`usize` and `isize` are Wip's own; C's are built in as `size_t`, `ssize_t`, `uintptr_t` and `intptr_t`",
                );
            }
            self.report(diagnostic);
        }
    }
}
