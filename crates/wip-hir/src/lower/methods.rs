//! Methods: `self`, a call `x.m(…)`, and a static function `Type::name(…)`.

use super::*;

impl<'a> Lowerer<'a> {
    /// `self`, the receiver of the method being checked. It is the first
    /// parameter, so it resolves like any other name.
    pub(super) fn self_expr(&mut self, span: Span) -> ExprId {
        let sym = self.interner.self_symbol();
        if self.lookup(sym).is_some() {
            return self.name(sym, None, span);
        }
        // A closure or a generator in a method captures it as any name
        // around it.
        if let Some(captured) = self.captured_name(sym, span) {
            return captured;
        }
        let diagnostic = Diagnostic::error(
            codes::SELF_OUTSIDE_METHOD,
            "`self` is only available in a method",
            span,
            "no receiver here",
        )
        .with_note(
            "a method is declared in a type, and takes its receiver from the call: `fn`, `var fn` or `move fn`",
        );
        self.report(diagnostic);
        self.error_expr(span)
    }

    /// The type a method is looked up in: through `&`, `&var` and `own<T>`,
    /// as a field access looks through them.
    pub(super) fn owner_of(&self, ty: Ty) -> Option<TypeDef> {
        match self.kind(ty) {
            TyKind::Struct(id, _) => Some(TypeDef::Struct(id)),
            TyKind::Enum(id, _) => Some(TypeDef::Enum(id)),
            TyKind::Ref(inner, _) | TyKind::Own(inner) => self.owner_of(inner),
            // An array has a slice's methods: what they take is `&[T]`,
            // and an array lends one.
            TyKind::Array(..) => self
                .program
                .builtins
                .contains_key(&BuiltinOwner::Slice)
                .then_some(TypeDef::Builtin(BuiltinOwner::Slice)),
            // A built-in type the prelude gave methods to.
            kind => BuiltinOwner::of(kind)
                .filter(|builtin| self.program.builtins.contains_key(builtin))
                .map(TypeDef::Builtin),
        }
    }

    /// The method or static function of `owner` with this name.
    /// Which of several implementations a static call means: the one whose
    /// parameter is the type of what is passed. A call that
    /// fits none of them, or more than one, is reported here.
    fn overload(
        &mut self,
        owner: TypeDef,
        candidates: &[FnId],
        member: ast::Name,
        item: &ItemUse<'a>,
    ) -> Option<FnId> {
        let args = item.args.unwrap_or_default();
        let text = self.text(member.sym).to_string();
        let type_name = self.type_name(owner).to_string();
        let interfaces: Vec<String> = candidates
            .iter()
            .map(|&id| self.implemented_name(id))
            .collect();
        let report = |lowerer: &mut Self, label: String, note: String| {
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                format!("`{type_name}` has more than one `{text}`"),
                member.span,
                label,
            )
            .with_note(note)
            .with_help(format!("the implementations are {}", interfaces.join(", ")));
            lowerer.report(diagnostic);
        };
        if args.len() != 1 {
            report(
                self,
                format!("called with {}", plural(args.len(), "argument", "arguments")),
                "each implementation of an interface that takes types declares it, so one argument says which is meant".to_string(),
            );
            return None;
        }
        // What is passed decides, so it is checked on its own first.
        let value = self.infer(args[0], None);
        let ty = self.ty_of(value);
        let fits: Vec<FnId> = candidates
            .iter()
            .copied()
            .filter(|&id| {
                self.program.fns[id]
                    .params
                    .first()
                    .is_some_and(|param| param.ty == ty)
            })
            .collect();
        match fits.len() {
            1 => Some(fits[0]),
            0 => {
                let passed = self.ty_name(ty).to_string();
                report(
                    self,
                    format!("none of them takes {passed}"),
                    "the implementation is chosen by what is passed, and no implementation takes this".to_string(),
                );
                None
            }
            _ => {
                report(
                    self,
                    "more than one of them fits".to_string(),
                    "the implementation is chosen by what is passed, and this fits several"
                        .to_string(),
                );
                None
            }
        }
    }

    /// `From<i64>`, as an implementation is written, for a message.
    fn implemented_name(&self, method: FnId) -> String {
        let owner = self.program.fns[method].owner;
        let name = self.program.fns[method].name;
        let found = self.program.impls.iter().find(|i| {
            Some(i.ty) == owner && i.methods.contains(&method) || i.methods.contains(&method)
        });
        let Some(def) = found else {
            return format!("`{}`", self.text(name));
        };
        let interface = self.text(self.program.interfaces[def.interface].name);
        let args: Vec<String> = self
            .program
            .types
            .list(def.args)
            .iter()
            .map(|&ty| self.ty_name(ty).trim_matches('`').to_string())
            .collect();
        if args.is_empty() {
            format!("`{interface}`")
        } else {
            format!("`{interface}<{}>`", args.join(", "))
        }
    }

    /// Every method of `owner` with this name: more than one where an
    /// interface that takes types is implemented more than once,
    /// or where a lending pair reads and writes.
    pub(super) fn methods_named(&self, owner: TypeDef, name: Symbol) -> Vec<FnId> {
        let methods = match owner {
            TypeDef::Struct(id) => &self.program.structs[id].methods,
            TypeDef::Enum(id) => &self.program.enums[id].methods,
            TypeDef::Builtin(builtin) => match self.program.builtins.get(&builtin) {
                Some(built) => &built.methods,
                None => return Vec::new(),
            },
        };
        methods
            .iter()
            .copied()
            .filter(|&id| self.program.fns[id].name == name)
            .collect()
    }

    pub(super) fn method_of(&self, owner: TypeDef, name: Symbol) -> Option<FnId> {
        // A lending pair is two methods of one name, and a call reads
        // unless it is where a write happens.
        let mut found = self.methods_named(owner, name);
        found.sort_by_key(|&id| self.program.fns[id].receiver == Some(Receiver::Var));
        found.first().copied()
    }

    /// The writing half of a lending pair, for a call to the reading one.
    pub(super) fn writing_twin(&self, reader: FnId) -> Option<FnId> {
        let def = &self.program.fns[reader];
        // An interface's own declaration has no type to look in; what
        // implements it does, pair and all.
        if def.receiver != Some(Receiver::Read) {
            return None;
        }
        let (owner, name) = (def.owner?, def.name);
        self.methods_named(owner, name).into_iter().find(|&id| {
            let other = &self.program.fns[id];
            other.receiver == Some(Receiver::Var)
                && matches!(self.kind(other.ret), TyKind::Ref(_, crate::RefKind::Var))
        })
    }

    /// Whether two methods of one name are a lending pair: the same
    /// parameters, one reading and one writing.
    pub(super) fn is_lending_pair(&self, first: FnId, second: FnId) -> bool {
        let (a, b) = (&self.program.fns[first], &self.program.fns[second]);
        let receivers = [a.receiver, b.receiver];
        if !receivers.contains(&Some(Receiver::Read)) || !receivers.contains(&Some(Receiver::Var)) {
            return false;
        }
        let lends =
            |def: &crate::FnDef, kind| matches!(self.kind(def.ret), TyKind::Ref(_, k) if k == kind);
        let (reader, writer) = if a.receiver == Some(Receiver::Read) {
            (a, b)
        } else {
            (b, a)
        };
        if !lends(reader, crate::RefKind::Shared) || !lends(writer, crate::RefKind::Var) {
            return false;
        }
        // The receiver is the first parameter, and differs by design.
        let rest = |def: &crate::FnDef| {
            def.params
                .iter()
                .skip(1)
                .map(|p| (p.name, p.ty))
                .collect::<Vec<_>>()
        };
        rest(reader) == rest(writer)
    }

    /// `x.m(…)`: the method `m` of `x`'s type, if it has one. The receiver
    /// is borrowed or moved as the method declares.
    /// `s.toStr()`: the bytes C owns, seen as a `str`. The
    /// length is found by walking to the NUL, once, here.
    fn cstr_to_str(&mut self, receiver: ExprId, name: ast::Name, item: ItemUse<'a>) -> ExprId {
        if let Some(args) = item.args
            && !args.is_empty()
        {
            let diagnostic = Diagnostic::error(
                codes::ARGUMENT_COUNT,
                "`toStr` takes no arguments",
                item.span,
                "arguments",
            );
            self.report(diagnostic);
            return self.give_up(args.iter().copied(), item.span);
        }
        if item.args.is_none() {
            let text = self.text(name.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                format!("`{text}` is a method, and is called"),
                name.span,
                "a method, not a field",
            )
            .with_fix("call it", [Edit::insert(name.span.hi, "()")]);
            self.report(diagnostic);
            return self.error_expr(item.span);
        }
        self.alloc(ExprKind::CstrToStr(receiver), Types::STR, item.span)
    }

    pub(super) fn method_call(
        &mut self,
        base: ast::ExprId,
        name: ast::Name,
        item: ItemUse<'a>,
    ) -> Option<ExprId> {
        // A receiver is lent where it is written, so a range of elements
        // may be one: `window[at..at + n].copyFrom(&block)`.
        let receiver = self.infer_borrowed(base, None);
        let ty = self.ty_of(receiver);
        // The receiver is already broken, and looking for a field of this
        // name would lower it a second time and report it twice.
        if self.is_poisoned(ty) {
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        }
        // `x.len()` of an array, a slice or a `str` is the length the value
        // carries, known where the program is compiled for an array.
        // The prelude declares the method, so it
        // is listed and can answer for an interface; a direct call is read
        // here, at no cost.
        let mut held = ty;
        while let TyKind::Ref(inner, _) | TyKind::Own(inner) = self.kind(held) {
            held = inner;
        }
        if self.text(name.sym) == "len"
            && item.args.is_some_and(|args| args.is_empty())
            && item.names.is_empty()
            && matches!(
                self.kind(held),
                TyKind::Array(..) | TyKind::Str | TyKind::Slice(_)
            )
        {
            let value = self.autoderef(receiver);
            return Some(self.alloc(ExprKind::Len(value), Types::I64, item.span));
        }
        // A call through a `&dyn`, and a method of a type parameter, come
        // from an interface.
        if let TyKind::Dyn(interface, _) = self.kind(self.under_refs(ty)) {
            return self.dyn_method_call(receiver, interface, name, item);
        }
        // A C string can say how long it is, which costs a walk through it.
        if self.kind(ty) == TyKind::Cstring && self.text(name.sym) == "toStr" {
            return Some(self.cstr_to_str(receiver, name, item));
        }
        // Its length is not something it carries, so it has no `len()`:
        // the text is walked first.
        if self.kind(ty) == TyKind::Cstring && self.text(name.sym) == "len" {
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                "no method `len` on `cstring`",
                name.span,
                "its length is not known",
            )
            .with_note(
                "a `cstring` is a pointer to bytes that end with a NUL, so its length is not known until it is walked",
            )
            .with_fix(
                "walk it first, with `toStr()`",
                [Edit::insert(name.span.lo, "toStr().")],
            );
            self.report(diagnostic);
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        }
        // Plain data is its own clone, which no method of its type need
        // say.
        let own_clone = self
            .owner_of(ty)
            .is_some_and(|owner| self.method_of(owner, name.sym).is_some());
        if !own_clone
            && self.text(name.sym) == "clone"
            && let Some(call) = self.plain_clone(receiver, ty, name, item)
        {
            return Some(call);
        }
        let Some(owner) = self.owner_of(ty) else {
            return self.constrained_call(receiver, ty, name, item);
        };
        // A `String` has the methods of the text it holds: where it has none
        // of its own by that name, it lends its bytes for the call, as it
        // does for an argument. Its own win, so nothing it
        // declares is hidden.
        if self.method_of(owner, name.sym).is_none()
            && let Some(text) = self.lend_text(receiver, name.sym)
        {
            let ty = self.ty_of(text);
            let owner = self.owner_of(ty).expect("`str` is a method owner");
            return self.dispatch(text, owner, name, item);
        }
        self.dispatch(receiver, owner, name, item)
    }

    /// The receiver, read as the text it holds, when it is a `String` and
    /// `str` has the method it is asked for. What comes back
    /// borrows the receiver, and lives as long as the call.
    fn lend_text(&mut self, receiver: ExprId, name: Symbol) -> Option<ExprId> {
        self.method_of(TypeDef::Builtin(BuiltinOwner::Str), name)?;
        self.lend_string(receiver)
    }

    /// The method of `owner` that `name` asks for, called on `receiver`.
    pub(super) fn dispatch(
        &mut self,
        receiver: ExprId,
        owner: TypeDef,
        name: ast::Name,
        item: ItemUse<'a>,
    ) -> Option<ExprId> {
        let ty = self.ty_of(receiver);
        // A type's own method wins; otherwise what its interfaces give it,
        // which must be one interface's.
        let mut unmet = None;
        let found = match self.method_of(owner, name.sym) {
            Some(id) if !self.through_an_interface(owner, id) => Some(id),
            found => {
                let (offered, missed) = self.offered(owner, ty, name.sym);
                if distinct_interfaces(&offered) > 1 {
                    let subject = self.type_name(owner).to_string();
                    self.ambiguous_method(&offered, &subject, name);
                    return Some(
                        self.give_up(item.args.unwrap_or_default().iter().copied(), item.span),
                    );
                }
                unmet = missed.first().copied();
                found.or_else(|| offered.first().map(|o| o.method))
            }
        };
        let id = match found {
            Some(id) => id,
            // An extension of that name whose condition is not met.
            None if unmet.is_some() => {
                let subject = self.type_name(owner).to_string();
                self.unmet_extension(unmet.expect("just tested"), &subject, name);
                return Some(
                    self.give_up(item.args.unwrap_or_default().iter().copied(), item.span),
                );
            }
            // A field of function type can be called; any
            // other name is a method that does not exist.
            None if self.field_named(owner, name.sym) => return None,
            // An interpolated literal appends each of its values through
            // `Text`, so a type that does not implement it is reported here.
            None if name.sym == Symbol::append_to() => {
                let type_name = self.type_name(owner).to_string();
                let diagnostic = Diagnostic::error(
                    codes::NOT_A_METHOD,
                    format!("no method `appendTo` on `{type_name}`"),
                    name.span,
                    "unknown method",
                )
                .with_note(format!(
                    "a value written between `\\(` and `)` is appended by `Text`, which `{type_name}` does not implement"
                ))
                .with_help(format!(
                    "write `extend {type_name}: Text`, or build the text with `String` yourself"
                ));
                self.report(diagnostic);
                // The argument is the `String` being built, which nothing
                // else would report on.
                return Some(self.give_up(std::iter::empty(), item.span));
            }
            // A number written to a precision or in a radix, of a type that
            // is not that number, is reported for what the option asks.
            None if Symbol::written()[2..].contains(&name.sym) => {
                let type_name = self.type_name(owner).to_string();
                let precision = name.sym == Symbol::written()[2];
                let (message, label, note) = match precision {
                    true => (
                        format!("a precision is a float's, and this is `{type_name}`"),
                        "not a float",
                        "a precision is how many digits a float is written with after its point: `\\(seconds, precision: 3)`",
                    ),
                    false => (
                        format!("a radix is an integer's, and this is `{type_name}`"),
                        "not an integer",
                        "a radix is the base an integer is written in: `\\(byte, radix: 16)`",
                    ),
                };
                let diagnostic =
                    Diagnostic::error(codes::INTERPOLATION_OPTION, message, name.span, label)
                        .with_note(note);
                self.report(diagnostic);
                return Some(
                    self.give_up(item.args.unwrap_or_default().iter().copied(), item.span),
                );
            }
            None => {
                let type_name = self.type_name(owner).to_string();
                let text = self.text(name.sym).to_string();
                let names: Vec<&str> = self.member_names(owner);
                let mut diagnostic = Diagnostic::error(
                    codes::NOT_A_METHOD,
                    format!("no method `{text}` on `{type_name}`"),
                    name.span,
                    "unknown method",
                );
                if let Some(similar) = suggest(&text, names.iter().copied()) {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `{similar}`?"),
                        [Edit::replace(name.span, similar)],
                    );
                } else if !names.is_empty() {
                    let names: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
                    diagnostic =
                        diagnostic.with_note(format!("`{type_name}` has {}", names.join(", ")));
                }
                self.report(diagnostic);
                return Some(
                    self.give_up(item.args.unwrap_or_default().iter().copied(), item.span),
                );
            }
        };
        // `destroy` runs where a value ends; a program that could call it
        // could run it twice.
        if self.program.drop_method(self.under_refs(ty)) == Some(id) {
            let type_name = self.type_name(owner).to_string();
            let diagnostic = Diagnostic::error(
                codes::DROP_RULES,
                format!("`{type_name}`'s `destroy` cannot be called"),
                name.span,
                "a `destroy`",
            )
            .with_note("`destroy` runs where a value ends, and exactly once")
            .with_help("to end a value early, write `destroy(move value)`");
            self.report(diagnostic);
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        }
        let def = &self.program.fns[id];
        let kind = match (def.receiver, def.params.first()) {
            (Some(Receiver::Static), _) => {
                let type_name = self.type_name(owner).to_string();
                let method = self.text(name.sym).to_string();
                let diagnostic = Diagnostic::error(
                    codes::NOT_A_METHOD,
                    format!("`{method}` takes no receiver"),
                    name.span,
                    "a `static fn`",
                )
                .with_note("a `static fn` is a function of the type, not a method")
                .with_help(format!("call it as `{type_name}::{method}(…)`"));
                self.report(diagnostic);
                return Some(
                    self.give_up(item.args.unwrap_or_default().iter().copied(), item.span),
                );
            }
            (Some(kind), Some(_)) => kind,
            _ => return None,
        };
        let pub_ok = self.method_is_visible(id);
        if !pub_ok {
            let type_name = self.type_name(owner).to_string();
            let method = self.text(name.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::PRIVATE_ITEM,
                format!("`{method}` of `{type_name}` is not exported"),
                name.span,
                "not `pub`",
            )
            .with_note("a method is used outside its module only if it is `pub`");
            self.report(diagnostic);
        }
        // A slice's methods take `&[T]`, which an array lends.
        let as_slice = owner == TypeDef::Builtin(BuiltinOwner::Slice);
        let Some(receiver) = self.receiver_expr(receiver, kind, name, as_slice) else {
            // Reported; the call is given up on, not looked for elsewhere.
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        };
        // A default body is the interface's own function, generic in the
        // interface's types too; what they are here is what this type
        // implements it with: `Count: Iterator<i64>` makes `T` an `i64`.
        let interface_args = self.default_args(id, owner, ty);
        let item = ItemUse {
            receiver: Some(receiver),
            interface_args,
            ..item
        };
        Some(self.call(id, item))
    }

    /// The interface's own type arguments for a call of its default method
    /// `id` on a value of `ty`, whose type is `owner`; none for any other
    /// method.
    fn default_args(&mut self, id: FnId, owner: TypeDef, ty: Ty) -> crate::TyList {
        let def = &self.program.fns[id];
        let Some(interface) = def.interface else {
            return crate::TyList::EMPTY;
        };
        if def.owner.is_some() {
            return crate::TyList::EMPTY;
        }
        let found = self
            .program
            .impls
            .iter()
            .filter(|i| i.interface == interface && i.ty == owner)
            .find(|i| i.methods.contains(&id))
            .or_else(|| {
                self.program
                    .impls
                    .iter()
                    .find(|i| i.interface == interface && i.ty == owner)
            });
        let Some(found) = found else {
            return crate::TyList::EMPTY;
        };
        let written = self.program.types.list(found.args).to_vec();
        // They are written in the type's own parameters, which are this
        // value's type arguments: a slice's element is its one.
        let own: Vec<Ty> = self.owner_args(self.under_refs(ty));
        let args: Vec<Ty> = written
            .into_iter()
            .map(|arg| self.program.types.subst(arg, &own))
            .collect();
        self.program.types.intern_list(&args)
    }

    /// `x.m(…)` through a `&dyn Interface`: the method is found in the table
    /// the reference carries.
    fn dyn_method_call(
        &mut self,
        receiver: ExprId,
        interface: InterfaceId,
        name: ast::Name,
        item: ItemUse<'a>,
    ) -> Option<ExprId> {
        let methods = self.program.interfaces[interface].methods.clone();
        let found = methods
            .iter()
            .position(|m| self.program.fns[m.id].name == name.sym);
        let Some(index) = found else {
            let text = self.text(name.sym).to_string();
            let interface_name = self
                .text(self.program.interfaces[interface].name)
                .to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                format!("no method `{text}` on `dyn {interface_name}`"),
                name.span,
                "unknown method",
            )
            .with_note("a `&dyn` gives the methods of its interface");
            self.report(diagnostic);
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        };
        let method = methods[index].id;
        let args = item.args.expect("a call has arguments");
        let kind = self.program.fns[method]
            .receiver
            .expect("an interface's method has a receiver");
        // The receiver is the reference itself: it already carries the table.
        let Some(receiver) = self.dyn_receiver(receiver, kind, name) else {
            return Some(self.give_up(args.iter().copied(), item.span));
        };
        let params = self.program.fns[method].params.clone();
        let mut hir_args = vec![receiver];
        let mut checked = Vec::new();
        // Checked as any call's arguments are: `&place` and `&var place`
        // lend, where a parameter takes a reference.
        for (i, &arg) in args.iter().enumerate() {
            let expected = params.get(i + 1).map(|p| p.ty);
            let value = match expected {
                Some(ty) => self.check_argument(arg, ty, None),
                None => self.infer_argument(arg, None),
            };
            checked.push(value);
        }
        if args.len() + 1 != params.len() {
            let name_text = self.text(name.sym).to_string();
            let slots: Vec<Slot> = params[1..]
                .iter()
                .map(|p| Slot {
                    name: p.name,
                    ty: p.ty,
                    span: p.span,
                    default: p.default.clone(),
                })
                .collect();
            let names = item.arg_names();
            let matched = self.match_arguments(&slots, args, &names, "parameter", &name_text);
            let sig_span = self.program.fns[method].span;
            self.report_argument_count(&name_text, &slots, &matched, &names, sig_span, item.span);
        }
        hir_args.extend(checked);
        let ret = self.program.fns[method].ret;
        // The receiver is a pointer to a value whose type is not known here.
        let mut param_tys = vec![Types::PTR_U8];
        param_tys.extend(params[1..].iter().map(|p| p.ty));
        let list = self.program.types.intern_list(&param_tys);
        let fn_ty = self.intern(TyKind::Fn(list, ret));
        Some(self.alloc(
            ExprKind::DynCall {
                interface,
                index: index as u32,
                fn_ty,
                args: hir_args,
                order: Vec::new(),
            },
            ret,
            item.span,
        ))
    }

    /// The receiver of a call through a `&dyn`: the reference itself, whose
    /// kind must allow what the method does.
    fn dyn_receiver(
        &mut self,
        receiver: ExprId,
        kind: Receiver,
        name: ast::Name,
    ) -> Option<ExprId> {
        // A reference parameter is seen through where it is named;
        // the reference itself carries the table.
        let reference = match self.state.body.exprs[receiver].kind {
            ExprKind::Deref(inner) => inner,
            _ => receiver,
        };
        let TyKind::Ref(_, ref_kind) = self.kind(self.ty_of(reference)) else {
            return None;
        };
        if kind == Receiver::Var && ref_kind == crate::RefKind::Shared {
            let text = self.text(name.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::CANNOT_BORROW_VAR,
                format!("`{text}` writes through its receiver"),
                name.span,
                "a `var fn`",
            )
            .with_note("a `&dyn` reference only reads; `&var dyn` writes");
            self.report(diagnostic);
            return None;
        }
        Some(reference)
    }

    /// `x.m(…)` where `x` is a type parameter: the method of one of the
    /// interfaces it is constrained by.
    pub(super) fn constrained_call(
        &mut self,
        receiver: ExprId,
        ty: Ty,
        name: ast::Name,
        item: ItemUse<'a>,
    ) -> Option<ExprId> {
        let TyKind::Param(param) = self.kind(self.under_refs(ty)) else {
            return None;
        };
        let constraints = self
            .type_params
            .get(param.index as usize)
            .map(|p| p.interfaces.clone())
            .unwrap_or_default();
        let subject = self.under_refs(ty);
        let (offered, missed) = self.offered_by_constraints(subject, &constraints, name.sym);
        if distinct_interfaces(&offered) > 1 {
            let param_name = self.text(param.name).to_string();
            self.ambiguous_method(&offered, &param_name, name);
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        }
        let Some(&found) = offered.first() else {
            if constraints.is_empty() {
                return None;
            }
            if let Some(&unmet) = missed.first() {
                let param_name = self.text(param.name).to_string();
                self.unmet_extension(unmet, &param_name, name);
                return Some(
                    self.give_up(item.args.unwrap_or_default().iter().copied(), item.span),
                );
            }
            let names: Vec<String> = constraints
                .iter()
                .map(|&c| format!("`{}`", self.constraint_name(c, self.under_refs(ty))))
                .collect();
            let text = self.text(name.sym).to_string();
            let param_name = self.text(param.name).to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                format!("no method `{text}` on `{param_name}`"),
                name.span,
                "unknown method",
            )
            .with_note(format!(
                "`{param_name}` gives the methods of {}",
                names.join(" and ")
            ));
            self.report(diagnostic);
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        };
        let kind = self.program.fns[found.method]
            .receiver
            .expect("an interface's method has a receiver");
        let Some(receiver) = self.receiver_expr(receiver, kind, name, false) else {
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        };
        let item = ItemUse {
            receiver: Some(receiver),
            // What the constraint says the interface's own types are.
            interface_args: found.args,
            ..item
        };
        Some(self.call(found.method, item))
    }

    /// Whether a type's method is one an interface gives it: a default, or
    /// the method an implementation writes.
    fn through_an_interface(&self, owner: TypeDef, id: FnId) -> bool {
        self.program.fns[id].owner.is_none()
            || self
                .program
                .impls
                .iter()
                .any(|i| i.ty == owner && i.methods.contains(&id))
    }

    /// `x.clone()` of plain data: the prelude's `Clone::clone`, which the
    /// monomorphizer answers with a copy where the type has no
    /// implementation of its own. Nothing where the value
    /// is not plain data.
    fn plain_clone(
        &mut self,
        receiver: ExprId,
        ty: Ty,
        name: ast::Name,
        item: ItemUse<'a>,
    ) -> Option<ExprId> {
        let held = self.under_refs(ty);
        let interface = self
            .program
            .prelude_items
            .interface(KnownInterface::Clone)?;
        crate::mono::complete_type(&mut self.program, held);
        if self.is_poisoned(held) || !self.program.clones_by_copy(held) {
            return None;
        }
        let method = *self.program.interfaces[interface]
            .methods
            .iter()
            .find(|m| self.program.fns[m.id].name == name.sym)?;
        let kind = self.program.fns[method.id]
            .receiver
            .expect("an interface's method has a receiver");
        let Some(receiver) = self.receiver_expr(receiver, kind, name, false) else {
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        };
        let item = ItemUse {
            receiver: Some(receiver),
            ..item
        };
        Some(self.call(method.id, item))
    }

    /// Whether a method may be used here: its own module sees all of them,
    /// another only what is `pub`.
    fn method_is_visible(&self, id: FnId) -> bool {
        let def = &self.program.fns[id];
        def.is_pub || def.module as usize == self.current
    }

    /// The receiver as the method takes it: `&self`, `&var self` or by
    /// value, with the borrow or the move the call site does not write.
    /// The innermost `&` reference of a value that is a reference to a
    /// reference: `&&T` read as the `&T` it holds.
    fn autoderef_to_ref(&mut self, mut id: ExprId) -> ExprId {
        loop {
            let expr = &self.state.body.exprs[id];
            let TyKind::Ref(inner, crate::RefKind::Shared) = self.kind(expr.ty) else {
                return id;
            };
            if !matches!(self.kind(inner), TyKind::Ref(_, crate::RefKind::Shared)) {
                return id;
            }
            let span = expr.span;
            id = self.alloc(ExprKind::Deref(id), inner, span);
        }
    }

    fn receiver_expr(
        &mut self,
        receiver: ExprId,
        kind: Receiver,
        name: ast::Name,
        as_slice: bool,
    ) -> Option<ExprId> {
        let ty = self.ty_of(receiver);
        let span = self.state.body.exprs[receiver].span;
        match kind {
            // A `lend fn` is declared as its two halves, and a call is to
            // one of them.
            Receiver::Lend => {
                unreachable!("a `lend fn` is declared as its reading and writing halves")
            }
            Receiver::Read | Receiver::Var => {
                let ref_kind = match kind {
                    Receiver::Var => crate::RefKind::Var,
                    _ => crate::RefKind::Shared,
                };
                // The method belongs to what the `own`s point to, as the
                // `&own<T>` to `&T` coercion does for an argument.
                let mut place = receiver;
                let mut ty = ty;
                while let TyKind::Own(inner) = self.kind(ty) {
                    place = self.alloc(ExprKind::Deref(place), inner, span);
                    ty = inner;
                }
                // An array lends a slice of all of it, which is what a
                // slice's methods take.
                if as_slice && let TyKind::Array(elem, _) = self.kind(ty) {
                    ty = self.intern(TyKind::Slice(elem));
                    let whole = ExprKind::SubSlice {
                        base: place,
                        lo: None,
                        hi: None,
                    };
                    place = self.alloc(whole, ty, span);
                }
                // A `&T` — a parameter, one kept in a variable, or one a
                // call, a `?` or an element answers — is the reference the
                // method takes: it is passed on, not lent again from what
                // holds it. For a slice's method, so is a
                // `&[T]`; a `&[T; N]` is lent as a slice first.
                if ref_kind == crate::RefKind::Shared
                    && place == receiver
                    && let TyKind::Ref(inner, crate::RefKind::Shared) = self.kind(ty)
                    && !matches!(self.kind(inner), TyKind::Own(_))
                    && (!as_slice || matches!(self.kind(inner), TyKind::Slice(_)))
                {
                    return Some(self.autoderef_to_ref(receiver));
                }
                // A value made where it is called is lent `&var` for the
                // call, and ends with its statement: what the method
                // changed in it goes with it.
                if ref_kind == crate::RefKind::Var && !self.made_here(place) {
                    self.require_writable(place, Writing::Borrow);
                }
                let ref_ty = self.intern(TyKind::Ref(ty, ref_kind));
                Some(self.alloc(ExprKind::Ref(place), ref_ty, span))
            }
            // A `move fn` takes the value itself; `own<T>` waits for a
            // receiver of its own.
            Receiver::Move => {
                if matches!(self.kind(ty), TyKind::Own(_)) {
                    let method = self.text(name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::NOT_A_METHOD,
                        format!(
                            "`{method}` takes its receiver by value, and {} is on the heap",
                            self.ty_name(ty)
                        ),
                        name.span,
                        "a `move fn`",
                    )
                    .with_note("a receiver of type `own<Self>` comes later");
                    self.report(diagnostic);
                    return None;
                }
                if self.state.body.is_place(receiver) && self.owns(ty) {
                    return Some(self.alloc(ExprKind::Move(receiver), ty, span));
                }
                Some(receiver)
            }
            Receiver::Static => None,
        }
    }

    /// `T::name(…)`: a `static fn` of one of the interfaces a type parameter
    /// is constrained by.
    pub(super) fn constrained_static(
        &mut self,
        type_name: ast::Name,
        member: ast::Name,
        item: ItemUse<'a>,
    ) -> Option<ExprId> {
        let self_ty = self.type_param(type_name.sym)?;
        let TyKind::Param(param) = self.kind(self_ty) else {
            return None;
        };
        let constraints = self
            .type_params
            .get(param.index as usize)
            .map(|p| p.interfaces.clone())
            .unwrap_or_default();
        let method = constraints.iter().find_map(|&constraint| {
            let method = self.program.interfaces[constraint.interface]
                .methods
                .iter()
                .find(|m| self.program.fns[m.id].name == member.sym)?;
            Some(*method)
        });
        if let (Some(method), None) = (method, item.args) {
            let written = format!("{}::{}", self.text(type_name.sym), self.text(member.sym));
            return Some(self.function_not_a_value(method.id, &written, item.span));
        }
        let Some(method) = method else {
            let text = self.text(member.sym).to_string();
            let param_name = self.text(param.name).to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                format!("no function `{text}` on `{param_name}`"),
                member.span,
                "unknown function",
            )
            .with_note("a type parameter gives the methods of the interfaces it is constrained by");
            self.report(diagnostic);
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        };
        if self.program.fns[method.id].receiver != Some(Receiver::Static) {
            let text = self.text(member.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                format!("`{text}` takes a receiver"),
                member.span,
                "not a `static fn`",
            )
            .with_note("a method is called on a value: `x.name(…)`");
            self.report(diagnostic);
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        }
        let item = ItemUse {
            self_ty: Some(self_ty),
            ..item
        };
        Some(self.call(method.id, item))
    }

    /// `Type::name` not called: the function's value, a method's receiver its
    /// first parameter. Among several of the name — `Meters::from`, for two
    /// interfaces — the expected type says which.
    fn type_fn_value(
        &mut self,
        candidates: &[FnId],
        written: &str,
        member: ast::Name,
        item: ItemUse<'a>,
    ) -> ExprId {
        // A `lend fn` is its two halves, and lends a place, which no value
        // does: it is refused as any projection is.
        if candidates
            .iter()
            .any(|&id| matches!(self.kind(self.program.fns[id].ret), TyKind::Ref(..)))
        {
            return self.fn_value(candidates[0], item, Some(written));
        }
        let id = match candidates {
            [only] => *only,
            _ => {
                let fits: Vec<FnId> = candidates
                    .iter()
                    .copied()
                    .filter(|&id| item.hint.is_some_and(|hint| self.value_fits(id, hint)))
                    .collect();
                match fits[..] {
                    [only] => only,
                    _ => {
                        let interfaces: Vec<String> = candidates
                            .iter()
                            .map(|&id| self.implemented_name(id))
                            .collect();
                        let diagnostic = Diagnostic::error(
                            codes::NOT_A_METHOD,
                            format!("`{written}` is more than one function"),
                            member.span,
                            "which one is not said",
                        )
                        .with_note(format!(
                            "the type has one for each of {}; the type of the value expected says which",
                            interfaces.join(" and ")
                        ))
                        .with_help(format!(
                            "give the value a type, or write a lambda that calls `{written}(…)`"
                        ));
                        self.report(diagnostic);
                        return self.error_expr(item.span);
                    }
                }
            }
        };
        if !self.method_is_visible(id) {
            let diagnostic = Diagnostic::error(
                codes::PRIVATE_ITEM,
                format!("`{written}` is not exported"),
                member.span,
                "not `pub`",
            )
            .with_note("a function of a type is used outside its module only if it is `pub`");
            self.report(diagnostic);
        }
        self.fn_value(id, item, Some(written))
    }

    /// Whether the function's type is the type expected of it, or the type
    /// a closure lent or owned would be made from.
    fn value_fits(&self, id: FnId, hint: Ty) -> bool {
        let def = &self.program.fns[id];
        let params: Vec<Ty> = def.params.iter().map(|p| p.ty).collect();
        let wanted = match self.kind(hint) {
            TyKind::Ref(inner, _) | TyKind::Own(inner) => inner,
            _ => hint,
        };
        let TyKind::Fn(wanted_params, wanted_ret) = self.kind(wanted) else {
            return false;
        };
        let wanted_params = self.program.types.list(wanted_params);
        params.len() == wanted_params.len()
            && params.iter().zip(wanted_params).all(|(&a, &b)| a == b)
            && def.ret == wanted_ret
    }

    /// `T::name` written where it is not called: a function a type
    /// parameter's constraint answers is not a value yet.
    fn function_not_a_value(&mut self, id: FnId, written: &str, span: Span) -> ExprId {
        // The lambda that stands in for it takes what it takes, by the
        // names it gives them.
        let names: Vec<&str> = self.program.fns[id]
            .params
            .iter()
            .map(|p| self.text(p.name))
            .collect();
        let names = names.join(", ");
        let diagnostic = Diagnostic::error(
            codes::NOT_A_METHOD,
            format!("`{written}` is a function of a type parameter, and is not a value"),
            span,
            "not called",
        )
        .with_note(
            "what a type parameter's constraint answers is not a value yet: it can only be called",
        )
        .with_help(format!("to call it, write `{written}(…)`"))
        .with_help(format!(
            "where a function is wanted, a lambda can call it: `({names}) => {written}({names})`"
        ));
        self.report(diagnostic);
        self.error_expr(span)
    }

    /// `Type::name(…)`: a function of the type, with no receiver.
    pub(super) fn static_fn(
        &mut self,
        module: usize,
        type_name: ast::Name,
        member: ast::Name,
        item: ItemUse<'a>,
    ) -> Option<ExprId> {
        // `type Grid = Map<Id, Cell>`: a `static fn` of the type the
        // alias names, whose parameters the alias decides.
        let mut item = item;
        let mut owner_from_alias = None;
        if !self.modules[module].types.contains_key(&type_name.sym)
            && self.modules[module].aliases.contains_key(&type_name.sym)
        {
            let args: &[ast::TypeId] = item
                .type_args
                .map(|written| written.args.as_slice())
                .unwrap_or(&[]);
            let ty = self.alias_type_of(module, type_name.sym, args, type_name.span)?;
            // The arguments belong to the alias, not to the function.
            item = ItemUse {
                type_args: None,
                owner_ty: Some(ty),
                ..item
            };
            owner_from_alias = match self.kind(ty) {
                TyKind::Struct(id, _) => Some(TypeDef::Struct(id)),
                TyKind::Enum(id, _) => Some(TypeDef::Enum(id)),
                _ => {
                    let text = self.text(type_name.sym).to_string();
                    let named = self.ty_name(ty).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::NOT_A_METHOD,
                        format!("`{text}` names {named}, which has no functions of its own"),
                        type_name.span,
                        "not a struct or an enum",
                    )
                    .with_note(
                        "an alias stands for the type it names, and a `static fn` belongs to a type a program declares",
                    );
                    self.report(diagnostic);
                    return Some(
                        self.give_up(item.args.unwrap_or_default().iter().copied(), item.span),
                    );
                }
            };
        }
        // A type of the prelude, where the path named this module.
        let owner = match owner_from_alias {
            Some(owner) => owner,
            None => match self.modules[module].types.get(&type_name.sym) {
                Some(&(owner, _)) => owner,
                // A built-in type the prelude gave a `static fn` — `Slots<T>`,
                // and whatever else it comes to have.
                None if self.text(type_name.sym) == "Slots" => {
                    TypeDef::Builtin(BuiltinOwner::Slots)
                }
                None if module == self.current => {
                    self.prelude_type(type_name.sym).or_else(|| {
                        let ty = self.builtin(type_name.sym)?;
                        let builtin = BuiltinOwner::of(self.kind(ty))?;
                        self.program
                            .builtins
                            .contains_key(&builtin)
                            .then_some(TypeDef::Builtin(builtin))
                    })?
                }
                None => return None,
            },
        };
        // `Ball` in `Ball::new()` names the type; through an alias, the
        // alias is what is written.
        if owner_from_alias.is_none() {
            self.program
                .names
                .push((type_name.span, Named::Owner(owner)));
        }
        let candidates = self.methods_named(owner, member.sym);
        // Not called, it is the function itself, a method's receiver its
        // first parameter.
        if !candidates.is_empty() && item.args.is_none() {
            let written = format!("{}::{}", self.text(type_name.sym), self.text(member.sym));
            return Some(self.type_fn_value(&candidates, &written, member, item));
        }
        let id = match candidates.len() {
            0 => return None,
            1 => candidates[0],
            // `Meters::from(x)`, where the type implements `From` more than
            // once: the argument says which.
            _ => match self.overload(owner, &candidates, member, &item) {
                Some(id) => id,
                // Reported there; a call that fits no implementation is
                // not something else.
                None => {
                    return Some(
                        self.give_up(item.args.unwrap_or_default().iter().copied(), item.span),
                    );
                }
            },
        };
        if self.program.fns[id].receiver != Some(Receiver::Static) {
            let type_name = self.type_name(owner).to_string();
            let method = self.text(member.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                format!("`{method}` is a method of `{type_name}`, and takes a receiver"),
                member.span,
                "not a `static fn`",
            )
            .with_note("a method is called on a value: `x.name(…)`");
            self.report(diagnostic);
            return Some(self.give_up(item.args.unwrap_or_default().iter().copied(), item.span));
        }
        if !self.method_is_visible(id) {
            let type_name = self.type_name(owner).to_string();
            let method = self.text(member.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::PRIVATE_ITEM,
                format!("`{method}` of `{type_name}` is not exported"),
                member.span,
                "not `pub`",
            )
            .with_note("a static function is used outside its module only if it is `pub`");
            self.report(diagnostic);
        }
        Some(self.call(id, item))
    }

    /// Whether the type has a field of this name; a variant is not one.
    fn field_named(&self, owner: TypeDef, name: Symbol) -> bool {
        match owner {
            TypeDef::Builtin(_) => false,
            TypeDef::Struct(id) => self.program.structs[id]
                .fields
                .iter()
                .any(|f| f.name == name),
            TypeDef::Enum(_) => false,
        }
    }

    /// The names of a type's methods and static functions, for a suggestion.
    fn member_names(&self, owner: TypeDef) -> Vec<&'a str> {
        let methods = match owner {
            TypeDef::Struct(id) => &self.program.structs[id].methods,
            TypeDef::Enum(id) => &self.program.enums[id].methods,
            TypeDef::Builtin(builtin) => match self.program.builtins.get(&builtin) {
                Some(built) => &built.methods,
                None => return Vec::new(),
            },
        };
        // A static function is not a method: it is called through the type
        // rather than through a value. A lending pair is
        // one name, and is named once.
        let mut names: Vec<&'a str> = Vec::new();
        for &id in methods {
            if matches!(self.program.fns[id].receiver, None | Some(Receiver::Static)) {
                continue;
            }
            // What could not be called here is not offered.
            if !self.method_is_visible(id) {
                continue;
            }
            let name = self.text(self.program.fns[id].name);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }

    /// The name of a type, for messages.
    pub(super) fn type_name(&self, owner: TypeDef) -> String {
        match owner {
            TypeDef::Struct(id) => {
                let def = &self.program.structs[id];
                self.program
                    .declared_name(def.name, def.module, self.interner)
            }
            TypeDef::Enum(id) => {
                let def = &self.program.enums[id];
                self.program
                    .declared_name(def.name, def.module, self.interner)
            }
            TypeDef::Builtin(builtin) => builtin.text(),
        }
    }
}

/// How many interfaces offer a call's method.
fn distinct_interfaces(offered: &[super::extensions::Offered]) -> usize {
    let mut seen: Vec<InterfaceId> = Vec::new();
    for offer in offered {
        if !seen.contains(&offer.interface) {
            seen.push(offer.interface);
        }
    }
    seen.len()
}
