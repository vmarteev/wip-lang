//! Items: structs, enums, functions and extern blocks.

use super::*;

/// What a struct's or an enum's body holds: its fields or variants, and its
/// methods, which may be written among them.
enum Member<E> {
    Item(E),
    Method(FnDecl),
}

fn split_members<E>(members: Vec<Member<E>>) -> (Vec<E>, Vec<FnDecl>) {
    let mut items = Vec::new();
    let mut methods = Vec::new();
    for member in members {
        match member {
            Member::Item(item) => items.push(item),
            Member::Method(decl) => methods.push(decl),
        }
    }
    (items, methods)
}

/// What an extern block declares: a C function, or an opaque C type.
enum ExternMember {
    Fn(ExternFn),
    Type(ExternType),
    Global(ExternGlobal),
    /// A `struct` or a `union` written in the block, which is C's layout
    /// without saying `extern` again.
    Struct(StructDecl),
}

/// An annotation's argument begins with a literal, or with the name before
/// `=`.
fn starts_annotation_arg(kind: T) -> bool {
    is_ident(kind) || matches!(kind, T::Int | T::Str(_) | T::True | T::False)
}

/// A member begins with its name, with the words that declare a method, or
/// with an annotation.
fn starts_member(kind: T) -> bool {
    is_ident(kind)
        || matches!(
            kind,
            T::At | T::Fn | T::Pub | T::Var | T::Move | T::Static | T::Lend
        )
}

impl<'a> Parser<'a> {
    /// A `package.wip`: annotations, `package NAME`, and the end of the
    /// file.
    pub(super) fn package_file(&mut self) -> (Vec<Annotation>, Option<Name>) {
        while self.eat(T::Semi) {}
        let annotations = self.annotations();
        let name = if matches!(self.peek(), T::Ident(_)) && self.nth_text(0) == "package" {
            self.bump();
            self.name("the package's name")
        } else {
            let diagnostic = self
                .expected("`package` and the package's name")
                .with_note("a `package.wip` is annotations and `package NAME`, and nothing else");
            self.report(diagnostic);
            None
        };
        while self.eat(T::Semi) {}
        if name.is_some() && !self.at(T::Eof) {
            let diagnostic = Diagnostic::error(
                codes::EXPECTED,
                "a `package.wip` holds nothing after its `package`",
                self.span(),
                "not part of a package's declaration",
            )
            .with_note("the package's modules are its directories; `package.wip` only names it and says what it depends on");
            self.report(diagnostic);
        }
        (annotations, name)
    }

    pub(super) fn program(&mut self) {
        loop {
            while self.eat(T::Semi) {}
            if self.at(T::Eof) {
                return;
            }
            self.begin_region();
            // Annotations come first, then `pub`, then the item.
            let annotations = self.annotations();
            let is_pub = self.at(T::Pub);
            let pub_span = is_pub.then(|| self.bump());
            let item = match self.peek() {
                T::Val => self.val_decl(is_pub, annotations).map(Item::Val),
                // Checked while the program is compiled.
                T::Assert(message) if !is_pub => {
                    let start = self.span();
                    let assert = self.assert_expr(message);
                    let span = start.to(self.prev_span());
                    Some(Item::Assert(AssertDecl {
                        annotations,
                        assert,
                        span,
                    }))
                }
                T::Var | T::Let => {
                    let span = self.span();
                    let diagnostic = Diagnostic::error(
                        codes::EXPECTED,
                        "a global variable is not supported yet",
                        span,
                        "not a constant",
                    )
                    .with_note(
                        "a top-level `val` is a constant, whose value is known where it is written; a variable that lives as long as the program raises questions of its own",
                    )
                    .with_help("write `val`, or declare it inside a function");
                    self.report(diagnostic);
                    self.skip_to_item();
                    None
                }
                T::Struct => self.struct_decl(is_pub, annotations).map(Item::Struct),
                // `view struct Name { … }`: a struct that borrows.
                // `view` is a word only here, so a program
                // may still name something `view`.
                T::Ident(_) if self.nth_text(0) == "view" && self.nth(1) == T::Struct => {
                    let view = self.bump();
                    self.struct_decl(is_pub, annotations).map(|mut decl| {
                        decl.is_view = true;
                        decl.span = view.to(decl.span);
                        Item::Struct(decl)
                    })
                }
                // `view enum Name { … }`: an enum whose variants borrow.
                T::Ident(_) if self.nth_text(0) == "view" && self.nth(1) == T::Enum => {
                    let view = self.bump();
                    self.enum_decl(is_pub, annotations).map(|mut decl| {
                        decl.is_view = true;
                        decl.span = view.to(decl.span);
                        Item::Enum(decl)
                    })
                }
                // `type Id = i64`: another name for a type.
                // Inside an extern block `type` declares
                // a C type instead.
                T::Type => {
                    self.no_annotations(&annotations, "a type alias");
                    self.type_alias(is_pub).map(Item::Type)
                }
                T::Enum => self.enum_decl(is_pub, annotations).map(Item::Enum),
                T::Fn => self.fn_decl(pub_span, annotations).map(Item::Fn),
                // `extend Type: Interface { … }`. `extend`
                // is a word only where an item begins, so a program may
                // still name something `extend`.
                _ if !is_pub && self.at_extend() => {
                    self.extend_block(annotations).map(Item::Extend)
                }
                T::Impl => {
                    let span = self.bump();
                    let diagnostic = Diagnostic::error(
                        codes::IMPL_KEYWORD,
                        "`impl` is written `extend`",
                        span,
                        "not a word of the language",
                    )
                    .with_fix("write `extend`", [Edit::replace(span, "extend")])
                    .with_note(
                        "a type's methods are `extend Type { … }`, and an interface's are `extend Type: Interface { … }`",
                    );
                    self.report(diagnostic);
                    self.skip_braced();
                    None
                }
                T::Interface => self
                    .interface_decl(is_pub, annotations)
                    .map(Item::Interface),
                T::Import if !is_pub => {
                    self.no_annotations(&annotations, "an import");
                    self.import_decl().map(Item::Import)
                }
                // `extern struct Name { … }` is a C layout, not a block of
                // declarations.
                T::Extern if self.nth(1) == T::Struct => {
                    self.bump();
                    self.struct_decl_with(is_pub, true, false, annotations)
                        .map(Item::Struct)
                }
                // `extern union Name { … }`: every field over the same
                // bytes. `union` is a word only here, so a
                // program may still name something `union`.
                T::Extern if is_ident(self.nth(1)) && self.nth_text(1) == "union" => {
                    self.bump();
                    self.struct_decl_with(is_pub, true, true, annotations)
                        .map(Item::Struct)
                }
                T::Extern if !is_pub => self.extern_block(annotations).map(Item::Extern),
                _ if is_pub => {
                    let diagnostic = Diagnostic::error(
                        codes::EXPECTED,
                        "`pub` belongs on a function, struct or enum",
                        pub_span.unwrap_or_else(|| self.span()),
                        "nothing to export here",
                    )
                    .with_note("`import` and `extern` declarations are never exported");
                    self.report(diagnostic);
                    self.skip_to_item();
                    None
                }
                _ => {
                    let mut diagnostic = self.expected(
                        "an item (`fn`, `val`, `struct`, `enum`, `interface`, `extend`, `import` or `extern`)",
                    );
                    if is_stmt_keyword(self.peek()) || can_start_expr(self.peek()) {
                        diagnostic =
                            diagnostic.with_help("statements belong inside a function body");
                    }
                    self.report(diagnostic);
                    self.bump();
                    self.skip_to_item();
                    None
                }
            };
            self.ast.items.extend(item);
        }
    }

    /// `interface Name { … }`: method signatures, each with the receiver it
    /// takes and a default body where one is written.
    fn interface_decl(
        &mut self,
        is_pub: bool,
        annotations: Vec<Annotation>,
    ) -> Option<InterfaceDecl> {
        let start = self.bump();
        let Some(name) = self.name("an interface name") else {
            self.skip_to_item();
            return None;
        };
        // `interface From<T>`: an interface about other types.
        let generics = self.generic_params();
        let open = self.item_body_open()?;
        let list = BraceList {
            sep: T::Comma,
            what: "a method",
            items: "methods",
            code: codes::SEPARATOR,
        };
        let (methods, close) = self.brace_list(open, list, starts_member, Self::interface_method);
        Some(InterfaceDecl {
            annotations,
            name,
            generics,
            methods,
            is_pub,
            span: start.to(close),
        })
    }

    /// One method of an interface: `fn area(): f64`, or with `= expr` a
    /// default body.
    fn interface_method(&mut self) -> Option<InterfaceMethod> {
        // An interface method is a signature, which nothing annotates yet.
        let annotations = self.annotations();
        self.no_annotations(&annotations, "an interface method");
        let start = self.span();
        let receiver = match self.peek() {
            T::Fn => Receiver::Read,
            T::Var => Receiver::Var,
            T::Move => Receiver::Move,
            T::Static => Receiver::Static,
            _ => {
                let diagnostic = self.expected("a method");
                self.report(diagnostic);
                self.skip_until(|k, line_break| line_break || k == T::Comma);
                return None;
            }
        };
        let mut keyword = start;
        if receiver != Receiver::Read {
            keyword = self.bump();
            if !self.at(T::Fn) {
                let diagnostic = Diagnostic::error(
                    codes::MEMBER_SYNTAX,
                    format!("expected `fn` after `{}`", self.text(keyword)),
                    Span::at(keyword.hi),
                    "expected `fn`",
                )
                .with_note(format!("`{}` declares a method", receiver.text()));
                self.report(diagnostic);
                self.skip_until(|k, line_break| line_break || k == T::Comma);
                return None;
            }
        }
        let sig = self.fn_sig()?;
        let default = self.eat(T::Eq).then(|| self.expr());
        let end = default.map_or(sig.span, |body| self.expr_span(body));
        Some(InterfaceMethod {
            sig,
            receiver: (receiver, keyword),
            default,
            span: start.to(end),
        })
    }

    /// `extend [T] { … }`, at the `[`. The name inside it
    /// declares the element, as a type parameter of the block.
    fn slice_extend(&mut self, start: Span, annotations: Vec<Annotation>) -> Option<ExtendBlock> {
        let open = self.bump();
        let Some(elem) = self.name("the element's name") else {
            self.skip_to_item();
            return None;
        };
        // `extend [T: Ord] { … }`: methods a slice has where its element
        // answers the constraint.
        let bounds = self.bounds()?;
        self.expect_closing(T::RBracket, open);
        let (interface, interface_args) = self.extended_interface()?;
        let body = self.item_body_open()?;
        let list = BraceList {
            sep: T::Comma,
            what: "a method",
            items: "methods",
            code: codes::SEPARATOR,
        };
        let (members, close) = self.brace_list(body, list, starts_member, |p| {
            p.member(|p, _| {
                let diagnostic = p.expected("a method");
                p.report(diagnostic);
                p.skip_until(|k, line_break| line_break || k == T::Comma);
                None::<Field>
            })
        });
        let (_, methods) = split_members(members);
        Some(ExtendBlock {
            annotations,
            slice_of: Some(elem),
            interface,
            interface_args,
            path: vec![elem],
            generics: vec![GenericParam {
                name: elem,
                span: elem.span.to(self.prev_span()),
                bounds,
                default: None,
            }],
            methods,
            derived: None,
            span: start.to(close),
        })
    }

    /// `: Interface`, with the interface's own type arguments where it
    /// takes any.
    fn extended_interface(&mut self) -> Option<(Option<Name>, Option<TypeArgs>)> {
        if !self.eat(T::Colon) {
            return Some((None, None));
        }
        let Some(named) = self.name("an interface name") else {
            self.skip_braced();
            return None;
        };
        let args = self.at(T::Lt).then(|| {
            let (args, span) = self.type_arg_list();
            TypeArgs {
                after: 0,
                args,
                span,
            }
        });
        // One interface per block, so that every method belongs to one.
        if self.at(T::Plus) {
            let span = self.span();
            let diagnostic = Diagnostic::error(
                codes::EXPECTED,
                "one interface at a time",
                span,
                "a second interface",
            )
            .with_note(
                "a block is exactly one interface's methods, so that a method belongs to one of them",
            )
            .with_help("write another `extend` block for it");
            self.report(diagnostic);
            self.skip_braced();
            return None;
        }
        Some((Some(named), args))
    }

    /// Whether an item begins `extend`, which is a word only here.
    fn at_extend(&self) -> bool {
        is_ident(self.peek()) && self.text(self.span()) == "extend"
    }

    /// `extend Type { … }` and `extend Type: Interface { … }`: the type
    /// first, and the interface written as a constraint is.
    fn extend_block(&mut self, annotations: Vec<Annotation>) -> Option<ExtendBlock> {
        let start = self.bump();
        // `extend [T] { … }`: methods of a slice, whose element the block
        // declares.
        if self.at(T::LBracket) {
            return self.slice_extend(start, annotations);
        }
        let Some(name) = self.name("a type name") else {
            self.skip_to_item();
            return None;
        };
        // A path is parsed so that `extend pkg::Type` is reported as what
        // it is: a type of another module.
        let mut path = vec![name];
        while self.eat(T::ColonColon) {
            match self.name("a type name") {
                Some(name) => path.push(name),
                None => {
                    self.skip_to_item();
                    return None;
                }
            }
        }
        // The type's own parameters: `extend Slots<T>`, `extend Boxed<V>`.
        let generics = self.generic_params();
        let (interface, interface_args) = self.extended_interface()?;
        let open = self.item_body_open()?;
        let list = BraceList {
            sep: T::Comma,
            what: "a method",
            items: "methods",
            code: codes::SEPARATOR,
        };
        let (members, close) = self.brace_list(open, list, starts_member, |p| {
            p.member(|p, _| {
                let diagnostic = p.expected("a method");
                p.report(diagnostic);
                p.skip_until(|k, line_break| line_break || k == T::Comma);
                None::<Field>
            })
        });
        let (_, methods) = split_members(members);
        Some(ExtendBlock {
            annotations,
            slice_of: None,
            interface,
            interface_args,
            path,
            generics,
            methods,
            derived: None,
            span: start.to(close),
        })
    }

    /// Expects `{` to open the body of an item; on failure, skips to the next
    /// item.
    pub(super) fn item_body_open(&mut self) -> Option<Span> {
        if self.at(T::LBrace) {
            return Some(self.bump());
        }
        let diagnostic = self.expected("`{`");
        self.report(diagnostic);
        self.skip_to_item();
        None
    }

    /// `type Name<T> = type`.
    fn type_alias(&mut self, is_pub: bool) -> Option<TypeAlias> {
        let start = self.bump();
        let name = self.name("a name for the type")?;
        let generics = self.generic_params();
        if !self.expect(T::Eq) {
            self.skip_to_item();
            return None;
        }
        let ty = self.ty();
        Some(TypeAlias {
            is_pub,
            name,
            generics,
            ty,
            span: start.to(self.prev_span()),
        })
    }

    pub(super) fn struct_decl(
        &mut self,
        is_pub: bool,
        annotations: Vec<Annotation>,
    ) -> Option<StructDecl> {
        self.struct_decl_with(is_pub, false, false, annotations)
    }

    /// `extern struct Name { … }`: C's layout, and fields C can write.
    pub(super) fn struct_decl_with(
        &mut self,
        is_pub: bool,
        is_extern: bool,
        is_union: bool,
        annotations: Vec<Annotation>,
    ) -> Option<StructDecl> {
        let start = self.bump();
        let what = if is_union {
            "a union name"
        } else {
            "a struct name"
        };
        let Some(name) = self.name(what) else {
            self.skip_to_item();
            return None;
        };
        let generics = self.generic_params();
        let open = self.item_body_open()?;
        let list = BraceList {
            sep: T::Comma,
            what: "a field",
            items: "fields",
            code: codes::SEPARATOR,
        };
        let (members, close) = self.brace_list(open, list, starts_member, |p| {
            // `name: T = value`: what a literal that leaves the field out
            // puts there.
            p.member(|p, pub_span| {
                // `pub var x: i64` is written by anyone; `pub x: i64` is
                // read by anyone and written where it is declared.
                let var_span = p.at(T::Var).then(|| p.bump());
                let mut field = p.field("a field name")?;
                field.is_pub = pub_span.is_some();
                field.is_var = var_span.is_some();
                if let Some(var_span) = var_span
                    && pub_span.is_none()
                {
                    let diagnostic = Diagnostic::error(
                        codes::MEMBER_SYNTAX,
                        "`var` on a field says who else may write it",
                        var_span,
                        "not exported",
                    )
                    .with_note(
                        "a field its module does not export is that module's to read and write; `pub var` is what lets another module write it",
                    )
                    .with_fix("remove `var`", [Edit::replace(var_span, "")]);
                    p.report(diagnostic);
                }
                p.field_default(&mut field);
                if let Some(pub_span) = pub_span {
                    field.span = pub_span.to(field.span);
                }
                Some(field)
            })
        });
        let (fields, methods) = split_members(members);
        Some(StructDecl {
            annotations,
            is_extern,
            is_union,
            is_view: false,
            name,
            generics,
            fields,
            methods,
            is_pub,
            span: start.to(close),
        })
    }

    pub(super) fn enum_decl(
        &mut self,
        is_pub: bool,
        annotations: Vec<Annotation>,
    ) -> Option<EnumDecl> {
        let start = self.bump();
        let Some(name) = self.name("an enum name") else {
            self.skip_to_item();
            return None;
        };
        let generics = self.generic_params();
        let open = self.item_body_open()?;
        let list = BraceList {
            sep: T::Comma,
            what: "a variant",
            items: "variants",
            code: codes::SEPARATOR,
        };
        let (members, close) = self.brace_list(open, list, starts_member, |p| {
            p.member(|p, pub_span| {
                // A variant is exported with its enum: there is nothing
                // for `pub` to say.
                if let Some(pub_span) = pub_span {
                    let diagnostic = Diagnostic::error(
                        codes::MEMBER_SYNTAX,
                        "`pub` belongs on a field or a method",
                        pub_span,
                        "not a field or a method",
                    )
                    .with_note(
                        "an enum's variants are exported with it, and so are the fields they carry",
                    )
                    .with_fix("remove `pub`", [Edit::replace(pub_span, "")]);
                    p.report(diagnostic);
                }
                p.variant()
            })
        });
        let (variants, methods) = split_members(members);
        Some(EnumDecl {
            annotations,
            name,
            generics,
            variants,
            methods,
            is_pub,
            is_view: false,
            span: start.to(close),
        })
    }

    /// A member of a struct or an enum: a field or a variant, or a method.
    /// They may be written in any order.
    fn member<E>(
        &mut self,
        elem: impl FnOnce(&mut Self, Option<Span>) -> Option<E>,
    ) -> Option<Member<E>> {
        // A method may carry annotations; a field or a variant may not.
        let annotations = self.annotations();
        let is_pub = self.at(T::Pub);
        let pub_span = is_pub.then(|| self.bump());
        let receiver = match self.peek() {
            T::Fn => Some(Receiver::Read),
            T::Var => Some(Receiver::Var),
            T::Move => Some(Receiver::Move),
            T::Static => Some(Receiver::Static),
            // `lend fn`: both halves of a lending pair.
            T::Lend if self.nth(1) == T::Fn => Some(Receiver::Lend),
            _ => None,
        };
        // `pub var x: i64`: a field another module may write, not a
        // method. `var` before a name is the field's;
        // before `fn` it is the method's receiver.
        let receiver = match receiver {
            Some(Receiver::Var) if is_ident(self.nth(1)) && self.nth(2) == T::Colon => None,
            other => other,
        };
        let Some(receiver) = receiver else {
            self.no_annotations(&annotations, "a field or a variant");
            // `pub` on a field exports it; what the
            // element is decides whether it may carry one.
            return elem(self, pub_span).map(Member::Item);
        };
        // `var`, `move`, `static` and `lend` stand before `fn`.
        let start = self.span();
        let mut keyword = start;
        if receiver != Receiver::Read {
            keyword = self.bump();
            if !self.at(T::Fn) {
                let diagnostic = Diagnostic::error(
                    codes::MEMBER_SYNTAX,
                    format!("expected `fn` after `{}`", self.text(keyword)),
                    Span::at(keyword.hi),
                    "expected `fn`",
                )
                .with_note(format!("`{}` declares a method", receiver.text()));
                self.report(diagnostic);
                // The rest of the line is skipped, and the body goes on with
                // the next member.
                self.skip_until(|k, line_break| line_break || k == T::Comma);
                return None;
            }
        }
        let mut decl = self.fn_decl(pub_span, annotations)?;
        decl.receiver = Some((receiver, keyword));
        decl.span = pub_span.unwrap_or(start).to(decl.span);
        Some(Member::Method(decl))
    }

    pub(super) fn variant(&mut self) -> Option<Variant> {
        let name = self.name("a variant name")?;
        let mut fields = Vec::new();
        if self.at(T::LParen) {
            let open = self.bump();
            // A type with no name before it, as a tuple variant is written
            // in Rust or Swift: each is read as a type, so that the rest
            // parses, and all are reported together below.
            let mut unnamed: Vec<Span> = Vec::new();
            // A field may say what it is when a literal leaves it out, as
            // a struct's may.
            let (parsed, close) = self.list(open, T::RParen, "a field", can_start_type, |p| {
                let named =
                    is_ident(p.peek()) && (p.nth(1) == T::Colon || can_start_type(p.nth(1)));
                if !named {
                    let ty = p.ty();
                    unnamed.push(p.ast.types[ty].span);
                    return None;
                }
                let mut field = p.field("a field name")?;
                p.field_default(&mut field);
                Some(field)
            });
            if let Some(&first) = unnamed.first() {
                let mut diagnostic = Diagnostic::error(
                    codes::EXPECTED,
                    "a variant's fields are named",
                    first,
                    "a type without a name",
                )
                .with_note(
                    "a field is `name: type`, as a struct's is: a pattern binds it by its name, and a variant built with two or more fields names each",
                );
                for &other in &unnamed[1..] {
                    diagnostic = diagnostic.with_secondary(other, "and this");
                }
                diagnostic = if unnamed.len() == 1 && parsed.is_empty() {
                    let help = format!("name it, as `value: {}`", self.text(first));
                    diagnostic.with_fix(help, [Edit::insert(first.lo, "value: ")])
                } else {
                    diagnostic.with_help("name each, as `name: type`")
                };
                self.report(diagnostic);
            } else if parsed.is_empty() && self.text(close) == ")" {
                let parens = open.to(close);
                let diagnostic = Diagnostic::error(
                    codes::EXPECTED,
                    "a variant without fields is written without parentheses",
                    parens,
                    "empty parentheses",
                )
                .with_fix("remove the parentheses", [Edit::replace(parens, "")]);
                self.report(diagnostic);
            }
            fields = parsed;
        }
        Some(Variant {
            name,
            fields,
            span: name.span.to(self.prev_span()),
        })
    }

    /// `name: type` — struct fields, variant fields and parameters.
    pub(super) fn field(&mut self, what: &str) -> Option<Field> {
        let name = self.name(what)?;
        self.field_after(name)
    }

    /// A field's or a parameter's `: Type`, after its name.
    pub(super) fn field_after(&mut self, name: Name) -> Option<Field> {
        if !self.eat(T::Colon) {
            let mut diagnostic = self.expected("`:` and a type");
            let type_follows = can_start_type(self.peek()) && !self.on_later_line();
            if type_follows {
                diagnostic = diagnostic.with_fix("add `:`", [Edit::insert(name.span.hi, ":")]);
            }
            self.report(diagnostic);
            if !type_follows {
                return None;
            }
        }
        let ty = self.ty();
        Some(Field {
            is_pub: false,
            is_var: false,
            name,
            ty,
            default: None,
            span: name.span.to(self.ast.types[ty].span),
        })
    }

    /// `= value` after a field's or a parameter's type: what is put there
    /// when nothing else is.
    pub(super) fn field_default(&mut self, field: &mut Field) {
        if self.eat(T::Eq) {
            let value = self.expr();
            field.default = Some(value);
            field.span = field.span.to(self.expr_span(value));
        }
    }

    pub(super) fn param(&mut self) -> Option<Param> {
        if self.at(T::DotDotDot) {
            // `...` is not a parameter: the signature remembers it, and
            // what a call may pass there is the checker's.
            self.variadic = Some(self.bump());
            return None;
        }
        let name = self.param_name("a parameter name")?;
        let mut param = self.field_after(name)?;
        self.field_default(&mut param);
        Some(param)
    }

    pub(super) fn fn_sig(&mut self) -> Option<FnSig> {
        let start = self.bump();
        let Some(name) = self.name("a function name") else {
            self.skip_to_item();
            return None;
        };
        let generics = self.generic_params();
        if !self.at(T::LParen) {
            let diagnostic = self.expected("`(`");
            self.report(diagnostic);
            self.skip_to_item();
            return None;
        }
        let open = self.bump();
        let outer = self.variadic.take();
        let (params, close) =
            self.list(open, T::RParen, "a parameter", can_start_param, Self::param);
        let variadic = std::mem::replace(&mut self.variadic, outer);
        let ret = match self.peek() {
            T::Colon => {
                self.bump();
                Some(self.ty())
            }
            T::Arrow => {
                let arrow = self.bump();
                let diagnostic = Diagnostic::error(
                    codes::ARROW_RETURN_TYPE,
                    "return types are written with `:`",
                    arrow,
                    "`->` is not used in Wip",
                )
                .with_fix(
                    "write `: T` after the parameters",
                    [Edit::replace(Span::new(close.hi, arrow.hi), ":")],
                );
                self.report(diagnostic);
                Some(self.ty())
            }
            _ => None,
        };
        let end = ret.map_or(close, |t| self.ast.types[t].span);
        Some(FnSig {
            name,
            generics,
            params,
            variadic,
            ret,
            span: start.to(end),
        })
    }

    /// The annotations before a declaration: `@name`, or `@name(a, b = 1)`.
    /// The names mean nothing to the parser; the checker
    /// knows the set and where each may be written.
    pub(super) fn annotations(&mut self) -> Vec<Annotation> {
        let mut annotations = Vec::new();
        while self.at(T::At) {
            let start = self.bump();
            let Some(name) = self.name("an annotation name") else {
                self.skip_until(|k, line_break| line_break || k == T::At);
                continue;
            };
            let mut args = Vec::new();
            let mut parens = None;
            if self.at(T::LParen) {
                let open = self.bump();
                let (parsed, close) =
                    self.list(open, T::RParen, "an argument", starts_annotation_arg, |p| {
                        p.annotation_arg()
                    });
                args = parsed;
                parens = Some(open.to(close));
            }
            let span = start.to(parens.unwrap_or(name.span));
            annotations.push(Annotation {
                name,
                args,
                parens,
                span,
            });
            // One annotation per line is the style; several on a line are
            // read the same way.
            while self.eat(T::Semi) {}
        }
        annotations
    }

    /// One argument of an annotation: a literal, or `name = literal`.
    fn annotation_arg(&mut self) -> Option<AnnotationArg> {
        let start = self.span();
        let name = if is_ident(self.peek()) && self.nth(1) == T::Eq {
            let name = self.name("an argument name")?;
            self.bump();
            Some(name)
        } else {
            None
        };
        let (value, span) = match self.peek() {
            T::Int => {
                let span = self.span();
                let value = self.int_value(span);
                self.bump();
                (AnnotationValue::Int(value), span)
            }
            T::Str(sym) => (AnnotationValue::Str(sym), self.bump()),
            T::True => (AnnotationValue::Bool(true), self.bump()),
            T::False => (AnnotationValue::Bool(false), self.bump()),
            // A name, as `@derive(Eq, Hash)` takes. What it
            // names is the checker's to decide.
            T::Ident(sym) => (AnnotationValue::Name(sym), self.bump()),
            _ => {
                let span = self.span();
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION_SYNTAX,
                    "an annotation takes numbers, strings, `true` or `false`, and names",
                    span,
                    "not a literal",
                )
                .with_note(
                    "an annotation is read before the program has a meaning, so nothing in it is computed",
                );
                self.report(diagnostic);
                self.bump();
                (AnnotationValue::Error, span)
            }
        };
        Some(AnnotationArg {
            name,
            value,
            span: start.to(span),
        })
    }

    /// Reports annotations that bind to nothing, or to a declaration that
    /// cannot carry them.
    pub(super) fn no_annotations(&mut self, annotations: &[Annotation], what: &str) {
        for annotation in annotations {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION_SYNTAX,
                format!("an annotation cannot be written on {what}"),
                annotation.span,
                "nothing here takes an annotation",
            )
            .with_note(
                "an annotation belongs on a function, a struct, an enum, an interface or an `extend` block",
            );
            self.report(diagnostic);
        }
    }

    /// `val NAME: T = value` at the top level: a constant.
    fn val_decl(&mut self, is_pub: bool, annotations: Vec<Annotation>) -> Option<ValDecl> {
        let start = self.bump();
        let name = self.name("a constant's name")?;
        let ty = if self.eat(T::Colon) {
            Some(self.ty())
        } else {
            None
        };
        let value = if self.expect(T::Eq) {
            self.expr()
        } else {
            self.error_expr()
        };
        Some(ValDecl {
            annotations,
            name,
            ty,
            value,
            is_pub,
            span: start.to(self.expr_span(value)),
        })
    }

    /// A `fn` declaration. A method's receiver is set by its caller.
    pub(super) fn fn_decl(
        &mut self,
        pub_span: Option<Span>,
        annotations: Vec<Annotation>,
    ) -> Option<FnDecl> {
        let is_pub = pub_span.is_some();
        let sig = self.fn_sig()?;
        // `@intrinsic` says the compiler writes the body, so none is here.
        let intrinsic = annotations
            .iter()
            .any(|annotation| self.text(annotation.name.span) == "intrinsic");
        if intrinsic && !self.at(T::Eq) {
            return Some(FnDecl {
                annotations,
                span: sig.span,
                sig,
                body: None,
                is_pub,
                pub_span,
                receiver: None,
            });
        }
        let body = if self.eat(T::Eq) {
            self.expr()
        } else if self.at(T::LBrace) {
            let brace = self.span();
            let diagnostic = Diagnostic::error(
                codes::FUNCTION_BODY,
                "expected `=` before the function body",
                brace,
                "the body starts here",
            )
            .with_note("a function is declared `fn name(…): T = body`")
            .with_fix("add `=`", [Edit::insert(brace.lo, "= ")]);
            self.report(diagnostic);
            self.expr()
        } else {
            let diagnostic = Diagnostic::error(
                codes::FUNCTION_BODY,
                format!("function `{}` has no body", self.text(sig.name.span)),
                Span::at(sig.span.hi),
                "expected `= …` here",
            )
            .with_help("to declare a C function, put it in an `extern \"C\" { … }` block");
            self.report(diagnostic);
            self.error_expr()
        };
        Some(FnDecl {
            annotations,
            span: sig.span.to(self.expr_span(body)),
            sig,
            body: Some(body),
            is_pub,
            pub_span,
            receiver: None,
        })
    }

    /// `import a::b::c`, `import a::b::c as name`, or
    /// `import a::b::{self, Item, other as name}`.
    pub(super) fn import_decl(&mut self) -> Option<ImportDecl> {
        let start = self.bump();
        let mut path = vec![self.name("a module name")?];
        let mut items = None;
        while self.eat(T::ColonColon) {
            if self.at(T::LBrace) {
                items = Some(self.import_items(self.prev_span()));
                break;
            }
            path.push(self.name("a module name")?);
        }
        let mut alias = None;
        if self.at(T::As) {
            let keyword = self.bump();
            let name = self.name("a name for the module")?;
            if items.is_some() {
                let diagnostic = Diagnostic::error(
                    codes::IMPORT_LIST,
                    "an import list cannot be renamed",
                    keyword.to(name.span),
                    "renames the whole list",
                )
                .with_help(format!(
                    "to bind the module, name it in the list: `{{self as {}}}`",
                    self.text(name.span)
                ))
                .with_note("an import list binds only the names in it");
                self.report(diagnostic);
            } else {
                alias = Some(name);
            }
        }
        Some(ImportDecl {
            span: start.to(self.prev_span()),
            path,
            alias,
            items,
        })
    }

    /// The braces of an import, after the `::` at `colons`:
    /// `{self, Item, other as name}`.
    fn import_items(&mut self, colons: Span) -> Vec<ImportItem> {
        let open = self.bump();
        let list = BraceList {
            sep: T::Comma,
            what: "a name to import",
            items: "names",
            code: codes::SEPARATOR,
        };
        let starts_item = |kind| is_ident(kind) || kind == T::SelfKw;
        let (items, close) = self.brace_list(open, list, starts_item, |p| {
            // `self` in the list binds the module itself.
            let (name, span) = if p.at(T::SelfKw) {
                (None, p.bump())
            } else {
                let name = p.name("a name to import")?;
                (Some(name), name.span)
            };
            let alias = if p.eat(T::As) {
                Some(p.name("a name for it")?)
            } else {
                None
            };
            Some(ImportItem { name, span, alias })
        });
        if items.is_empty() {
            let diagnostic = Diagnostic::error(
                codes::IMPORT_LIST,
                "an import list must name something",
                open.to(close),
                "names nothing",
            )
            .with_fix(
                "import the module itself",
                [Edit::replace(colons.to(close), "")],
            );
            self.report(diagnostic);
        }
        items
    }

    pub(super) fn extern_block(&mut self, annotations: Vec<Annotation>) -> Option<ExternBlock> {
        let start = self.bump();
        let abi = if let T::Str(_) = self.peek() {
            let span = self.bump();
            let text = self.text(span);
            if text != "\"C\"" {
                let diagnostic = Diagnostic::error(
                    codes::UNSUPPORTED_ABI,
                    format!("unsupported ABI {text}"),
                    span,
                    "only \"C\" is supported",
                )
                .with_note("the prototype can call C functions and nothing else");
                self.report(diagnostic);
            }
            span
        } else {
            let at = self.span().lo;
            let diagnostic = self
                .expected("an ABI string")
                .with_fix("add the C ABI", [Edit::insert(at, "\"C\" ")]);
            self.report(diagnostic);
            Span::at(self.prev_span().hi)
        };
        let open = self.item_body_open()?;
        let list = BraceList {
            sep: T::Semi,
            what: "a function declaration",
            items: "declarations",
            code: codes::SEPARATOR,
        };
        let (members, close) = self.brace_list(
            open,
            list,
            |k| {
                matches!(
                    k,
                    T::Fn | T::Pub | T::Type | T::Val | T::Var | T::At | T::Struct
                ) || is_ident(k)
            },
            Self::extern_member,
        );
        let mut fns = Vec::new();
        let mut types = Vec::new();
        let mut globals = Vec::new();
        let mut structs = Vec::new();
        for member in members {
            match member {
                ExternMember::Fn(sig) => fns.push(sig),
                ExternMember::Type(name) => types.push(name),
                ExternMember::Global(global) => globals.push(global),
                ExternMember::Struct(declared) => structs.push(declared),
            }
        }
        Some(ExternBlock {
            structs,
            annotations,
            abi,
            fns,
            types,
            globals,
            span: start.to(close),
        })
    }

    /// A declaration inside an extern block: a function, an opaque C type,
    /// or a variable C owns.
    fn extern_member(&mut self) -> Option<ExternMember> {
        // `@symbol("sqlite3_open")` says what C calls it, so a module of
        // bindings can use Wip's own names.
        let annotations = self.annotations();
        // `pub` exports a C declaration from its module, so that bindings
        // are a module people share.
        let is_pub = self.at(T::Pub);
        if is_pub {
            self.bump();
        }
        // `struct Name { … }` and `union Name { … }` in a block are C's
        // layout: the block says `extern` once.
        if self.at(T::Struct) {
            return self
                .struct_decl_with(is_pub, true, false, annotations)
                .map(ExternMember::Struct);
        }
        if is_ident(self.peek()) && self.text(self.span()) == "union" {
            return self
                .struct_decl_with(is_pub, true, true, annotations)
                .map(ExternMember::Struct);
        }
        if self.at(T::Type) {
            self.bump();
            return self.name("a C type's name").map(|name| {
                ExternMember::Type(ExternType {
                    name,
                    annotations,
                    is_pub,
                })
            });
        }
        // `val optind: c_int` reads it; `var` writes it too (item 13).
        if matches!(self.peek(), T::Val | T::Var) {
            let is_mut = self.at(T::Var);
            let start = self.bump();
            let name = self.name("a variable's name")?;
            if !self.eat(T::Colon) {
                let at = self.prev_span().hi;
                let diagnostic = self
                    .expected("`:` and the variable's type")
                    .with_fix("give it a type", [Edit::insert(at, ": c_int")]);
                self.report(diagnostic);
                return None;
            }
            let ty = self.ty();
            return Some(ExternMember::Global(ExternGlobal {
                name,
                annotations,
                ty,
                is_mut,
                is_pub,
                span: start.to(self.ast.types[ty].span),
            }));
        }
        self.extern_fn().map(|sig| {
            ExternMember::Fn(ExternFn {
                sig,
                annotations,
                is_pub,
            })
        })
    }

    pub(super) fn extern_fn(&mut self) -> Option<FnSig> {
        let sig = self.fn_sig()?;
        if matches!(self.peek(), T::Eq | T::LBrace) {
            let start = self.span();
            self.eat(T::Eq);
            let body = self.expr();
            let name = self.text(sig.name.span);
            let diagnostic = Diagnostic::error(
                codes::FUNCTION_BODY,
                "extern functions cannot have a body",
                start,
                "body starts here",
            )
            .with_help(format!(
                "extern functions are defined in C; to define `{name}` in Wip, move it out of the extern block"
            ))
            .with_fix(
                "remove the body",
                [Edit::replace(Span::new(sig.span.hi, self.expr_span(body).hi), "")],
            );
            self.report(diagnostic);
        }
        Some(sig)
    }
}
