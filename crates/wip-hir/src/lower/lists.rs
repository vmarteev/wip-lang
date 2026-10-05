//! A list built as it runs: `own [ … ]` whose elements
//! include statements that `yield`, and `own for`, whose loop does. A
//! `yield` hands its value to the innermost list being built. The list is
//! written into what the rest of the compiler already knows, a vector
//! that is filled and then handed over as a buffer:
//!
//! ```text
//! own [a, if c { yield b }, for x in xs { yield f(x) }]
//!     ⇒ { var list# = Vec<T>::withCapacity(1)
//!         list#.push(a)
//!         if c { list#.push(b) }
//!         for x in xs { list#.push(f(x)) }
//!         list#.intoBuffer() }
//! ```
//!
//! `if`, `for` and the rest are the statements they always are; only
//! `yield` is new. `T` is what is expected of the list, or else the type
//! of the first value that goes into it.

use super::*;

/// A list being built: its variable, and its element's type once known.
pub(super) struct ListBuild {
    local: LocalId,
    elem: Option<Ty>,
}

/// Whether an element is a statement that gives the list its elements by
/// `yield` — a `for`, or an `if`, a `match` or a block with a `yield`
/// inside — rather than a value that is one element.
pub(super) fn yields(ast: &Ast, e: ast::ExprId) -> bool {
    match &ast.exprs[e].kind {
        ast::ExprKind::ForElement { .. } => true,
        ast::ExprKind::If {
            then_block,
            else_branch,
            ..
        } => block_yields(ast, then_block) || else_branch.is_some_and(|e| yields(ast, e)),
        ast::ExprKind::Block(block) => block_yields(ast, block),
        ast::ExprKind::Match { arms, .. } => arms.iter().any(|arm| yields(ast, arm.body)),
        _ => false,
    }
}

/// Whether a block has a `yield` in it, not counting one inside a list of
/// its own or a lambda, which yield elsewhere.
fn block_yields(ast: &Ast, block: &ast::Block) -> bool {
    block.stmts.iter().any(|&stmt| match &ast.stmts[stmt].kind {
        ast::StmtKind::Yield(_) => true,
        ast::StmtKind::For { body, .. } | ast::StmtKind::While { body, .. } => {
            block_yields(ast, body)
        }
        ast::StmtKind::Expr(e) => yields(ast, *e),
        _ => false,
    })
}

impl<'a> Lowerer<'a> {
    /// `own [ … ]` with an element that yields, or `own for`: an
    /// `own<[T]>`, built as above.
    pub(super) fn built_list(
        &mut self,
        elems: &[ast::ExprId],
        hint: Option<Ty>,
        span: Span,
    ) -> ExprId {
        let elem_hint = match hint.map(|h| self.kind(h)) {
            Some(TyKind::Own(inner)) => match self.kind(inner) {
                TyKind::Slice(t) | TyKind::Array(t, _) => Some(t),
                _ => None,
            },
            _ => None,
        };
        let Some(TypeDef::Struct(vec)) = self.prelude_type(Symbol::vec_type()) else {
            // The prelude builds nothing with this; it is not for it.
            let diagnostic = Diagnostic::error(
                codes::CANNOT_INFER,
                "a list built as it runs needs the prelude's `Vec`",
                span,
                "not here",
            );
            self.report(diagnostic);
            return self.error_expr(span);
        };
        let local = self.state.body.locals.alloc(Local {
            name: Symbol::list(),
            ty: Types::ERROR,
            kind: LocalKind::Var,
            span,
        });
        self.state.lists.push(ListBuild {
            local,
            elem: elem_hint,
        });
        let mut stmts = Vec::new();
        for &e in elems {
            stmts.push(self.element(e));
        }
        let build = self.state.lists.pop().expect("pushed above");
        let Some(elem) = build.elem else {
            // Nothing went in to go by, and nothing is expected.
            let diagnostic = Diagnostic::error(
                codes::CANNOT_INFER,
                "cannot infer the element type of this list",
                span,
                "type unknown",
            )
            .with_help("say what it holds: `val rows: own<[Row]> = own [ … ]`");
            self.report(diagnostic);
            return self.error_expr(span);
        };
        if self.is_poisoned(elem) {
            return self.error_expr(span);
        }
        let args = self.program.types.intern_list(&[elem]);
        let vec_ty = self.intern(TyKind::Struct(vec, args));
        self.state.body.locals[local].ty = vec_ty;
        let (Some(with_capacity), Some(into_buffer)) = (
            self.vec_method(vec, Symbol::with_capacity()),
            self.vec_method(vec, Symbol::into_buffer()),
        ) else {
            return self.error_expr(span);
        };

        // `var list# = Vec<T>::withCapacity(n)`, room for the elements
        // that are one each.
        let plain = elems.iter().filter(|&&e| !yields(self.ast, e)).count();
        let count = self.alloc(ExprKind::Int(plain as u128), Types::I64, span);
        let init = self.alloc(
            ExprKind::Call {
                callee: with_capacity,
                args: vec![count],
                type_args: args,
                order: vec![0],
            },
            vec_ty,
            span,
        );
        let declare = self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Let { local, init },
            span,
        });
        stmts.insert(0, declare);

        // `list#.intoBuffer()`, which takes the vector.
        let taken = self.alloc(ExprKind::Local(local), vec_ty, span);
        let moved = self.alloc(ExprKind::Move(taken), vec_ty, span);
        let slice = self.intern(TyKind::Slice(elem));
        let ty = self.intern(TyKind::Own(slice));
        let value = self.alloc(
            ExprKind::Call {
                callee: into_buffer,
                args: vec![moved],
                type_args: args,
                order: vec![0],
            },
            ty,
            span,
        );
        let block = Block {
            stmts,
            value: Some(value),
            span,
        };
        self.alloc(ExprKind::Block(block), ty, span)
    }

    /// One of the prelude `Vec`'s functions, by name.
    fn vec_method(&self, vec: StructId, name: Symbol) -> Option<FnId> {
        self.methods_named(TypeDef::Struct(vec), name)
            .first()
            .copied()
    }

    /// The statement an element is: a loop or a branch whose `yield`s put
    /// elements in, or `list#.push(value)` for a value.
    fn element(&mut self, e: ast::ExprId) -> StmtId {
        let ast = self.ast;
        let span = ast.exprs[e].span;
        match &ast.exprs[e].kind {
            ast::ExprKind::ForElement {
                binding,
                source,
                body,
            } => {
                let kind = self.for_stmt(None, binding, *source, body);
                self.state.body.stmts.alloc(Stmt { kind, span })
            }
            _ if yields(ast, e) => {
                let id = self.discarded(e, None);
                self.state.body.stmts.alloc(Stmt {
                    kind: StmtKind::Expr(id),
                    span,
                })
            }
            ast::ExprKind::If { .. } if body::lacks_else(ast, e) => {
                self.adds_nothing(span);
                let id = self.discarded(e, None);
                self.state.body.stmts.alloc(Stmt {
                    kind: StmtKind::Expr(id),
                    span,
                })
            }
            _ => {
                let value = self.list_value(e);
                self.pushed(value, span)
            }
        }
    }

    /// `if c { x }` among a list's elements, as it might be read: an `if`
    /// without `else` has no value, and hands none over.
    pub(super) fn adds_nothing(&mut self, span: Span) {
        let diagnostic = Diagnostic::error(
            codes::ELEMENT_ADDS_NOTHING,
            "an `if` without `else` adds nothing to a list",
            span,
            "no value, and no `yield`",
        )
        .with_help("hand the value to a list built as it runs, `own [if c { yield x }]`, or give the `if` an `else`")
        .with_note("an `if` without `else` is a statement; among a list's elements, what it yields is what it adds");
        self.report(diagnostic);
    }

    /// `yield value`: to the innermost list being built.
    pub(super) fn yield_stmt(&mut self, value: ast::ExprId, span: Span) -> StmtKind {
        // A list built inside a generator takes its own `yield`s; the rest
        // are the generator's.
        if self.state.lists.is_empty() && self.state.generator.is_some() {
            return self.generator_yield(value, span);
        }
        if self.state.lists.is_empty() {
            let diagnostic = Diagnostic::error(
                codes::YIELD_OUTSIDE_LIST,
                "`yield` with no list being built, and no generator",
                span,
                "nothing to hand the value to",
            )
            .with_note("`yield` hands a value to the list being built around it, `own [ … ]` or `own for`, or to whoever asks a generator for its next one: a loop where a value stands, or a function that answers `Iterator<T>`");
            self.report(diagnostic);
            // Checked for what else is wrong with it, and nothing more said.
            let id = self.infer(value, None);
            return StmtKind::Expr(self.wrap_discarded(id));
        }
        let value = self.list_value(value);
        StmtKind::Expr(self.push_call(value, span))
    }

    /// A value that goes into the list, checked against its element type,
    /// which the first one sets where nothing is expected.
    fn list_value(&mut self, e: ast::ExprId) -> ExprId {
        let build = self.state.lists.last().expect("inside a list");
        match build.elem {
            Some(t) => self.check(e, t),
            None => {
                let value = self.infer(e, None);
                let ty = self.ty_of(value);
                if let Some(build) = self.state.lists.last_mut() {
                    build.elem = Some(ty);
                }
                value
            }
        }
    }

    /// `list#.push(value);`, as a statement.
    fn pushed(&mut self, value: ExprId, span: Span) -> StmtId {
        let expr = self.push_call(value, span);
        self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Expr(expr),
            span,
        })
    }

    /// `list#.push(value)`.
    fn push_call(&mut self, value: ExprId, span: Span) -> ExprId {
        let build = self.state.lists.last().expect("inside a list");
        let (local, elem) = (build.local, build.elem.unwrap_or(Types::ERROR));
        let found = match self.prelude_type(Symbol::vec_type()) {
            Some(TypeDef::Struct(vec)) => {
                self.vec_method(vec, Symbol::push()).map(|push| (vec, push))
            }
            _ => None,
        };
        match found {
            Some((vec, push)) if !self.is_poisoned(elem) => {
                let args = self.program.types.intern_list(&[elem]);
                let vec_ty = self.intern(TyKind::Struct(vec, args));
                let list = self.alloc(ExprKind::Local(local), vec_ty, span);
                let lent_ty = self.intern(TyKind::Ref(vec_ty, crate::RefKind::Var));
                let lent = self.alloc(ExprKind::Ref(list), lent_ty, span);
                self.alloc(
                    ExprKind::Call {
                        callee: push,
                        args: vec![lent, value],
                        type_args: args,
                        order: vec![0, 1],
                    },
                    Types::UNIT,
                    span,
                )
            }
            _ => value,
        }
    }

    /// A list whose length its `yield`s decide, where no `own` builds it:
    /// an array literal, or a loop standing for a value.
    pub(super) fn needs_own(&mut self, at: Span, whole: Span, what: &str) -> ExprId {
        let diagnostic = Diagnostic::error(codes::LIST_NEEDS_OWN, format!("{what} is built with `own`"), at, "its `yield`s decide how long it is")
            .with_fix("build it on the heap, with `own`", [Edit::insert(whole.lo, "own ")])
            .with_note("a list whose `yield`s decide its length is an `own<[T]>`, built as it runs, and `own` says where it goes; a loop standing alone for a value is a generator, which yields one value at a time as it is asked");
        self.report(diagnostic);
        self.error_expr(whole)
    }
}
