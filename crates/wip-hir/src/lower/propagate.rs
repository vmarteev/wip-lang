//! `expr?`: the value, or an early return of the other variant.
//!
//! It lowers to what a reader would write: a `match` whose first arm takes
//! the value and whose second returns, so the move checker, the MIR and code
//! generation see nothing new.

use super::*;

/// The `expr?` being lowered: the enum it is of, its value, and how the
/// arms bind that value's fields.
#[derive(Clone, Copy)]
struct Tried {
    id: EnumId,
    ty: Ty,
    /// Where the value is a place, the kind of reference its bindings are:
    /// aliases for its fields.
    alias: Option<crate::RefKind>,
    scrutinee: ExprId,
    span: Span,
}

impl<'a> Lowerer<'a> {
    pub(super) fn try_expr(&mut self, value: ast::ExprId, span: Span) -> ExprId {
        let scrutinee = self.infer(value, None);
        let ty = self.ty_of(scrutinee);
        if self.is_poisoned(ty) {
            return self.error_expr(span);
        }
        let TyKind::Enum(id, args) = self.kind(ty) else {
            let diagnostic = self.not_propagatable(ty, span);
            self.report(diagnostic);
            return self.error_expr(span);
        };
        let Some(kind) = self.propagated_enum(id) else {
            let diagnostic = self.not_propagatable(ty, span);
            self.report(diagnostic);
            return self.error_expr(span);
        };
        // The function must return the same one, with the same error type.
        let ret = self.state.ret;
        let matches_result = match self.kind(ret) {
            TyKind::Enum(ret_id, ret_args) if ret_id == id => match kind {
                Propagated::Option => true,
                Propagated::Result => {
                    self.program.types.list(args).get(1) == self.program.types.list(ret_args).get(1)
                }
            },
            _ => false,
        };
        // Where the error types differ, the function's error type may say
        // how it is made from this one.
        let conversion = match (matches_result, kind) {
            (false, Propagated::Result) => self.error_conversion(ty, ret, id),
            _ => None,
        };
        if !matches_result && conversion.is_none() {
            let diagnostic = self.wrong_result(ty, ret, kind, span);
            self.report(diagnostic);
            return self.error_expr(span);
        }
        let (taken, other) = match kind {
            // `.Some` is the first variant, `.None` the second; `.Ok` and
            // `.Err` likewise.
            Propagated::Option | Propagated::Result => (0u32, 1u32),
        };
        // A value that owns memory is taken; plain data is read where it is,
        // as a `match` on a place does.
        let scrutinee = if self.state.body.is_place(scrutinee) && self.owns(ty) {
            let span = self.state.body.exprs[scrutinee].span;
            self.alloc(ExprKind::Move(scrutinee), ty, span)
        } else {
            scrutinee
        };
        let tried = Tried {
            id,
            ty,
            alias: self.alias_kind(scrutinee),
            scrutinee,
            span,
        };
        // The arm that takes the value.
        let value_ty = self.program.variant_field_ty(ty, taken, 0);
        // The arms' bindings are named as the variants' fields are —
        // `value`, `error` — in a scope of their own, so that they do not
        // hide a program's own `value` after the `?`.
        self.state.scopes.push(FxHashMap::default());
        let (local, body) = self.field_binding(&tried, taken);
        let taken_arm = Arm {
            pattern: Pattern::Variant {
                variant: taken,
                binders: vec![crate::Binder::Bind(local)],
            },
            guard: None,
            body,
            span,
        };
        // The arm that returns what was not taken.
        let other_arm = self.returning_arm(&tried, other, ret, conversion);
        self.state.scopes.pop();
        self.alloc(
            ExprKind::Match {
                scrutinee,
                arms: vec![taken_arm, other_arm],
            },
            value_ty,
            span,
        )
    }

    /// The arm that returns: `.None` as it is, and `.Err(error)` with the
    /// error it was given.
    fn returning_arm(
        &mut self,
        tried: &Tried,
        variant: u32,
        ret: Ty,
        conversion: Option<FnId>,
    ) -> Arm {
        let Tried { id, span, .. } = *tried;
        let fields = self.program.enums[id].variants[variant as usize]
            .fields
            .len();
        let (binders, args) = if fields == 0 {
            (Vec::new(), Vec::new())
        } else {
            let (local, mut error) = self.field_binding(tried, variant);
            // `extend TheirError: From<ThisError>`: the error is made
            // into the one the function returns.
            if let Some(from) = conversion {
                let made = self.program.fns[from].ret;
                error = self.alloc(
                    ExprKind::Call {
                        callee: from,
                        args: vec![error],
                        type_args: crate::TyList::EMPTY,
                        order: Vec::new(),
                    },
                    made,
                    span,
                );
            }
            (vec![crate::Binder::Bind(local)], vec![error])
        };
        let returned = self.alloc(
            ExprKind::Variant {
                id,
                variant,
                args,
                order: Vec::new(),
            },
            ret,
            span,
        );
        // `return` is a statement, so the arm is a block of one.
        let stmt = self.state.body.stmts.alloc(Stmt {
            kind: StmtKind::Return(Some(returned)),
            span,
        });
        let body = self.alloc(
            ExprKind::Block(Block {
                stmts: vec![stmt],
                value: None,
                span,
            }),
            Types::NEVER,
            span,
        );
        Arm {
            pattern: Pattern::Variant { variant, binders },
            guard: None,
            body,
            span,
        }
    }

    /// The binding of a variant's one field, `value` or `error`, and the
    /// value read from it.
    fn field_binding(&mut self, tried: &Tried, variant: u32) -> (LocalId, ExprId) {
        let Tried {
            id,
            ty,
            alias,
            scrutinee,
            span,
        } = *tried;
        let field_ty = self.program.variant_field_ty(ty, variant, 0);
        let bound_ty = match alias {
            Some(kind) => self.intern(TyKind::Ref(field_ty, kind)),
            None => field_ty,
        };
        let name = self.program.enums[id].variants[variant as usize].fields[0].name;
        let local = self.declare(name, bound_ty, LocalKind::Binding, span);
        if alias.is_some() {
            self.state.body.aliases.insert(local, scrutinee);
            self.state.body.alias_bindings.insert(local);
        }
        let mut value = self.alloc(ExprKind::Local(local), bound_ty, span);
        // An alias is read through, as a `match` binding is; a field that is
        // itself a reference, `Option<&Json>`'s, is the value, and copied
        // as a view is.
        if alias.is_some() {
            value = self.alloc(ExprKind::Deref(value), field_ty, span);
        } else if self.owns(bound_ty) {
            // The binding owns its field, which is taken from it.
            value = self.alloc(ExprKind::Move(value), bound_ty, span);
        }
        (local, value)
    }

    /// The error type of a `Result`, which is its second type argument.
    fn error_of(&self, ty: Ty) -> Option<Ty> {
        let TyKind::Enum(id, args) = self.kind(ty) else {
            return None;
        };
        self.propagated_enum(id)
            .filter(|kind| *kind == Propagated::Result)?;
        self.program.types.list(args).get(1).copied()
    }

    /// The `from` of `extend TheirError: From<ThisError>`, where the
    /// function returns a `Result` whose error type says how it is made
    /// from this one.
    fn error_conversion(&mut self, ty: Ty, ret: Ty, result: EnumId) -> Option<FnId> {
        let interface = self.program.prelude_items.interface(KnownInterface::From)?;
        let TyKind::Enum(ret_id, ret_args) = self.kind(ret) else {
            return None;
        };
        if ret_id != result {
            return None;
        }
        let TyKind::Enum(_, args) = self.kind(ty) else {
            return None;
        };
        let source = *self.program.types.list(args).get(1)?;
        let target = *self.program.types.list(ret_args).get(1)?;
        let owner = self.owner_of(target)?;
        let wanted = self.program.types.intern_list(&[source]);
        let found = self
            .program
            .impls
            .iter()
            .find(|i| i.interface == interface && i.ty == owner && i.args == wanted)?;
        let method = *found.methods.first()?;
        // The conversion is the interface's one method, and an
        // implementation that only has the interface's default would
        // convert nothing.
        self.program.fns[method].body.is_some().then_some(method)
    }

    /// Which of the prelude's two types an enum is, if it is one.
    fn propagated_enum(&self, id: EnumId) -> Option<Propagated> {
        let prelude = self.prelude?;
        let name = self.program.enums[id].name;
        let declared = |what: &str| {
            self.modules[prelude]
                .types
                .get(&name)
                .is_some_and(|&(def, _)| def == TypeDef::Enum(id))
                && self.text(name) == what
        };
        if declared("Option") {
            Some(Propagated::Option)
        } else if declared("Result") {
            Some(Propagated::Result)
        } else {
            None
        }
    }

    fn not_propagatable(&self, ty: Ty, span: Span) -> Diagnostic {
        Diagnostic::error(
            codes::CANNOT_PROPAGATE,
            format!(
                "`?` needs an `Option` or a `Result`, and this is {}",
                self.ty_name(ty)
            ),
            span,
            "not an `Option` or a `Result`",
        )
        .with_note("`?` takes the value, or returns the other variant to the caller")
    }

    fn wrong_result(&mut self, ty: Ty, ret: Ty, kind: Propagated, span: Span) -> Diagnostic {
        let (value, result) = (self.ty_name(ty), self.ty_name(ret));
        let diagnostic = Diagnostic::error(
            codes::CANNOT_PROPAGATE,
            format!("`?` on {value} needs the function to return one too"),
            span,
            format!("this function returns {result}"),
        );
        match kind {
            Propagated::Result => {
                // Where the two error types are known, the help names the
                // implementation that would bridge them.
                let names = match (self.error_of(ty), self.error_of(ret)) {
                    // The names go inside an `extend` the help writes out, so
                    // they are written plainly.
                    (Some(source), Some(target)) => Some((
                        self.ty_name(source).trim_matches('`').to_string(),
                        self.ty_name(target).trim_matches('`').to_string(),
                    )),
                    _ => None,
                };
                let help = match names {
                    Some((source, target)) => format!(
                        "write `extend {target}: From<{source}>`, and `?` will use it; or convert it here with `mapErr`"
                    ),
                    None => "convert it first, with `mapErr`".to_string(),
                };
                diagnostic
                    .with_note(
                        "`?` returns the error as it is, unless the function's error type says how it is made from this one",
                    )
                    .with_help(help)
            }
            Propagated::Option => diagnostic.with_note(
                "`?` on an `Option` returns `.None`, so the function must return an `Option`",
            ),
        }
    }
}

/// The two types `?` knows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Propagated {
    Option,
    Result,
}
