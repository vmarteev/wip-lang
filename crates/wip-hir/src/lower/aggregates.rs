//! Struct literals and enum variants.

use super::*;

/// A struct literal as it is written, `Name<…>(field: value, ..rest)`,
/// and the type expected where it is.
#[derive(Clone, Copy)]
pub(super) struct StructLit<'a> {
    pub type_args: Option<&'a ast::TypeArgs>,
    pub fields: Written<'a>,
    pub rest: Option<ast::ExprId>,
    pub hint: Option<Ty>,
    pub span: Span,
}

/// The fields a struct literal is given, as written: a call's arguments,
/// by position and then by name.
#[derive(Clone, Copy)]
pub(super) struct Written<'a> {
    pub args: &'a [ast::ExprId],
    pub names: &'a [Option<ast::Name>],
}

impl Written<'_> {
    /// The values given, in the order written.
    fn values(self) -> Vec<ast::ExprId> {
        self.args.to_vec()
    }
}

/// A field a literal sets, under its name: a positional argument is given
/// the name of the field it fills.
#[derive(Clone, Copy)]
struct FieldInit {
    name: ast::Name,
    value: ast::ExprId,
}

impl<'a> Lowerer<'a> {
    pub(super) fn struct_lit(&mut self, path: &[ast::Name], lit: StructLit<'a>) -> ExprId {
        let StructLit {
            type_args,
            fields,
            span,
            ..
        } = lit;
        if let Some(type_args) = type_args
            && let Some(diagnostic) = self.variable_with_type_args(path, type_args)
        {
            self.report(diagnostic);
            return self.give_up(fields.values(), span);
        }
        // `Self { … }` builds the type the method belongs to.
        if let [only] = path
            && self.text(only.sym) == "Self"
        {
            let Some(self_ty) = self.self_ty else {
                let diagnostic = Diagnostic::error(
                    codes::SELF_OUTSIDE_METHOD,
                    "`Self` is only available in a method",
                    only.span,
                    "no type here",
                )
                .with_note("`Self` is the type a method belongs to");
                self.report(diagnostic);
                return self.give_up(fields.values(), span);
            };
            if let TyKind::Struct(id, _) = self.kind(self_ty) {
                // The struct under its own name, so the literal is checked
                // as any other is.
                let name = ast::Name {
                    sym: self.program.structs[id].name,
                    span: only.span,
                };
                let lit = StructLit {
                    type_args: None,
                    ..lit
                };
                return self.struct_lit_of(self.current, name, 0, lit);
            }
            let diagnostic = Diagnostic::error(
                codes::STRUCT_LITERAL_FIELDS,
                format!("{} is an enum, not a struct", self.ty_name(self_ty)),
                only.span,
                "not a struct",
            )
            .with_help("construct a variant with `.Variant(…)`");
            self.report(diagnostic);
            return self.give_up(fields.values(), span);
        }
        let PathTarget::Item(module, name) = self.resolve_path(path, false, paths::MODULE_NOTE)
        else {
            return self.give_up(fields.values(), span);
        };
        self.struct_lit_of(module, name, path.len() - 1, lit)
    }

    /// The fields a literal did not name, read from the value after `..`.
    /// It must be a value that is there — a variable, a
    /// field, an element — since each field is read from it where it lies,
    /// and each must be one that copies.
    fn rest_fields(
        &mut self,
        rest: ast::ExprId,
        ty: Ty,
        taken: &[usize],
        name: ast::Name,
        span: Span,
    ) -> Vec<(usize, ExprId)> {
        let context = Some((name.span, "the literal's type"));
        let value = self.check_in(rest, ty, context);
        let rest_span = self.state.body.exprs[value].span;
        // Where the value is not one the fields can be read from, the
        // fields it would have given are errors of their own, so that
        // nothing reports them missing as well.
        let broken = |lowerer: &mut Self| -> Vec<(usize, ExprId)> {
            let error = lowerer.error_expr(rest_span);
            taken.iter().map(|&i| (i, error)).collect()
        };
        if self.ty_of(value) != ty {
            // Reported by the check above.
            return broken(self);
        }
        if !self.state.body.is_place(value) {
            let diagnostic = Diagnostic::error(
                codes::STRUCT_LITERAL_FIELDS,
                "`..` takes the rest of the fields from a value that is there",
                rest_span,
                "not a variable, a field or an element",
            )
            .with_help("put it in a variable first, and name that after `..`")
            .with_note(
                "each field is copied from where it lies, so the value is read and not built",
            );
            self.report(diagnostic);
            return broken(self);
        }
        if taken.is_empty() {
            let diagnostic = Diagnostic::warning(
                codes::STRUCT_LITERAL_FIELDS,
                "`..` has no fields to take",
                rest_span,
                "every field is named already",
            );
            self.report(diagnostic);
            return Vec::new();
        }
        let mut fields = Vec::new();
        for &i in taken {
            let field_ty = self.program.field_ty(ty, i as u32);
            // A field that owns memory would be moved out of a value that
            // goes on living, which nothing else in the language does.
            if self.owns(field_ty) {
                let TyKind::Struct(id, _) = self.kind(ty) else {
                    unreachable!("a struct literal's type is a struct")
                };
                let field = self
                    .text(self.program.structs[id].fields[i].name)
                    .to_string();
                let diagnostic = Diagnostic::error(
                    codes::STRUCT_LITERAL_FIELDS,
                    format!("`{field}` is {}, which owns memory", self.ty_name(field_ty)),
                    rest_span,
                    "not a field `..` can copy",
                )
                .with_help(format!(
                    "give `{field}` a value of its own in the literal, or build the whole value with `move`"
                ))
                .with_note(
                    "`..` copies the fields it takes, and a value that owns memory has one owner",
                );
                self.report(diagnostic);
                // Reported here, so it is not reported again as missing.
                fields.push((i, self.error_expr(rest_span)));
                continue;
            }
            let read = self.alloc(
                ExprKind::Field {
                    base: value,
                    index: i as u32,
                },
                field_ty,
                span,
            );
            fields.push((i, read));
        }
        fields
    }

    /// `Event { key: … }`, `Event {}`: a C union.
    fn union_literal(
        &mut self,
        id: StructId,
        name: ast::Name,
        fields: &[FieldInit],
        span: Span,
    ) -> ExprId {
        let decls = self.program.structs[id].fields.clone();
        let union_name = self.text(name.sym).to_string();
        // Two fields would be two values over the same bytes, and only the
        // last one written would be there.
        if let Some(second) = fields.get(1) {
            let diagnostic = Diagnostic::error(
                codes::STRUCT_LITERAL_FIELDS,
                format!("a `{union_name}` holds one field at a time"),
                second.name.span,
                "a second field",
            )
            .with_secondary(fields[0].name.span, "this one is already set")
            .with_note(
                "a union's fields are the same bytes, so a literal sets one of them, or none and every byte is zero",
            );
            self.report(diagnostic);
        }
        let mut written = None;
        for (n, f) in fields.iter().enumerate() {
            let Some(i) = decls.iter().position(|d| d.name == f.name.sym) else {
                let field = self.text(f.name.sym);
                let mut diagnostic = Diagnostic::error(
                    codes::STRUCT_LITERAL_FIELDS,
                    format!("union `{union_name}` has no field `{field}`"),
                    f.name.span,
                    "unknown field",
                );
                if let Some(similar) = suggest(field, decls.iter().map(|d| self.text(d.name))) {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `{similar}`?"),
                        [Edit::replace(f.name.span, similar)],
                    );
                }
                self.report(diagnostic);
                self.infer(f.value, None);
                continue;
            };
            let context = Some((decls[i].span, "field declared here"));
            let value = self.check_in(f.value, decls[i].ty, context);
            if n == 0 {
                written = Some((i as u32, value));
            }
        }
        let ty = self.intern(TyKind::Struct(id, crate::TyList::EMPTY));
        self.alloc(ExprKind::Union { id, field: written }, ty, span)
    }

    /// The fields a literal sets, each under its name, by a variant's rules:
    /// with two or more fields each is named; a value by position fills the
    /// field in its position, which is how a struct of one field and a
    /// tuple are built. More of them than the struct has fields is
    /// reported, and answers nothing.
    fn written_fields(
        &mut self,
        id: StructId,
        name: ast::Name,
        written: Written<'a>,
    ) -> Option<Vec<FieldInit>> {
        let Written { args, names } = written;
        let names = match self.program.structs[id].is_tuple {
            true => names.to_vec(),
            false => {
                let decls = self.program.structs[id].fields.clone();
                let path = self.text(name.sym).to_string();
                self.field_names(&path, &decls, args, names.to_vec())
            }
        };
        let mut fields = Vec::new();
        for (i, &value) in args.iter().enumerate() {
            let span = self.ast.exprs[value].span;
            let field = match names.get(i).copied().flatten() {
                Some(named) => FieldInit { name: named, value },
                None => {
                    let Some(decl) = self.program.structs[id].fields.get(i) else {
                        let count = self.program.structs[id].fields.len();
                        let text = self.text(name.sym).to_string();
                        let diagnostic = Diagnostic::error(
                            codes::STRUCT_LITERAL_FIELDS,
                            format!(
                                "`{text}` has {}, and {} are given",
                                plural(count, "field", "fields"),
                                args.len()
                            ),
                            span,
                            "no field is left for this one",
                        )
                        .with_note("a call's arguments set a struct's fields in the order they are declared, then by name");
                        self.report(diagnostic);
                        return None;
                    };
                    FieldInit {
                        name: ast::Name {
                            sym: decl.name,
                            span,
                        },
                        value,
                    }
                }
            };
            fields.push(field);
        }
        Some(fields)
    }

    /// Whether every byte zero is a value of `ty`, as C takes a field it
    /// leaves out to be: a number, a `bool`, a C pointer or string, an
    /// `Option` of a function, and C's structs and arrays of those; not a
    /// reference, which is never null.
    pub(super) fn zeroable(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Int(_)
            | TyKind::Float(_)
            | TyKind::Bool
            | TyKind::Char
            | TyKind::Ptr(_)
            | TyKind::Cstring
            | TyKind::Unit => true,
            TyKind::Array(elem, _) => self.zeroable(elem),
            TyKind::Struct(id, args) if self.program.structs[id].is_extern => {
                let args = self.program.types.list(args).to_vec();
                self.program.structs[id]
                    .fields
                    .iter()
                    .all(|f| self.zeroable(self.program.types.subst_find(f.ty, &args)))
            }
            _ => self.program.nullable_function(ty),
        }
    }

    /// A struct literal whose struct is known: its fields, checked together,
    /// since they may decide its type arguments.
    pub(super) fn struct_lit_of(
        &mut self,
        module: usize,
        name: ast::Name,
        name_segment: usize,
        lit: StructLit<'a>,
    ) -> ExprId {
        let (type_args, fields, span) = (lit.type_args, lit.fields, lit.span);
        // `type Grid = Map<Id, Cell>`: the struct the alias names, with the
        // type arguments the alias gives it.
        if !self.modules[module].types.contains_key(&name.sym)
            && self.modules[module].aliases.contains_key(&name.sym)
        {
            let args: &[ast::TypeId] = type_args.map_or(&[], |written| written.args.as_slice());
            let Some(ty) = self.alias_type_of(module, name.sym, args, name.span) else {
                return self.give_up(fields.values(), span);
            };
            let TyKind::Struct(id, list) = self.kind(ty) else {
                if !self.is_poisoned(ty) {
                    let text = self.text(name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::STRUCT_LITERAL_FIELDS,
                        format!("`{text}` names {}, not a struct", self.ty_name(ty)),
                        name.span,
                        "not a struct",
                    );
                    self.report(diagnostic);
                }
                return self.give_up(fields.values(), span);
            };
            let known = self.program.types.list(list).to_vec();
            return self.struct_lit_known(id, name, name_segment, Some(known), lit);
        }
        // A struct of the prelude, whose names are in scope in every file.
        // A tuple is one of them.
        let module = match self.modules[module].types.contains_key(&name.sym) {
            false => self.prelude_module(name.sym).unwrap_or(module),
            true => module,
        };
        let id = match self.modules[module].types.get(&name.sym) {
            Some(&(TypeDef::Struct(id), _))
                if self.visible(module, self.program.structs[id].is_pub) =>
            {
                id
            }
            Some(&(TypeDef::Struct(id), _)) => {
                self.private_item(module, "struct", name);
                id
            }
            other => {
                let text = self.text(name.sym);
                let diagnostic = match other {
                    Some(_) => Diagnostic::error(
                        codes::STRUCT_LITERAL_FIELDS,
                        format!("`{text}` is an enum, not a struct"),
                        name.span,
                        "not a struct",
                    )
                    .with_help(format!("construct a variant with `{text}::Variant(…)`")),
                    None => Diagnostic::error(
                        codes::UNKNOWN_TYPE,
                        format!("cannot find struct `{text}`"),
                        name.span,
                        "unknown struct",
                    ),
                };
                self.report(diagnostic);
                return self.give_up(fields.values(), span);
            }
        };
        self.struct_lit_known(id, name, name_segment, None, lit)
    }

    /// A struct literal of struct `id`, whose type arguments are `known`
    /// where an alias said them, and are written or inferred otherwise.
    fn struct_lit_known(
        &mut self,
        id: StructId,
        name: ast::Name,
        name_segment: usize,
        known: Option<Vec<Ty>>,
        lit: StructLit<'a>,
    ) -> ExprId {
        let StructLit {
            type_args,
            fields,
            rest,
            hint,
            span,
        } = lit;
        // `@opaque`: nothing here knows how large one is, so there is
        // nothing to make.
        if self.program.structs[id].is_opaque {
            let text = self.text(name.sym).to_string();
            let declared = self.program.structs[id].span;
            let diagnostic = Diagnostic::error(
                codes::OPAQUE_VALUE,
                format!("`{text}` is laid out by C, so Wip cannot make one"),
                span,
                "not a value Wip can build",
            )
            .with_secondary(declared, "declared `@opaque` here")
            .with_note(
                "`@opaque` says where the fields are is C's business, so C is what makes one and hands it over",
            );
            self.report(diagnostic);
            return self.give_up(fields.values(), span);
        }
        // A call's arguments are its fields: by position, in the order the
        // fields are declared, then by name.
        let written = fields;
        let Some(fields) = self.written_fields(id, name, written) else {
            return self.give_up(written.values(), span);
        };
        let fields = &fields[..];
        // A union is one field over the bytes of all of them, so a literal
        // names one or none, and the rest of the bytes are zero.
        if self.program.structs[id].is_union {
            return self.union_literal(id, name, fields, span);
        }
        // Its fields' defaults, which a literal in another's default may
        // need before their turn.
        self.ensure_defaults(TypeDef::Struct(id));
        let decls = self.program.structs[id].fields.clone();
        let generics = self.program.structs[id].generics.clone();
        let struct_name = self.text(name.sym);
        // Which field each initializer sets: `None` for an unknown field.
        // The values are checked together below, since they may decide the
        // struct's type arguments.
        let mut targets: Vec<Option<(usize, bool)>> = Vec::new();
        let mut set = vec![false; decls.len()];
        for f in fields {
            let Some(i) = decls.iter().position(|d| d.name == f.name.sym) else {
                let field = self.text(f.name.sym);
                let mut diagnostic = Diagnostic::error(
                    codes::STRUCT_LITERAL_FIELDS,
                    format!("struct `{struct_name}` has no field `{field}`"),
                    f.name.span,
                    "unknown field",
                );
                let similar = suggest(field, decls.iter().map(|d| self.text(d.name)));
                if let Some(similar) = similar {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `{similar}`?"),
                        [Edit::replace(f.name.span, similar)],
                    );
                }
                self.report(diagnostic);
                // Treat a misspelt field as the field it was probably meant to
                // be, so the literal is not also reported as missing it.
                let meant = similar
                    .and_then(|s| decls.iter().position(|d| self.text(d.name) == s))
                    .filter(|&i| !set[i]);
                if let Some(i) = meant {
                    set[i] = true;
                }
                targets.push(meant.map(|i| (i, false)));
                continue;
            };
            if set[i] {
                let diagnostic = Diagnostic::error(
                    codes::STRUCT_LITERAL_FIELDS,
                    format!("field `{}` is set more than once", self.text(f.name.sym)),
                    f.name.span,
                    "set again here",
                );
                self.report(diagnostic);
            }
            // Another module may set only what is `pub var`: a `pub`
            // field is read out there and written here, and building a
            // value is writing every one of its fields.
            if !self.field_visible(id, i) {
                self.private_field(id, i, f.name.span);
            } else if !self.field_settable(id, i) {
                self.readonly_field(id, i, f.name.span, "set here");
            }
            set[i] = true;
            targets.push(Some((i, true)));
        }
        // Type arguments written, or taken from the expected type, were
        // checked where that type was made.
        let mut inference = Inference::new(match known {
            Some(known) => known.into_iter().map(Some).collect(),
            None if type_args.is_some() => {
                self.explicit_type_args(name.sym, &generics, type_args, name_segment)
            }
            None => self.expected_args(TypeDef::Struct(id), hint),
        });
        // A view's reference field is given a `&`, as a parameter is.
        let is_view = self.program.structs[id].is_view;
        let operands: Vec<GenericOperand<'_>> = fields
            .iter()
            .zip(&targets)
            .filter_map(|(f, target)| {
                let (i, declared_here) = (*target)?;
                Some(GenericOperand {
                    expr: f.value,
                    declared: decls[i].ty,
                    context: declared_here.then_some((decls[i].span, "field declared here")),
                    is_arg: is_view && matches!(self.kind(decls[i].ty), TyKind::Ref(..)),
                })
            })
            .collect();
        let checked = self.generic_operands(&operands, &mut inference.tys);
        let mut checked = checked.into_iter();
        let mut values: Vec<Option<ExprId>> = vec![None; decls.len()];
        // The fields are evaluated in the order they are written.
        let mut written = Vec::new();
        for (f, target) in fields.iter().zip(&targets) {
            match target {
                Some((i, _)) => {
                    let value = checked.next().expect("one value per target");
                    if values[*i].is_none() {
                        values[*i] = Some(value);
                        written.push(*i as u32);
                    }
                }
                None => {
                    self.infer(f.value, None);
                }
            }
        }
        let mut list = crate::TyList::EMPTY;
        if !generics.is_empty() {
            let written_form = format!(
                "{struct_name}<{}> {{ … }}",
                vec!["…"; generics.len()].join(", ")
            );
            let tys = self.finish_type_args(struct_name, &written_form, &generics, inference, span);
            list = self.program.types.intern_list(&tys);
        }
        let ty = self.intern(TyKind::Struct(id, list));
        // `..value`: the fields the literal does not name are copied from a
        // value that has them.
        if let Some(rest) = rest {
            let taken: Vec<usize> = (0..decls.len()).filter(|&i| values[i].is_none()).collect();
            for (i, value) in self.rest_fields(rest, ty, &taken, name, span) {
                values[i] = Some(value);
                written.push(i as u32);
            }
        }
        // A field that is still not set takes its default, evaluated here.
        // The value after `..` comes first: it is what
        // the literal names, and a default is what the declaration says.
        let mark = self.state.unsettled_defaults.len();
        for i in 0..decls.len() {
            if values[i].is_some() {
                continue;
            }
            let Some(default) = decls[i].default.clone() else {
                continue;
            };
            values[i] = Some(self.use_default(&default, span));
            written.push(i as u32);
        }
        let args = self.program.types.list(list).to_vec();
        self.settle_defaults(mark, &args);
        // A C struct takes C's zero for a field it is not given, as a
        // designated initializer leaves one.
        let c_struct = self.program.structs[id].is_extern && !self.program.structs[id].is_union;
        for i in 0..decls.len() {
            if !c_struct || values[i].is_some() {
                continue;
            }
            let field_ty = self.program.types.subst(decls[i].ty, &args);
            if self.zeroable(field_ty) {
                values[i] = Some(self.alloc(ExprKind::Zeroed, field_ty, span));
                written.push(i as u32);
            }
        }
        // A field another module cannot name is one it cannot leave out
        // either: the value has to be built where the field is seen.
        let hidden: Vec<usize> = (0..decls.len())
            .filter(|&i| values[i].is_none() && !self.field_settable(id, i))
            .collect();
        for i in hidden {
            if self.field_visible(id, i) {
                self.readonly_field(id, i, name.span, "not set here");
            } else {
                self.private_field(id, i, name.span);
            }
            values[i] = Some(self.error_expr(span));
            written.push(i as u32);
        }
        let missing: Vec<String> = decls
            .iter()
            .zip(&values)
            .filter(|(_, v)| v.is_none())
            .map(|(d, _)| format!("`{}`", self.text(d.name)))
            .collect();
        if !missing.is_empty() {
            let diagnostic = Diagnostic::error(
                codes::STRUCT_LITERAL_FIELDS,
                format!(
                    "missing {} in `{struct_name}`",
                    if missing.len() == 1 {
                        "field"
                    } else {
                        "fields"
                    }
                ),
                name.span,
                format!("missing {}", missing.join(", ")),
            );
            self.report(diagnostic);
        }
        let mut field_values = Vec::new();
        for (i, value) in values.iter().enumerate() {
            if value.is_none() {
                written.push(i as u32);
            }
        }
        for value in values {
            let value = match value {
                Some(value) => value,
                None => self.error_expr(span),
            };
            field_values.push(value);
        }
        self.alloc(
            ExprKind::Struct {
                id,
                fields: field_values,
                order: order_of(written),
            },
            ty,
            span,
        )
    }

    /// Resolves `Enum::Variant` to the enum and variant index, reporting
    /// what is wrong if it does not resolve.
    /// The enum and variant that `Enum::Variant` or `.Variant` names. Without
    /// an enum name, the enum is the one expected here.
    pub(super) fn resolve_variant(
        &mut self,
        enum_name: Option<(usize, ast::Name)>,
        variant: ast::Name,
        expected: Option<Ty>,
    ) -> Option<(EnumId, u32)> {
        let id = match enum_name {
            Some((module, enum_name)) => {
                let id = self.named_enum(module, enum_name)?;
                self.program
                    .names
                    .push((enum_name.span, Named::Owner(TypeDef::Enum(id))));
                id
            }
            None => self.expected_enum(variant, expected)?,
        };
        let text = self.text(self.program.enums[id].name).to_string();
        let variants = &self.program.enums[id].variants;
        if let Some(i) = variants.iter().position(|v| v.name == variant.sym) {
            self.program
                .names
                .push((variant.span, Named::Variant(id, i as u32)));
            return Some((id, i as u32));
        }
        let name = self.text(variant.sym);
        let mut diagnostic = Diagnostic::error(
            codes::UNKNOWN_VARIANT,
            format!("no variant `{name}` in enum `{text}`"),
            variant.span,
            "unknown variant",
        );
        if let Some(similar) = suggest(name, variants.iter().map(|v| self.text(v.name))) {
            diagnostic = diagnostic.with_fix(
                format!("did you mean `{similar}`?"),
                [Edit::replace(variant.span, similar)],
            );
        }
        self.report(diagnostic);
        None
    }

    /// The enum named by `Enum` in `Enum::Variant`.
    fn named_enum(&mut self, module: usize, enum_name: ast::Name) -> Option<EnumId> {
        let text = self.text(enum_name.sym);
        // A type of the prelude, where the path named this module.
        let own = self.modules[module].types.get(&enum_name.sym).copied();
        let found = own.or_else(|| {
            (module == self.current)
                .then(|| Some((self.prelude_type(enum_name.sym)?, enum_name.span)))
                .flatten()
        });
        let id = match found.as_ref() {
            Some(&(TypeDef::Enum(id), _)) => id,
            Some(&(TypeDef::Builtin(_), _)) => unreachable!("a module declares no built-in type"),
            Some(&(TypeDef::Struct(_), _)) => {
                let diagnostic = Diagnostic::error(
                    codes::UNKNOWN_VARIANT,
                    format!("`{text}` is a struct, not an enum"),
                    enum_name.span,
                    "not an enum",
                );
                self.report(diagnostic);
                return None;
            }
            // `package::NAME` where nothing names the package.
            None if text == "package" => {
                let diagnostic = Diagnostic::error(
                    codes::PACKAGE,
                    "this program is not a package",
                    enum_name.span,
                    "`package::` names the package the code is in",
                )
                .with_help("write a `package.wip` beside the program, holding `package NAME`");
                self.report(diagnostic);
                return None;
            }
            None => {
                let candidates = self.modules[module].types.keys().map(|&s| self.text(s));
                // `Name::member` may be an enum's variant or a type's
                // static function, so what is missing is a type.
                let mut diagnostic = Diagnostic::error(
                    codes::UNKNOWN_TYPE,
                    format!("cannot find type `{text}`"),
                    enum_name.span,
                    "unknown type",
                );
                if let Some(help) = left_the_prelude(text) {
                    diagnostic = diagnostic.with_help(help);
                } else if let Some(similar) = suggest(text, candidates) {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `{similar}`?"),
                        [Edit::replace(enum_name.span, similar)],
                    );
                }
                self.report(diagnostic);
                return None;
            }
        };
        if !self.visible(module, self.program.enums[id].is_pub) {
            self.private_item(module, "enum", enum_name);
            return None;
        }
        Some(id)
    }

    /// The enum a `.Variant` belongs to: the one expected where it is written,
    /// through any `own` or reference around it.
    fn expected_enum(&mut self, variant: ast::Name, expected: Option<Ty>) -> Option<EnumId> {
        let expected = expected.map(|ty| self.under_refs(ty));
        // With nothing expected, `.Some` and `.None` are the prelude's
        // `Option`, whose one type argument `.Some(x)` says.
        if expected.is_none()
            && matches!(self.text(variant.sym), "Some" | "None")
            && let Some(id) = self.program.prelude_items.enumeration(KnownEnum::Option)
        {
            return Some(id);
        }
        match expected.map(|ty| (ty, self.kind(ty))) {
            Some((_, TyKind::Enum(id, _))) => return Some(id),
            Some((ty, _)) if self.is_poisoned(ty) => return None,
            other => {
                let name = self.text(variant.sym);
                // A name in lower case is a method's or a field's:
                // most likely a chain meant to go on.
                let help = if name.starts_with(|c: char| c.is_lowercase()) {
                    format!(
                        "to call `{name}` on the value above, end that line with the `.`, or wrap the chain in parentheses"
                    )
                } else {
                    format!("write `Enum::{name}`, or give the value it belongs to a type")
                };
                let mut diagnostic = Diagnostic::error(
                    codes::CANNOT_INFER,
                    format!("cannot tell which enum `.{name}` belongs to"),
                    variant.span,
                    "no enum is expected here",
                )
                .with_help(help);
                if let Some((ty, _)) = other {
                    let ty = self.ty_name(ty);
                    diagnostic = diagnostic.with_note(format!("the expected type is {ty}"));
                }
                self.report(diagnostic);
            }
        }
        None
    }

    /// Looks through `own` and references to the value's own type.
    pub(super) fn under_refs(&self, mut ty: Ty) -> Ty {
        loop {
            match self.kind(ty) {
                TyKind::Own(inner) | TyKind::Ref(inner, _) => ty = inner,
                _ => return ty,
            }
        }
    }

    /// A variant, with the type arguments written in its path, if any.
    pub(super) fn variant_lit(
        &mut self,
        enum_name: Option<(usize, ast::Name)>,
        variant: ast::Name,
        item: ItemUse<'a>,
    ) -> ExprId {
        let (args, hint, span) = (item.args, item.hint, item.span);
        let Some((id, index)) = self.resolve_variant(enum_name, variant, hint) else {
            return self.give_up(args.unwrap_or_default().iter().copied(), span);
        };
        // Defaults that are code, called with the variant's type arguments
        // once they are known.
        let defaults = self.state.unsettled_defaults.len();
        self.ensure_defaults(TypeDef::Enum(id));
        let fields = self.program.enums[id].variants[index as usize]
            .fields
            .clone();
        let generics = self.program.enums[id].generics.clone();
        let enum_sym = self.program.enums[id].name;
        let is_view = self.program.enums[id].is_view;
        let path = format!("{}::{}", self.text(enum_sym), self.text(variant.sym));
        let mut inference = Inference::new(if item.type_args.is_some() {
            self.explicit_type_args(enum_sym, &generics, item.type_args, item.name_segment)
        } else {
            let expected = hint.map(|h| self.under_refs(h));
            self.expected_args(TypeDef::Enum(id), expected)
        });
        let mut hir_args = Vec::new();
        let mut order = Vec::new();
        match args {
            None if !fields.is_empty() => {
                let diagnostic = Diagnostic::error(
                    codes::ARGUMENT_COUNT,
                    format!("`{path}` has {}", plural(fields.len(), "field", "fields")),
                    span,
                    "missing its fields",
                );
                // The one field is `void`, here: `.Ok()` says there is a
                // payload and it is nothing.
                let void_field = match &fields[..] {
                    [only] => {
                        let ty = match self.kind(only.ty) {
                            TyKind::Param(param) => inference
                                .tys
                                .get(param.index as usize)
                                .copied()
                                .flatten()
                                .unwrap_or(only.ty),
                            _ => only.ty,
                        };
                        ty == Types::UNIT
                    }
                    _ => false,
                };
                // Every field has a default: `()` takes them all, as a
                // call does.
                let diagnostic = if void_field {
                    let written = match enum_name {
                        Some(_) => path.clone(),
                        None => format!(".{}", self.text(variant.sym)),
                    };
                    diagnostic
                        .with_fix(
                            format!("write `{written}()`"),
                            [Edit::insert(span.hi, "()")],
                        )
                        .with_note("a field of type `void` is written with nothing in the parentheses: the parentheses say there is a payload, and that it is nothing")
                } else if fields.iter().all(|f| f.default.is_some()) {
                    diagnostic.with_fix(
                        format!("write `{path}()`, which takes the defaults"),
                        [Edit::insert(span.hi, "()")],
                    )
                } else {
                    diagnostic.with_help(format!("write `{path}(…)`"))
                };
                self.report(diagnostic);
            }
            None => {}
            Some(args) if fields.is_empty() => {
                let parens = Span::new(variant.span.hi, span.hi);
                let mut diagnostic = Diagnostic::error(
                    codes::ARGUMENT_COUNT,
                    format!("`{path}` has no fields"),
                    parens,
                    "unexpected parentheses",
                );
                if args.is_empty() {
                    diagnostic =
                        diagnostic.with_fix("remove the parentheses", [Edit::replace(parens, "")]);
                }
                self.report(diagnostic);
                for &a in args {
                    self.infer(a, None);
                }
            }
            // `.Ok()` where the payload is nothing: the parentheses say
            // there is one, and writing `{}` for it says so twice.
            Some(args)
                if args.is_empty()
                    && fields.iter().all(|f| self.holds_nothing(f.ty, &inference)) =>
            {
                hir_args = fields.iter().map(|_| self.nothing(span)).collect();
                order = (0..fields.len() as u32).collect();
            }
            Some(args) => {
                let names = self.field_names(&path, &fields, args, item.arg_names());
                // A field left out takes its default, as a parameter does.
                let slots: Vec<Slot> = fields
                    .iter()
                    .map(|f| Slot {
                        name: f.name,
                        ty: f.ty,
                        span: f.span,
                        default: f.default.clone(),
                    })
                    .collect();
                let matched = self.match_arguments(&slots, args, &names, "field", &path);
                let missing = matched.missing(&slots);
                if !missing.is_empty() || matched.extra > 0 {
                    // Where a field has a default, the count says nothing:
                    // what is missing is named.
                    let plain = names.iter().all(Option::is_none)
                        && slots.iter().all(|s| s.default.is_none());
                    let diagnostic = if missing.is_empty() || plain {
                        Diagnostic::error(
                            codes::ARGUMENT_COUNT,
                            format!(
                                "`{path}` has {} but {} given",
                                plural(fields.len(), "field", "fields"),
                                plural(args.len(), "value was", "values were")
                            ),
                            span,
                            format!("expected {}", plural(fields.len(), "value", "values")),
                        )
                    } else {
                        let listed: Vec<String> = missing
                            .iter()
                            .map(|&i| format!("`{}`", self.text(fields[i].name)))
                            .collect();
                        Diagnostic::error(
                            codes::ARGUMENT_COUNT,
                            format!(
                                "`{path}` is missing {} {}",
                                if missing.len() == 1 {
                                    "the field"
                                } else {
                                    "the fields"
                                },
                                listed.join(", ")
                            ),
                            span,
                            format!("missing {}", listed.join(", ")),
                        )
                    };
                    self.report(diagnostic);
                }
                // The values as written.
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
                        declared: fields[slot].ty,
                        context: Some((fields[slot].span, "field declared here")),
                        // A view's reference field is given a `&`, as a
                        // parameter is.
                        is_arg: is_view && matches!(self.kind(fields[slot].ty), TyKind::Ref(..)),
                    })
                    .collect();
                let values = self.generic_operands(&operands, &mut inference.tys);
                let mut checked: Vec<Option<ExprId>> = vec![None; args.len()];
                for (&(arg, _), value) in written.iter().zip(values) {
                    checked[arg] = Some(value);
                }
                for &arg in &matched.unmatched {
                    self.infer(args[arg], None);
                }
                let (values, arranged) = self.arrange_arguments(&slots, &matched, &checked, span);
                hir_args = values;
                order = arranged;
            }
        }
        let mut list = crate::TyList::EMPTY;
        if !generics.is_empty() {
            let written = format!(
                "{}<{}>::{}",
                self.text(enum_sym),
                vec!["…"; generics.len()].join(", "),
                self.text(variant.sym)
            );
            let tys = self.finish_type_args(&path, &written, &generics, inference, span);
            list = self.program.types.intern_list(&tys);
            self.settle_defaults(defaults, &tys);
        }
        let ty = self.intern(TyKind::Enum(id, list));
        self.alloc(
            ExprKind::Variant {
                id,
                variant: index,
                args: hir_args,
                order,
            },
            ty,
            span,
        )
    }

    /// Whether a field holds nothing in this instance: its type is `void`,
    /// or a parameter the instance fills with `void`.
    fn holds_nothing(&self, ty: Ty, inference: &Inference) -> bool {
        match self.kind(ty) {
            TyKind::Param(param) => {
                inference.tys.get(param.index as usize).copied().flatten() == Some(Types::UNIT)
            }
            _ => ty == Types::UNIT,
        }
    }

    /// The unit value, which is what an empty block is.
    fn nothing(&mut self, span: Span) -> ExprId {
        let block = Block {
            stmts: Vec::new(),
            value: None,
            span,
        };
        self.alloc(ExprKind::Block(block), Types::UNIT, span)
    }

    /// The name of the field each value of a variant or a struct is for.
    /// With two or more fields, every value is named, as a call's are where
    /// they must be: a value by position is reported, with a fix that names
    /// it — a variable with a field's name, that field; anything else, the
    /// field in its position. It is then taken as the field in its position,
    /// so that the rest of the literal is still checked.
    fn field_names(
        &mut self,
        path: &str,
        fields: &[FieldDef],
        args: &[ast::ExprId],
        mut names: Vec<Option<ast::Name>>,
    ) -> Vec<Option<ast::Name>> {
        if fields.len() < 2 {
            return names;
        }
        let ast = self.ast;
        let positional: Vec<usize> = (0..names.len()).filter(|&i| names[i].is_none()).collect();
        if positional.is_empty() {
            return names;
        }
        // The field each value is meant for: a variable named as one of the
        // fields, that one; anything else, the first field nothing else
        // names, in order.
        let named_as = |i: usize| match ast.exprs[args[i]].kind {
            ast::ExprKind::Name(sym) if fields.iter().any(|f| f.name == sym) => Some(sym),
            _ => None,
        };
        let mut taken: Vec<Symbol> = names.iter().flatten().map(|n| n.sym).collect();
        taken.extend(positional.iter().filter_map(|&i| named_as(i)));
        let mut free = fields.iter().map(|f| f.name).filter(|f| !taken.contains(f));
        let meant: FxHashMap<usize, Symbol> = positional
            .iter()
            .filter_map(|&i| Some((i, named_as(i).or_else(|| free.next())?)))
            .collect();
        let meant = |i: usize| meant.get(&i).copied();
        let edits: Vec<Edit> = positional
            .iter()
            .filter_map(|&i| {
                let field = self.text(meant(i)?);
                Some(Edit::insert(
                    ast.exprs[args[i]].span.lo,
                    format!("{field}: "),
                ))
            })
            .collect();
        let first = ast.exprs[args[positional[0]]].span;
        let mut diagnostic = Diagnostic::error(
            codes::UNNAMED_FIELDS,
            format!("the fields of `{path}` are named where it is built"),
            first,
            "a value without its field's name",
        )
        .with_note("a variant or a struct with two or more fields names each one, a variable of a field's name too; with one field, a name is optional");
        if edits.len() == positional.len() {
            diagnostic = diagnostic.with_fix("name the fields", edits);
        }
        self.report(diagnostic);
        for i in positional {
            if let Some(sym) = meant(i) {
                names[i] = Some(ast::Name {
                    sym,
                    span: ast.exprs[args[i]].span,
                });
            }
        }
        names
    }

    /// `Enum.Variant` written with a dot, which is not Wip. If
    /// `base` names an enum, reports it with a fix and returns the variant
    /// literal it was meant to be.
    pub(super) fn variant_with_dot(
        &mut self,
        base: ast::ExprId,
        name: ast::Name,
        item: ItemUse<'a>,
    ) -> Option<ExprId> {
        let ast = self.ast;
        let ast::ExprKind::Name(sym) = ast.exprs[base].kind else {
            return None;
        };
        if self.lookup(sym).is_some() {
            return None;
        }
        // The enum may be one the file imported by name.
        let named = ast::Name {
            sym,
            span: ast.exprs[base].span,
        };
        let (module, enum_sym) = match self.imported(named) {
            Some(ImportedItem::Item(module, item)) => (module, item.sym),
            Some(ImportedItem::Broken) => return None,
            None => (self.current, sym),
        };
        let Some(&(TypeDef::Enum(id), _)) = self.modules[module].types.get(&enum_sym) else {
            return None;
        };
        // Only a variant: `Json.parse` is not one written with a dot, and
        // neither is what an interpolation of the enum's name calls on it.
        if !self.program.enums[id]
            .variants
            .iter()
            .any(|v| v.name == name.sym)
        {
            return None;
        }
        let base_span = ast.exprs[base].span;
        let path = format!("{}::{}", self.text(sym), self.text(name.sym));
        let diagnostic = Diagnostic::error(
            codes::VARIANT_WITH_DOT,
            "enum variants are written with `::`",
            base_span.to(name.span),
            format!("write `{path}`"),
        )
        .with_note("after a name, `.` accesses a field; a variant is `Enum::Variant`, or `.Variant` where the enum is known")
        .with_fix(
            "use `::`",
            [Edit::replace(Span::new(base_span.hi, name.span.lo), "::")],
        );
        self.report(diagnostic);
        Some(self.variant_lit(
            Some((
                module,
                ast::Name {
                    sym: enum_sym,
                    span: base_span,
                },
            )),
            name,
            ItemUse { hint: None, ..item },
        ))
    }
}
