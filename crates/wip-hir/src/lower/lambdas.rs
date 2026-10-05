//! Lambdas: `(x) => x * 2`, and the closures they make.
//!
//! A lambda that captures nothing is a plain function value, so its body
//! becomes a function of its own and the expression is that function's
//! address. A lambda that captures is a closure: a pair of what it captured
//! and that address. A closure lent for one call, `&(…) => R` or
//! `&var (…) => R`, keeps references to the variables it names in the frame
//! of the function that wrote it; an owned closure, `own<(…) => R>`, holds
//! the values themselves on the heap, and carries its own drop function so
//! that it can be dropped without knowing which lambda made it.

use super::*;

/// How a closure holds what it captured.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureKind {
    /// Lent for one call: references to the creator's variables, `&` to read
    /// them and `&var` to write them as well.
    Lent(crate::RefKind),
    /// Owned: the values themselves, copied or moved in when it is created.
    Owned,
    /// A generator's: references to read them through, after where it
    /// stopped.
    Generator,
}

impl CaptureKind {
    /// The first field of the environment a capture goes in. An owned
    /// closure's environment holds its drop function before them.
    fn first_field(self) -> u32 {
        match self {
            CaptureKind::Lent(_) => 0,
            CaptureKind::Owned | CaptureKind::Generator => 1,
        }
    }
}

/// A variable of the function around a lambda that the lambda uses: where
/// it is, and the field of the environment that holds a reference to it.
pub(super) struct Capture {
    pub from: Captured,
    pub name: Symbol,
    /// The type of the field: a reference to the variable, or the reference
    /// it already is.
    pub ty: Ty,
    pub span: Span,
    /// Whether the closure writes it: a `&var` closure keeps a `&var` to
    /// what it writes, and a `&` to what it only reads.
    pub written: bool,
}

/// Where a captured variable is, in the body that makes the closure.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Captured {
    /// One of its own variables.
    Local(LocalId),
    /// One it captured itself, from the body around it: the field of its
    /// environment that holds the reference, which is copied.
    Field(u32),
}

impl<'a> Lowerer<'a> {
    /// What a closure or a generator captured, read where it is made: the
    /// variable, or the reference the body holds to it.
    pub(super) fn captured_value(&mut self, capture: &Capture) -> (ExprId, Ty) {
        match capture.from {
            Captured::Local(local) => {
                let ty = self.state.body.locals[local].ty;
                (self.alloc(ExprKind::Local(local), ty, capture.span), ty)
            }
            Captured::Field(index) => {
                let env_local = self.state.body.params[0];
                let env_ref_ty = self.state.body.locals[env_local].ty;
                let TyKind::Ref(env_ty, _) = self.kind(env_ref_ty) else {
                    unreachable!("what captured it holds it behind its first parameter")
                };
                let env = self.alloc(ExprKind::Local(env_local), env_ref_ty, capture.span);
                let env = self.alloc(ExprKind::Deref(env), env_ty, capture.span);
                let field = self.alloc(
                    ExprKind::Field { base: env, index },
                    capture.ty,
                    capture.span,
                );
                (field, capture.ty)
            }
        }
    }

    /// A function for every lambda of a file, before any body is checked:
    /// bodies are checked on threads of their own, each with its own copy of
    /// the declarations, so nothing may be declared while they run.
    pub(super) fn collect_lambdas(
        &mut self,
    ) -> (FxHashMap<ast::ExprId, FnId>, FxHashMap<FnId, StructId>) {
        let ast = self.ast;
        let mut lambdas = FxHashMap::default();
        let mut envs = FxHashMap::default();
        for (expr_id, expr) in ast.exprs.iter() {
            if !matches!(expr.kind, ast::ExprKind::Lambda { .. }) {
                continue;
            }
            let def = FnDef {
                name: self.interner.lambda_symbol(),
                name_span: expr.span,
                receiver: None,
                owner: None,
                interface: None,
                generics: Vec::new(),
                instance_of: None,
                params: Vec::new(),
                ret: Types::UNIT,
                ret_span: None,
                is_extern: false,
                exports_c: false,
                header: None,
                symbol: None,
                accesses: None,
                is_variadic: false,
                variadic_of: None,
                is_lambda: true,
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
                span: expr.span,
            };
            let id = self.program.fns.alloc(def);
            // Where the lambda turns out to be a closure, this holds what it
            // captured.
            let env = self.program.structs.alloc(StructDef {
                is_extern: false,
                is_union: false,
                is_opaque: false,
                is_intrinsic: false,
                header: None,
                accessors: Vec::new(),
                name: self.interner.lambda_symbol(),
                generics: Vec::new(),
                fields: Vec::new(),
                methods: Vec::new(),
                is_tuple: false,
                generator: None,
                is_env: true,
                is_view: false,
                module: self.current as u32,
                is_pub: false,
                span: expr.span,
            });
            lambdas.insert(expr_id, id);
            envs.insert(id, env);
        }
        (lambdas, envs)
    }

    pub(super) fn lambda(
        &mut self,
        expr_id: ast::ExprId,
        hint: Option<Ty>,
        owned: bool,
        span: Span,
    ) -> ExprId {
        let ast::ExprKind::Lambda {
            ref params,
            ret,
            body,
        } = self.ast.exprs[expr_id].kind
        else {
            unreachable!("a lambda is lowered from a lambda")
        };
        // What kind of closure is expected: a reference to a function type is
        // one lent for one call, and `own` before the lambda makes an owned
        // one.
        let expected_fn = hint.and_then(|ty| match self.kind(ty) {
            TyKind::Ref(inner, kind) if matches!(self.kind(inner), TyKind::Fn(..)) => {
                Some((inner, Some(CaptureKind::Lent(kind))))
            }
            TyKind::Own(inner) if matches!(self.kind(inner), TyKind::Fn(..)) => {
                Some((inner, Some(CaptureKind::Owned)))
            }
            TyKind::Fn(..) => Some((ty, None)),
            _ => None,
        });
        // An owned closure is written with `own`, as every other allocation
        // is; the type alone does not make one.
        if owned {
            if !matches!(expected_fn, None | Some((_, Some(CaptureKind::Owned)))) {
                let diagnostic = Diagnostic::error(
                    codes::MISMATCHED_TYPES,
                    format!(
                        "an owned closure is written here, and {} is expected",
                        self.ty_name(hint.expect("an expected type")),
                    ),
                    span,
                    "an owned closure",
                )
                .with_note("`own (x) => …` allocates an environment that holds what it captured")
                .with_help("leave out `own`, so the lambda is lent for the call");
                self.report(diagnostic);
                return self.error_expr(span);
            }
        } else if let Some((_, Some(CaptureKind::Owned))) = expected_fn {
            let diagnostic = Diagnostic::error(
                codes::MISMATCHED_TYPES,
                format!(
                    "expected {}, found a lambda",
                    self.ty_name(hint.expect("an expected type")),
                ),
                span,
                "a lambda that is not owned",
            )
            .with_note(
                "an owned closure holds what it captured on the heap, so it is written with `own`, as every other allocation is",
            )
            .with_fix("write `own` before it", [Edit::insert(span.lo, "own ")]);
            self.report(diagnostic);
            return self.error_expr(span);
        }
        // With `own` written, it is an owned closure whether or not a type is
        // expected: its parameters' types are then written, as any lambda's
        // are where nothing expects them.
        // A lambda that is a `val`'s whole value may keep what it captures
        // by `&`, as a view keeps what it borrows; one that captures
        // nothing is a plain function value after all.
        let kept =
            std::mem::take(&mut self.state.keeping_closure) && !owned && expected_fn.is_none();
        let closure = match (owned, expected_fn) {
            (true, _) => Some(CaptureKind::Owned),
            (false, Some((_, kind))) => kind,
            (false, None) if kept => Some(CaptureKind::Lent(crate::RefKind::Shared)),
            (false, None) => None,
        };
        // The types the expected function type gives, where it is one.
        let expected = expected_fn
            .map(|(inner, _)| inner)
            .and_then(|ty| match self.kind(ty) {
                TyKind::Fn(params, ret) => Some((self.program.types.list(params).to_vec(), ret)),
                _ => None,
            });
        if let Some((expected_params, _)) = &expected
            && expected_params.len() != params.len()
        {
            let diagnostic = Diagnostic::error(
                codes::LAMBDA_PARAMS,
                format!(
                    "this lambda takes {}, and {} is expected here",
                    plural(params.len(), "parameter", "parameters"),
                    self.ty_name(hint.expect("an expected type")),
                ),
                span,
                "a different number of parameters",
            );
            self.report(diagnostic);
            return self.error_expr(span);
        }
        // Each parameter's type: written, or from the expected type.
        let mut defs = Vec::new();
        let mut seen: FxHashMap<Symbol, Span> = FxHashMap::default();
        for (index, param) in params.iter().enumerate() {
            match seen.get(&param.name.sym) {
                Some(&first) => self.duplicate(param.name, first),
                None => {
                    seen.insert(param.name.sym, param.name.span);
                }
            }
            let ty = match (param.ty, expected.as_ref()) {
                (Some(written), _) => {
                    let ty = self.resolve_ty(written);
                    self.param_ty(ty, written)
                }
                (None, Some((expected, _))) if !self.has_error(expected[index]) => expected[index],
                (None, _) => {
                    let name = self.text(param.name.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::LAMBDA_PARAMS,
                        format!("the type of `{name}` is not known here"),
                        param.name.span,
                        "no type for this parameter",
                    )
                    .with_note(
                        "a lambda takes its parameters' types from the function type expected where it is written",
                    )
                    .with_help(format!("write the type: `{name}: i64`"));
                    self.report(diagnostic);
                    Types::ERROR
                }
            };
            defs.push(ParamDef {
                name: param.name.sym,
                name_span: param.name.span,
                ty,
                span: param.span,
                default: None,
            });
        }
        // The result: written, or from the expected type, or the body's.
        let written_ret = ret.map(|t| {
            let ty = self.resolve_ty(t);
            (ty, self.ast.types[t].span)
        });
        // A result that is not known yet comes from the body instead.
        let expected_ret = written_ret
            .map(|(ty, _)| ty)
            .or(expected.map(|(_, r)| r))
            .filter(|&ty| !self.has_error(ty));
        let id = self.lambdas[&expr_id];
        // A closure's code takes what it captured before the arguments.
        let mut all_params = Vec::new();
        if let Some(kind) = closure {
            let env_id = self.lambda_envs[&id];
            // In a generic function, what it captured has the function's
            // type parameters in its types, and so has the environment.
            let list = self.own_type_args();
            self.program.structs[env_id].generics = self.type_params.clone();
            let env_ty = self.intern(TyKind::Struct(env_id, list));
            // What a lent closure captured it reaches through references of
            // its own, so writing one goes through the field; what an owned
            // closure captured is the environment itself, which it only
            // reads, so the environment is lent to the code to read.
            let env_kind = match kind {
                CaptureKind::Lent(_) => crate::RefKind::Var,
                CaptureKind::Owned | CaptureKind::Generator => crate::RefKind::Shared,
            };
            let env_ref = self.intern(TyKind::Ref(env_ty, env_kind));
            all_params.push(ParamDef {
                name: self.interner.lambda_symbol(),
                name_span: span,
                ty: env_ref,
                span,
                default: None,
            });
        }
        all_params.extend(defs.iter().cloned());
        let def = &mut self.program.fns[id];
        // A lambda inside a generic function is generic in the same way: its
        // instance comes with the enclosing one.
        def.generics = self.type_params.clone();
        def.params = all_params;
        def.ret = expected_ret.unwrap_or(Types::UNIT);
        def.ret_span = written_ret.map(|(_, span)| span);
        def.module = self.current as u32;
        let captures = self.check_lambda_body(id, body, expected_ret, closure);
        // The lambda's type takes the parameters it is called with, without
        // what it captured.
        let param_tys: Vec<Ty> = defs.iter().map(|p| p.ty).collect();
        let list = self.program.types.intern_list(&param_tys);
        let value_ty = self.intern(TyKind::Fn(list, self.program.fns[id].ret));
        // A kept lambda that captured nothing takes no environment: it is a
        // plain function value, as it was before it could be kept.
        let closure = if kept && captures.is_empty() {
            self.program.fns[id].params.remove(0);
            if let Some(body) = &mut self.program.fns[id].body {
                body.params.remove(0);
            }
            None
        } else {
            closure
        };
        // A closure is a pair: what it captured, and its code. A lambda that
        // captures nothing is a plain function value.
        match closure {
            Some(kind) => {
                let env = self.build_env(id, captures, kind, span);
                let ty = match kind {
                    CaptureKind::Lent(k) => self.intern(TyKind::Ref(value_ty, k)),
                    CaptureKind::Owned => self.intern(TyKind::Own(value_ty)),
                    CaptureKind::Generator => unreachable!("a lambda is not a generator"),
                };
                self.alloc(ExprKind::Closure { id, env }, ty, span)
            }
            None => {
                let type_args = self.own_type_args();
                self.alloc(ExprKind::FnRef { id, type_args }, value_ty, span)
            }
        }
    }

    /// Checks a lambda's body as a function of its own, whose parameters
    /// are those of its definition `id`, and returns what it captured. Where
    /// no result is expected, the body's type is the result.
    fn check_lambda_body(
        &mut self,
        id: FnId,
        body: ast::ExprId,
        expected_ret: Option<Ty>,
        captures_allowed: Option<CaptureKind>,
    ) -> Vec<Capture> {
        // The enclosing body waits, whole, while this one is checked: a name
        // this one does not declare is looked for in it, and is a capture.
        self.outer.push(std::mem::take(&mut self.state));
        self.state.scopes = vec![FxHashMap::default()];
        self.state.ret = expected_ret.unwrap_or(Types::UNIT);
        self.state.capture_kind = captures_allowed;
        for p in self.program.fns[id].params.clone() {
            let local = self.declare(p.name, p.ty, LocalKind::Param, p.name_span);
            self.state.body.params.push(local);
        }
        // Without an expected result, the body's own type is the lambda's.
        let value = match expected_ret {
            Some(_) if self.state.ret == Types::UNIT => self.unit_body(body, id),
            Some(_) => self.value_body(body, id),
            None => {
                self.state.inferring_ret = true;
                let value = self.infer(body, None);
                let ty = self.ty_of(value);
                let value = if self.state.inferring_ret {
                    // No `return` said: the body's own type is the result.
                    self.state.inferring_ret = false;
                    self.state.ret = if ty == Types::NEVER { Types::UNIT } else { ty };
                    value
                } else if ty == Types::NEVER {
                    value
                } else {
                    // A `return` said, and the body's own value must agree.
                    self.coerce(value, self.state.ret, None)
                };
                self.program.fns[id].ret = self.state.ret;
                value
            }
        };
        self.state.body.value = Some(value);
        self.settle_bindings();
        self.program.fns[id].body = Some(std::mem::take(&mut self.state.body));
        let captures = std::mem::take(&mut self.state.captures);
        self.state = self.outer.pop().expect("a lambda put its body aside");
        captures
    }

    /// What a closure carries, and the expression that builds it where the
    /// lambda is written: references to the creator's variables for a closure
    /// lent for one call, the values themselves on the heap for an owned one.
    fn build_env(
        &mut self,
        id: FnId,
        captures: Vec<Capture>,
        kind: CaptureKind,
        span: Span,
    ) -> ExprId {
        let env_id = self.lambda_envs[&id];
        let list = self.own_type_args();
        let env_ty = self.intern(TyKind::Struct(env_id, list));
        let mut fields = Vec::new();
        let mut values = Vec::new();
        // An owned closure is dropped where its type says nothing about which
        // lambda made it, so its environment carries its own drop function.
        if kind == CaptureKind::Owned {
            let address = self.program.types.intern_list(&[Types::PTR_U8]);
            let drop_ty = self.intern(TyKind::Fn(address, Types::UNIT));
            fields.push(FieldDef {
                is_pub: false,
                is_var: false,
                name: self.interner.lambda_symbol(),
                ty: drop_ty,
                span,
                default: None,
            });
            values.push(self.alloc(ExprKind::DropRef(env_ty), drop_ty, span));
        }
        // A closure lent for writing writes what it writes, and reads the
        // rest: what it only reads it holds by `&`, so that another
        // closure lent to the same call may read it too, as two arguments
        // of one call may.
        let mut captures = captures;
        let mut reads: Vec<u32> = Vec::new();
        if kind == CaptureKind::Lent(crate::RefKind::Var) {
            for (index, capture) in captures.iter_mut().enumerate() {
                if let TyKind::Ref(inner, crate::RefKind::Var) = self.kind(capture.ty)
                    && !capture.written
                    && !matches!(self.kind(inner), TyKind::Fn(..))
                {
                    capture.ty = self.intern(TyKind::Ref(inner, crate::RefKind::Shared));
                    reads.push(index as u32);
                }
            }
            self.read_captures(id, &captures, &reads);
        }
        for (index, capture) in captures.iter().enumerate() {
            let read = reads.contains(&(index as u32));
            fields.push(FieldDef {
                is_pub: false,
                is_var: false,
                name: capture.name,
                ty: capture.ty,
                span: capture.span,
                default: None,
            });
            let (place, local_ty) = self.captured_value(capture);
            let value = match kind {
                // A value that owns memory moves into an owned closure, as it
                // would into any other value; plain data is copied.
                CaptureKind::Owned => {
                    // An owned closure may be kept anywhere, so it would
                    // outlive what the `str` borrows.
                    self.stored_view_at(local_ty, capture.span, "an `own` closure", None);
                    if self.owns(local_ty) {
                        self.alloc(ExprKind::Move(place), local_ty, capture.span)
                    } else {
                        place
                    }
                }
                // A reference is copied: what it refers to outlives the call.
                // A `&var` one that is only read is lent on as a `&`.
                CaptureKind::Lent(_) if matches!(self.kind(local_ty), TyKind::Ref(..)) => {
                    if read {
                        let TyKind::Ref(inner, _) = self.kind(local_ty) else {
                            unreachable!("matched as a reference")
                        };
                        let target = self.alloc(ExprKind::Deref(place), inner, capture.span);
                        self.alloc(ExprKind::Ref(target), capture.ty, capture.span)
                    } else {
                        place
                    }
                }
                CaptureKind::Lent(k) => {
                    if k == crate::RefKind::Var
                        && !read
                        && let Some(diagnostic) = self.not_writable(place, Writing::Borrow)
                    {
                        self.report(diagnostic);
                    }
                    self.alloc(ExprKind::Ref(place), capture.ty, capture.span)
                }
                CaptureKind::Generator => unreachable!("a generator builds its own struct"),
            };
            values.push(value);
        }
        self.program.structs[env_id].fields = fields;
        self.program.structs[env_id].module = self.current as u32;
        let order: Vec<u32> = (0..values.len() as u32).collect();
        let env = self.alloc(
            ExprKind::Struct {
                id: env_id,
                fields: values,
                order,
            },
            env_ty,
            span,
        );
        match kind {
            CaptureKind::Lent(_) => env,
            // The environment of an owned closure is on the heap: the closure
            // outlives the call that made it.
            CaptureKind::Owned => {
                let ty = self.intern(TyKind::Own(env_ty));
                self.alloc(ExprKind::Own(env), ty, span)
            }
            CaptureKind::Generator => unreachable!("a generator builds its own struct"),
        }
    }

    /// The fields of a closure's environment that its body only reads, as it
    /// reads them: the body was checked with each as the `&var` it might
    /// have been, and now names the `&` it is.
    fn read_captures(&mut self, id: FnId, captures: &[Capture], reads: &[u32]) {
        if reads.is_empty() {
            return;
        }
        let Some(body) = self.program.fns[id].body.as_mut() else {
            return;
        };
        let Some(&env) = body.params.first() else {
            return;
        };
        let ids: Vec<ExprId> = body.exprs.iter().map(|(expr, _)| expr).collect();
        for expr in ids {
            let ExprKind::Field { base, index } = body.exprs[expr].kind else {
                continue;
            };
            let ExprKind::Deref(inner) = body.exprs[base].kind else {
                continue;
            };
            if matches!(body.exprs[inner].kind, ExprKind::Local(local) if local == env)
                && reads.contains(&index)
            {
                body.exprs[expr].ty = captures[index as usize].ty;
            }
        }
    }

    /// Notes that the body writes what it captured under `place`, if it
    /// captured it: a `&var` closure keeps a `&var` to what it writes. A
    /// closure written inside one that writes what the one around it
    /// captured makes that one write it too.
    pub(super) fn note_capture_written(&mut self, place: ExprId) {
        let Some(mut index) = self.captured_under(place) else {
            return;
        };
        let mut level = self.outer.len();
        let mut from = {
            let capture = &mut self.state.captures[index];
            capture.written = true;
            capture.from
        };
        while let Captured::Field(field) = from {
            if level == 0 {
                return;
            }
            level -= 1;
            let around = &mut self.outer[level];
            if !matches!(around.capture_kind, Some(CaptureKind::Lent(_))) {
                return;
            }
            index = field as usize;
            let Some(capture) = around.captures.get_mut(index) else {
                return;
            };
            capture.written = true;
            from = capture.from;
        }
    }

    /// Which of the closure's captures `place` is, or is part of.
    fn captured_under(&self, place: ExprId) -> Option<usize> {
        if !matches!(self.state.capture_kind, Some(CaptureKind::Lent(_))) {
            return None;
        }
        let env = *self.state.body.params.first()?;
        let exprs = &self.state.body.exprs;
        let mut at = place;
        loop {
            match exprs[at].kind {
                ExprKind::Field { base, index } => {
                    if let ExprKind::Deref(inner) = exprs[base].kind
                        && matches!(exprs[inner].kind, ExprKind::Local(local) if local == env)
                    {
                        return Some(index as usize);
                    }
                    at = base;
                }
                ExprKind::Index { base, .. } | ExprKind::SubSlice { base, .. } => at = base,
                ExprKind::Deref(inner) => at = inner,
                _ => return None,
            }
        }
    }

    /// A named function where a closure lent for a call is expected: a
    /// closure of its own, whose code takes an environment that holds
    /// nothing and calls the function with what it is given. A plain
    /// function converts to a lent closure because it captures nothing.
    ///
    /// Bodies are checked on threads of their own, which cannot add a
    /// function to the program, so this only marks the closure: its code is
    /// the named function itself, until [`Self::make_fn_closures`] writes the
    /// function that takes the environment, once the bodies are in.
    pub(super) fn fn_as_closure(&mut self, value: ExprId, expected: Ty) -> Option<ExprId> {
        let TyKind::Ref(want, _) = self.kind(expected) else {
            return None;
        };
        if !matches!(self.kind(want), TyKind::Fn(..)) {
            return None;
        }
        // A generic function is left to a lambda that calls it: its instance
        // would need the arguments the mark has no room for.
        let ExprKind::FnRef {
            id: target,
            type_args: crate::TyList::EMPTY,
        } = self.state.body.exprs[value].kind
        else {
            return None;
        };
        if self.ty_of(value) != want {
            return None;
        }
        let env_id = self.program.fn_closure_env?;
        let span = self.state.body.exprs[value].span;
        let empty = self.program.types.intern_list(&[]);
        let env_ty = self.intern(TyKind::Struct(env_id, empty));
        let env = self.alloc(
            ExprKind::Struct {
                id: env_id,
                fields: Vec::new(),
                order: Vec::new(),
            },
            env_ty,
            span,
        );
        Some(self.alloc(ExprKind::Closure { id: target, env }, expected, span))
    }

    /// Writes the code of every closure [`Self::fn_as_closure`] marked: the
    /// environment, then the function's parameters, passed on to it.
    pub(super) fn make_fn_closures(&mut self) {
        let Some(env_id) = self.program.fn_closure_env else {
            return;
        };
        let mut marked: Vec<(FnId, ExprId, FnId, Ty, Span)> = Vec::new();
        for (owner, def) in self.program.fns.iter() {
            let Some(body) = &def.body else { continue };
            for (at, expr) in body.exprs.iter() {
                if let ExprKind::Closure { id, .. } = expr.kind
                    && !self.program.fns[id].is_lambda
                {
                    marked.push((owner, at, id, expr.ty, expr.span));
                }
            }
        }
        let empty = self.program.types.intern_list(&[]);
        let env_ty = self.intern(TyKind::Struct(env_id, empty));
        let env_ref = self.intern(TyKind::Ref(env_ty, crate::RefKind::Var));
        for (owner, at, target, ty, span) in marked {
            let TyKind::Ref(want, _) = self.kind(ty) else {
                continue;
            };
            let TyKind::Fn(params, ret) = self.kind(want) else {
                continue;
            };
            let param_tys = self.program.types.list(params).to_vec();
            let target_params = self.program.fns[target].params.clone();
            let mut body = Body::default();
            let mut defs = vec![ParamDef {
                name: self.interner.lambda_symbol(),
                name_span: span,
                ty: env_ref,
                span,
                default: None,
            }];
            let env_local = body.locals.alloc(Local {
                name: self.interner.lambda_symbol(),
                ty: env_ref,
                kind: LocalKind::Param,
                span,
            });
            body.params.push(env_local);
            let mut args = Vec::new();
            for (i, &param_ty) in param_tys.iter().enumerate() {
                let name = target_params
                    .get(i)
                    .map_or_else(|| self.interner.lambda_symbol(), |p| p.name);
                defs.push(ParamDef {
                    name,
                    name_span: span,
                    ty: param_ty,
                    span,
                    default: None,
                });
                let local = body.locals.alloc(Local {
                    name,
                    ty: param_ty,
                    kind: LocalKind::Param,
                    span,
                });
                body.params.push(local);
                let read = body.exprs.alloc(Expr {
                    kind: ExprKind::Local(local),
                    ty: param_ty,
                    span,
                });
                // What owns memory is handed on, not copied.
                let arg = if self.owns(param_ty) {
                    body.exprs.alloc(Expr {
                        kind: ExprKind::Move(read),
                        ty: param_ty,
                        span,
                    })
                } else {
                    read
                };
                args.push(arg);
            }
            let call = body.exprs.alloc(Expr {
                kind: ExprKind::Call {
                    callee: target,
                    args,
                    type_args: crate::TyList::EMPTY,
                    order: Vec::new(),
                },
                ty: ret,
                span,
            });
            body.value = Some(call);
            let module = self.program.fns[owner].module;
            let code = self.program.fns.alloc(FnDef {
                name: self.interner.lambda_symbol(),
                name_span: span,
                receiver: None,
                owner: None,
                interface: None,
                generics: Vec::new(),
                instance_of: None,
                params: defs,
                ret,
                ret_span: None,
                is_extern: false,
                exports_c: false,
                header: None,
                symbol: None,
                accesses: None,
                is_variadic: false,
                variadic_of: None,
                is_lambda: true,
                generator: None,
                is_tailrec: false,
                is_test: false,
                is_inline: false,
                generated: None,
                intrinsic: None,
                body: Some(body),
                projects: None,
                compile_time: false,
                module,
                is_pub: false,
                span,
            });
            if let Some(body) = &mut self.program.fns[owner].body
                && let ExprKind::Closure { id, .. } = &mut body.exprs[at].kind
            {
                *id = code;
            }
        }
    }

    /// Whether a name this body does not declare is one of a body around
    /// it, which a closure or a generator written here would capture.
    pub(super) fn named_around(&self, sym: Symbol) -> bool {
        self.outer
            .iter()
            .any(|state| state.scopes.iter().any(|scope| scope.contains_key(&sym)))
    }

    /// A name the body around the one being checked does not declare, but
    /// captures itself from the body around it, through a reference: the
    /// field that holds that reference, and the reference's type.
    /// A closure that holds values holds no reference to
    /// pass on.
    fn captured_around(&mut self, sym: Symbol, span: Span) -> Option<(Captured, Ty)> {
        if self.outer.len() < 2 {
            return None;
        }
        let around = self.outer.last()?.capture_kind?;
        if around == CaptureKind::Owned {
            return None;
        }
        // Resolved in the body around, which captures it in turn.
        let inner = std::mem::take(&mut self.state);
        self.state = self.outer.pop().expect("checked above");
        let resolved = self.captured_name(sym, span);
        let captured = self
            .state
            .captures
            .iter()
            .position(|c| c.name == sym)
            .map(|i| (i as u32 + around.first_field(), self.state.captures[i].ty));
        let back = std::mem::replace(&mut self.state, inner);
        self.outer.push(back);
        resolved?;
        let (index, ty) = captured?;
        Some((Captured::Field(index), ty))
    }

    /// The type parameters of the function being checked, as arguments for a
    /// lambda declared inside it.
    pub(super) fn own_type_args(&mut self) -> crate::TyList {
        if self.type_params.is_empty() {
            return crate::TyList::EMPTY;
        }
        let generics = self.type_params.clone();
        let tys = self.param_tys(&generics);
        self.program.types.intern_list(&tys)
    }

    /// Whether `sym` is a variable of a body around the one being checked:
    /// the function a lambda is written in, or one further out. Such a
    /// name is a capture, and shadows what the module or the prelude calls
    /// by it, as any variable does.
    pub(super) fn names_around(&self, sym: Symbol) -> bool {
        self.outer
            .iter()
            .any(|state| state.scopes.iter().any(|scope| scope.contains_key(&sym)))
    }

    /// A name of the function around the lambda being checked: a capture. A
    /// closure reads it through what it captured; a plain function may
    /// capture nothing, which is an error.
    pub(super) fn captured_name(&mut self, sym: Symbol, span: Span) -> Option<ExprId> {
        let outer = self.outer.last()?;
        let found = outer
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&sym).copied());
        let (from, from_ty) = match found {
            Some(local) => (Captured::Local(local), outer.body.locals[local].ty),
            // One the body around captured itself from the one around it:
            // a generator in a generator's loop.
            None => self.captured_around(sym, span)?,
        };
        let Some(kind) = self.state.capture_kind else {
            let name = self.text(sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::LAMBDA_CAPTURE,
                format!("a lambda that uses `{name}` from around it is a closure"),
                span,
                "captured here",
            )
            .with_note(
                "a plain function value captures nothing; a closure is `&(…) => R` or `&var (…) => R`, lent for one call, or kept in a `val` it is the whole value of, reading what it captures",
            )
            .with_help("take it as a parameter of the lambda, declare the parameter a closure, or keep the lambda in a `val`");
            self.report(diagnostic);
            return Some(self.error_expr(span));
        };
        // The field of the environment that holds it, made on first use.
        let index = match self.state.captures.iter().position(|c| c.from == from) {
            Some(index) => index,
            None => {
                let local_ty = from_ty;
                let ty = match kind {
                    // An owned closure holds the value itself: it may outlive
                    // anything a reference points to.
                    CaptureKind::Owned => {
                        if matches!(self.kind(local_ty), TyKind::Ref(..)) {
                            let name = self.text(sym).to_string();
                            let diagnostic = Diagnostic::error(
                                codes::LAMBDA_CAPTURE,
                                format!("an owned closure cannot capture `{name}`, a reference"),
                                span,
                                "captured here",
                            )
                            .with_note(
                                "an owned closure outlives the call that made it, and a reference does not",
                            )
                            .with_help(
                                "pass it to the closure as a parameter, or capture what it refers to by value",
                            );
                            self.report(diagnostic);
                            return Some(self.error_expr(span));
                        }
                        local_ty
                    }
                    // A reference is copied; anything else is captured by one.
                    CaptureKind::Lent(_) | CaptureKind::Generator
                        if matches!(self.kind(local_ty), TyKind::Ref(..)) =>
                    {
                        local_ty
                    }
                    CaptureKind::Lent(k) => self.intern(TyKind::Ref(local_ty, k)),
                    // A generator reads what it names.
                    CaptureKind::Generator => {
                        self.intern(TyKind::Ref(local_ty, crate::RefKind::Shared))
                    }
                };
                self.state.captures.push(Capture {
                    from,
                    name: sym,
                    ty,
                    span,
                    written: false,
                });
                self.state.captures.len() - 1
            }
        };
        let field_ty = self.state.captures[index].ty;
        let env_local = self.state.body.params[0];
        let env_ref_ty = self.state.body.locals[env_local].ty;
        let TyKind::Ref(env_ty, _) = self.kind(env_ref_ty) else {
            unreachable!("a closure's first parameter is a reference to what it captured")
        };
        let env = self.alloc(ExprKind::Local(env_local), env_ref_ty, span);
        let env = self.alloc(ExprKind::Deref(env), env_ty, span);
        let field = self.alloc(
            ExprKind::Field {
                base: env,
                index: index as u32 + kind.first_field(),
            },
            field_ty,
            span,
        );
        match kind {
            // What an owned closure captured is in the environment itself.
            CaptureKind::Owned => Some(field),
            // The field is a reference, seen through where it is named, as a
            // reference parameter is.
            CaptureKind::Lent(_) | CaptureKind::Generator => {
                let TyKind::Ref(inner, _) = self.kind(field_ty) else {
                    unreachable!("a capture is held by reference")
                };
                // A closure lent to the body around is a pair, the code and
                // what it captured, and is called as it is, as a closure
                // parameter is.
                if matches!(self.kind(inner), TyKind::Fn(..)) {
                    return Some(field);
                }
                Some(self.alloc(ExprKind::Deref(field), inner, span))
            }
        }
    }
}
