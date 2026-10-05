//! Interfaces: what a type may implement, and what a type parameter may be
//! required to implement.
//!
//! An interface's methods are checked with `Self` as its one type parameter,
//! so an implementation is that method with the type substituted for it, and
//! a default body is a generic function with one instance per implementing
//! type.

use super::*;

impl<'a> Lowerer<'a> {
    /// The interfaces of a file, by name. Their methods are declared later,
    /// once every type has a name.
    pub(super) fn collect_interfaces(&mut self) -> Vec<(&'a ast::InterfaceDecl, InterfaceId)> {
        let ast = self.ast;
        let mut interfaces = Vec::new();
        for item in &ast.items {
            let ast::Item::Interface(decl) = item else {
                continue;
            };
            let annotations =
                self.annotations(&decl.annotations, super::annotations::Target::Interface);
            // `interface From<T>` is about other types, and
            // `interface Add<Rhs = Self, Out = Self>` says what they are
            // where a use leaves them out.
            let generics = self.interface_generic_params(&decl.generics);
            let def = InterfaceDef {
                name: decl.name.sym,
                generics,
                methods: Vec::new(),
                // Checked against the methods once they are declared.
                one_of: annotations.one_of,
                module: self.current as u32,
                is_pub: decl.is_pub,
                span: decl.name.span,
            };
            let id = self.program.interfaces.alloc(def);
            // The interfaces of the prelude's that the compiler asks for,
            // each for what `KnownInterface` says.
            if self.prelude == Some(self.current)
                && let Some(known) = KnownInterface::named(self.text(decl.name.sym))
            {
                self.program.prelude_items.set_interface(known, id);
            }
            if self.prelude_name(decl.name, "an interface named") {
                interfaces.push((decl, id));
                continue;
            }
            match self.declared_at(decl.name.sym) {
                Some(first) => self.duplicate(decl.name, first),
                None => {
                    self.interfaces_mut()
                        .insert(decl.name.sym, (id, decl.name.span));
                }
            }
            interfaces.push((decl, id));
        }
        interfaces
    }

    /// A type of the prelude, which every file sees without an import.
    pub(super) fn prelude_type(&self, name: Symbol) -> Option<TypeDef> {
        let prelude = self.prelude?;
        if prelude == self.current {
            return None;
        }
        let &(def, _) = self.modules[prelude].types.get(&name)?;
        Some(def)
    }

    /// The prelude, when it holds a type of this name and is not the
    /// module being lowered.
    pub(super) fn prelude_module(&self, name: Symbol) -> Option<usize> {
        let prelude = self.prelude?;
        if prelude == self.current {
            return None;
        }
        self.modules[prelude]
            .types
            .contains_key(&name)
            .then_some(prelude)
    }

    /// A function of the prelude.
    pub(super) fn prelude_fn(&self, name: Symbol) -> Option<FnId> {
        let prelude = self.prelude?;
        if prelude == self.current {
            return None;
        }
        self.modules[prelude].fns.get(&name).copied()
    }

    /// A constant of the prelude, which every file sees.
    pub(super) fn prelude_const(&self, name: Symbol) -> Option<ConstId> {
        let prelude = self.prelude?;
        if prelude == self.current {
            return None;
        }
        self.modules[prelude].consts.get(&name).copied()
    }

    /// Reports a name the prelude declares, where another module declares
    /// or imports it. Returns whether it did.
    pub(super) fn prelude_name(&mut self, name: ast::Name, what: &str) -> bool {
        let Some(prelude) = self.prelude else {
            return false;
        };
        if prelude == self.current {
            return false;
        }
        // Only what the prelude exports is reserved: a name it keeps to
        // itself is in no file's scope, so nothing can mean two things.
        let scope = &self.modules[prelude];
        let is_type = scope
            .types
            .get(&name.sym)
            .is_some_and(|&(def, _)| match def {
                TypeDef::Struct(id) => self.program.structs[id].is_pub,
                TypeDef::Enum(id) => self.program.enums[id].is_pub,
                TypeDef::Builtin(_) => true,
            });
        let is_interface = scope
            .interfaces
            .get(&name.sym)
            .is_some_and(|&(id, _)| self.program.interfaces[id].is_pub);
        let is_fn = scope
            .fns
            .get(&name.sym)
            .is_some_and(|&id| self.program.fns[id].is_pub);
        let is_const = scope
            .consts
            .get(&name.sym)
            .is_some_and(|&id| self.program.consts[id].is_pub);
        if !(is_type || is_interface || is_fn || is_const) {
            return false;
        }
        let text = self.text(name.sym).to_string();
        let diagnostic = Diagnostic::error(
            codes::PRELUDE_NAME,
            format!("`{text}` is a name of the prelude"),
            name.span,
            format!("{what} `{text}`"),
        )
        .with_note(
            "the prelude's names are in scope in every file, and mean one thing in every program",
        )
        .with_help(format!(
            "give this one another name, or name the prelude's as `{}::{text}`",
            crate::PRELUDE
        ));
        self.report(diagnostic);
        true
    }

    /// Where a name is already a type or an interface of this module.
    pub(super) fn declared_at(&self, name: Symbol) -> Option<Span> {
        if let Some(&(_, span)) = self.types().get(&name) {
            return Some(span);
        }
        self.interfaces().get(&name).map(|&(_, span)| span)
    }

    /// The interface a constraint names, here or in the module it was
    /// imported from.
    pub(super) fn interface_named(&self, name: ast::Name) -> Option<InterfaceId> {
        if let Some(&(id, _)) = self.interfaces().get(&name.sym) {
            return Some(id);
        }
        // An interface of the prelude, which every file sees without an
        // import.
        if let Some(prelude) = self.prelude
            && prelude != self.current
            && let Some(&(id, _)) = self.modules[prelude].interfaces.get(&name.sym)
        {
            return Some(id);
        }
        let ImportedItem::Item(module, item) = self.imported(name)? else {
            return None;
        };
        self.modules[module]
            .interfaces
            .get(&item.sym)
            .map(|&(id, _)| id)
    }

    /// The signature of every method of every interface, with `Self` as the
    /// type parameter an implementation substitutes.
    pub(super) fn declare_interface_methods(
        &mut self,
        interfaces: &[(&'a ast::InterfaceDecl, InterfaceId)],
        bodies: &mut Vec<BodyWork>,
    ) {
        for &(decl, id) in interfaces {
            let mut methods = Vec::new();
            let mut seen: FxHashMap<Symbol, (Span, FnId)> = FxHashMap::default();
            for method in &decl.methods {
                let (receiver, keyword) = method.receiver;
                // A method's type parameters are `Self`, then the
                // interface's own.
                let mut type_params = vec![self.self_param(id, method.sig.name.span)];
                type_params.extend(self.program.interfaces[id].generics.iter().cloned());
                let member = Member {
                    receiver,
                    keyword,
                    owner: MemberOwner::Interface(id),
                    type_params,
                    overloaded: false,
                    lent: false,
                };
                let fn_id = self.declare_fn(&method.sig, &[], false, decl.is_pub, Some(member));
                // Two methods of one name are a lending pair, here as on a
                // type: one reads and one writes.
                match seen.get(&method.sig.name.sym) {
                    Some(&(first, id)) if !self.is_lending_pair(id, fn_id) => {
                        self.duplicate(method.sig.name, first);
                    }
                    Some(_) => {}
                    None => {
                        seen.insert(method.sig.name.sym, (method.sig.name.span, fn_id));
                    }
                }
                if let Some(body) = method.default {
                    bodies.push(BodyWork {
                        body,
                        id: fn_id,
                        span: method.span,
                    });
                }
                methods.push(InterfaceMethodDef {
                    id: fn_id,
                    has_default: method.default.is_some(),
                });
            }
            self.program.interfaces[id].methods = methods;
            self.check_one_of(id);
        }
    }

    /// Each `@oneOf` of an interface names two of its methods or more, each
    /// once, and each with a default, since one without is written anyway;
    /// a group that does not is reported, and dropped.
    fn check_one_of(&mut self, id: InterfaceId) {
        let groups = std::mem::take(&mut self.program.interfaces[id].one_of);
        let interface = self.text(self.program.interfaces[id].name).to_string();
        let mut kept = Vec::new();
        for group in groups {
            let mut fits = true;
            for (i, &(name, span)) in group.names.iter().enumerate() {
                let text = self.text(name).to_string();
                if let Some(&(_, first)) = group.names[..i].iter().find(|&&(n, _)| n == name) {
                    let diagnostic = Diagnostic::error(
                        codes::ANNOTATION,
                        format!("`{text}` is named twice"),
                        span,
                        "named again",
                    )
                    .with_secondary(first, "named here");
                    self.report(diagnostic);
                    fits = false;
                    continue;
                }
                let method = self.program.interfaces[id]
                    .methods
                    .iter()
                    .find(|m| self.program.fns[m.id].name == name)
                    .copied();
                match method {
                    None => {
                        let diagnostic = Diagnostic::error(
                            codes::ANNOTATION,
                            format!("`{interface}` has no method `{text}`"),
                            span,
                            "not a method of the interface",
                        )
                        .with_note("`@oneOf` names methods of the interface it is written on");
                        self.report(diagnostic);
                        fits = false;
                    }
                    Some(method) if !method.has_default => {
                        let diagnostic = Diagnostic::error(
                            codes::ANNOTATION,
                            format!("`{text}` has no default, so every implementation writes it"),
                            span,
                            "a method without a default",
                        )
                        .with_note("`@oneOf` names methods with defaults, each written with another, of which an implementation writes one at least");
                        self.report(diagnostic);
                        fits = false;
                    }
                    Some(_) => {}
                }
            }
            if group.names.len() < 2 {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    "`@oneOf` names two methods or more",
                    group.span,
                    plural(group.names.len(), "method named", "methods named"),
                )
                .with_note("one method an implementation must write is a method without a default");
                self.report(diagnostic);
                fits = false;
            }
            if fits {
                kept.push(group);
            }
        }
        self.program.interfaces[id].one_of = kept;
    }

    /// Reads the defaults of this file's interfaces' type parameters,
    /// once every type has a name. A default is read as a
    /// method's types are, with `Self` and then the interface's own
    /// parameters in scope, and may name `Self` and the parameters before
    /// it: `interface Add<Rhs = Self, Out = Self>`.
    pub(super) fn resolve_interface_defaults(
        &mut self,
        interfaces: &[(&'a ast::InterfaceDecl, InterfaceId)],
    ) {
        for &(decl, id) in interfaces {
            for (index, param) in decl.generics.iter().enumerate() {
                let Some(written) = param.default else {
                    continue;
                };
                let self_param = self.self_param(id, decl.name.span);
                let self_ty = self.intern(TyKind::Param(crate::TyParam {
                    index: 0,
                    name: self_param.name,
                    copy: false,
                }));
                let mut scope = vec![self_param];
                scope.extend(self.program.interfaces[id].generics.iter().cloned());
                let outer = std::mem::replace(&mut self.type_params, scope);
                let outer_self = self.self_ty.replace(self_ty);
                let mut ty = self.type_arg(written);
                self.self_ty = outer_self;
                self.type_params = outer;
                let later = self.program.types.any(
                    ty,
                    &|kind| matches!(kind, TyKind::Param(p) if p.index as usize > index),
                );
                if later {
                    let text = self.text(param.name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::TYPE_PARAMETER_DEFAULT,
                        format!("the default of `{text}` names a parameter that is not before it"),
                        self.ast.types[written].span,
                        "names a later parameter",
                    )
                    .with_note("a use leaves out type arguments from the end, so a default is worked out from `Self` and the ones before it");
                    self.report(diagnostic);
                    ty = Types::ERROR;
                }
                self.program.interfaces[id].generics[index].default = crate::ParamDefault::Ty(ty);
            }
        }
    }

    /// `impl Interface for Type { … }`: the type's methods for that
    /// interface, checked against it.
    pub(super) fn declare_impl(
        &mut self,
        block: &'a ast::ExtendBlock,
        interface: ast::Name,
        owner: TypeDef,
        type_params: &[GenericParamDef],
        bodies: &mut Vec<BodyWork>,
    ) {
        let Some(id) = self.interface_named(interface) else {
            let text = self.text(interface.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::UNKNOWN_INTERFACE,
                format!("cannot find interface `{text}`"),
                interface.span,
                "unknown interface",
            )
            .with_note("an interface is declared `interface Name { … }`");
            self.report(diagnostic);
            return;
        };
        // A built-in type is copied, so a drop of one would run per copy.
        if self
            .program
            .prelude_items
            .interface(KnownInterface::Destroy)
            == Some(id)
            && matches!(owner, TypeDef::Builtin(_))
        {
            let name = self.type_name(owner);
            let diagnostic = Diagnostic::error(
                codes::IMPL_TARGET,
                format!("`{name}` cannot clean up after itself"),
                interface.span,
                "a built-in type",
            )
            .with_note(
                "a built-in type is copied freely, and `destroy` must run exactly once for exactly one owner",
            );
            self.report(diagnostic);
            return;
        }
        let def = &self.program.interfaces[id];
        let (interface_name, interface_pub, interface_module) =
            (def.name, def.is_pub, def.module as usize);
        // `extend ConfigError: From<io::IoError>`: what the interface's
        // own type parameters are here.
        let generics = self.program.interfaces[id].generics.clone();
        let written: Vec<ast::TypeId> = block
            .interface_args
            .as_ref()
            .map(|args| args.args.clone())
            .unwrap_or_default();
        if written.len() < generics::required_type_args(&generics) || written.len() > generics.len()
        {
            for &arg in &written {
                self.resolve_ty(arg);
            }
            self.type_arg_count(interface_name, &generics, written.len(), interface.span);
            return;
        }
        let mut args = Vec::new();
        // The block's own parameters are in scope for them, which is what
        // `extend Vec<T>: Items<T>` says.
        let outer = std::mem::replace(&mut self.type_params, type_params.to_vec());
        for (&arg, param) in written.iter().zip(&generics) {
            let ty = self.type_arg(arg);
            self.check_copy(ty, param, self.ast.types[arg].span);
            self.check_constraints(ty, param, self.ast.types[arg].span);
            args.push(ty);
        }
        self.type_params = outer;
        // What is left out, as the interface's defaults say, with the type
        // the block is for as `Self`: `extend Money: Add` is
        // `Add<Money, Money>`.
        let implementing = self.self_ty_of(owner, type_params);
        let args = self.interface_defaults(id, args, implementing);
        // An operator a built-in type answers itself is an instruction and
        // never calls an implementation: `extend f32: Multiply<f32, Vec2>`
        // would never run, so it is refused where it is written.
        if matches!(owner, TypeDef::Builtin(_))
            && let Some(op) = KnownInterface::named(self.text(interface_name))
                .filter(|known| self.program.prelude_items.interface(*known) == Some(id))
                .and_then(|known| known.binary_op())
            && args.first() == Some(&implementing)
            && self.operator_applies(op, implementing)
        {
            let type_name = self.ty_name(implementing).trim_matches('`').to_string();
            let diagnostic = Diagnostic::error(
                codes::IMPL_TARGET,
                format!("`{type_name} {} {type_name}` is the machine's", op.text()),
                interface.span,
                "an operator the type answers itself",
            )
            .with_note("an operator between two built-in numbers is an instruction, and never calls an implementation");
            self.report(diagnostic);
            return;
        }
        let args = self.program.types.intern_list(&args);
        if !self.visible(interface_module, interface_pub) {
            self.private_item(interface_module, "interface", interface);
        }
        // One implementation per interface and type.
        if let Some(first) = self
            .program
            .impls
            .iter()
            .find(|i| i.interface == id && i.ty == owner && i.args == args)
        {
            let first = first.span;
            let text = self.text(interface_name).to_string();
            let type_name = self.type_name(owner).to_string();
            // What `@derive` writes comes after the module's own files, so
            // it is the second.
            let (label, note) = match block.derived {
                Some(_) => (
                    format!("`@derive({text})` implements it again"),
                    format!(
                        "`@derive({text})` writes an implementation, and this type has one: a type implements an interface once"
                    ),
                ),
                None => (
                    "implemented again here".to_string(),
                    "a type implements an interface once, so which implementation applies is never in question".to_string(),
                ),
            };
            let diagnostic = Diagnostic::error(
                codes::DUPLICATE_IMPL,
                format!("`{type_name}` already implements `{text}`"),
                block.path.last().expect("a path has segments").span,
                label,
            )
            .with_secondary(first, "implemented here")
            .with_note(note);
            self.report(diagnostic);
            return;
        }
        // The methods the block writes, declared as the type's own. An
        // interface that takes types may be implemented more than once,
        // and then one name is declared per implementation.
        let mut written = Vec::new();
        let overloaded = !generics.is_empty();
        // A conformance's methods are as visible as the interface, so `pub`
        // on one says nothing.
        for method in &block.methods {
            let Some(pub_span) = method.pub_span else {
                continue;
            };
            let name = self.text(interface_name).to_string();
            let diagnostic = Diagnostic::error(
                codes::REDUNDANT_PUB,
                "`pub` says nothing here",
                pub_span,
                format!("a method of `{name}`, which says how visible it is"),
            )
            .with_fix("remove `pub`", [Edit::replace(pub_span, "")])
            .with_note("a conformance's methods are as visible as the interface it implements");
            self.report(diagnostic);
        }
        // The methods are as visible as the interface, which is what makes
        // the `pub` above say nothing.
        self.declare_methods_of(
            owner,
            type_params,
            &block.methods,
            bodies,
            overloaded,
            Some(interface_pub),
        );
        for method in &block.methods {
            let name = method.sig.name;
            let id = self
                .methods_named(owner, name.sym)
                .into_iter()
                .find(|&id| self.program.fns[id].name_span == name.span);
            written.push((name, id));
        }
        if block.derived.is_some() {
            self.program
                .derived
                .extend(written.iter().filter_map(|&(_, id)| id));
        }
        let wanted = self.program.interfaces[id].methods.clone();
        let mut methods = Vec::new();
        let mut missing = Vec::new();
        for want in &wanted {
            let want_name = self.program.fns[want.id].name;
            // A lending pair is two methods of one name, so which half
            // implements which is the receiver.
            let want_receiver = self.program.fns[want.id].receiver;
            let halves = |lowerer: &Self, id: &Option<FnId>| {
                id.is_none_or(|id| lowerer.program.fns[id].receiver == want_receiver)
            };
            let found = written
                .iter()
                .find(|(name, id)| name.sym == want_name && halves(self, id))
                .or_else(|| written.iter().find(|(name, _)| name.sym == want_name));
            match found {
                Some(&(name, Some(implementation))) => {
                    self.check_signature(
                        want.id,
                        implementation,
                        owner,
                        args,
                        name,
                        type_params.len(),
                    );
                    methods.push(implementation);
                }
                Some(&(_, None)) => methods.push(want.id),
                // A method with a default needs no implementation: the
                // interface's own is used, with `Self` the type.
                None if want.has_default => {
                    self.add_default(owner, want.id);
                    methods.push(want.id);
                }
                None => {
                    missing.push(self.text(want_name).to_string());
                    methods.push(want.id);
                }
            }
        }
        // A method the interface does not have.
        for &(name, _) in &written {
            if !wanted
                .iter()
                .any(|want| self.program.fns[want.id].name == name.sym)
            {
                let text = self.text(name.sym).to_string();
                let interface_text = self.text(interface_name).to_string();
                let diagnostic = Diagnostic::error(
                    codes::IMPL_METHODS,
                    format!("`{interface_text}` has no method `{text}`"),
                    name.span,
                    "not a method of the interface",
                )
                .with_help("methods that are not the interface's belong in `extend Type { … }`");
                self.report(diagnostic);
            }
        }
        // Of each `@oneOf`, one at least, since each of its methods'
        // defaults is written with another of them.
        let interface_text = self.text(interface_name).to_string();
        for group in self.program.interfaces[id].one_of.clone() {
            if group
                .names
                .iter()
                .any(|&(name, _)| written.iter().any(|(w, _)| w.sym == name))
            {
                continue;
            }
            let names: Vec<String> = group
                .names
                .iter()
                .map(|&(name, _)| format!("`{}`", self.text(name)))
                .collect();
            let either = match &names[..] {
                [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
                [] => String::new(),
            };
            let type_name = self.type_name(owner).to_string();
            let diagnostic = Diagnostic::error(
                codes::IMPL_METHODS,
                format!("`{type_name}` does not implement {either} of `{interface_text}`"),
                block.path.last().expect("a path has segments").span,
                "write one of them",
            )
            .with_note(format!(
                "`{interface_text}`'s defaults for them are each written with another, so one must be written here: `@oneOf` says so"
            ));
            self.report(diagnostic);
        }
        if !missing.is_empty() {
            let type_name = self.type_name(owner).to_string();
            let names: Vec<String> = missing.iter().map(|n| format!("`{n}`")).collect();
            let diagnostic = Diagnostic::error(
                codes::IMPL_METHODS,
                format!(
                    "`{type_name}` does not implement {} of `{interface_text}`",
                    plural(missing.len(), "method", "methods")
                ),
                block.path.last().expect("a path has segments").span,
                format!("missing {}", names.join(", ")),
            )
            .with_note("every method without a default must be implemented");
            self.report(diagnostic);
        }
        self.program.impls.push(ImplDef {
            interface: id,
            args,
            ty: owner,
            methods,
            // When it holds: what the block's parameters must satisfy.
            conditions: type_params.to_vec(),
            module: self.current as u32,
            span: block.path.last().expect("a path has segments").span,
        });
    }

    /// Why an interface cannot be a `&dyn`: a call through a pointer knows
    /// nothing of the type, so it cannot make one, take one by value, or
    /// choose types for it.
    pub(super) fn not_dispatchable(
        &self,
        interface: InterfaceId,
        span: Span,
    ) -> Option<Diagnostic> {
        let def = &self.program.interfaces[interface];
        let name = self.text(def.name);
        // A table of methods is made for a type and an interface; one per
        // set of type arguments is a different thing.
        if !def.generics.is_empty() {
            return Some(
                Diagnostic::error(
                    codes::NOT_DISPATCHABLE,
                    format!("`{name}` cannot be used behind `dyn`"),
                    span,
                    "an interface that takes types",
                )
                .with_secondary(def.span, "declared here")
                .with_note(
                    "a table of methods is made for a type and an interface, and a type may implement this one more than once",
                ),
            );
        }
        for method in &def.methods {
            let fn_def = &self.program.fns[method.id];
            let method_name = self.text(fn_def.name);
            let (what, why) = match fn_def.receiver {
                Some(Receiver::Static) => (
                    "a `static fn`",
                    "a function without a receiver needs the type, which a `&dyn` does not name",
                ),
                Some(Receiver::Move) => (
                    "a `move fn`",
                    "taking the receiver by value needs its size, which a `&dyn` does not know",
                ),
                _ if fn_def.generics.len() > 1 => (
                    "type parameters of its own",
                    "each set of type arguments is a function of its own, and a table holds one",
                ),
                // `Self` in a parameter or the result needs the type, which
                // a call through a table does not know.
                _ if fn_def
                    .params
                    .iter()
                    .skip(1)
                    .any(|p| self.mentions_self(p.ty))
                    || self.mentions_self(fn_def.ret) =>
                {
                    (
                        "written with `Self`",
                        "a call through a table does not know what `Self` is",
                    )
                }
                _ => continue,
            };
            return Some(
                Diagnostic::error(
                    codes::NOT_DISPATCHABLE,
                    format!("`{name}` cannot be used behind `dyn`"),
                    span,
                    format!("`{method_name}` is {what}"),
                )
                .with_secondary(fn_def.name_span, "declared here")
                .with_note(why.to_string())
                .with_help(format!(
                    "`{name}` still works as a constraint: `fn f<T: {name}>(x: &T)`"
                )),
            );
        }
        None
    }

    /// Whether a type mentions the `Self` of an interface's method, which is
    /// its first type parameter.
    fn mentions_self(&self, ty: Ty) -> bool {
        self.program.types.is_generic(ty)
    }

    /// Whether a type implements an interface: an implementation for it, or,
    /// for a type parameter, a constraint that requires it.
    pub(super) fn implements(&self, ty: Ty, interface: InterfaceId) -> bool {
        self.implements_with(
            ty,
            crate::Constraint {
                interface,
                args: crate::TyList::EMPTY,
            },
        )
    }

    /// Whether the type implements the interface for any arguments at
    /// all, which is what `x[i]` asks before it looks for an `at`: the
    /// arguments are the implementation's to say.
    pub(super) fn implements_any(&self, ty: Ty, interface: InterfaceId) -> bool {
        // Plain data is its own clone.
        if self.program.is_clone(interface) && self.program.clones_by_copy(ty) {
            return true;
        }
        if let Some(inner) = self.program.referred(ty) {
            return self.program.answered_through_references(interface)
                && self.implements_any(inner, interface);
        }
        if let TyKind::Param(param) = self.kind(ty) {
            return self.type_params.get(param.index as usize).is_some_and(|p| {
                p.interfaces
                    .iter()
                    .any(|constraint| constraint.interface == interface)
            });
        }
        let Some(owner) = self.owner_of(ty) else {
            return false;
        };
        let owner_args: Vec<Ty> = match self.kind(ty) {
            TyKind::Struct(_, list) | TyKind::Enum(_, list) => {
                self.program.types.list(list).to_vec()
            }
            TyKind::Slice(elem) => vec![elem],
            _ => Vec::new(),
        };
        self.program
            .impls
            .iter()
            .enumerate()
            .filter(|(_, i)| i.interface == interface && i.ty == owner)
            .any(|(at, _)| self.unmet_condition(at, &owner_args).is_none())
    }

    /// The same, for an interface that takes types: the implementation must
    /// be the one for those types.
    pub(super) fn implements_with(&self, ty: Ty, wanted: crate::Constraint) -> bool {
        let args = self.program.types.list(wanted.args).to_vec();
        self.implements_args(ty, wanted.interface, &args)
    }

    /// [`Self::implements_with`], with the interface's arguments as they
    /// are rather than interned.
    pub(super) fn implements_args(&self, ty: Ty, interface: InterfaceId, wanted: &[Ty]) -> bool {
        // Plain data is its own clone, a `&T` among it, and a `&T` answers what
        // `T` answers about itself.
        if self.program.is_clone(interface) && self.program.clones_by_copy(ty) {
            return true;
        }
        if let Some(inner) = self.program.referred(ty) {
            return self.program.answered_through_references(interface)
                && self.implements_args(inner, interface, wanted);
        }
        if let TyKind::Param(param) = self.kind(ty) {
            return self.type_params.get(param.index as usize).is_some_and(|p| {
                p.interfaces
                    .iter()
                    .any(|c| c.interface == interface && self.program.types.list(c.args) == wanted)
            });
        }
        let Some(owner) = self.owner_of(ty) else {
            return false;
        };
        // An implementation of a generic type is written in that type's
        // parameters — `extend Vec<T>: Items<T>` — so its arguments are
        // read with the type's own.
        let owner_args: Vec<Ty> = self.owner_args(ty);
        let candidates: Vec<usize> = self
            .program
            .impls
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                i.interface == interface
                    && i.ty == owner
                    && crate::args_match_with(&self.program.types, i.args, &owner_args, wanted)
            })
            .map(|(at, _)| at)
            .collect();
        // An implementation may hold only for some type arguments.
        candidates
            .into_iter()
            .any(|at| self.unmet_condition(at, &owner_args).is_none())
    }

    /// A type's own type arguments: what an implementation's conditions
    /// are asked of.
    pub(super) fn owner_args(&self, ty: Ty) -> Vec<Ty> {
        match self.kind(ty) {
            TyKind::Struct(_, list) | TyKind::Enum(_, list) => {
                self.program.types.list(list).to_vec()
            }
            // An array has a slice's implementations.
            TyKind::Slice(elem) | TyKind::Array(elem, _) => vec![elem],
            _ => Vec::new(),
        }
    }

    /// The method an interface's implementation gives `ty`, where the
    /// implementation holds: `extend Option<T: Eq>: Eq` gives `equals` to
    /// an `Option<i64>` and to no `Option<T>` whose `T` has none.
    pub(super) fn method_of_impl(&self, ty: Ty, interface: InterfaceId) -> Option<FnId> {
        let owner = self.owner_of(ty)?;
        let args = self.owner_args(ty);
        let at = self
            .program
            .impls
            .iter()
            .position(|i| i.interface == interface && i.ty == owner)?;
        if self.unmet_condition(at, &args).is_some() {
            return None;
        }
        self.program.impls[at].methods.first().copied()
    }

    /// The condition that keeps a type from implementing an interface it
    /// otherwise would, for a message.
    pub(super) fn unmet_for(
        &self,
        ty: Ty,
        wanted: crate::Constraint,
    ) -> Option<(Ty, crate::Constraint)> {
        let owner = self.owner_of(ty)?;
        let args = self.owner_args(ty);
        let at = self.program.impls.iter().position(|i| {
            i.interface == wanted.interface
                && i.ty == owner
                && crate::args_match(&self.program.types, i.args, &args, wanted.args)
        })?;
        self.unmet_condition(at, &args)
    }

    /// The condition an implementation asks for that this type's arguments
    /// do not meet, if there is one.
    pub(super) fn unmet_condition(
        &self,
        at: usize,
        args: &[Ty],
    ) -> Option<(Ty, crate::Constraint)> {
        let conditions = &self.program.impls[at].conditions;
        for (param, &arg) in conditions.iter().zip(args) {
            for &constraint in &param.interfaces {
                // A condition may name the type's other parameters:
                // `extend Mapped<I: Iterator<T>, T, U>` asks the first
                // argument for the second.
                let types = &self.program.types;
                let wanted: Option<Vec<Ty>> = types
                    .list(constraint.args)
                    .iter()
                    .map(|&t| types.try_subst_find(t, args))
                    .collect();
                let met = match wanted {
                    Some(wanted) => self.implements_args(arg, constraint.interface, &wanted),
                    // What it would be was never made, so nothing
                    // implements it.
                    None => false,
                };
                if !met {
                    return Some((arg, constraint));
                }
            }
        }
        None
    }

    /// `Items<Card>`, as a constraint is written, for a message.
    /// A constraint as a program writes it, on `subject`: the arguments that
    /// are what their defaults would be left out, as a type's are, so `T: Add`
    /// is not shown as `Add<T, T>`.
    pub(super) fn constraint_name(&self, constraint: crate::Constraint, subject: Ty) -> String {
        let def = &self.program.interfaces[constraint.interface];
        let name = self.text(def.name).to_string();
        let args = self.program.types.list(constraint.args);
        let mut shown = args.len();
        while shown > 0 {
            let Some(crate::ParamDefault::Ty(default)) =
                def.generics.get(shown - 1).map(|p| p.default)
            else {
                break;
            };
            let mut known = vec![subject];
            known.extend(&args[..shown - 1]);
            if self.program.types.try_subst_find(default, &known) != Some(args[shown - 1]) {
                break;
            }
            shown -= 1;
        }
        if shown == 0 {
            return name;
        }
        let args: Vec<String> = args[..shown]
            .iter()
            .map(|&ty| self.program.ty_name(ty, self.interner))
            .collect();
        format!("{name}<{}>", args.join(", "))
    }

    /// A type argument against the interfaces its parameter requires.
    pub(super) fn check_constraints(&mut self, ty: Ty, param: &GenericParamDef, span: Span) {
        self.check_constraints_in(ty, param, span, &[]);
    }

    /// The same, where the constraint's own type arguments name the item's
    /// parameters: they are substituted with what those turned out to be.
    pub(super) fn check_constraints_in(
        &mut self,
        ty: Ty,
        param: &GenericParamDef,
        span: Span,
        solution: &[Ty],
    ) {
        if self.is_poisoned(ty) {
            return;
        }
        let constraints: Vec<crate::Constraint> = param
            .interfaces
            .iter()
            .map(|&declared| crate::Constraint {
                interface: declared.interface,
                args: match solution.is_empty() {
                    // The item's other arguments are not known here, but
                    // this parameter's is: `T: Add` is `Add<T, T>`, and
                    // `T` is the type being checked.
                    true => {
                        let args = self.program.types.list(declared.args).to_vec();
                        let args: Vec<Ty> = args
                            .into_iter()
                            .map(|arg| self.program.types.replace_param(arg, param.name, ty))
                            .collect();
                        self.program.types.intern_list(&args)
                    }
                    false => self.program.types.subst_list(declared.args, solution),
                },
            })
            .collect();
        // Nothing implements anything until the `extend` blocks are declared,
        // so a type written before then — a field's, an alias's — is checked
        // once they are. A type that mentions a type parameter is not one of
        // those: what it implements is what the parameter promises, which is
        // known here and forgotten later.
        if self.phase < Phase::ImplsDeclared && !self.program.types.is_generic(ty) {
            for constraint in constraints {
                self.pending_constraints
                    .push((ty, constraint, param.written, param.span, span));
            }
            return;
        }
        for constraint in constraints {
            self.check_one_constraint(ty, constraint, param.written, param.span, span);
        }
    }

    /// A type indexed by a position says so once: by `Sequence<T>`, which
    /// is `x[i]` and `for` together, or by `Index<i64, V>`, not both, or
    /// `x[i]` would have two meanings.
    fn one_way_by_position(&mut self) {
        let (Some(index), Some(sequence)) = (
            self.program.prelude_items.interface(KnownInterface::Index),
            self.program
                .prelude_items
                .interface(KnownInterface::Sequence),
        ) else {
            return;
        };
        let clashes: Vec<(Span, Span, String)> = self
            .program
            .impls
            .iter()
            .filter(|i| i.interface == sequence)
            .filter_map(|walked| {
                let indexed = self.program.impls.iter().find(|i| {
                    i.interface == index
                        && i.ty == walked.ty
                        && self.program.types.list(i.args).first() == Some(&Types::I64)
                })?;
                Some((walked.span, indexed.span, self.type_name(walked.ty)))
            })
            .collect();
        for (walked, indexed, name) in clashes {
            let diagnostic = Diagnostic::error(
                codes::DUPLICATE_IMPL,
                format!("`{name}` is indexed by a position twice"),
                walked,
                "`Sequence` says what `x[i]` finds",
            )
            .with_secondary(indexed, "and so does `Index<i64, …>`")
            .with_help("keep `Sequence`, which gives `x[i]` and `for` both, and remove the `Index`")
            .with_note("a container indexed by a position from 0 implements `Sequence`; `Index` is for a lookup whose key is not a position");
            self.report(diagnostic);
        }
    }

    /// Runs the constraint checks that waited for the implementations.
    pub(super) fn flush_constraint_checks(&mut self) {
        self.phase = Phase::ImplsDeclared;
        self.one_way_by_position();
        for (ty, constraint, written, declared, span) in
            std::mem::take(&mut self.pending_constraints)
        {
            self.check_one_constraint(ty, constraint, written, declared, span);
        }
    }

    /// One constraint, against the type given for the parameter.
    fn check_one_constraint(
        &mut self,
        ty: Ty,
        constraint: crate::Constraint,
        written: Symbol,
        declared: Span,
        span: Span,
    ) {
        {
            // Whether it is plain data, and so its own clone, is read from
            // its fields' types.
            crate::mono::complete_type(&mut self.program, ty);
            if self.implements_with(ty, constraint) {
                return;
            }
            let interface = constraint.interface;
            let name = self.constraint_name(constraint, ty);
            // An implementation that would hold but for a condition says
            // which one.
            let unmet = self.unmet_for(ty, constraint);
            let param_name = self.text(written).to_string();
            let mut diagnostic = Diagnostic::error(
                codes::UNSATISFIED_CONSTRAINT,
                format!("{} does not implement `{name}`", self.ty_name(ty)),
                span,
                format!("`{param_name}` requires `{name}`"),
            )
            .with_secondary(declared, format!("`{param_name}: {name}` declared here"))
            .with_note(match self.program.referred(ty) {
                // A reference answers only what reads its value.
                Some(_) if !self.program.answered_through_references(interface) => format!(
                    "a `&` reference answers what its value answers about itself, through `&` alone — `Eq`, `Ord`, `Hash`, `Text` — and is `Clone`; `{name}` takes, changes or answers a `Self`"
                ),
                _ => format!(
                    "a type implements an interface by `extend …: {name}`"
                ),
            });
            // Text is left out of the sequences on purpose, and says so.
            let text = self.kind(ty) == TyKind::Str
                || matches!(self.kind(ty), TyKind::Struct(id, _)
                    if self.text(self.program.structs[id].name) == "String"
                        && Some(self.program.structs[id].module as usize) == self.prelude);
            if text
                && Some(interface)
                    == self
                        .program
                        .prelude_items
                        .interface(KnownInterface::Sequence)
            {
                diagnostic = diagnostic.with_help(
                    "text is not a sequence: its positions are bytes, not characters; walk the characters with `chars()`, or lend the bytes with `items()`",
                );
            }
            // The three the compiler can write are worth naming, since a
            // struct or an enum asks for them with an annotation rather
            // than a block.
            let declares = matches!(self.kind(ty), TyKind::Struct(..) | TyKind::Enum(..));
            let annotation = match self.program.prelude_items.which_interface(interface) {
                Some(KnownInterface::Eq) => Some("@derive(Eq)"),
                Some(KnownInterface::Ord) => Some("@derive(Ord)"),
                Some(KnownInterface::Hash) => Some("@derive(Hash)"),
                Some(KnownInterface::Clone) => Some("@derive(Clone)"),
                _ => None,
            };
            if let Some((argument, asked)) = unmet {
                let asked = self.constraint_name(asked, argument);
                diagnostic = diagnostic
                    .with_help(format!(
                        "{} implements `{name}` when its argument does `{asked}`",
                        self.ty_name(ty)
                    ))
                    .with_note(format!(
                        "{} does not implement `{asked}`",
                        self.ty_name(argument)
                    ));
            }
            if let Some(annotation) = annotation
                && declares
                && unmet.is_none()
            {
                diagnostic = diagnostic.with_help(format!(
                    "`{annotation}` on the declaration writes it from the fields"
                ));
            }
            self.report(diagnostic);
        }
    }

    /// A method with a default becomes the type's own, so it is called like
    /// any other; it is generic in `Self`, and each type gets an instance.
    pub(super) fn add_default(&mut self, owner: TypeDef, method: FnId) {
        let name = self.program.fns[method].name;
        if self.method_of(owner, name).is_some() {
            return;
        }
        match owner {
            TypeDef::Struct(id) => self.program.structs[id].methods.push(method),
            TypeDef::Enum(id) => self.program.enums[id].methods.push(method),
            TypeDef::Builtin(owner) => self
                .program
                .builtins
                .entry(owner)
                .or_default()
                .methods
                .push(method),
        }
    }

    /// An implementation's method against the interface's, with `Self` read
    /// as the type: the same receiver, the same parameters and the same
    /// result.
    fn check_signature(
        &mut self,
        want: FnId,
        got: FnId,
        owner: TypeDef,
        interface_args: crate::TyList,
        name: ast::Name,
        type_params: usize,
    ) {
        let self_ty = self.self_ty(owner);
        let wanted = self.program.fns[want].clone();
        let got_def = self.program.fns[got].clone();
        let interface = wanted
            .interface
            .expect("an interface's method knows its interface");
        let interface_name = self
            .text(self.program.interfaces[interface].name)
            .to_string();
        let method = self.text(wanted.name).to_string();
        let report = |lowerer: &mut Self, label: String, note: String| {
            let diagnostic = Diagnostic::error(
                codes::IMPL_METHODS,
                format!("`{method}` does not match `{interface_name}`"),
                name.span,
                label,
            )
            .with_secondary(wanted.name_span, "declared here")
            .with_note(note);
            lowerer.report(diagnostic);
        };
        if wanted.receiver != got_def.receiver {
            let (want_text, got_text) = (
                wanted.receiver.expect("a method has a receiver").text(),
                got_def.receiver.expect("a method has a receiver").text(),
            );
            report(
                self,
                format!("declared `{got_text}`, and the interface declares `{want_text}`"),
                "a method takes the receiver its interface declares".to_string(),
            );
            return;
        }
        // The interface's own type parameters come after `Self`, which the
        // type replaces.
        let mut all = vec![self_ty];
        all.extend_from_slice(self.program.types.list(interface_args));
        let args = self.program.types.intern_list(&all);
        // The interface's method has `Self` and its own parameters; the
        // implementation has the type's and its own.
        let interface_generics = self.program.types.list(interface_args).len();
        if wanted.generics.len() - 1 - interface_generics != got_def.generics.len() - type_params {
            report(
                self,
                "a different number of type parameters".to_string(),
                "a method has the type parameters its interface declares".to_string(),
            );
            return;
        }
        if wanted.params.len() != got_def.params.len() {
            report(
                self,
                format!(
                    "takes {}, and the interface declares {}",
                    plural(got_def.params.len(), "parameter", "parameters"),
                    wanted.params.len()
                ),
                "a method takes the parameters its interface declares".to_string(),
            );
            return;
        }
        for (want_param, got_param) in wanted.params.iter().zip(&got_def.params).skip(1) {
            let expected = self.subst_self(want_param.ty, args);
            if expected != got_param.ty && !self.has_error(expected) {
                let (expected, actual) = (
                    self.ty_name(expected).to_string(),
                    self.ty_name(got_param.ty).to_string(),
                );
                let param = self.text(got_param.name).to_string();
                report(
                    self,
                    format!("`{param}` is {actual}, and the interface declares {expected}"),
                    "a method's parameters are its interface's, with `Self` the type".to_string(),
                );
                return;
            }
        }
        let expected = self.subst_self(wanted.ret, args);
        if expected != got_def.ret && !self.has_error(expected) {
            let (expected, actual) = (
                self.ty_name(expected).to_string(),
                self.ty_name(got_def.ret).to_string(),
            );
            report(
                self,
                format!("returns {actual}, and the interface declares {expected}"),
                "a method returns what its interface declares, with `Self` the type".to_string(),
            );
        }
    }

    /// A type of an interface's method, with the implementing type in place
    /// of `Self`.
    fn subst_self(&mut self, ty: Ty, args: crate::TyList) -> Ty {
        let tys = self.program.types.list(args).to_vec();
        self.program.types.subst(ty, &tys)
    }

    /// `Self`: the one type parameter of an interface's methods, required to
    /// implement the interface itself, so a default body may call its other
    /// methods.
    fn self_param(&mut self, interface: InterfaceId, span: Span) -> GenericParamDef {
        let name = self.interner.self_type_symbol();
        // `Self: From<T>` inside `interface From<T>`: the interface's own
        // parameters, which follow `Self` in a method's type arguments, so
        // a call substitutes them along with it.
        let generics = self.program.interfaces[interface].generics.clone();
        let args: Vec<Ty> = generics
            .iter()
            .enumerate()
            .map(|(i, param)| {
                self.intern(TyKind::Param(crate::TyParam {
                    index: i as u32 + 1,
                    name: param.name,
                    copy: param.copy,
                }))
            })
            .collect();
        let args = self.program.types.intern_list(&args);
        GenericParamDef {
            name,
            written: name,
            copy: false,
            interfaces: vec![crate::Constraint { interface, args }],
            default: crate::ParamDefault::None,
            span,
        }
    }
}
