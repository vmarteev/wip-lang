//! `for` over an iterator: what lends no slice is walked by
//! calling `next()` until it answers nothing.
//!
//! The loop is written into what the rest of the compiler already knows:
//!
//! ```text
//! for x in it { body }
//!     ⇒ { var iterator# = it                  // only where `it` is not a place
//!         while true {
//!             match iterator#.next() {
//!                 .Some(x) => { body },
//!                 .None => { break },
//!             }
//!         } }
//! ```
//!
//! so the move checker, the MIR and the code generator see a loop and a
//! `match`, and nothing new. A place is walked where it is: the loop moves
//! it on, and after a `break` it is where the loop left it.

use super::*;

/// A type's `next`, as `for` calls it.
pub(super) struct Next {
    method: FnId,
    type_args: crate::TyList,
    /// What `next` answers: `Option<T>`.
    answer: Ty,
    /// `T`.
    elem: Ty,
}

/// A type's `len` and `at`, as `for` calls them.
pub(super) struct SequenceOf {
    len: FnId,
    place_at: FnId,
    type_args: crate::TyList,
    /// `T`.
    elem: Ty,
}

impl<'a> Lowerer<'a> {
    /// How `ty` is walked one element at a time, if it implements the
    /// prelude's `Iterator`, directly or through a constraint.
    pub(super) fn iterator_of(&mut self, ty: Ty) -> Option<Next> {
        let interface = self
            .program
            .prelude_items
            .interface(KnownInterface::Iterator)?;
        let (method, args) = match self.kind(ty) {
            TyKind::Param(param) => {
                let elem = self
                    .type_params
                    .get(param.index as usize)
                    .and_then(|p| p.interfaces.iter().find(|c| c.interface == interface))
                    .and_then(|c| self.program.types.list(c.args).first().copied())?;
                let method = self.program.interfaces[interface].methods[0].id;
                (method, vec![ty, elem])
            }
            _ => {
                let owner = self.owner_of(ty)?;
                let found: Vec<&crate::ImplDef> = self
                    .program
                    .impls
                    .iter()
                    .filter(|i| i.interface == interface && i.ty == owner)
                    .collect();
                // Which kind of element must not be in question, as for
                // `Items`.
                if found.len() != 1 {
                    return None;
                }
                let method = found[0].methods.first().copied()?;
                let args = match self.kind(ty) {
                    TyKind::Struct(_, args) | TyKind::Enum(_, args) => {
                        self.program.types.list(args).to_vec()
                    }
                    _ => Vec::new(),
                };
                (method, args)
            }
        };
        let answer = self
            .program
            .types
            .subst(self.program.fns[method].ret, &args);
        let TyKind::Enum(option, elems) = self.kind(answer) else {
            return None;
        };
        if Some(option) != self.program.prelude_items.enumeration(KnownEnum::Option) {
            return None;
        }
        let elem = *self.program.types.list(elems).first()?;
        Some(Next {
            method,
            type_args: self.program.types.intern_list(&args),
            answer,
            elem,
        })
    }

    /// `c.intoIterator()`, where `c`, moved, gives up its elements to an
    /// iterator: the call, which takes `c`, and how the
    /// iterator it answers is walked. `None` where `c`'s type does not.
    pub(super) fn given_up_iterator(&mut self, moved: ExprId) -> Option<(ExprId, Next)> {
        let interface = self
            .program
            .prelude_items
            .interface(KnownInterface::IntoIterator)?;
        let ty = self.ty_of(moved);
        // A type parameter gives them up because a constraint says so, and
        // the call goes through the interface's own method, which its
        // instance replaces.
        let (method, args) = match self.kind(ty) {
            TyKind::Param(param) => {
                let iterator = self
                    .type_params
                    .get(param.index as usize)
                    .and_then(|p| p.interfaces.iter().find(|c| c.interface == interface))
                    .and_then(|c| self.program.types.list(c.args).first().copied())?;
                let method = self.program.interfaces[interface].methods[0].id;
                (method, vec![ty, iterator])
            }
            _ => {
                let owner = self.owner_of(ty)?;
                let found: Vec<&crate::ImplDef> = self
                    .program
                    .impls
                    .iter()
                    .filter(|i| i.interface == interface && i.ty == owner)
                    .collect();
                if found.len() != 1 {
                    return None;
                }
                let method = found[0].methods.first().copied()?;
                (method, self.owner_args(ty))
            }
        };
        let iterator = self
            .program
            .types
            .subst(self.program.fns[method].ret, &args);
        let next = self.iterator_of(iterator)?;
        let span = self.state.body.exprs[moved].span;
        let type_args = self.program.types.intern_list(&args);
        let call = self.alloc(
            ExprKind::Call {
                callee: method,
                args: vec![moved],
                type_args,
                order: vec![0],
            },
            iterator,
            span,
        );
        Some((call, next))
    }

    /// `for binding in walked { body }`, where `walked` is an iterator:
    /// the loop above.
    pub(super) fn walk_iterator(
        &mut self,
        label: Option<&ast::Name>,
        binding: &'a ast::Pattern,
        walked: ExprId,
        next: Next,
        body: &'a ast::Block,
    ) -> StmtKind {
        let span = self.state.body.exprs[walked].span;
        let ty = self.ty_of(walked);
        self.state.scopes.push(FxHashMap::default());
        let mut stmts = Vec::new();
        // An iterator made on the heap where the loop is written, `own
        // walk(child)`, is walked in a variable of the loop's own that
        // holds the `own`: a recursive generator's.
        let place = if let ExprKind::Deref(owned) = self.state.body.exprs[walked].kind
            && matches!(self.kind(self.ty_of(owned)), TyKind::Own(_))
            && matches!(self.place_root(owned), PlaceRoot::NotAPlace)
        {
            let owned_ty = self.ty_of(owned);
            let local = self.state.body.locals.alloc(Local {
                name: Symbol::walked(),
                ty: owned_ty,
                kind: LocalKind::Var,
                span,
            });
            stmts.push(self.state.body.stmts.alloc(Stmt {
                kind: StmtKind::Let { local, init: owned },
                span,
            }));
            let held = self.alloc(ExprKind::Local(local), owned_ty, span);
            self.alloc(ExprKind::Deref(held), ty, span)
        // A place is walked where it is, so it must be one that can change;
        // anything else is walked in a variable of the loop's own.
        } else if matches!(self.place_root(walked), PlaceRoot::NotAPlace) {
            let local = self.state.body.locals.alloc(Local {
                name: Symbol::walked(),
                ty,
                kind: LocalKind::Var,
                span,
            });
            stmts.push(self.state.body.stmts.alloc(Stmt {
                kind: StmtKind::Let {
                    local,
                    init: walked,
                },
                span,
            }));
            self.alloc(ExprKind::Local(local), ty, span)
        } else {
            if !self.writable(walked) {
                self.walked_unchangeable(walked);
            }
            walked
        };
        let lent_ty = self.intern(TyKind::Ref(ty, crate::RefKind::Var));
        let lent = self.alloc(ExprKind::Ref(place), lent_ty, span);
        let call = self.alloc(
            ExprKind::Call {
                callee: next.method,
                args: vec![lent],
                type_args: next.type_args,
                order: vec![0],
            },
            next.answer,
            span,
        );
        // Each element is the loop's own: the binding takes it, as a
        // `match` on a value does.
        let inner = self.pattern(binding, next.elem, matching::Binds::default());
        let binder = match inner {
            Pattern::Binding(local) => Binder::Bind(local),
            Pattern::Wildcard => Binder::Ignored,
            other => Binder::Nested(other),
        };
        let (some, none) = self.option_variants();
        let body_span = body.span;
        let user = self.loop_body(label, body);
        let user = self.alloc(ExprKind::Block(user), Types::UNIT, body_span);
        let stop = self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Break { depth: 0 },
            span,
        });
        let stop = self.alloc(
            ExprKind::Block(Block {
                stmts: vec![stop],
                value: None,
                span,
            }),
            Types::NEVER,
            span,
        );
        let arms = vec![
            Arm {
                pattern: Pattern::Variant {
                    variant: some,
                    binders: vec![binder],
                },
                guard: None,
                body: user,
                span: body_span,
            },
            Arm {
                pattern: Pattern::Variant {
                    variant: none,
                    binders: Vec::new(),
                },
                guard: None,
                body: stop,
                span,
            },
        ];
        let step = self.alloc(
            ExprKind::Match {
                scrutinee: call,
                arms,
            },
            Types::UNIT,
            body_span,
        );
        let step = self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Expr(step),
            span: body_span,
        });
        let cond = self.alloc(ExprKind::Bool(true), Types::BOOL, span);
        stmts.push(self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::While {
                cond,
                body: Block {
                    stmts: vec![step],
                    value: None,
                    span: body_span,
                },
            },
            span: span.to(body_span),
        }));
        self.state.scopes.pop();
        let whole = self.alloc(
            ExprKind::Block(Block {
                stmts,
                value: None,
                span: span.to(body_span),
            }),
            Types::UNIT,
            span.to(body_span),
        );
        StmtKind::Expr(whole)
    }

    /// How `ty` is walked one place at a time, if it
    /// implements the prelude's `Sequence`, directly or through a constraint.
    pub(super) fn sequence_of(&mut self, ty: Ty) -> Option<SequenceOf> {
        let interface = self
            .program
            .prelude_items
            .interface(KnownInterface::Sequence)?;
        let (len, place_at, args) = match self.kind(ty) {
            TyKind::Param(param) => {
                let elem = self
                    .type_params
                    .get(param.index as usize)
                    .and_then(|p| p.interfaces.iter().find(|c| c.interface == interface))
                    .and_then(|c| self.program.types.list(c.args).first().copied())?;
                let methods = &self.program.interfaces[interface].methods;
                let named = |name: &str| {
                    methods
                        .iter()
                        .find(|m| self.text(self.program.fns[m.id].name) == name)
                        .map(|m| m.id)
                };
                (named("len")?, named("at")?, vec![ty, elem])
            }
            _ => {
                let owner = self.owner_of(ty)?;
                let found: Vec<&crate::ImplDef> = self
                    .program
                    .impls
                    .iter()
                    .filter(|i| i.interface == interface && i.ty == owner)
                    .collect();
                // Which kind of element must not be in question, as for
                // `Items`.
                if found.len() != 1 {
                    return None;
                }
                let named = |name: &str| {
                    found[0]
                        .methods
                        .iter()
                        .copied()
                        .find(|&m| self.text(self.program.fns[m].name) == name)
                };
                let (len, place_at) = (named("len")?, named("at")?);
                let args = match self.kind(ty) {
                    TyKind::Struct(_, args) | TyKind::Enum(_, args) => {
                        self.program.types.list(args).to_vec()
                    }
                    _ => Vec::new(),
                };
                (len, place_at, args)
            }
        };
        let lent = self
            .program
            .types
            .subst(self.program.fns[place_at].ret, &args);
        let TyKind::Ref(elem, _) = self.kind(lent) else {
            return None;
        };
        Some(SequenceOf {
            len,
            place_at,
            type_args: self.program.types.intern_list(&args),
            elem,
        })
    }

    /// `for binding in walked { body }`, where `walked` lends its elements
    /// one place at a time:
    ///
    /// ```text
    /// { var index# = 0
    ///   while index# < walked.len() {
    ///       index# += 1
    ///       match walked.at(index# - 1) { binding => body }
    ///   } }
    /// ```
    ///
    /// The binding is an alias of the place, for the whole body, as a `match`
    /// binding of a place is, and the place is lent for writing where `walked`
    /// can be written and its type has the writing half. The count goes up
    /// before the body, so that `continue` moves on.
    pub(super) fn walk_sequence(
        &mut self,
        label: Option<&ast::Name>,
        binding: &'a ast::Pattern,
        walked: ExprId,
        sequence: SequenceOf,
        body: &'a ast::Block,
    ) -> StmtKind {
        let span = self.state.body.exprs[walked].span;
        let ty = self.ty_of(walked);
        self.state.scopes.push(FxHashMap::default());
        let mut stmts = Vec::new();
        // What is not a place is walked in a variable of the loop's own,
        // which the loop may write.
        let place = if matches!(self.place_root(walked), PlaceRoot::NotAPlace) {
            let local = self.state.body.locals.alloc(Local {
                name: Symbol::walked(),
                ty,
                kind: LocalKind::Var,
                span,
            });
            stmts.push(self.state.body.stmts.alloc(Stmt {
                kind: StmtKind::Let {
                    local,
                    init: walked,
                },
                span,
            }));
            self.alloc(ExprKind::Local(local), ty, span)
        } else {
            walked
        };
        let index = self.state.body.locals.alloc(Local {
            name: Symbol::walked(),
            ty: Types::I64,
            kind: LocalKind::Var,
            span,
        });
        let zero = self.alloc(ExprKind::Int(0), Types::I64, span);
        stmts.push(self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Let {
                local: index,
                init: zero,
            },
            span,
        }));
        let shared = self.intern(TyKind::Ref(ty, crate::RefKind::Shared));

        // `index# < walked.len()`
        let counted = self.alloc(ExprKind::Local(index), Types::I64, span);
        let lent = self.alloc(ExprKind::Ref(place), shared, span);
        let len = self.alloc(
            ExprKind::Call {
                callee: sequence.len,
                args: vec![lent],
                type_args: sequence.type_args,
                order: vec![0],
            },
            Types::I64,
            span,
        );
        let cond = self.alloc(
            ExprKind::Binary {
                op: BinaryOp::Lt,
                lhs: counted,
                rhs: len,
                wrapping: false,
            },
            Types::BOOL,
            span,
        );

        // `index# += 1`
        let target = self.alloc(ExprKind::Local(index), Types::I64, span);
        let one = self.alloc(ExprKind::Int(1), Types::I64, span);
        let step = self.alloc(
            ExprKind::Assign {
                place: target,
                op: Some(BinaryOp::Add),
                wrapping: false,
                value: one,
            },
            Types::UNIT,
            span,
        );
        let step = self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Expr(step),
            span,
        });

        // `walked.at(index# - 1)`, the writing half where it can be.
        let current = self.alloc(ExprKind::Local(index), Types::I64, span);
        let one = self.alloc(ExprKind::Int(1), Types::I64, span);
        let at = self.alloc(
            ExprKind::Binary {
                op: BinaryOp::Sub,
                lhs: current,
                rhs: one,
                wrapping: false,
            },
            Types::I64,
            span,
        );
        let lent = self.alloc(ExprKind::Ref(place), shared, span);
        let reads = self.intern(TyKind::Ref(sequence.elem, crate::RefKind::Shared));
        let call = self.alloc(
            ExprKind::Call {
                callee: sequence.place_at,
                args: vec![lent, at],
                type_args: sequence.type_args,
                order: vec![0, 1],
            },
            reads,
            span,
        );
        // The writing half, where the walked place can be written; the
        // receivers along the way are switched to theirs too.
        self.swap_in_writer(call);
        let element = self.alloc(ExprKind::Deref(call), sequence.elem, span);

        // `match … { binding => body }`: the binding refers to the place,
        // and can write it where the place can be written, as a `for`
        // binding of a slice's element does.
        let kind = match self.writable(element) {
            true => crate::RefKind::Var,
            false => crate::RefKind::Shared,
        };
        let element_ty = self.intern(TyKind::Ref(sequence.elem, kind));
        // Writing through the binding writes the element, which is what the
        // walk is for: the container, which the `match` holds, is what may
        // not change (E0414).
        let pattern = self.element_pattern(binding, element_ty, Some(sequence.elem), kind, None);
        let body_span = body.span;
        let user = self.loop_body(label, body);
        let user = self.alloc(ExprKind::Block(user), Types::UNIT, body_span);
        let visit = self.alloc(
            ExprKind::Match {
                scrutinee: element,
                arms: vec![Arm {
                    pattern,
                    guard: None,
                    body: user,
                    span: body_span,
                }],
            },
            Types::UNIT,
            body_span,
        );
        self.state.body.sequence_walks.insert(visit);
        let visit = self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Expr(visit),
            span: body_span,
        });
        stmts.push(self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::While {
                cond,
                body: Block {
                    stmts: vec![step, visit],
                    value: None,
                    span: body_span,
                },
            },
            span: span.to(body_span),
        }));
        self.state.scopes.pop();
        let whole = self.alloc(
            ExprKind::Block(Block {
                stmts,
                value: None,
                span: span.to(body_span),
            }),
            Types::UNIT,
            span.to(body_span),
        );
        StmtKind::Expr(whole)
    }

    /// Where `Option`'s `Some` and `None` are among its variants.
    pub(super) fn option_variants(&self) -> (u32, u32) {
        let option = self
            .program
            .prelude_items
            .enumeration(KnownEnum::Option)
            .expect("an iterator answers the prelude's `Option`");
        let variants = &self.program.enums[option].variants;
        let at = |name: &str| {
            variants
                .iter()
                .position(|v| self.text(v.name) == name)
                .expect("`Option` has `Some` and `None`") as u32
        };
        (at("Some"), at("None"))
    }

    /// A place a `for` would walk, which cannot change: `val words`.
    fn walked_unchangeable(&mut self, walked: ExprId) {
        let span = self.state.body.exprs[walked].span;
        let mut diagnostic = Diagnostic::error(
            codes::NOT_ITERABLE,
            "a `for` over an iterator moves it on, and this cannot change",
            span,
            "walked where it is",
        )
        .with_note("each pass calls `next()`, which changes the iterator; a variable is walked where it is, so after a `break` it is where the loop left it");
        if let ExprKind::Local(local) = self.state.body.exprs[walked].kind
            && let LocalKind::Let { keyword } = self.state.body.locals[local].kind
        {
            diagnostic =
                diagnostic.with_fix("declare it with `var`", [Edit::replace(keyword, "var")]);
        }
        self.report(diagnostic);
    }
}
