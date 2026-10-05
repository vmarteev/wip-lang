//! Generators: a loop that yields where a value stands is
//! an `Iterator`, which runs as it is asked.
//!
//! ```text
//! val evens = for x in xs { if x % 2 == 0 { yield x } }
//!     ⇒ val evens = generator# { state#: 0, xs: &xs }
//!
//! extend generator#: Iterator<i64> {
//!     var fn next(): Option<i64> = { for x in *self.xs { if x % 2 == 0 { yield .Some(x) } } }
//! }
//! ```
//!
//! The struct holds where it stopped and references to what the loop
//! names, as a closure lent for a call holds them, so it
//! is a view of them. Its `next` is the loop, checked as a body of its
//! own; the MIR runs it on from the `yield` it stopped at, and keeps its
//! locals in the struct between calls, after the fields declared here.

use super::*;

/// A generator whose body is being checked: what it yields, once a
/// `yield` or the type expected of it says.
pub(super) struct GeneratorBuild {
    pub(super) elem: Option<Ty>,
    /// Where the loop is written, for a loop that is a generator, which a
    /// `return` in it cannot leave; a function that yields can.
    pub(super) loop_span: Option<Span>,
    /// Whether a `yield` hands it a value.
    pub(super) yielded: bool,
}

/// Whether each `for` of a file that stands for a value is a generator:
/// one among a list's elements, or after `own`, is a list built as it
/// runs.
fn list_elements(ast: &Ast) -> FxHashSet<ast::ExprId> {
    let mut elements = FxHashSet::default();
    for (_, expr) in ast.exprs.iter() {
        match &expr.kind {
            ast::ExprKind::Array(elems) => elements.extend(elems.iter().copied()),
            ast::ExprKind::Unary {
                op: ast::UnaryOp::Own,
                operand,
                ..
            } => {
                elements.insert(*operand);
            }
            _ => {}
        }
    }
    elements
}

impl<'a> Lowerer<'a> {
    /// A struct and its `next` for every loop of a file that is a
    /// generator, before any body is checked: bodies are checked on
    /// threads of their own, which cannot declare anything.
    /// What they hold and yield is filled in when the
    /// loop is checked.
    pub(super) fn collect_generators(&mut self) -> FxHashMap<ast::ExprId, StructId> {
        let ast = self.ast;
        let in_lists = list_elements(ast);
        let mut generators = FxHashMap::default();
        for (expr_id, expr) in ast.exprs.iter() {
            if !matches!(expr.kind, ast::ExprKind::ForElement { .. }) || in_lists.contains(&expr_id)
            {
                continue;
            }
            let id = self.declare_generator(expr.span);
            generators.insert(expr_id, id);
        }
        generators
    }

    /// A generator's struct, and its `next`, empty.
    fn declare_generator(&mut self, span: Span) -> StructId {
        let next = self.program.fns.alloc(FnDef {
            name: Symbol::next(),
            name_span: span,
            receiver: Some(Receiver::Var),
            owner: None,
            interface: None,
            generics: Vec::new(),
            instance_of: None,
            params: Vec::new(),
            ret: Types::ERROR,
            ret_span: None,
            is_extern: false,
            exports_c: false,
            header: None,
            symbol: None,
            accesses: None,
            is_variadic: false,
            variadic_of: None,
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
            module: self.current as u32,
            is_pub: false,
            span,
        });
        let id = self.program.structs.alloc(StructDef {
            name: Symbol::generator(),
            is_extern: false,
            is_union: false,
            is_opaque: false,
            is_intrinsic: false,
            header: None,
            accessors: Vec::new(),
            generics: Vec::new(),
            fields: Vec::new(),
            methods: vec![next],
            is_env: false,
            is_view: false,
            is_tuple: false,
            generator: Some(GeneratorDef {
                next,
                elem: Types::ERROR,
                of: None,
            }),
            module: self.current as u32,
            is_pub: false,
            span,
        });
        let def = &mut self.program.fns[next];
        def.owner = Some(TypeDef::Struct(id));
        def.generator = Some(id);
        id
    }

    /// Each generator of the file implements `Iterator`, with the
    /// interface's methods its `next` does not replace: `map`, `filter`
    /// and the rest. What it yields is known once its
    /// loop is checked, and filled in then.
    pub(super) fn declare_generator_impls(&mut self) {
        let Some(interface) = self
            .program
            .prelude_items
            .interface(KnownInterface::Iterator)
        else {
            return;
        };
        let mut generators: Vec<StructId> = self.generators.values().copied().collect();
        generators.sort_by_key(|id| id.into_raw());
        let wanted = self.program.interfaces[interface].methods.clone();
        for id in generators {
            let owner = TypeDef::Struct(id);
            let next = self.program.structs[id]
                .generator
                .expect("a generator")
                .next;
            let mut methods = vec![next];
            for want in wanted.iter().skip(1) {
                self.add_default(owner, want.id);
                methods.push(want.id);
            }
            let span = self.program.structs[id].span;
            self.program.impls.push(ImplDef {
                interface,
                args: crate::TyList::EMPTY,
                ty: owner,
                methods,
                conditions: Vec::new(),
                module: self.current as u32,
                span,
            });
        }
    }

    /// `for binding in source { … }` where a value stands: a generator,
    /// whose loop runs as its values are asked for.
    pub(super) fn loop_generator(
        &mut self,
        expr: ast::ExprId,
        binding: &'a ast::Pattern,
        source: ast::ForSource,
        body: &'a ast::Block,
        span: Span,
    ) -> ExprId {
        let Some(&id) = self.generators.get(&expr) else {
            // Among a list's elements, which the list collects; reported
            // where the list is.
            return self.error_expr(span);
        };
        let next = self.program.structs[id]
            .generator
            .expect("a generator")
            .next;
        let type_args = self.own_type_args();
        let gen_ty = self.intern(TyKind::Struct(id, type_args));
        let self_ty = self.intern(TyKind::Ref(gen_ty, crate::RefKind::Var));

        // The loop is checked as a body of its own, as a lambda's is: a
        // name it does not declare is one around it, which it captures.
        self.outer.push(std::mem::take(&mut self.state));
        self.state.scopes = vec![FxHashMap::default()];
        self.state.ret = Types::UNIT;
        self.state.capture_kind = Some(lambdas::CaptureKind::Generator);
        self.state.generator = Some(GeneratorBuild {
            elem: None,
            loop_span: Some(span),
            yielded: false,
        });
        let this = self.declare(Symbol::generator(), self_ty, LocalKind::Param, span);
        self.state.body.params.push(this);
        let kind = self.for_stmt(None, binding, source, body);
        let stmt = self.state.body.stmts.alloc(Stmt { kind, span });
        let block = Block {
            stmts: vec![stmt],
            value: None,
            span,
        };
        let value = self.alloc(ExprKind::Block(block), Types::UNIT, span);
        self.state.body.value = Some(value);
        let elem = self.state.generator.take().and_then(|build| build.elem);
        self.settle_bindings();
        let code = std::mem::take(&mut self.state.body);
        let captures = std::mem::take(&mut self.state.captures);
        self.state = self.outer.pop().expect("a generator put its body aside");

        let Some(elem) = elem else {
            let diagnostic = Diagnostic::error(
                codes::GENERATOR_YIELDS_NOTHING,
                "this loop stands for a value, and yields none",
                span,
                "no `yield` in it",
            )
            .with_note(
                "a loop where a value stands is a generator, whose values are what it yields",
            )
            .with_help("hand each value over with `yield`, or write the loop as a statement");
            self.report(diagnostic);
            return self.error_expr(span);
        };
        // What it yields was reported already.
        if self.is_poisoned(elem) {
            return self.error_expr(span);
        }
        let Some(option) = self.program.prelude_items.enumeration(KnownEnum::Option) else {
            return self.error_expr(span);
        };
        let elem_list = self.program.types.intern_list(&[elem]);
        let ret = self.intern(TyKind::Enum(option, elem_list));

        // Its `next`: the loop, run on from where it stopped.
        let generics = self.type_params.clone();
        let def = &mut self.program.fns[next];
        def.params = vec![ParamDef {
            name: Symbol::generator(),
            name_span: span,
            ty: self_ty,
            span,
            default: None,
        }];
        def.ret = ret;
        def.generics = generics.clone();
        def.module = self.current as u32;
        def.body = Some(code);

        // Its struct: where it stopped, then a reference to each variable
        // the loop names, as a closure lent for a call holds them.
        let mut fields = vec![FieldDef {
            is_pub: false,
            is_var: true,
            name: Symbol::state(),
            ty: Types::I32,
            span,
            default: None,
        }];
        let mut values = vec![self.alloc(ExprKind::Int(0), Types::I32, span)];
        for capture in &captures {
            fields.push(FieldDef {
                is_pub: false,
                is_var: false,
                name: capture.name,
                ty: capture.ty,
                span: capture.span,
                default: None,
            });
            let (place, local_ty) = self.captured_value(capture);
            // A reference is copied: what it refers to outlives the loop.
            let value = if matches!(self.kind(local_ty), TyKind::Ref(..)) {
                place
            } else {
                self.alloc(ExprKind::Ref(place), capture.ty, capture.span)
            };
            values.push(value);
        }
        let def = &mut self.program.structs[id];
        def.generics = generics.clone();
        def.fields = fields;
        def.is_view = !captures.is_empty();
        def.generator = Some(GeneratorDef {
            next,
            elem,
            of: None,
        });
        def.module = self.current as u32;
        let owner = TypeDef::Struct(id);
        if let Some(implementation) = self.program.impls.iter_mut().find(|i| {
            i.ty == owner
                && Some(i.interface)
                    == self
                        .program
                        .prelude_items
                        .interface(KnownInterface::Iterator)
        }) {
            implementation.args = elem_list;
            implementation.conditions = generics;
        }
        let order: Vec<u32> = (0..values.len() as u32).collect();
        self.alloc(
            ExprKind::Struct {
                id,
                fields: values,
                order,
            },
            gen_ty,
            span,
        )
    }

    /// `yield value` in a generator: `.Some(value)`, handed to whoever
    /// asked. The first sets what the generator yields.
    pub(super) fn generator_yield(&mut self, value: ast::ExprId, span: Span) -> StmtKind {
        if let Some(build) = self.state.generator.as_mut() {
            build.yielded = true;
        }
        let elem = self.state.generator.as_ref().and_then(|build| build.elem);
        let value = match elem {
            Some(t) => self.check(value, t),
            None => {
                let value = self.infer(value, None);
                let ty = self.ty_of(value);
                if let Some(build) = self.state.generator.as_mut() {
                    build.elem = Some(ty);
                }
                value
            }
        };
        let elem = self.ty_of(value);
        let Some(option) = self.program.prelude_items.enumeration(KnownEnum::Option) else {
            return StmtKind::Expr(self.wrap_discarded(value));
        };
        let (some, _) = self.option_variants();
        let list = self.program.types.intern_list(&[elem]);
        let ty = self.intern(TyKind::Enum(option, list));
        let wrapped = self.alloc(
            ExprKind::Variant {
                id: option,
                variant: some,
                args: vec![value],
                order: Vec::new(),
            },
            ty,
            span,
        );
        StmtKind::Yield(wrapped)
    }

    /// `return` in a loop that is a generator, which has no function of
    /// its own to leave.
    pub(super) fn return_in_generator(&mut self, span: Span) -> bool {
        let Some(loop_span) = self.state.generator.as_ref().and_then(|b| b.loop_span) else {
            return false;
        };
        let diagnostic = Diagnostic::error(
            codes::RETURN_IN_GENERATOR,
            "cannot `return` from a loop that is a generator",
            span,
            "the loop runs as its values are asked for",
        )
        .with_secondary(loop_span, "a generator")
        .with_note("the loop runs later, from whoever asks for its values, so there is no function around it to leave; `break` ends it")
        .with_help("end the loop with `break`");
        self.report(diagnostic);
        true
    }
}

impl<'a> Lowerer<'a> {
    /// What a result written `Iterator<T>` yields: `T`, where it names the
    /// prelude's `Iterator`.
    pub(super) fn generator_elem(&mut self, t: ast::TypeId) -> Option<Ty> {
        let ast = self.ast;
        let ast::TypeKind::Named { name, ref args } = ast.types[t].kind else {
            return None;
        };
        let [arg] = args[..] else {
            return None;
        };
        let span = ast.types[t].span;
        let interface = self.interface_named(ast::Name {
            sym: name,
            span: Span::new(span.lo, span.lo + self.text(name).len() as u32),
        })?;
        if Some(interface)
            != self
                .program
                .prelude_items
                .interface(KnownInterface::Iterator)
        {
            return None;
        }
        Some(self.resolve_ty(arg))
    }

    /// The generator a function that yields answers: a struct that holds
    /// what the function was given, its `next`, which is the function's
    /// body run on from where it stopped, and its implementation of
    /// `Iterator`.
    pub(super) fn declare_fn_generator(
        &mut self,
        id: FnId,
        elem: Ty,
        args: crate::TyList,
        receiver: Option<Receiver>,
    ) {
        let span = self.program.fns[id].name_span;
        let generics = self.program.fns[id].generics.clone();
        let params = self.program.fns[id].params.clone();
        // What it holds between calls may not be changed by anyone else;
        // a `&var` would be.
        for param in &params {
            if let TyKind::Ref(_, crate::RefKind::Var) = self.kind(param.ty) {
                let name = self.text(param.name).to_string();
                let what = if receiver == Some(Receiver::Var)
                    && param.name == self.interner.self_symbol()
                {
                    "a `var fn` that yields".to_string()
                } else {
                    format!("`{name}`, a `&var`")
                };
                let diagnostic = Diagnostic::error(
                    codes::GENERATOR_HOLDS_VAR,
                    format!("a function that yields cannot take {what}"),
                    param.span,
                    "the generator would hold it between calls",
                )
                .with_note("a function that yields answers a generator, which keeps what it was given until its values are asked for; it may read what it borrows, but not change it")
                .with_help("take it as `&`, or collect the values with `own for`, which runs while the call does");
                self.report(diagnostic);
            }
        }
        let gen_id = self.declare_generator(span);
        let next = self.program.structs[gen_id]
            .generator
            .expect("a generator")
            .next;
        let gen_ty = self.intern(TyKind::Struct(gen_id, args));
        let this_ty = self.intern(TyKind::Ref(gen_ty, crate::RefKind::Var));
        let elem_list = self.program.types.intern_list(&[elem]);
        let ret = match self.program.prelude_items.enumeration(KnownEnum::Option) {
            Some(option) => self.intern(TyKind::Enum(option, elem_list)),
            None => Types::ERROR,
        };
        // Its struct: where it stopped, then what the function was given.
        let mut fields = vec![FieldDef {
            is_pub: false,
            is_var: true,
            name: Symbol::state(),
            ty: Types::I32,
            span,
            default: None,
        }];
        for param in &params {
            fields.push(FieldDef {
                is_pub: false,
                is_var: false,
                name: param.name,
                ty: param.ty,
                span: param.span,
                default: None,
            });
        }
        let is_view = params.iter().any(|p| self.program.holds_view(p.ty));
        let def = &mut self.program.structs[gen_id];
        def.generics = generics.clone();
        def.fields = fields;
        def.is_view = is_view;
        def.generator = Some(GeneratorDef {
            next,
            elem,
            of: Some(id),
        });
        let def = &mut self.program.fns[next];
        def.params = vec![ParamDef {
            name: Symbol::generator(),
            name_span: span,
            ty: this_ty,
            span,
            default: None,
        }];
        def.ret = ret;
        def.generics = generics.clone();
        self.program.fns[id].ret = gen_ty;
        // It is an `Iterator`, with the interface's other methods.
        if let Some(interface) = self
            .program
            .prelude_items
            .interface(KnownInterface::Iterator)
        {
            let owner = TypeDef::Struct(gen_id);
            let wanted = self.program.interfaces[interface].methods.clone();
            let mut methods = vec![next];
            for want in wanted.iter().skip(1) {
                self.add_default(owner, want.id);
                methods.push(want.id);
            }
            self.program.impls.push(ImplDef {
                interface,
                args: elem_list,
                ty: owner,
                methods,
                conditions: generics,
                module: self.current as u32,
                span,
            });
        }
    }

    /// The generator a function that yields answers, if it is one.
    pub(super) fn generator_of_fn(&self, id: FnId) -> Option<StructId> {
        match self.kind(self.program.fns[id].ret) {
            TyKind::Struct(gen_id, _)
                if self.program.structs[gen_id]
                    .generator
                    .is_some_and(|g| g.of == Some(id)) =>
            {
                Some(gen_id)
            }
            _ => None,
        }
    }

    /// The body of a function that yields. Checked as written, it is the
    /// body of the generator's `next`, whose parameters are the generator
    /// and then the function's own, which the MIR takes out of the
    /// generator when it starts. The function's own body makes the
    /// generator from what it was given.
    pub(super) fn check_generator_fn(&mut self, body: ast::ExprId, id: FnId, gen_id: StructId) {
        let generator = self.program.structs[gen_id].generator.expect("a generator");
        let next = generator.next;
        let gen_ty = self.program.fns[id].ret;
        let this_ty = self.program.fns[next].params[0].ty;
        let params = self.program.fns[id].params.clone();
        let span = self.program.fns[id].name_span;

        // `next`: the body as written, which yields.
        self.state.body = Body::default();
        self.state.scopes = vec![FxHashMap::default()];
        self.state.ret = Types::UNIT;
        self.state.generator = Some(GeneratorBuild {
            elem: Some(generator.elem),
            loop_span: None,
            yielded: false,
        });
        let this = self.declare(Symbol::generator(), this_ty, LocalKind::Param, span);
        self.state.body.params.push(this);
        for p in &params {
            let local = self.declare(p.name, p.ty, LocalKind::Param, p.name_span);
            self.state.body.params.push(local);
        }
        let ast = self.ast;
        let whole = ast.exprs[body].span;
        // A loop written as the body is the function's own, and its
        // `yield`s are the function's.
        let value = if let ast::ExprKind::ForElement {
            binding,
            source,
            body: loop_body,
        } = &ast.exprs[body].kind
        {
            let kind = self.for_stmt(None, binding, *source, loop_body);
            let stmt = self.state.body.stmts.alloc(Stmt { kind, span: whole });
            let block = Block {
                stmts: vec![stmt],
                value: None,
                span: whole,
            };
            self.alloc(ExprKind::Block(block), Types::UNIT, whole)
        } else {
            let value = self.infer(body, Some(Types::UNIT));
            self.wrap_discarded(value)
        };
        self.state.body.value = Some(value);
        let yielded = self.state.generator.take().is_some_and(|b| b.yielded);
        self.settle_bindings();
        let code = std::mem::take(&mut self.state.body);
        if !yielded {
            let def = &self.program.fns[id];
            let name = self.text(def.name).to_string();
            let mut diagnostic = Diagnostic::error(
                codes::GENERATOR_YIELDS_NOTHING,
                format!("`{name}` answers an `Iterator`, and yields nothing"),
                def.name_span,
                "no `yield` in its body",
            )
            .with_note("a function that answers `Iterator<T>` is a generator, whose values are what its body yields");
            if let Some(ret_span) = def.ret_span {
                diagnostic = diagnostic.with_secondary(ret_span, "a generator");
            }
            self.report(diagnostic.with_help("hand each value over with `yield`"));
        }
        self.program.fns[next].body = Some(code);

        // The function itself: the generator, holding what it was given.
        self.state.body = Body::default();
        self.state.scopes = vec![FxHashMap::default()];
        self.state.ret = gen_ty;
        let mut values = vec![self.alloc(ExprKind::Int(0), Types::I32, span)];
        for p in &params {
            let local = self.declare(p.name, p.ty, LocalKind::Param, p.name_span);
            self.state.body.params.push(local);
            let read = self.alloc(ExprKind::Local(local), p.ty, p.name_span);
            let value = if self.owns(p.ty) {
                self.alloc(ExprKind::Move(read), p.ty, p.name_span)
            } else {
                read
            };
            values.push(value);
        }
        let order: Vec<u32> = (0..values.len() as u32).collect();
        let made = self.alloc(
            ExprKind::Struct {
                id: gen_id,
                fields: values,
                order,
            },
            gen_ty,
            whole,
        );
        self.state.body.value = Some(made);
        self.settle_bindings();
        self.program.fns[id].body = Some(std::mem::take(&mut self.state.body));
    }
}

impl Lowerer<'_> {
    /// The generators a value of `ty` holds in itself, rather than behind
    /// an `own` or a reference: a generator holds what its `next` keeps
    /// between calls, so one of these in its `next` is part of it.
    /// The fields' types are worked out here, since not every instance's
    /// are interned yet.
    fn generators_held(&mut self, ty: Ty, out: &mut Vec<StructId>, seen: &mut FxHashSet<Ty>) {
        if !seen.insert(ty) {
            return;
        }
        match self.kind(ty) {
            TyKind::Struct(id, _) if self.program.structs[id].generator.is_some() => {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            TyKind::Struct(id, args) => {
                let args = self.program.types.list(args).to_vec();
                let fields: Vec<Ty> = self.program.structs[id]
                    .fields
                    .iter()
                    .map(|f| f.ty)
                    .collect();
                for field in fields {
                    let field = self.program.types.subst(field, &args);
                    self.generators_held(field, out, seen);
                }
            }
            TyKind::Enum(id, args) => {
                let args = self.program.types.list(args).to_vec();
                let fields: Vec<Ty> = self.program.enums[id]
                    .variants
                    .iter()
                    .flat_map(|v| v.fields.iter().map(|f| f.ty))
                    .collect();
                for field in fields {
                    let field = self.program.types.subst(field, &args);
                    self.generators_held(field, out, seen);
                }
            }
            TyKind::Array(elem, _) => self.generators_held(elem, out, seen),
            _ => {}
        }
    }

    /// A generator whose `next` keeps a generator that keeps it, however
    /// many steps away, would be endlessly large: the one that walks a
    /// tree, walking itself for each child. The inner one goes on the heap.
    pub(super) fn check_generator_cycles(&mut self) {
        let generators: Vec<(StructId, FnId)> = self
            .program
            .structs
            .iter()
            .filter_map(|(id, def)| Some((id, def.generator?.next)))
            .filter(|&(_, next)| self.program.fns[next].body.is_some())
            .collect();
        // What each one holds, and the first expression of its `next` that
        // makes each.
        let mut holds: FxHashMap<StructId, Vec<(StructId, ExprId)>> = FxHashMap::default();
        for &(id, next) in &generators {
            let body = self.program.fns[next].body.clone().expect("filtered above");
            let mut edges: Vec<(StructId, ExprId)> = Vec::new();
            // What `own` puts on the heap is made there, and never kept
            // in the generator.
            let on_heap: FxHashSet<ExprId> = body
                .exprs
                .iter()
                .filter_map(|(_, expr)| match expr.kind {
                    ExprKind::Own(inner) => Some(inner),
                    _ => None,
                })
                .collect();
            // A place reads what is there already, which a local of the
            // body holds, and makes nothing.
            let mut found = Vec::new();
            for (_, local) in body.locals.iter() {
                self.generators_held(local.ty, &mut found, &mut FxHashSet::default());
            }
            let mut seen_locals = found;
            for (expr_id, expr) in body.exprs.iter() {
                if on_heap.contains(&expr_id)
                    || matches!(
                        expr.kind,
                        ExprKind::Deref(_)
                            | ExprKind::Field { .. }
                            | ExprKind::Index { .. }
                            | ExprKind::Local(_)
                    )
                {
                    continue;
                }
                let mut found = Vec::new();
                self.generators_held(expr.ty, &mut found, &mut FxHashSet::default());
                for held in found {
                    if !edges.iter().any(|&(h, _)| h == held) {
                        edges.push((held, expr_id));
                    }
                }
            }
            // A local that holds one it got from a place is held too; the
            // expression that made it is where it is reported.
            seen_locals.retain(|held| !edges.iter().any(|&(h, _)| h == *held));
            if let Some(&held) = seen_locals.first()
                && let Some(value) = body.value
            {
                edges.push((held, value));
            }
            holds.insert(id, edges);
        }
        for &(id, next) in &generators {
            // The generators this one reaches, and whether it is among them.
            let mut reached: FxHashSet<StructId> = FxHashSet::default();
            let mut work: Vec<StructId> = holds[&id].iter().map(|&(h, _)| h).collect();
            while let Some(g) = work.pop() {
                if reached.insert(g) {
                    work.extend(holds.get(&g).into_iter().flatten().map(|&(h, _)| h));
                }
            }
            if !reached.contains(&id) {
                continue;
            }
            // Where it holds the first one on the way back to itself.
            let Some(&(_, at)) = holds[&id].iter().find(|&&(h, _)| {
                h == id || {
                    let mut seen = FxHashSet::default();
                    let mut work = vec![h];
                    let mut back = false;
                    while let Some(g) = work.pop() {
                        if g == id {
                            back = true;
                            break;
                        }
                        if seen.insert(g) {
                            work.extend(holds.get(&g).into_iter().flatten().map(|&(h, _)| h));
                        }
                    }
                    back
                }
            }) else {
                continue;
            };
            let body = self.program.fns[next]
                .body
                .as_ref()
                .expect("filtered above");
            let span = body.exprs[at].span;
            let name = match self.program.structs[id].generator.and_then(|g| g.of) {
                Some(of) => format!("`{}`", self.text(self.program.fns[of].name)),
                None => "this loop".to_string(),
            };
            let diagnostic = Diagnostic::error(
                codes::GENERATOR_HOLDS_ITSELF,
                format!("{name} would hold itself, and be endlessly large"),
                span,
                "a generator held here, by value, that holds this one",
            )
            .with_note("a generator keeps what it walks inside itself, so one that walks itself holds itself; on the heap, it holds a pointer")
            .with_fix("put the inner one on the heap", [Edit::insert(span.lo, "own ")]);
            self.report(diagnostic);
        }
    }
}
