//! What `@derive` asks for, and the numbering an enum whose variants carry
//! nothing gets.
//!
//! What `@derive` writes is code of the module's own, expanded before the
//! module is checked (`wip_syntax::derive`); here each type
//! that asked has its fields asked for the interface, where the code is
//! written, and a field that lacks it is reported where it is written.
//! `count` and `fromIndex` are declared here as methods of the type, with
//! `FnDef::generated` saying which body the code generator is to write.

use super::*;
use annotations::Derived;
use items::Declared;

impl<'a> Lowerer<'a> {
    /// Where `@derive` names this interface on a declaration, if it does.
    /// What it may name was checked when the type was collected; this only
    /// asks what is there.
    fn derives(&self, written: &[ast::Annotation], wanted: Derived) -> Option<Span> {
        written
            .iter()
            .filter(|a| self.text(a.name.sym) == "derive")
            .flat_map(|a| a.args.iter())
            .find(|arg| {
                matches!(arg.value, ast::AnnotationValue::Name(sym) if self.text(sym) == wanted.name())
            })
            .map(|arg| arg.span)
    }

    /// Records which type each argument of `@derive` was written on, which
    /// what it wrote is for.
    pub(super) fn note_derives(&mut self, types: &[(Declared<'a>, TypeDef)]) {
        for &(item, def) in types {
            let annotations = item.annotations();
            if annotations.is_empty() {
                continue;
            }
            let interner = self.interner;
            for arg in annotations
                .iter()
                .filter(|a| interner.resolve(a.name.sym) == "derive")
                .flat_map(|a| a.args.iter())
            {
                self.derive_owners.insert(arg.span, def);
            }
        }
    }

    /// The types that asked for `@derive`, once every implementation is
    /// declared: only then can a field's type be asked whether it compares.
    pub(super) fn declare_equality(&mut self, types: &[(Declared<'a>, TypeDef)]) {
        for &(item, def) in types {
            let annotations = item.annotations();
            if annotations.is_empty() {
                continue;
            }
            let eq = self.derives(annotations, Derived::Eq);
            let ord = self.derives(annotations, Derived::Ord);
            let hash = self.derives(annotations, Derived::Hash);
            let text = self.derives(annotations, Derived::Text);
            let clone = self.derives(annotations, Derived::Clone);
            if eq.is_none() && ord.is_none() && hash.is_none() && text.is_none() && clone.is_none()
            {
                continue;
            }
            self.type_params = self.type_generics(def).to_vec();
            if let Some(at) = eq {
                self.check_derived_fields(def, at, Derived::Eq);
            }
            if let Some(at) = ord {
                self.check_orderable(def, at, eq.is_some());
            }
            if let Some(at) = hash {
                self.check_hashable(def, at, eq.is_some());
            }
            if let Some(at) = text {
                self.check_derived_fields(def, at, Derived::Text);
            }
            if let Some(at) = clone {
                self.check_derived_fields(def, at, Derived::Clone);
            }
        }
        self.type_params.clear();
    }

    /// What `@derive` wrote for `owner` — an `equals`, a `compare` or a
    /// `clone`, as code of the module's own — asks each
    /// field for the interface. This says which field keeps it from
    /// applying, where the field is written rather than at the annotation
    /// the code is placed at, and then leaves the code unchecked.
    fn check_derived_fields(&mut self, owner: TypeDef, at: Span, wanted: Derived) {
        let interface = match wanted {
            Derived::Eq => self.program.prelude_items.interface(KnownInterface::Eq),
            Derived::Ord => self.program.prelude_items.interface(KnownInterface::Ord),
            Derived::Hash => self.program.prelude_items.interface(KnownInterface::Hash),
            Derived::Text => self.program.prelude_items.interface(KnownInterface::Text),
            Derived::Clone => self.program.prelude_items.interface(KnownInterface::Clone),
        };
        let Some(interface) = interface else {
            return;
        };
        // Nothing was written where the implementation was refused, as a
        // second one of the type's, and reported.
        let Some(written) = self.derived_impl(owner, interface) else {
            return;
        };
        // Each field is asked where the method is written, whose type
        // parameters are bound by the interface too: a `Vec<T>` field has
        // it where `T` does.
        let outer = std::mem::replace(
            &mut self.type_params,
            self.program.fns[written[0]].generics.clone(),
        );
        let mut applies = true;
        for (name, ty, span) in self.fields_of(owner) {
            let kind = self.kind(ty);
            // What an `own<T>` or a reference points at may have it, which
            // a call looks through to; the field itself does not. A clone
            // is asked of the field's own type, whose `clone` a `&T` has.
            let pointer =
                wanted != Derived::Clone && matches!(kind, TyKind::Own(_) | TyKind::Ref(..));
            // Whether it is plain data is read from its fields' types.
            crate::mono::complete_type(&mut self.program, ty);
            // Two pointers compare by where they point.
            let compared = wanted == Derived::Eq && matches!(kind, TyKind::Ptr(_));
            if self.is_poisoned(ty) || compared || (!pointer && self.implements(ty, interface)) {
                continue;
            }
            applies = false;
            let diagnostic = self.field_lacks(wanted, interface, (name, ty, span), at);
            self.report(diagnostic);
        }
        self.type_params = outer;
        if !applies {
            self.withheld.extend(written);
        }
    }

    /// The report of a field whose type lacks what `@derive` asks of it.
    fn field_lacks(
        &self,
        wanted: Derived,
        interface: InterfaceId,
        (name, ty, span): (Symbol, Ty, Span),
        at: Span,
    ) -> Diagnostic {
        let field = self.text(name).to_string();
        let type_name = self.ty_name(ty).to_string();
        let (code, lacks, label, writes, note) = match wanted {
            Derived::Eq => (
                codes::NOT_COMPARABLE,
                "cannot be compared",
                "not a comparable field",
                "`@derive(Eq)` writes the comparison field by field",
                "a field is comparable when it is a number, a `bool`, a `str`, a `cstring`, a `ptr<T>`, an array of those, or a type that implements `Eq`",
            ),
            Derived::Ord => (
                codes::NOT_COMPARABLE,
                "has no order",
                "not an ordered field",
                "`@derive(Ord)` writes the comparison field by field",
                "a field is ordered when it is a number, a `bool`, a `str`, an array of those, or a type that implements `Ord`",
            ),
            Derived::Text => (
                codes::NOT_COMPARABLE,
                "cannot be written",
                "not a writable field",
                "`@derive(Text)` writes the value field by field",
                "a field is writable when it is a number, a `bool`, a `str`, a `cstring`, an array of those, or a type that implements `Text`",
            ),
            Derived::Hash => (
                codes::NOT_COMPARABLE,
                "is not hashed",
                "not a hashable field",
                "`@derive(Hash)` writes the hashing part by part",
                "a field is hashed when it is an integer, a `bool`, a `str`, a `cstring`, an array of those, or a type that implements `Hash`; the floats are left out, since `0.0` and `-0.0` are equal and their bytes are not",
            ),
            _ => (
                codes::NOT_CLONEABLE,
                "cannot be cloned",
                "not a field that clones",
                "`@derive(Clone)` writes the clone field by field",
                "a field clones when its type implements `Clone`: plain data, which owns no memory, is its own clone, and so are a `String`, a `Vec`, an `Option` or a tuple of what clones, and a type that asks for `@derive(Clone)` or writes `extend …: Clone`",
            ),
        };
        let mut diagnostic = Diagnostic::error(
            code,
            format!("`{field}` is {type_name}, which {lacks}"),
            span,
            label,
        )
        .with_secondary(at, writes)
        .with_note(note);
        let named = wanted.name();
        let declared = |ty: Ty| matches!(self.kind(ty), TyKind::Struct(..) | TyKind::Enum(..));
        // A type that has it where its arguments do is told which does not:
        // `Vec<T>` compares where `T` does.
        let constraint = crate::Constraint {
            interface,
            args: crate::TyList::EMPTY,
        };
        if let Some((arg, _)) = self.unmet_for(ty, constraint) {
            let inside = self.ty_name(arg).to_string();
            diagnostic = diagnostic.with_help(format!(
                "{type_name} implements `{named}` where {inside} does, and {inside} does not"
            ));
            if declared(arg) {
                diagnostic = diagnostic.with_help(format!(
                    "write `@derive({named})` on {inside} too, or an `extend` block that implements `{named}` for it"
                ));
            }
        } else if declared(ty) {
            diagnostic = diagnostic.with_help(format!(
                "write `@derive({named})` on {type_name} too, or an `extend` block that implements `{named}` for it"
            ));
        }
        diagnostic
    }

    /// Every field of a struct, or of each variant of an enum, with its
    /// type and where it was written.
    fn fields_of(&self, owner: TypeDef) -> Vec<(Symbol, Ty, Span)> {
        match owner {
            TypeDef::Struct(id) => self.program.structs[id]
                .fields
                .iter()
                .map(|f| (f.name, f.ty, f.span))
                .collect(),
            TypeDef::Enum(id) => self.program.enums[id]
                .variants
                .iter()
                .flat_map(|v| v.fields.iter())
                .map(|f| (f.name, f.ty, f.span))
                .collect(),
            TypeDef::Builtin(_) => Vec::new(),
        }
    }

    /// Leaves what `@derive` wrote for `owner`'s implementation of
    /// `interface` unchecked, where it has been refused and reported.
    fn withhold_derived(&mut self, owner: TypeDef, interface: Option<InterfaceId>) {
        if let Some(written) = interface.and_then(|i| self.derived_impl(owner, i)) {
            self.withheld.extend(written);
        }
    }

    /// The methods of `owner`'s implementation of `interface` where
    /// `@derive` wrote it: those it wrote first, then the
    /// interface's defaults the implementation has with them.
    fn derived_impl(&self, owner: TypeDef, interface: InterfaceId) -> Option<Vec<FnId>> {
        let derived = &self.program.derived;
        self.program
            .impls
            .iter()
            // An interface's default methods are the implementation's too,
            // and nothing derived them, as `Text`'s: the
            // implementation is the derived one where `@derive` wrote any
            // of it, and those come first.
            .find(|i| {
                i.ty == owner
                    && i.interface == interface
                    && i.methods.iter().any(|m| derived.contains(m))
            })
            .map(|i| {
                let mut methods = i.methods.clone();
                methods.sort_by_key(|m| !derived.contains(m));
                methods
            })
    }

    /// An enum whose variants carry nothing gets `count` and `fromIndex`,
    /// whose bodies the compiler writes, and `all`.
    /// A program that declares a method of one of those
    /// names keeps its own.
    pub(super) fn declare_enum_numbers(&mut self, types: &[(Declared<'a>, TypeDef)]) {
        for &(item, def) in types {
            let (Some(e), TypeDef::Enum(id)) = (item.enum_decl(), def) else {
                continue;
            };
            if self.program.enums[id]
                .variants
                .iter()
                .any(|v| !v.fields.is_empty())
            {
                continue;
            }
            self.type_params = self.type_generics(def).to_vec();
            let at = e.name.span;
            let declared = |lowerer: &Self, name: &str| {
                lowerer.program.enums[id]
                    .methods
                    .iter()
                    .any(|&m| lowerer.text(lowerer.program.fns[m].name) == name)
            };
            if !declared(self, "count") {
                self.generated_static(
                    def,
                    Symbol::count(),
                    Vec::new(),
                    Types::I64,
                    Generated::VariantCount,
                    at,
                );
            }
            if !declared(self, "fromIndex")
                && let Some(option) = self.program.prelude_items.enumeration(KnownEnum::Option)
            {
                let self_ty = self.self_ty(def);
                let args = self.program.types.intern_list(&[self_ty]);
                let ret = self.intern(TyKind::Enum(option, args));
                self.generated_static(
                    def,
                    Symbol::from_index(),
                    vec![(Symbol::index_param(), Types::I64)],
                    ret,
                    Generated::FromIndex,
                    at,
                );
            }
            if !declared(self, "all") {
                let self_ty = self.self_ty(def);
                let count = self.program.enums[id].variants.len() as u64;
                let ret = self.intern(TyKind::Array(self_ty, count));
                self.generated_static(
                    def,
                    Symbol::all(),
                    Vec::new(),
                    ret,
                    Generated::AllVariants,
                    at,
                );
            }
            self.type_params.clear();
        }
    }

    /// A static function whose body the compiler writes.
    fn generated_static(
        &mut self,
        owner: TypeDef,
        name: Symbol,
        params: Vec<(Symbol, Ty)>,
        ret: Ty,
        generated: Generated,
        at: Span,
    ) -> FnId {
        let generics = self.type_generics(owner).to_vec();
        let params = params
            .into_iter()
            .map(|(name, ty)| ParamDef {
                name,
                name_span: at,
                ty,
                span: at,
                default: None,
            })
            .collect();
        let id = self.program.fns.alloc(FnDef {
            name,
            name_span: at,
            symbol: None,
            receiver: Some(Receiver::Static),
            owner: Some(owner),
            interface: None,
            generics,
            instance_of: None,
            params,
            ret,
            ret_span: None,
            is_extern: false,
            exports_c: false,
            header: None,
            accesses: None,
            is_variadic: false,
            variadic_of: None,
            is_lambda: false,
            generator: None,
            is_tailrec: false,
            is_test: false,
            is_inline: false,
            generated: Some(generated),
            intrinsic: None,
            body: None,
            projects: None,
            compile_time: false,
            module: self.current as u32,
            is_pub: true,
            span: at,
        });
        match owner {
            TypeDef::Struct(owner) => self.program.structs[owner].methods.push(id),
            TypeDef::Enum(owner) => self.program.enums[owner].methods.push(id),
            TypeDef::Builtin(_) => {}
        }
        id
    }

    /// `@derive(Ord)`: `Eq` beside it, and each field ordered.
    fn check_orderable(&mut self, owner: TypeDef, at: Span, has_eq: bool) {
        // Two values that compare `Same` and are not equal would make
        // every algorithm that uses both wrong.
        let equal = self
            .program
            .prelude_items
            .interface(KnownInterface::Eq)
            .is_some_and(|eq| {
                self.program
                    .impls
                    .iter()
                    .any(|i| i.interface == eq && i.ty == owner)
            });
        if !has_eq && !equal {
            let type_name = self.type_name(owner).to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_COMPARABLE,
                format!("`{type_name}` is ordered and does not say when two of it are equal"),
                at,
                "`Ord` derived without `Eq`",
            )
            .with_help("name `Eq` in the `@derive` as well")
            .with_note(
                "two values that compare `Same` and are not equal would make every algorithm that uses both wrong",
            );
            self.report(diagnostic);
            self.withhold_derived(
                owner,
                self.program.prelude_items.interface(KnownInterface::Ord),
            );
            return;
        }
        self.check_derived_fields(owner, at, Derived::Ord);
    }

    /// `@derive(Hash)`: `Eq` beside it, and each field hashed.
    fn check_hashable(&mut self, owner: TypeDef, at: Span, has_eq: bool) {
        // A map is wrong the moment two equal values hash differently, and
        // a hashing written from the fields only matches an equality
        // written from the fields.
        if !has_eq {
            let type_name = self.type_name(owner).to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_COMPARABLE,
                format!("`{type_name}` is hashed and does not say when two of it are equal"),
                at,
                "`Hash` derived without `Eq`",
            )
            .with_help("name `Eq` in the `@derive` as well, or write `extend …: Hash` by hand")
            .with_note(
                "a map is wrong the moment two equal values hash differently, and the compiler will not guess that a type whose equality is written by hand should be hashed the same way",
            );
            self.report(diagnostic);
            self.withhold_derived(
                owner,
                self.program.prelude_items.interface(KnownInterface::Hash),
            );
            return;
        }
        self.check_derived_fields(owner, at, Derived::Hash);
    }
}
