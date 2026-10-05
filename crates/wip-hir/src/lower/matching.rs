//! `match`: arms, patterns and exhaustiveness.

use super::*;

/// Whether anything in the pattern was reported already, so that what it
/// covers is not worth asking about.
fn pattern_has_error(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Error => true,
        Pattern::Any(alternatives) => alternatives.iter().any(pattern_has_error),
        Pattern::Variant { binders, .. } | Pattern::Fields(binders) => {
            binders.iter().any(|b| match b {
                Binder::Nested(nested) => pattern_has_error(nested),
                _ => false,
            })
        }
        _ => false,
    }
}

/// How the bindings of a pattern bind: to the value they name, or as
/// aliases of the place it lies in, which a `match` on a place makes them;
/// and whether plain data is copied out of the place instead, as a `val … else`
/// binds it.
#[derive(Clone, Copy, Default)]
pub(crate) struct Binds {
    /// The reference a binding aliases the place with, where it aliases.
    alias: Option<crate::RefKind>,
    /// The place the bindings alias, which the checks after them follow.
    source: Option<ExprId>,
    /// Plain data is copied out of the place.
    copy: bool,
    /// An alias of plain data that nothing writes through becomes a copy
    /// once the body is lowered: the bindings of a `match`
    /// and an `is`.
    settle: bool,
    /// The bindings are variables of their own, which hold their parts and
    /// can be written: a `var` pattern's.
    variables: bool,
}

/// What a pattern writes of a struct's or a variant's fields: the binders,
/// where it lists them, and whether `..` passes over the rest.
#[derive(Clone, Copy)]
pub(super) struct Fields<'a> {
    binders: Option<&'a [ast::Binder]>,
    rest: bool,
}

impl<'a> Lowerer<'a> {
    pub(super) fn match_expr(
        &mut self,
        scrutinee: ast::ExprId,
        arms: &'a [ast::Arm],
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        let ast = self.ast;
        // A tuple written here is matched where its elements are;
        // anything else is one value.
        let (scrut, places) = match self.tuple_written(scrutinee) {
            Some(elements) => self.places(elements, ast.exprs[scrutinee].span),
            None => (self.operand(scrutinee, None), None),
        };
        let scrut_ty = self.ty_of(scrut);
        let alias = match &places {
            // A guard may not take from what an arm binds where any element
            // is a value whose bindings take it.
            Some(binds) => binds
                .iter()
                .map(|b| b.alias)
                .try_fold(crate::RefKind::Var, |kind, alias| {
                    alias.map(|a| if a == crate::RefKind::Shared { a } else { kind })
                }),
            None => self.alias_kind(scrut),
        };
        let mut result = hint;
        let mut hir_arms = Vec::new();
        for arm in arms {
            self.state.scopes.push(FxHashMap::default());
            let binds = Binds {
                alias,
                source: Some(scrut),
                copy: false,
                settle: true,
                variables: false,
            };
            let pattern = match &places {
                Some(element_binds) => {
                    self.places_pattern(&arm.pattern, scrut_ty, element_binds.clone(), binds)
                }
                None => self.pattern(&arm.pattern, scrut_ty, binds),
            };
            // `if …` after the pattern: the bindings are in scope for it,
            // and it decides whether the arm matches.
            if let Some(written) = arm.guard {
                self.guard_takes_nothing(&pattern, alias, arm.pattern.span, written);
            }
            let guard = arm.guard.map(|guard| self.check(guard, Types::BOOL));
            // The first arm that produces a value sets the type of the others.
            let body = match result {
                Some(t) => self.check(arm.body, t),
                None => self.infer(arm.body, None),
            };
            let ty = self.ty_of(body);
            if result.is_none() && ty != Types::NEVER {
                result = Some(ty);
            }
            self.state.scopes.pop();
            hir_arms.push(Arm {
                pattern,
                guard,
                body,
                span: arm.span,
            });
        }
        self.exhaustiveness(scrut_ty, ast.exprs[scrutinee].span, arms, &hir_arms);
        let ty = result.unwrap_or(Types::NEVER);
        self.alloc(
            ExprKind::Match {
                scrutinee: scrut,
                arms: hir_arms,
            },
            ty,
            span,
        )
    }

    /// The elements of a tuple written as `e` itself, through parentheses:
    /// `(a, b)` is the prelude's `Tuple2(a, b)`.
    fn tuple_written(&self, e: ast::ExprId) -> Option<&'a [ast::ExprId]> {
        let ast = self.ast;
        match &ast.exprs[e].kind {
            ast::ExprKind::Paren(inner) => self.tuple_written(*inner),
            ast::ExprKind::Call {
                callee,
                args,
                names,
                rest: None,
            } if names.iter().all(Option::is_none)
                && (MIN_TUPLE..=MAX_TUPLE).contains(&args.len())
                && matches!(ast.exprs[*callee].kind, ast::ExprKind::Name(sym) if sym == Symbol::tuple(args.len())) =>
            {
                Some(args)
            }
            _ => None,
        }
    }

    /// A tuple matched where its elements are: each element
    /// checked as a `match` would check it alone, and how its bindings bind.
    /// The expression has the tuple's type, and nothing is built.
    fn places(&mut self, elements: &'a [ast::ExprId], span: Span) -> (ExprId, Option<Vec<Binds>>) {
        let mut lowered = Vec::new();
        let mut binds = Vec::new();
        for &element in elements {
            let id = self.operand(element, None);
            binds.push(Binds {
                alias: self.alias_kind(id),
                source: Some(id),
                copy: false,
                settle: true,
                variables: false,
            });
            lowered.push(id);
        }
        let tys: Vec<Ty> = lowered.iter().map(|&id| self.ty_of(id)).collect();
        // The prelude's tuple, found as a tuple pattern finds it: in the
        // module being lowered first, which is where it is while the
        // prelude itself is.
        let name = Symbol::tuple(tys.len());
        let declared = self
            .types()
            .get(&name)
            .map(|&(def, _)| def)
            .or_else(|| self.prelude_type(name));
        let ty = match declared {
            Some(TypeDef::Struct(id)) => {
                let list = self.program.types.intern_list(&tys);
                self.intern(TyKind::Struct(id, list))
            }
            _ => Types::ERROR,
        };
        (self.alloc(ExprKind::Places(lowered), ty, span), Some(binds))
    }

    /// An arm's pattern on a tuple matched in place: a
    /// tuple pattern, whose elements bind as their elements do, or `_`. A
    /// name for the whole has no tuple to bind.
    fn places_pattern(
        &mut self,
        pattern: &'a ast::Pattern,
        scrut_ty: Ty,
        element_binds: Vec<Binds>,
        binds: Binds,
    ) -> Pattern {
        match &pattern.kind {
            ast::PatternKind::Binding(sym)
                if self
                    .pattern_const(ast::Name {
                        sym: *sym,
                        span: pattern.span,
                    })
                    .is_none() =>
            {
                let name = self.text(*sym).to_string();
                let diagnostic = Diagnostic::error(
                    codes::INVALID_PATTERN,
                    format!("`{name}` would bind a tuple that is not made"),
                    pattern.span,
                    "names the whole",
                )
                .with_help("name the elements: `(a, b) => …`")
                .with_note("a tuple written as a `match`'s scrutinee is matched where its elements are, and is not built");
                self.report(diagnostic);
                Pattern::Error
            }
            ast::PatternKind::Variant {
                leading_dot: false,
                segments,
                ..
            } if segments.len() == 1 => {
                self.state.place_binds = Some(element_binds);
                let lowered = self.pattern(pattern, scrut_ty, binds);
                self.state.place_binds = None;
                lowered
            }
            _ => self.pattern(pattern, scrut_ty, binds),
        }
    }

    /// The kind of reference the bindings of a pattern on `scrut` are:
    /// matching a place binds aliases for its fields, which can be written
    /// when the place can; matching any other value binds the fields
    /// themselves.
    pub(super) fn alias_kind(&self, scrut: ExprId) -> Option<crate::RefKind> {
        self.roots_in_local(scrut).then(|| {
            if self.writable(scrut) {
                crate::RefKind::Var
            } else {
                crate::RefKind::Shared
            }
        })
    }

    /// `value is pattern`. `binds` says whether it is part
    /// of a condition, where the caller has made a scope for its bindings.
    pub(super) fn is_expr(
        &mut self,
        scrutinee: ast::ExprId,
        pattern: &'a ast::Pattern,
        binds: bool,
        span: Span,
    ) -> ExprId {
        let scrut = self.operand(scrutinee, None);
        let scrut_ty = self.ty_of(scrut);
        let alias = self.alias_kind(scrut);
        // What the pattern binds, at any depth: a pattern that only tests
        // binds nothing, whatever its shape.
        let mut names = Vec::new();
        self.names_of(pattern, &mut names);
        // A test alone outside a condition has nothing after it to see what
        // it binds. It is reported below, and bound in a scope of its own.
        let outside = !binds && !names.is_empty();
        if outside {
            self.state.scopes.push(FxHashMap::default());
        }
        let written = pattern;
        let binds = Binds {
            alias,
            source: Some(scrut),
            copy: false,
            settle: true,
            variables: false,
        };
        let pattern = self.pattern(pattern, scrut_ty, binds);
        if outside {
            self.state.scopes.pop();
        }
        // Any pattern that can fail is a test; one that matches every
        // value would always be true.
        let always = !pattern_has_error(&pattern)
            && !self.is_poisoned(scrut_ty)
            && self.always_matches(&pattern, scrut_ty);
        let pattern = if always {
            self.report_always_true(written, &pattern);
            Pattern::Error
        } else {
            if outside && !pattern_has_error(&pattern) {
                let (sym, span) = names[0];
                let diagnostic = self.unseen_binding(sym, span);
                self.report(diagnostic);
            }
            pattern
        };
        self.alloc(
            ExprKind::Is {
                scrutinee: scrut,
                pattern,
            },
            Types::BOOL,
            span,
        )
    }

    /// Reports an `is` test whose pattern matches every value, with what
    /// to write for what it was meant to do.
    fn report_always_true(&mut self, written: &ast::Pattern, pattern: &Pattern) {
        let diagnostic = Diagnostic::error(
            codes::INVALID_PATTERN,
            "this `is` test is always true",
            written.span,
            "matches every value",
        );
        let diagnostic = match pattern {
            Pattern::Wildcard | Pattern::Binding(_) => diagnostic
                .with_help("to bind the value, use `val`; to compare it with another, use `==`"),
            Pattern::Fields(_) => {
                diagnostic.with_help("take it apart where it is bound: `val Point(x, y) = point`")
            }
            _ => diagnostic.with_help("every value matches it, so the test can go"),
        }
        .with_note("`is` asks whether a value matches a pattern that can fail");
        self.report(diagnostic);
    }

    /// `val pattern = value else { … }`. The `else` block is
    /// checked first, without the bindings, and must leave; the bindings go
    /// into the enclosing block's scope. With `var`, each binding is a
    /// variable that holds its part.
    pub(super) fn guard(
        &mut self,
        mutable: bool,
        pattern: &'a ast::Pattern,
        value: ast::ExprId,
        else_block: Option<&'a ast::Block>,
    ) -> StmtKind {
        let scrut = self.operand(value, None);
        let scrut_ty = self.ty_of(scrut);
        // A variable aliases nothing: it holds its part, as `var x = …`
        // holds its value.
        let alias = if mutable {
            None
        } else {
            self.alias_kind(scrut)
        };
        let else_hir = else_block.map(|else_block| {
            let (else_hir, else_ty) = self.check_block(else_block, None);
            if else_ty != Types::NEVER && !self.is_poisoned(else_ty) {
                let end = Span::new(else_block.span.hi.saturating_sub(1), else_block.span.hi);
                let diagnostic = Diagnostic::error(
                    codes::GUARD_ELSE_FALLS_THROUGH,
                    format!(
                        "the `else` block of a `{}` pattern must leave",
                        if mutable { "var" } else { "val" }
                    ),
                    end,
                    "can reach its end",
                )
                .with_help("end it with `return`, `break` or `continue`")
                .with_note("the code after the `val` uses its bindings, which the `else` path does not have");
                self.report(diagnostic);
            }
            else_hir
        });
        // A binding of plain data takes a copy, so the place it came from
        // stays free for the rest of the block.
        let binds = Binds {
            alias,
            source: Some(scrut),
            copy: true,
            settle: false,
            variables: mutable,
        };
        let pattern = self.pattern(pattern, scrut_ty, binds);
        if mutable {
            self.variables_take(&pattern, scrut);
        }
        // A pattern that leaves nothing out always matches, so an `else`
        // beside it never runs, and one that can fail has no path to take
        // without it.
        let irrefutable = self.always_matches(&pattern, scrut_ty);
        match (&else_hir, irrefutable) {
            (Some(_), true) => {
                let span = else_block.expect("an else was checked").span;
                let diagnostic = Diagnostic::error(
                    codes::GUARD_REFUTABILITY,
                    "this pattern always matches, so `else` cannot run",
                    span,
                    "never reached",
                )
                .with_note("a struct has one shape, so taking it apart cannot fail");
                self.report(diagnostic);
            }
            (None, false) if !matches!(pattern, Pattern::Error) => {
                let diagnostic = Diagnostic::error(
                    codes::GUARD_REFUTABILITY,
                    "a pattern that can fail needs `else`",
                    self.state.body.exprs[scrut].span,
                    "may not match",
                )
                .with_note("`val pattern = value else { … }` runs the `else` block when the value does not match; the block must leave");
                self.report(diagnostic);
            }
            _ => {}
        }
        StmtKind::Guard {
            scrutinee: scrut,
            pattern,
            else_block: else_hir,
        }
    }

    /// The condition of an `if` or a `while`. An `is` test
    /// in it, alone or joined by `&&`, binds names into the scope the caller
    /// made, for the rest of the chain and for the block.
    pub(super) fn binding_condition(&mut self, e: ast::ExprId) -> ExprId {
        let ast = self.ast;
        let span = ast.exprs[e].span;
        match &ast.exprs[e].kind {
            ast::ExprKind::Paren(inner) => self.binding_condition(*inner),
            ast::ExprKind::Is {
                scrutinee,
                pattern,
                negated: false,
            } => self.is_expr(*scrutinee, pattern, true, span),
            ast::ExprKind::Binary {
                op: ast::BinaryOp::And,
                lhs,
                rhs,
                ..
            } => {
                let lhs = self.binding_condition(*lhs);
                let rhs = self.binding_condition(*rhs);
                let op = ast::BinaryOp::And;
                self.alloc(
                    ExprKind::Binary {
                        op,
                        lhs,
                        rhs,
                        wrapping: false,
                    },
                    Types::BOOL,
                    span,
                )
            }
            _ => self.condition(e),
        }
    }

    /// A `&&` chain outside a condition whose `is` tests bind: each test's
    /// names are bound for the rest of the chain, and nowhere after it. It is
    /// lowered as `if chain { true } else { false }`, so that the bindings, the
    /// borrows they hold and their drops end where a condition's end — with the
    /// `if`, which here is the chain.
    pub(super) fn bound_chain(&mut self, e: ast::ExprId, span: Span) -> ExprId {
        // The last test of the chain has nothing after it to see what it
        // binds.
        if let Some((pattern, _)) = self.last_test(e) {
            let mut names = Vec::new();
            self.names_of(pattern, &mut names);
            if let Some(&(sym, name_span)) = names.first() {
                let diagnostic = self.unseen_binding(sym, name_span);
                self.report(diagnostic);
            }
        }
        self.state.scopes.push(FxHashMap::default());
        let cond = self.binding_condition(e);
        self.state.scopes.pop();
        let answer = |this: &mut Self, value: bool| Block {
            stmts: Vec::new(),
            value: Some(this.alloc(ExprKind::Bool(value), Types::BOOL, span)),
            span,
        };
        let then_block = answer(self, true);
        let else_block = answer(self, false);
        self.alloc(
            ExprKind::If {
                cond,
                then_block,
                else_block: Some(else_block),
            },
            Types::BOOL,
            span,
        )
    }

    /// Whether `e` is a `&&` chain — through parentheses — with an `is` test
    /// in it that binds a name.
    pub(super) fn chain_binds(&self, e: ast::ExprId) -> bool {
        match &self.ast.exprs[e].kind {
            ast::ExprKind::Paren(inner) => self.chain_binds(*inner),
            ast::ExprKind::Binary {
                op: ast::BinaryOp::And,
                lhs,
                rhs,
                ..
            } => self.chain_binds(*lhs) || self.chain_binds(*rhs),
            ast::ExprKind::Is {
                pattern,
                negated: false,
                ..
            } => {
                let mut names = Vec::new();
                self.names_of(pattern, &mut names);
                !names.is_empty()
            }
            _ => false,
        }
    }

    /// The last `is` test of a `&&` chain, if the chain ends with one.
    fn last_test(&self, e: ast::ExprId) -> Option<(&'a ast::Pattern, Span)> {
        let ast = self.ast;
        match &ast.exprs[e].kind {
            ast::ExprKind::Paren(inner) => self.last_test(*inner),
            ast::ExprKind::Binary {
                op: ast::BinaryOp::And,
                rhs,
                ..
            } => self.last_test(*rhs),
            ast::ExprKind::Is {
                pattern,
                negated: false,
                ..
            } => Some((pattern, ast.exprs[e].span)),
            _ => None,
        }
    }

    /// `value !is pattern`: `!(value is pattern)`, which binds nothing.
    /// A name in the pattern would be bound only where the
    /// test fails, so it is refused, as a test's name nothing can see is.
    pub(super) fn negated_is(
        &mut self,
        scrutinee: ast::ExprId,
        pattern: &'a ast::Pattern,
        span: Span,
    ) -> ExprId {
        let mut names = Vec::new();
        self.names_of(pattern, &mut names);
        if let Some(&(sym, name_span)) = names.first() {
            let name = self.text(sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::IS_BINDING_OUTSIDE_CONDITION,
                format!("`!is` binds nothing, and `{name}` would be bound"),
                name_span,
                "a name of the pattern",
            )
            .with_help("test without the name, as `.Some(_)`; to name what is there when the test fails, write `val .Some(x) = value else { … }`")
            .with_note("`x !is P` is true where `x` is not `P`, where there is nothing for a name to hold");
            self.report(diagnostic);
        }
        // The test itself would say again, less to the point, that its
        // names go unseen.
        let before = self.diagnostics.len();
        let test = self.is_expr(scrutinee, pattern, false, span);
        if !names.is_empty() {
            let mut i = before;
            while i < self.diagnostics.len() {
                if self.diagnostics[i].code == codes::IS_BINDING_OUTSIDE_CONDITION {
                    self.diagnostics.remove(i);
                } else {
                    i += 1;
                }
            }
        }
        self.alloc(
            ExprKind::Unary {
                op: UnaryOp::Not,
                operand: test,
            },
            Types::BOOL,
            span,
        )
    }

    /// E0333: an `is` test binds a name nothing after it can see.
    fn unseen_binding(&self, sym: Symbol, span: Span) -> Diagnostic {
        Diagnostic::error(
            codes::IS_BINDING_OUTSIDE_CONDITION,
            format!("nothing after this `is` test can use `{}`", self.text(sym)),
            span,
            "a binding",
        )
        .with_note("an `is` test binds names for the rest of its `&&` chain, and, in the condition of an `if` or `while`, for its block")
        .with_help("to test without binding, write `(..)`")
    }

    /// Whether `e` is a place a pattern binds into: a variable, a field or
    /// element of one, or what any reference points to.
    fn roots_in_local(&self, e: ExprId) -> bool {
        match self.state.body.exprs[e].kind {
            ExprKind::Local(_) => true,
            // What a reference refers to is a place wherever the reference
            // came from — a call that lends one, as a projection does — and
            // is another's: a pattern binds into it, and takes nothing out.
            // What an `own` points to is the `own`'s, which may be a value
            // just moved here, to be taken apart.
            ExprKind::Deref(base) => {
                matches!(self.kind(self.ty_of(base)), TyKind::Ref(..)) || self.roots_in_local(base)
            }
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                self.roots_in_local(base)
            }
            _ => false,
        }
    }

    /// What a `for` binds to each element: a name, `_`, or a struct taken
    /// apart. The bindings refer into the element, as a `match` binding of a
    /// place does.
    pub(super) fn element_pattern(
        &mut self,
        pattern: &'a ast::Pattern,
        element_ty: Ty,
        elem: Option<Ty>,
        kind: crate::RefKind,
        source: Option<ExprId>,
    ) -> Pattern {
        match &pattern.kind {
            ast::PatternKind::Wildcard => Pattern::Wildcard,
            ast::PatternKind::Binding(sym) => {
                let local = self.declare(*sym, element_ty, LocalKind::Binding, pattern.span);
                if matches!(self.kind(element_ty), TyKind::Ref(..)) {
                    self.state.body.alias_bindings.insert(local);
                }
                Pattern::Binding(local)
            }
            ast::PatternKind::Error => Pattern::Error,
            // A struct taken apart: its binders refer into the element.
            _ => {
                let binds = Binds {
                    alias: Some(kind),
                    source,
                    copy: false,
                    settle: false,
                    variables: false,
                };
                let lowered = self.pattern(pattern, elem.unwrap_or(Types::ERROR), binds);
                // Every element is taken apart, so the pattern must match
                // them all.
                if let Some(elem) = elem
                    && !self.always_matches(&lowered, elem)
                {
                    let diagnostic = Diagnostic::error(
                        codes::INVALID_PATTERN,
                        "a `for` binding takes every element apart, so its pattern must always match",
                        pattern.span,
                        "can fail",
                    )
                    .with_help("bind the element and test it with `match` or `is` inside the loop")
                    .with_note("a name, `_`, a struct taken apart, and a pattern whose fields do the same, match every value");
                    self.report(diagnostic);
                    return Pattern::Error;
                }
                lowered
            }
        }
    }

    /// What a binder does with a field: binds it under a name, ignores it,
    /// or takes it apart in turn.
    fn binder(&mut self, b: &'a ast::Binder, field_ty: Ty, binds: Binds) -> Binder {
        let Some(pattern) = &b.pattern else {
            return Binder::Bind(self.declare_binding(b.field.sym, field_ty, b.span, binds));
        };
        match &pattern.kind {
            // `field: name` binds it under that name, which is what a bare
            // name in a pattern always means.
            ast::PatternKind::Binding(sym) => {
                Binder::Bind(self.declare_binding(*sym, field_ty, pattern.span, binds))
            }
            ast::PatternKind::Wildcard => Binder::Ignored,
            ast::PatternKind::Error => Binder::Ignored,
            // A pattern inside a pattern tests what its field holds, to
            // any depth.
            _ => Binder::Nested(self.pattern(pattern, field_ty, binds)),
        }
    }

    /// The variants of an enum no arm matches at all, as the patterns that
    /// would: `.Yellow`, `.Round(_)`.
    fn missing_variants(&self, scrut_ty: Ty, matrix: &[Vec<usefulness::Shape>]) -> Vec<String> {
        let TyKind::Enum(id, _) = self.kind(scrut_ty) else {
            return Vec::new();
        };
        let variants = self.program.enums[id].variants.len();
        let mut missing = Vec::new();
        for variant in 0..variants as u32 {
            let fields = self.program.enums[id].variants[variant as usize]
                .fields
                .len();
            let shape = usefulness::Shape::Ctor(
                usefulness::Ctor::Variant(variant),
                vec![usefulness::Shape::Wild; fields],
            );
            if let Some(witness) = usefulness::useful(&self.program, matrix, &[shape], &[scrut_ty])
            {
                missing.push(witness.text(&self.program, scrut_ty, self.interner));
            }
        }
        missing
    }

    /// Whether a pattern matches every value of its type: nothing is left
    /// for an `else` or a next arm.
    pub(super) fn always_matches(&self, pattern: &Pattern, ty: Ty) -> bool {
        if self.is_poisoned(ty) || matches!(pattern, Pattern::Error) {
            return true;
        }
        let row = vec![usefulness::Shape::of(&self.program, pattern, ty)];
        usefulness::useful(&self.program, &[row], &[usefulness::Shape::Wild], &[ty]).is_none()
    }

    /// A pattern where a field name could stand, in a pattern whose type
    /// has more than one field: which field it tests must be said.
    fn name_the_field(&mut self, path: &str, fields: &[crate::FieldDef], b: &ast::Binder) {
        let span = b.span;
        // `_` does not pass a field over by position, where there are
        // fields to confuse.
        if matches!(&b.pattern, Some(p) if matches!(p.kind, ast::PatternKind::Wildcard)) {
            let diagnostic = Diagnostic::error(
                codes::UNDERSCORE_BINDER,
                "`_` is not a binder",
                span,
                "a field name is expected here",
            )
            .with_help(
                "binders name their fields: leave a field out, and end the pattern with `..`",
            );
            self.report(diagnostic);
            return;
        }
        let names: Vec<String> = fields
            .iter()
            .map(|f| format!("`{}`", self.text(f.name)))
            .collect();
        let diagnostic = Diagnostic::error(
            codes::NO_SUCH_FIELD,
            format!("`{path}` has {} fields: say which this tests", fields.len()),
            span,
            "names no field",
        )
        .with_help(format!(
            "write `field: pattern`, with one of {}",
            names.join(", ")
        ))
        .with_note("a pattern that names no field tests the one field its variant has");
        self.report(diagnostic);
    }

    /// Declares a binder's names as errors, so that what follows resolves.
    fn binder_failed(&mut self, b: &'a ast::Binder) {
        let mut names = vec![(b.field.sym, b.span)];
        if let Some(pattern) = &b.pattern {
            names = Vec::new();
            self.names_of(pattern, &mut names);
        }
        for (sym, span) in names {
            self.declare(sym, Types::ERROR, LocalKind::Binding, span);
        }
    }

    /// The names a pattern binds, for recovery.
    fn names_of(&self, pattern: &'a ast::Pattern, out: &mut Vec<(Symbol, Span)>) {
        match &pattern.kind {
            ast::PatternKind::Binding(sym) => out.push((*sym, pattern.span)),
            ast::PatternKind::Variant { binders, .. } => {
                for b in binders.as_deref().unwrap_or_default() {
                    match &b.pattern {
                        Some(inner) => self.names_of(inner, out),
                        None => out.push((b.field.sym, b.span)),
                    }
                }
            }
            // Any one of several: each binds the same names, or is
            // reported.
            ast::PatternKind::Any(alternatives) => {
                for alternative in alternatives {
                    self.names_of(alternative, out);
                }
            }
            ast::PatternKind::Slice(elements) => {
                for element in elements {
                    match element {
                        ast::SliceElement::Pattern(p) => self.names_of(p, out),
                        // A constant's name after `..` binds nothing.
                        ast::SliceElement::Rest { name, .. } => out.extend(
                            name.filter(|n| self.pattern_const(*n).is_none())
                                .map(|n| (n.sym, n.span)),
                        ),
                    }
                }
            }
            ast::PatternKind::Wildcard
            | ast::PatternKind::Int { .. }
            | ast::PatternKind::Range { .. }
            | ast::PatternKind::Bool(_)
            | ast::PatternKind::Str(_)
            | ast::PatternKind::Char(_)
            | ast::PatternKind::Byte(_)
            | ast::PatternKind::Error => {}
        }
    }

    /// Reports a guard on an arm whose bindings take what they match: the
    /// guard runs after the value has been taken apart, and one that
    /// fails would leave it half taken.
    fn guard_takes_nothing(
        &mut self,
        pattern: &Pattern,
        alias: Option<crate::RefKind>,
        span: Span,
        guard: ast::ExprId,
    ) {
        if alias.is_some() {
            return;
        }
        let mut locals = Vec::new();
        pattern.locals(&mut locals);
        let taken = locals.into_iter().find(|&local| {
            let ty = self.state.body.locals[local].ty;
            !self.is_poisoned(ty) && self.owns(ty)
        });
        let Some(local) = taken else {
            return;
        };
        let name = self.text(self.state.body.locals[local].name).to_string();
        let type_name = self.ty_name(self.state.body.locals[local].ty).to_string();
        let diagnostic = Diagnostic::error(
            codes::INVALID_PATTERN,
            format!("an arm that takes `{name}` cannot have a guard"),
            self.ast.exprs[guard].span,
            "a guard",
        )
        .with_secondary(span, format!("`{name}` is {type_name}, which owns memory"))
        .with_help("match without `move`, and take what the arm needs in its body")
        .with_note(
            "a guard runs once the pattern has taken the value apart, so a guard that fails would leave it half taken",
        );
        self.report(diagnostic);
    }

    /// A number in a pattern, in the bits of the type being matched.
    fn int_pattern(
        &mut self,
        negative: bool,
        magnitude: u128,
        scrut_ty: Ty,
        span: Span,
    ) -> Pattern {
        if !self.program.types.is_integer(scrut_ty) {
            return self
                .literal_pattern(scrut_ty, Types::I64, "a number", span)
                .unwrap_or(Pattern::Error);
        }
        Pattern::Int(self.int_literal(negative, magnitude, scrut_ty, span))
    }

    /// `'a'..='z'`, `0..10`, `..0`, `10..`: the values between two ends,
    /// of an integer or a character type, both ends included once it is
    /// lowered; one left off is not tested.
    fn range_pattern(
        &mut self,
        lo: Option<&ast::RangeBound>,
        hi: Option<&ast::RangeBound>,
        inclusive: bool,
        scrut_ty: Ty,
        span: Span,
    ) -> Pattern {
        if self.is_poisoned(scrut_ty) {
            return Pattern::Error;
        }
        let Some((least, greatest)) = self.program.types.order_range(scrut_ty) else {
            let diagnostic = Diagnostic::error(
                codes::INVALID_PATTERN,
                format!("a range cannot match {}", self.ty_name(scrut_ty)),
                span,
                format!("a range, and the value is {}", self.ty_name(scrut_ty)),
            )
            .with_note("a range matches an integer, a character or a byte: values in an order with none between two neighbours");
            self.report(diagnostic);
            return Pattern::Error;
        };
        let lo_bits = match lo {
            Some(bound) => match self.range_end(bound, scrut_ty) {
                Some(bits) => Some(bits),
                None => return Pattern::Error,
            },
            None => None,
        };
        let hi_bits = match hi {
            Some(bound) => match self.range_end(bound, scrut_ty) {
                Some(bits) => Some(bits),
                None => return Pattern::Error,
            },
            None => None,
        };
        let types = &self.program.types;
        let key = |bits: u128| types.order_key(scrut_ty, bits).expect("an ordered type");
        let lo_key = lo_bits.map_or(least, key);
        let hi_key = hi_bits.map_or(greatest, key);
        // `lo..hi` leaves `hi` out: its last value is the one before.
        let last = match (hi_bits, inclusive) {
            (Some(_), false) => hi_key.checked_sub(1),
            _ => Some(hi_key),
        };
        let Some(last) = last.filter(|&last| last >= lo_key) else {
            let diagnostic = Diagnostic::error(
                codes::INVALID_PATTERN,
                "this range has nothing in it",
                span,
                "no value lies between its ends",
            )
            .with_note(
                "`lo..=hi` takes `hi` and `lo..hi` leaves it out, so its end comes after its start",
            );
            self.report(diagnostic);
            return Pattern::Error;
        };
        let from_key = |key: u128| self.program.types.from_order_key(scrut_ty, key);
        Pattern::Range {
            lo: lo_bits.map(|_| from_key(lo_key)),
            hi: hi_bits.map(|_| from_key(last)),
        }
    }

    /// The bits of one end of a range, of the type being matched: a
    /// literal, or a constant worked out here if nothing has asked for it
    /// yet. Reported, and nothing, where it is not one.
    fn range_end(&mut self, bound: &ast::RangeBound, scrut_ty: Ty) -> Option<u128> {
        let span = bound.span;
        match &bound.kind {
            ast::RangeBoundKind::Int {
                negative,
                magnitude,
            } => {
                if !self.program.types.is_integer(scrut_ty) {
                    self.literal_pattern(scrut_ty, Types::I64, "a number", span);
                    return None;
                }
                Some(self.int_literal(*negative, *magnitude, scrut_ty, span))
            }
            ast::RangeBoundKind::Char(c) => self
                .literal_pattern(scrut_ty, Types::CHAR, "a character", span)
                .is_none()
                .then_some(u128::from(*c)),
            ast::RangeBoundKind::Byte(b) => self
                .literal_pattern(scrut_ty, Types::U8, "a byte", span)
                .is_none()
                .then_some(u128::from(*b)),
            ast::RangeBoundKind::Const(path) => {
                let id = match &path[..] {
                    [name] => self.const_named(*name),
                    _ => match self.resolve_path(path, false, paths::MODULE_NOTE) {
                        PathTarget::Item(module, name) => {
                            self.modules[module].consts.get(&name.sym).copied()
                        }
                        _ => return None,
                    },
                };
                let written: Vec<&str> = path.iter().map(|n| self.text(n.sym)).collect();
                let written = written.join("::");
                let Some(id) = id else {
                    let diagnostic = Diagnostic::error(
                        codes::UNKNOWN_NAME,
                        format!("cannot find the constant `{written}`"),
                        span,
                        "not a constant",
                    )
                    .with_note("a range's ends are literals or constants");
                    self.report(diagnostic);
                    return None;
                };
                self.const_value(id);
                if !self.known_now(id, span, "a range's end") {
                    return None;
                }
                let ty = self.program.consts[id].ty;
                if ty != scrut_ty {
                    if !self.is_poisoned(ty) {
                        let diagnostic = Diagnostic::error(
                            codes::MISMATCHED_TYPES,
                            format!(
                                "`{written}` is {}, and the value matched is {}",
                                self.ty_name(ty),
                                self.ty_name(scrut_ty)
                            ),
                            span,
                            "an end of another type",
                        );
                        self.report(diagnostic);
                    }
                    return None;
                }
                match self.program.consts[id].value {
                    Some(crate::ConstValue::Int(bits)) => Some(bits),
                    _ => None,
                }
            }
        }
    }

    /// The constant a name in a pattern stands for: one of this module's,
    /// one it imported, or one the prelude exports. Another module's
    /// private constant is no more a pattern than it is a value.
    fn pattern_const(&self, name: ast::Name) -> Option<ConstId> {
        let id = self.const_named(name)?;
        let own = self.consts().get(&name.sym) == Some(&id);
        (own || self.program.consts[id].is_pub).then_some(id)
    }

    /// A constant in a pattern: the value the scrutinee must equal, as the
    /// literal it was written with would be.
    fn const_pattern(&mut self, id: ConstId, scrut_ty: Ty, span: Span) -> Pattern {
        self.const_value(id);
        if !self.known_now(id, span, "a pattern") {
            return Pattern::Error;
        }
        let ty = self.program.consts[id].ty;
        let name = self.text(self.program.consts[id].name).to_string();
        if self.is_poisoned(ty) || self.is_poisoned(scrut_ty) {
            return Pattern::Error;
        }
        if ty != scrut_ty {
            let diagnostic = Diagnostic::error(
                codes::MISMATCHED_TYPES,
                format!(
                    "`{name}` is {}, and the value matched is {}",
                    self.ty_name(ty),
                    self.ty_name(scrut_ty)
                ),
                span,
                "a constant of another type",
            )
            .with_note("a constant in a pattern asks whether the value equals it, so the two are of one type");
            self.report(diagnostic);
            return Pattern::Error;
        }
        match self.program.consts[id].value.clone() {
            Some(crate::ConstValue::Int(bits)) => Pattern::Int(bits),
            Some(crate::ConstValue::Bool(value)) => Pattern::Bool(value),
            Some(crate::ConstValue::Str(sym)) if ty == Types::STR => Pattern::Str(sym),
            Some(_) => {
                let diagnostic = Diagnostic::error(
                    codes::INVALID_PATTERN,
                    format!("`{name}` is {}, which a pattern cannot equal", self.ty_name(ty)),
                    span,
                    "not a number, a character, a `bool` or text",
                )
                .with_note("a constant in a pattern is one a literal could be written for: an integer, a character, a `bool` or a `str`; a float is compared with `==`");
                self.report(diagnostic);
                Pattern::Error
            }
            // Working it out failed, and said so.
            None => Pattern::Error,
        }
    }

    /// Reports a literal pattern against a type it cannot equal, and
    /// answers `None` where the type is the one it wants.
    fn literal_pattern(
        &mut self,
        scrut_ty: Ty,
        wanted: Ty,
        what: &str,
        span: Span,
    ) -> Option<Pattern> {
        if self.is_poisoned(scrut_ty) {
            return Some(Pattern::Error);
        }
        // A byte matches a `u8`, as `==` compares it with one.
        // A number is only asked here of what is not an
        // integer at all.
        if scrut_ty == wanted {
            return None;
        }
        let diagnostic = Diagnostic::error(
            codes::INVALID_PATTERN,
            format!(
                "{what} cannot match {}",
                self.ty_name(scrut_ty)
            ),
            span,
            format!("{what}, and the value is {}", self.ty_name(scrut_ty)),
        )
        .with_note(
            "a pattern that is a value asks whether the value being matched equals it, so the two are of one type",
        );
        self.report(diagnostic);
        Some(Pattern::Error)
    }

    /// Whether a binding takes a copy rather than aliasing the place it
    /// came from: plain data bound by a `val … else`, which leaves the
    /// place free for the rest of the block. What owns
    /// memory cannot be copied out of a place, and what holds a `str` or a
    /// view must keep referring to it, so that a change to what it borrows
    /// is caught.
    fn copied_binding(&mut self, ty: Ty, binds: Binds) -> bool {
        // A function value is always copied: a reference to one would be a
        // `&(…) => …`, which is a closure lent for a call, not an address,
        // so there is no alias of it to bind.
        if matches!(self.kind(ty), TyKind::Fn(..)) {
            return true;
        }
        binds.copy && !self.is_poisoned(ty) && !self.owns(ty) && !self.program.holds_view(ty)
    }

    /// A `var` pattern on a place copies its parts out, so a part that owns
    /// memory, which is never copied, needs the place to give the value up
    /// with `move`, as `var x = place` does.
    fn variables_take(&mut self, pattern: &Pattern, scrut: ExprId) {
        if !self.roots_in_local(scrut) {
            return;
        }
        let mut locals = Vec::new();
        pattern.locals(&mut locals);
        let owning = locals.into_iter().find(|&local| {
            let ty = self.state.body.locals[local].ty;
            !self.is_poisoned(ty) && self.owns(ty)
        });
        let Some(local) = owning else {
            return;
        };
        let (name, span, ty) = {
            let local = &self.state.body.locals[local];
            (self.text(local.name).to_string(), local.span, local.ty)
        };
        let scrut_span = self.state.body.exprs[scrut].span;
        let diagnostic = Diagnostic::error(
            codes::CANNOT_COPY_OWN,
            format!("`{name}` would hold a copy of a {}", self.ty_name(ty)),
            span,
            "owns memory, so it is never copied",
        )
        .with_secondary(scrut_span, "a place, which keeps its value")
        .with_note("the names of a `var` pattern are variables of their own, which hold their parts; a place gives its value up to them with `move`")
        .with_fix(
            "to take the value apart, move it",
            [Edit::insert(scrut_span.lo, "move ")],
        );
        self.report(diagnostic);
    }

    /// Declares a binding of a value of type `ty`: an alias for it, a
    /// reference to it, where `binds` aliases and it is not copied, and the
    /// value itself otherwise. It records which, and the place an alias
    /// refers into.
    fn declare_binding(&mut self, name: Symbol, ty: Ty, span: Span, binds: Binds) -> LocalId {
        let copied = self.copied_binding(ty, binds);
        let alias = binds.alias.filter(|_| !copied && !self.is_poisoned(ty));
        let local_ty = match alias {
            Some(kind) => self.intern(TyKind::Ref(ty, kind)),
            None => ty,
        };
        let kind = if binds.variables {
            LocalKind::Var
        } else {
            LocalKind::Binding
        };
        let local = self.declare(name, local_ty, kind, span);
        if alias.is_some() {
            self.state.body.alias_bindings.insert(local);
            if let Some(source) = binds.source {
                self.state.body.aliases.insert(local, source);
            }
            let plain = !self.is_poisoned(ty)
                && !self.owns(ty)
                && !self.program.holds_view(ty)
                && !matches!(self.kind(ty), TyKind::Ref(..) | TyKind::Fn(..));
            if binds.settle && plain {
                self.state.settling.push(local);
            }
        } else if binds.copy && !binds.variables {
            // A binding that took a copy refers to nothing.
            self.state.body.copied_bindings.insert(local);
        }
        local
    }

    pub(super) fn pattern(
        &mut self,
        pattern: &'a ast::Pattern,
        scrut_ty: Ty,
        binds: Binds,
    ) -> Pattern {
        match &pattern.kind {
            ast::PatternKind::Wildcard => Pattern::Wildcard,
            // A number, a `bool` or text the value must equal.
            ast::PatternKind::Int {
                negative,
                magnitude,
            } => self.int_pattern(*negative, *magnitude, scrut_ty, pattern.span),
            ast::PatternKind::Bool(value) => self
                .literal_pattern(scrut_ty, Types::BOOL, "a `bool`", pattern.span)
                .unwrap_or(Pattern::Bool(*value)),
            ast::PatternKind::Str(sym) => self
                .literal_pattern(scrut_ty, Types::STR, "text", pattern.span)
                .unwrap_or(Pattern::Str(*sym)),
            // A character is matched as the number it is.
            ast::PatternKind::Char(c) => self
                .literal_pattern(scrut_ty, Types::CHAR, "a character", pattern.span)
                .unwrap_or(Pattern::Int(*c as u128)),
            // And a byte as the `u8` it is.
            ast::PatternKind::Byte(b) => self
                .literal_pattern(scrut_ty, Types::U8, "a byte", pattern.span)
                .unwrap_or(Pattern::Int(u128::from(*b))),
            // `'a'..='z'`, `0..10`, `..0`, `10..`.
            ast::PatternKind::Range { lo, hi, inclusive } => {
                self.range_pattern(lo.as_ref(), hi.as_ref(), *inclusive, scrut_ty, pattern.span)
            }
            // `.Round(..) | .Square(..)`: any one of them.
            ast::PatternKind::Any(alternatives) => {
                let mut lowered = Vec::new();
                for alternative in alternatives {
                    let pattern = self.pattern(alternative, scrut_ty, binds);
                    // What an alternative binds would have to be bound by
                    // every other one, and the arm would have to know
                    // which it came from.
                    let mut names = Vec::new();
                    pattern.locals(&mut names);
                    if !names.is_empty() {
                        let diagnostic = Diagnostic::error(
                            codes::INVALID_PATTERN,
                            "a pattern among alternatives binds nothing",
                            alternative.span,
                            "binds a name",
                        )
                        .with_help("name the whole value in one arm, or write an arm for each")
                        .with_note(
                            "each alternative is a separate test, and what one binds the others would have to bind too",
                        );
                        self.report(diagnostic);
                        lowered.push(Pattern::Error);
                        continue;
                    }
                    lowered.push(pattern);
                }
                Pattern::Any(lowered)
            }
            ast::PatternKind::Binding(sym) => {
                // A constant's name is the value it stands for, as a
                // range's end is: `OP_ADD => …`.
                let name = ast::Name {
                    sym: *sym,
                    span: pattern.span,
                };
                if let Some(id) = self.pattern_const(name) {
                    // A local of the name hides the constant everywhere
                    // else, and a pattern cannot compare with a local, so
                    // the name says two things here.
                    if self.lookup(*sym).is_some() {
                        self.hidden_constant(name);
                        return Pattern::Error;
                    }
                    return self.const_pattern(id, scrut_ty, pattern.span);
                }
                if let TyKind::Enum(id, _) = self.kind(scrut_ty)
                    && self.program.enums[id]
                        .variants
                        .iter()
                        .any(|v| v.name == *sym)
                {
                    let enum_name = self.text(self.program.enums[id].name);
                    let name = self.text(*sym);
                    let diagnostic = Diagnostic::warning(
                        codes::BINDING_SHADOWS_VARIANT,
                        format!("`{name}` binds a new name; it does not match the variant `{enum_name}::{name}`"),
                        pattern.span,
                        "matches every value",
                    )
                    .with_fix(
                        format!("to match the variant, write `.{name}`"),
                        [Edit::replace(pattern.span, format!(".{name}"))],
                    );
                    self.report(diagnostic);
                }
                Pattern::Binding(self.declare_binding(*sym, scrut_ty, pattern.span, binds))
            }
            ast::PatternKind::Variant {
                leading_dot,
                segments,
                binders,
                rest,
            } => {
                let note =
                    "a variant pattern is `.Variant`, `Enum::Variant`, or a module path to one";
                // `Point(x, y)`: one name and no leading dot is a struct
                // taken apart, or a variant whose enum was left out.
                if !*leading_dot && segments.len() == 1 {
                    let name = segments[0];
                    // The file's own types, then what it imported, then
                    // the prelude's, as a type name resolves anywhere else.
                    let declared = self
                        .types()
                        .get(&name.sym)
                        .map(|&(def, _)| def)
                        .or_else(|| self.imported_type(name))
                        .or_else(|| self.prelude_type(name.sym));
                    return match declared {
                        Some(TypeDef::Struct(id)) => self.struct_pattern(
                            id,
                            name,
                            Fields {
                                binders: binders.as_ref().map(Vec::as_slice),
                                rest: *rest,
                            },
                            scrut_ty,
                            pattern.span,
                            binds,
                        ),
                        _ => {
                            let text = self.text(name.sym).to_string();
                            // The old mistake: a variant of the type being
                            // matched, written without its enum.
                            let is_variant = match self.kind(scrut_ty) {
                                TyKind::Enum(id, _) => self.program.enums[id]
                                    .variants
                                    .iter()
                                    .any(|v| v.name == name.sym),
                                _ => false,
                            };
                            let message = if is_variant {
                                "variant pattern is missing its enum".to_string()
                            } else {
                                format!("cannot find a struct named `{text}`")
                            };
                            let diagnostic = Diagnostic::error(
                                codes::INVALID_PATTERN,
                                message,
                                pattern.span,
                                format!("write `.{text}(…)` to match a variant"),
                            )
                            .with_fix(
                                format!("match the variant `.{text}`"),
                                [Edit::insert(name.span.lo, ".")],
                            )
                            .with_note(
                                "a name with fields after it is a struct taken apart; a variant is `.Variant(…)` or `Enum::Variant(…)`",
                            );
                            self.report(diagnostic);
                            for b in binders.as_deref().unwrap_or_default() {
                                self.binder_failed(b);
                            }
                            Pattern::Error
                        }
                    };
                }
                let enum_name = if *leading_dot {
                    if segments.len() != 1 {
                        self.unknown_module(segments[0], note);
                        return Pattern::Error;
                    }
                    None
                } else {
                    match self.resolve_path(segments, true, note) {
                        PathTarget::Variant(module, enum_name, _) => Some((module, enum_name)),
                        PathTarget::Item(_, item) => {
                            self.unknown_module(item, note);
                            return Pattern::Error;
                        }
                        PathTarget::Broken => return Pattern::Error,
                    }
                };
                let variant = *segments.last().expect("a path has segments");
                self.variant_pattern(
                    enum_name,
                    variant,
                    Fields {
                        binders: binders.as_ref().map(Vec::as_slice),
                        rest: *rest,
                    },
                    scrut_ty,
                    pattern.span,
                    binds,
                )
            }
            ast::PatternKind::Slice(elements) => {
                self.slice_pattern(elements, scrut_ty, pattern.span, binds)
            }
            ast::PatternKind::Error => Pattern::Error,
        }
    }

    /// `[first, ..rest]`: the elements of an array or a slice, from either
    /// end. An element binds as a field does; the rest
    /// binds the elements between as a slice of where they lie.
    fn slice_pattern(
        &mut self,
        elements: &'a [ast::SliceElement],
        scrut_ty: Ty,
        span: Span,
        binds: Binds,
    ) -> Pattern {
        let (elem, len) = match self.kind(scrut_ty) {
            TyKind::Array(elem, len) => (elem, Some(len)),
            TyKind::Slice(elem) => (elem, None),
            _ => {
                if !self.is_poisoned(scrut_ty) {
                    self.not_a_slice(scrut_ty, span);
                }
                self.slice_failed(elements);
                return Pattern::Error;
            }
        };
        // `..MAX`, where `MAX` is a constant, is an element below it, as a
        // constant's name is its value anywhere in a pattern; any other `..` is
        // the rest, of which there is one.
        let is_rest = |this: &Self, element: &ast::SliceElement| match element {
            ast::SliceElement::Rest {
                name: Some(name), ..
            } => this.pattern_const(*name).is_none(),
            ast::SliceElement::Rest { name: None, .. } => true,
            ast::SliceElement::Pattern(_) => false,
        };
        let rests: Vec<Span> = elements
            .iter()
            .filter(|element| is_rest(self, element))
            .map(|element| match element {
                ast::SliceElement::Rest { span, .. } => *span,
                ast::SliceElement::Pattern(p) => p.span,
            })
            .collect();
        if let [first, second, ..] = rests[..] {
            let diagnostic = Diagnostic::error(
                codes::SLICE_PATTERN_REST,
                "a slice pattern has one `..`",
                second,
                "a second `..`",
            )
            .with_secondary(first, "the elements between are these")
            .with_note("`..` stands for the elements the pattern does not name, so which would be in which `..` is not said");
            self.report(diagnostic);
            self.slice_failed(elements);
            return Pattern::Error;
        }
        let mut before: Vec<Binder> = Vec::new();
        let mut after: Vec<Binder> = Vec::new();
        let mut rest: Option<crate::SliceRest> = None;
        for element in elements {
            let binder = match element {
                ast::SliceElement::Pattern(p) => match self.pattern(p, elem, binds) {
                    Pattern::Wildcard => Binder::Ignored,
                    Pattern::Binding(local) => Binder::Bind(local),
                    other => Binder::Nested(other),
                },
                _ if is_rest(self, element) => {
                    let ast::SliceElement::Rest { name, .. } = element else {
                        unreachable!("a rest is a `..`")
                    };
                    rest = Some(match name {
                        Some(name) => self.declare_rest(*name, elem, scrut_ty, binds),
                        None => crate::SliceRest::Ignored,
                    });
                    continue;
                }
                // A constant's name after `..`: the elements below it.
                ast::SliceElement::Rest { name, span } => {
                    let name = name.expect("only a named `..` is a constant's");
                    let range = match self.lookup(name.sym) {
                        Some(_) => {
                            self.hidden_constant(name);
                            Pattern::Error
                        }
                        None => {
                            let bound = ast::RangeBound {
                                kind: ast::RangeBoundKind::Const(vec![name]),
                                span: name.span,
                            };
                            self.range_pattern(None, Some(&bound), false, elem, *span)
                        }
                    };
                    Binder::Nested(range)
                }
            };
            match rest {
                Some(_) => after.push(binder),
                None => before.push(binder),
            }
        }
        let named = before.len() + after.len();
        if let Some(len) = len
            && (if rest.is_some() {
                named as u64 > len
            } else {
                named as u64 != len
            })
        {
            let wanted = match rest.is_some() {
                true => format!("at least {named}"),
                false => named.to_string(),
            };
            let mut diagnostic = Diagnostic::error(
                codes::INVALID_PATTERN,
                format!("this array has {len} elements, and the pattern matches {wanted}"),
                span,
                format!("matches {wanted}"),
            )
            .with_note(format!(
                "the value being matched has type {}",
                self.ty_name(scrut_ty)
            ));
            if rest.is_none() && (named as u64) < len {
                diagnostic = diagnostic.with_help(format!(
                    "to match the first {named} and pass over the rest, end it with `..`"
                ));
            }
            self.report(diagnostic);
            return Pattern::Error;
        }
        Pattern::Slice {
            prefix: before,
            rest,
            suffix: after,
        }
    }

    /// Declares a slice pattern's names as errors, so that what follows
    /// resolves.
    fn slice_failed(&mut self, elements: &'a [ast::SliceElement]) {
        let mut names = Vec::new();
        for element in elements {
            match element {
                ast::SliceElement::Pattern(p) => self.names_of(p, &mut names),
                ast::SliceElement::Rest { name, .. } => {
                    names.extend(name.map(|n| (n.sym, n.span)));
                }
            }
        }
        for (sym, span) in names {
            self.declare(sym, Types::ERROR, LocalKind::Binding, span);
        }
    }

    /// A constant's name in a pattern where a local of that name hides
    /// it: a pattern cannot compare with a local, so the name says two
    /// things.
    fn hidden_constant(&mut self, name: ast::Name) {
        let text = self.text(name.sym).to_string();
        let diagnostic = Diagnostic::error(
            codes::INVALID_PATTERN,
            format!("`{text}` is a constant, and a local of that name hides it"),
            name.span,
            "a constant or a new name?",
        )
        .with_help("rename the local, or bind a name of its own here")
        .with_note("a constant's name in a pattern is the value it stands for");
        self.report(diagnostic);
    }

    /// A slice pattern on what is not an array or a slice.
    /// A container that lends its elements is matched through them.
    fn not_a_slice(&mut self, scrut_ty: Ty, span: Span) {
        let mut diagnostic = Diagnostic::error(
            codes::INVALID_PATTERN,
            format!("a slice pattern cannot match {}", self.ty_name(scrut_ty)),
            span,
            "matches the elements of an array or a slice",
        )
        .with_note(format!(
            "the value being matched has type {}",
            self.ty_name(scrut_ty)
        ));
        let lends = self
            .program
            .prelude_items
            .interface(KnownInterface::Items)
            .is_some_and(|items| self.implements_any(scrut_ty, items));
        if lends {
            diagnostic =
                diagnostic.with_help("match the elements it lends: `match value.items() { … }`");
        }
        self.report(diagnostic);
    }

    /// `..rest` in a slice pattern: the elements between, as a slice of
    /// where they lie. It refers into the value as an
    /// alias does, and so needs a place: a value made where it is matched
    /// is gone when its statement ends.
    fn declare_rest(
        &mut self,
        name: ast::Name,
        elem: Ty,
        scrut_ty: Ty,
        binds: Binds,
    ) -> crate::SliceRest {
        let in_place = matches!(self.kind(scrut_ty), TyKind::Slice(_))
            || binds
                .source
                .is_some_and(|source| self.roots_in_local(source));
        if !in_place {
            let text = self.text(name.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::INVALID_PATTERN,
                format!("`{text}` would borrow elements that are not kept"),
                name.span,
                "a slice of the elements between",
            )
            .with_note("`..rest` borrows the elements where they lie, and a value made where it is matched is gone when its statement ends")
            .with_help("keep the value in a variable, and match that");
            self.report(diagnostic);
            self.declare(name.sym, Types::ERROR, LocalKind::Binding, name.span);
            return crate::SliceRest::Ignored;
        }
        let slice = self.intern(TyKind::Slice(elem));
        let ty = self.intern(TyKind::Ref(
            slice,
            binds.alias.unwrap_or(crate::RefKind::Shared),
        ));
        // A `var` pattern's rest is a variable of its own, which holds a
        // reference.
        let kind = if binds.variables {
            LocalKind::Var
        } else {
            LocalKind::Binding
        };
        let local = self.declare(name.sym, ty, kind, name.span);
        if binds.alias.is_some() {
            self.state.body.alias_bindings.insert(local);
            if let Some(source) = binds.source {
                self.state.body.aliases.insert(local, source);
            }
        }
        crate::SliceRest::Bind(local)
    }

    /// `Point(x, y)`: a struct taken apart where it is bound.
    /// Its binders are a variant's binders, and it always
    /// matches, so it only declares them.
    pub(super) fn struct_pattern(
        &mut self,
        id: crate::StructId,
        name: ast::Name,
        fields: Fields<'a>,
        scrut_ty: Ty,
        span: Span,
        binds: Binds,
    ) -> Pattern {
        let Fields { binders, rest } = fields;
        let fields = self.program.structs[id].fields.clone();
        let path = self.text(self.program.structs[id].name).to_string();
        let matches = matches!(self.kind(scrut_ty), TyKind::Struct(scrut, _) if scrut == id);
        if !matches && !self.is_poisoned(scrut_ty) {
            let diagnostic = Diagnostic::error(
                codes::INVALID_PATTERN,
                "pattern does not match the type being matched",
                span,
                format!("a `{path}`"),
            )
            .with_note(format!(
                "the value being matched has type {}",
                self.ty_name(scrut_ty)
            ));
            self.report(diagnostic);
        }
        let binders = binders.unwrap_or_default();
        if !matches {
            for b in binders {
                self.binder_failed(b);
            }
            return Pattern::Error;
        }
        // The elements of a tuple matched in place bind each as its own
        // element does.
        let element_binds = self.state.place_binds.take();
        let mut locals: Vec<Binder> = (0..fields.len()).map(|_| Binder::Ignored).collect();
        let mut bound: Vec<Option<Span>> = vec![None; fields.len()];
        let mut failed = false;
        for b in binders {
            // With one field, a bare binder of any name binds it, as a
            // variant's does, and so does a pattern that names no field.
            let bare = b.pattern.is_none() || b.field.sym == Symbol::positional();
            let only = (fields.len() == 1 && bare).then_some(0);
            let position = match b.field.sym == Symbol::positional() {
                true => None,
                false => fields.iter().position(|f| f.name == b.field.sym),
            };
            let Some(i) = position.or(only) else {
                if b.field.sym == Symbol::positional() {
                    self.name_the_field(&path, &fields, b);
                    failed = true;
                    continue;
                }
                let field = self.text(b.field.sym);
                let mut diagnostic = Diagnostic::error(
                    codes::NO_SUCH_FIELD,
                    format!("`{path}` has no field `{field}`"),
                    b.field.span,
                    "unknown field",
                );
                if let Some(similar) = suggest(field, fields.iter().map(|f| self.text(f.name))) {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `{similar}`?"),
                        [Edit::replace(b.field.span, similar)],
                    );
                }
                self.report(diagnostic);
                self.binder_failed(b);
                failed = true;
                continue;
            };
            if let Some(first) = bound[i] {
                let field = self.text(b.field.sym);
                let diagnostic = Diagnostic::error(
                    codes::PATTERN_FIELDS,
                    format!("field `{field}` is named twice"),
                    b.field.span,
                    "named again",
                )
                .with_secondary(first, "first named here");
                self.report(diagnostic);
                self.binder_failed(b);
                failed = true;
                continue;
            }
            // Another module may take apart only what is `pub`.
            if !self.field_visible(id, i) {
                self.private_field(id, i, b.field.span);
            }
            bound[i] = Some(b.field.span);
            let field_ty = self.struct_field_ty(scrut_ty, i);
            let field_binds = element_binds
                .as_ref()
                .and_then(|e| e.get(i).copied())
                .unwrap_or(binds);
            locals[i] = self.binder(b, field_ty, field_binds);
        }
        if !rest && !failed {
            let missing: Vec<String> = fields
                .iter()
                .zip(&bound)
                .filter(|(_, bound)| bound.is_none())
                .map(|(f, _)| format!("`{}`", self.text(f.name)))
                .collect();
            if !missing.is_empty() {
                let diagnostic = Diagnostic::error(
                    codes::PATTERN_FIELDS,
                    format!(
                        "`{path}` has {} the pattern does not name: {}",
                        plural(missing.len(), "field", "fields"),
                        missing.join(", ")
                    ),
                    span,
                    "not every field is named",
                )
                .with_help("name them, or end the pattern with `..`");
                self.report(diagnostic);
                failed = true;
            }
        }
        let _ = name;
        if failed {
            return Pattern::Error;
        }
        Pattern::Fields(locals)
    }

    pub(super) fn variant_pattern(
        &mut self,
        enum_name: Option<(usize, ast::Name)>,
        variant: ast::Name,
        fields: Fields<'a>,
        scrut_ty: Ty,
        span: Span,
        binds: Binds,
    ) -> Pattern {
        let Fields { binders, rest } = fields;
        // Declare the bindings whatever happens, so the arm body resolves.
        let bind_all_as_error = |this: &mut Self| {
            for b in binders.unwrap_or_default() {
                this.binder_failed(b);
            }
            Pattern::Error
        };
        let Some((id, index)) = self.resolve_variant(enum_name, variant, Some(scrut_ty)) else {
            return bind_all_as_error(self);
        };
        let pattern_ty = self.intern(TyKind::Enum(id, crate::TyList::EMPTY));
        if !matches!(self.kind(scrut_ty), TyKind::Enum(scrut_id, _) if scrut_id == id) {
            if !self.is_poisoned(scrut_ty) {
                let diagnostic = Diagnostic::error(
                    codes::INVALID_PATTERN,
                    "pattern does not match the type being matched",
                    span,
                    format!("a variant of {}", self.ty_name(pattern_ty)),
                )
                .with_note(format!(
                    "the value being matched has type {}",
                    self.ty_name(scrut_ty)
                ));
                self.report(diagnostic);
            }
            return bind_all_as_error(self);
        }
        let fields = self.program.enums[id].variants[index as usize]
            .fields
            .clone();
        let path = format!(
            "{}::{}",
            self.text(self.program.enums[id].name),
            self.text(variant.sym)
        );
        let binders = match binders {
            None if fields.is_empty() => {
                return Pattern::Variant {
                    variant: index,
                    binders: Vec::new(),
                };
            }
            None => {
                let diagnostic = Diagnostic::error(
                    codes::INVALID_PATTERN,
                    format!(
                        "`{path}` has {}; the pattern must name them or end with `..`",
                        plural(fields.len(), "field", "fields")
                    ),
                    span,
                    "fields not mentioned",
                )
                .with_fix("ignore them", [Edit::insert(span.hi, "(..)")]);
                self.report(diagnostic);
                return Pattern::Error;
            }
            Some(binders) => binders,
        };
        if fields.is_empty() {
            let parens = Span::new(variant.span.hi, span.hi);
            let mut diagnostic = Diagnostic::error(
                codes::INVALID_PATTERN,
                format!("`{path}` has no fields"),
                parens,
                "unexpected parentheses",
            );
            if binders.is_empty() {
                diagnostic =
                    diagnostic.with_fix("remove the parentheses", [Edit::replace(parens, "")]);
            }
            self.report(diagnostic);
            return bind_all_as_error(self);
        }
        // Binders name their fields, in any order.
        let mut locals: Vec<Binder> = (0..fields.len()).map(|_| Binder::Ignored).collect();
        let mut bound = vec![None; fields.len()];
        let mut failed = false;
        for b in binders {
            // With one field, a bare binder of any name binds it, and so
            // does a pattern that names no field.
            let bare = b.pattern.is_none() || b.field.sym == Symbol::positional();
            let only = (fields.len() == 1 && bare).then_some(0);
            let position = match b.field.sym == Symbol::positional() {
                true => None,
                false => fields.iter().position(|f| f.name == b.field.sym),
            };
            let Some(i) = position.or(only) else {
                if b.field.sym == Symbol::positional() {
                    self.name_the_field(&path, &fields, b);
                    failed = true;
                    continue;
                }
                let field = self.text(b.field.sym);
                let mut diagnostic = Diagnostic::error(
                    codes::NO_SUCH_FIELD,
                    format!("`{path}` has no field `{field}`"),
                    b.field.span,
                    "unknown field",
                );
                if let Some(similar) = suggest(field, fields.iter().map(|f| self.text(f.name))) {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `{similar}`?"),
                        [Edit::replace(b.field.span, similar)],
                    );
                } else {
                    let names: Vec<String> = fields
                        .iter()
                        .map(|f| format!("`{}`", self.text(f.name)))
                        .collect();
                    diagnostic =
                        diagnostic.with_note(format!("the fields are {}", names.join(", ")));
                }
                self.report(diagnostic);
                self.binder_failed(b);
                failed = true;
                continue;
            };
            if let Some(first) = bound[i] {
                let field = self.text(b.field.sym);
                let diagnostic = Diagnostic::error(
                    codes::PATTERN_FIELDS,
                    format!("field `{field}` is named twice"),
                    b.field.span,
                    "named again",
                )
                .with_secondary(first, "first named here");
                self.report(diagnostic);
                self.binder_failed(b);
                failed = true;
                continue;
            }
            bound[i] = Some(b.field.span);
            let field_ty = self.variant_field_ty(scrut_ty, index as usize, i);
            locals[i] = self.binder(b, field_ty, binds);
        }
        // Only when every binder landed: after an unknown or repeated
        // field, the fields left over are not the real mistake.
        if !rest && !failed {
            let missing: Vec<String> = fields
                .iter()
                .zip(&bound)
                .filter(|(_, bound)| bound.is_none())
                .map(|(f, _)| format!("`{}`", self.text(f.name)))
                .collect();
            if !missing.is_empty() {
                let diagnostic = Diagnostic::error(
                    codes::PATTERN_FIELDS,
                    format!(
                        "`{path}` has {} the pattern does not name: {}",
                        plural(missing.len(), "field", "fields"),
                        missing.join(", ")
                    ),
                    span,
                    "not every field is named",
                )
                .with_help("name them, or end the pattern with `..`");
                self.report(diagnostic);
                failed = true;
            }
        }
        if failed {
            return Pattern::Error;
        }
        Pattern::Variant {
            variant: index,
            binders: locals,
        }
    }

    /// Whether the arms cover every value, and whether each can be reached:
    /// both are whether a pattern matches something the arms before it do not.
    pub(super) fn exhaustiveness(
        &mut self,
        scrut_ty: Ty,
        scrut_span: Span,
        ast_arms: &[ast::Arm],
        arms: &[Arm],
    ) {
        if self.is_poisoned(scrut_ty) {
            return;
        }
        let tys = [scrut_ty];
        let mut matrix: Vec<Vec<usefulness::Shape>> = Vec::new();
        let mut has_error = false;
        for (ast_arm, arm) in ast_arms.iter().zip(arms) {
            if pattern_has_error(&arm.pattern) {
                has_error = true;
                continue;
            }
            let row = vec![usefulness::Shape::of(&self.program, &arm.pattern, scrut_ty)];
            if usefulness::useful(&self.program, &matrix, &row, &tys).is_none() {
                let diagnostic = Diagnostic::warning(
                    codes::UNREACHABLE_ARM,
                    "unreachable match arm",
                    ast_arm.pattern.span,
                    "never reached",
                )
                .with_note("the arms above it match every value this one does");
                self.report(diagnostic);
            }
            // An arm with a guard may not match, whatever its pattern, so
            // it covers nothing.
            if arm.guard.is_none() {
                matrix.push(row);
            }
        }
        if has_error {
            return;
        }
        let Some(witness) =
            usefulness::useful(&self.program, &matrix, &[usefulness::Shape::Wild], &tys)
        else {
            return;
        };
        // Where whole variants are left out, each is named: that is what
        // an arm for it would say, and there may be several; so is each run
        // of numbers or characters no arm has. Anything
        // deeper is one value that no arm matches.
        let mut names = self.missing_variants(scrut_ty, &matrix);
        if names.is_empty() {
            names = usefulness::missing_ranges(&self.program, &matrix, scrut_ty);
        }
        if names.is_empty() {
            names.push(witness.text(&self.program, scrut_ty, self.interner));
        }
        let listed = match names.len() {
            1..=3 => format!("`{}`", names.join("`, `")),
            n => format!("`{}` and {} more", names[..3].join("`, `"), n - 3),
        };
        let covers = match names.len() {
            1 => "is not covered",
            _ => "are not covered",
        };
        let missing = names.swap_remove(0);
        // A type with more values than can be listed — text, or a number
        // no arm narrows — is covered only by a `_` or a name.
        if missing == "_" {
            let diagnostic = Diagnostic::error(
                codes::NON_EXHAUSTIVE_MATCH,
                format!("non-exhaustive match on {}", self.ty_name(scrut_ty)),
                scrut_span,
                "needs a `_` arm",
            )
            .with_note(
                "a `match` on a value that is not an enum ends with `_` or a name: the values it does not list are the ones left",
            );
            self.report(diagnostic);
            return;
        }
        let diagnostic = Diagnostic::error(
            codes::NON_EXHAUSTIVE_MATCH,
            format!("non-exhaustive match: {listed} {covers}"),
            scrut_span,
            format!("{listed} {covers}"),
        )
        .with_help(match covers {
            "is not covered" => "add an arm for it, or a `_` arm",
            _ => "add an arm for each, or a `_` arm",
        });
        self.report(diagnostic);
    }
}

impl Lowerer<'_> {
    /// Makes a copy of each binding of plain data that its arm or block
    /// only reads: nothing assigns through it, borrows it as
    /// `&var` or lends it, and each use reads it. Its local takes the
    /// value's type, each read of it becomes a read of the local, and it is
    /// no longer an alias, so the place it came from is free to change.
    pub(super) fn settle_bindings(&mut self) {
        let settling = std::mem::take(&mut self.state.settling);
        if settling.is_empty() {
            return;
        }
        let body = &self.state.body;
        // Each local: how often it is named, and how often read through.
        let mut uses: FxHashMap<LocalId, (u32, u32)> = FxHashMap::default();
        let mut written: FxHashSet<LocalId> = FxHashSet::default();
        // The binding a place is found through, where it is one.
        let through = |mut e: ExprId| loop {
            match body.exprs[e].kind {
                ExprKind::Field { base, .. }
                | ExprKind::Index { base, .. }
                | ExprKind::SubSlice { base, .. } => e = base,
                ExprKind::Deref(inner) => match body.exprs[inner].kind {
                    ExprKind::Local(local) => return Some(local),
                    _ => e = inner,
                },
                _ => return None,
            }
        };
        for (_, expr) in body.exprs.iter() {
            match expr.kind {
                ExprKind::Local(local) => uses.entry(local).or_default().0 += 1,
                ExprKind::Deref(inner) => {
                    if let ExprKind::Local(local) = body.exprs[inner].kind {
                        uses.entry(local).or_default().1 += 1;
                    }
                }
                ExprKind::Assign { place, .. } | ExprKind::Lend(place) => {
                    written.extend(through(place));
                }
                ExprKind::Ref(place)
                    if matches!(self.kind(expr.ty), TyKind::Ref(_, crate::RefKind::Var)) =>
                {
                    written.extend(through(place));
                }
                _ => {}
            }
        }
        let copies: FxHashSet<LocalId> = settling
            .into_iter()
            .filter(|local| {
                let (named, read) = uses.get(local).copied().unwrap_or_default();
                !written.contains(local) && named == read
            })
            .collect();
        if copies.is_empty() {
            return;
        }
        let reads: Vec<ExprId> = body
            .exprs
            .iter()
            .filter_map(|(id, expr)| match expr.kind {
                ExprKind::Deref(inner) => match body.exprs[inner].kind {
                    ExprKind::Local(local) if copies.contains(&local) => Some(id),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        for id in reads {
            let ExprKind::Deref(inner) = self.state.body.exprs[id].kind else {
                continue;
            };
            let local = self.state.body.exprs[inner].kind.clone();
            self.state.body.exprs[id].kind = local;
        }
        for local in copies {
            if let TyKind::Ref(value, _) = self.kind(self.state.body.locals[local].ty) {
                self.state.body.locals[local].ty = value;
            }
            self.state.body.alias_bindings.remove(&local);
            self.state.body.aliases.remove(&local);
            self.state.body.copied_bindings.insert(local);
        }
    }
}
