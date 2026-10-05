//! Reference arguments and `&var` parameters.

use super::lends::Lends;
use super::*;

/// Whether `path` goes into a payload field of the place `matched`, or its
/// elements. Only the bindings of a `match` on that place, or of one nested
/// inside it, reach there, so the `match` may write through them.
fn into_payload(path: &Path, matched: &Path) -> bool {
    path.within(matched)
        && matches!(
            path.projs.get(matched.projs.len()),
            Some(Proj::Payload(_) | Proj::Element | Proj::Elements)
        )
}

impl Checker<'_> {
    /// The arguments of a call, `params` in the order of the parameters of
    /// `callee`, where it is known, evaluated in `order`. A reference
    /// argument may not overlap a `&var` one (E0413), and must stay valid
    /// while the later arguments are evaluated (E0414).
    pub(super) fn call_args(
        &mut self,
        params: &[ExprId],
        order: &[u32],
        callee: Option<FnId>,
        state: &mut State,
    ) {
        let args: Vec<ExprId> = evaluation_order(order, params.len())
            .map(|i| params[i])
            .collect();
        let mark = self.borrows.len();
        // The places the call may change, through its `&var` arguments.
        let mut changes: Vec<(Path, Span)> = Vec::new();
        for &arg in &args {
            self.expr(arg, Ctx::Value, state);
            let body = self.body;
            let var = matches!(
                self.program.types.kind(body.exprs[arg].ty),
                TyKind::Ref(_, RefKind::Var)
            );
            // The places the argument refers to: the one `&place` names, or
            // those a reference chosen by an `if`, or kept in a variable,
            // may point into; each with whether the call
            // may write it.
            let (place, paths): (Option<ExprId>, Vec<(Path, bool)>) = match body.exprs[arg].kind {
                ExprKind::Ref(place) if self.kept_reference_behind(place).is_none() => (
                    Some(place),
                    self.access_path(place)
                        .into_iter()
                        .map(|p| (p, var))
                        .collect(),
                ),
                _ if self.borrows_as_view(body.exprs[arg].ty)
                    && !self.is_view(body.exprs[arg].ty) =>
                {
                    let roots = self.lent_roots(arg, Lends::ALL, state);
                    (None, roots.paths.into_iter().map(|p| (p, var)).collect())
                }
                // A lent closure refers to what it captured, for as long as the
                // call may call it: so it may not capture what the call is also
                // given `&var`. A closure written here holds a `&var` to what
                // it writes and a `&` to what it only reads, so it writes only
                // the first.
                _ if matches!(
                    self.program.types.kind(body.exprs[arg].ty),
                    TyKind::Ref(inner, _) if matches!(self.program.types.kind(inner), TyKind::Fn(..))
                ) =>
                {
                    let captured = self.closure_captures(arg, state);
                    let paths = match captured {
                        Some(paths) => paths,
                        None => {
                            let roots = self.lent_roots(arg, Lends::ALL, state);
                            roots.paths.into_iter().map(|p| (p, var)).collect()
                        }
                    };
                    (None, paths)
                }
                _ => continue,
            };
            let span = body.exprs[arg].span;
            for (path, var) in paths {
                if var {
                    changes.push((path.clone(), span));
                }
                // A `&var` argument could change what a `match` or `for`
                // around the call refers into.
                if var && let Some(place) = place {
                    self.check_not_held(
                        place,
                        &path,
                        ("borrow", " as `&var`"),
                        span,
                        "borrowed here",
                        ..mark,
                    );
                }
                let conflict = self.borrows[mark..]
                    .iter()
                    .find(|b| (var || b.var) && (path.within(&b.path) || b.path.within(&path)))
                    .map(|b| (b.span, b.var));
                if let Some((earlier, earlier_var)) = conflict {
                    let label = |var: bool| {
                        if var {
                            "a `&var` reference"
                        } else {
                            "a `&` reference"
                        }
                    };
                    let name = match place {
                        Some(place) => self.display(place),
                        None => self.root_name(&path),
                    };
                    let message = if var {
                        format!(
                            "`{name}` is passed as `&var` together with another reference to it"
                        )
                    } else {
                        format!("`{name}` is passed together with a `&var` reference to it")
                    };
                    let diagnostic = Diagnostic::error(
                        codes::CONFLICTING_REFERENCES,
                        message,
                        span,
                        label(var),
                    )
                    .with_secondary(earlier, label(earlier_var))
                    .with_note("while a function runs, a `&var` argument must be the only reference to its place");
                    self.report(diagnostic);
                }
                self.borrows.push(Borrow {
                    heap: false,
                    path,
                    span,
                    var,
                    holder: Holder::Argument,
                });
            }
        }
        self.borrows.truncate(mark);
        // What the call changes, every `str` that borrows it is stale from
        // here; one passed to the same call is an error now.
        if !changes.is_empty() {
            self.check_call_strs(&args, state);
            self.var_takes_roots(params, callee, state);
            for (path, span) in changes {
                self.changed(&path, span, "passed as `&var`", state);
            }
        }
    }

    /// What a closure written as the argument captured, each place with
    /// whether it holds a `&var` to it — what it writes — rather than a `&`;
    /// nothing where the argument is not a closure made
    /// there.
    fn closure_captures(&mut self, arg: ExprId, state: &mut State) -> Option<Vec<(Path, bool)>> {
        let body = self.body;
        let ExprKind::Closure { env, .. } = body.exprs[arg].kind else {
            return None;
        };
        let ExprKind::Struct { fields, .. } = &body.exprs[env].kind else {
            return None;
        };
        let mut paths = Vec::new();
        for &field in fields {
            let writes = matches!(
                self.program.types.kind(body.exprs[field].ty),
                TyKind::Ref(_, RefKind::Var)
            );
            match body.exprs[field].kind {
                ExprKind::Ref(place) if self.kept_reference_behind(place).is_none() => {
                    paths.extend(self.access_path(place).map(|p| (p, writes)));
                }
                _ => {
                    let roots = self.lent_roots(field, Lends::ALL, state);
                    paths.extend(roots.paths.into_iter().map(|p| (p, writes)));
                }
            }
        }
        Some(paths)
    }

    /// The place a reference argument refers to, for comparing arguments:
    /// through `own` and references, and the whole array for any element of
    /// it, since indices are not known.
    pub(super) fn access_path(&self, id: ExprId) -> Option<Path> {
        match self.body.exprs[id].kind {
            // A `match` alias refers into the place it was matched from.
            ExprKind::Local(local) => Some(self.aliases.get(&local).cloned().unwrap_or(Path {
                local,
                projs: Vec::new(),
            })),
            ExprKind::Field { base, index } => {
                let mut path = self.access_path(base)?;
                path.projs.push(Proj::Field(index));
                Some(path)
            }
            ExprKind::Deref(inner) => {
                // The `&own<T>` to `&T` coercion wraps the reference it
                // converts: `Deref(Ref(x))` is `x`.
                if let ExprKind::Ref(x) = self.body.exprs[inner].kind {
                    return self.access_path(x);
                }
                // What a projection lends is somewhere in its argument.
                if let ExprKind::Call { .. } = self.body.exprs[inner].kind {
                    return self.access_path(inner);
                }
                let mut path = self.access_path(inner)?;
                path.projs.push(Proj::Deref);
                Some(path)
            }
            ExprKind::Index { base, .. } | ExprKind::SubSlice { base, .. } => {
                self.access_path(base)
            }
            // `&place` refers to what it borrows.
            ExprKind::Ref(place) => self.access_path(place),
            // What a projection lends belongs to the argument it was taken
            // from, so a borrow of it is a borrow of that.
            ExprKind::Call {
                callee, ref args, ..
            } => {
                // A projection of tables alone holds nothing.
                let wip_hir::Lent::Param(base) = self.program.fns[callee].projects? else {
                    return None;
                };
                self.access_path(*args.get(base as usize)?)
            }
            _ => None,
        }
    }

    /// A place borrowed by an earlier argument of a call whose arguments are
    /// being evaluated cannot be moved or assigned before the call (E0414).
    pub(super) fn check_not_borrowed(&mut self, id: ExprId, ctx: Ctx) {
        // Taking something from behind a `&` reference is E0405 already, and
        // writing through one is a type error.
        if self.borrows.is_empty() || self.through_shared_ref(id) {
            return;
        }
        let Some(path) = self.access_path(id) else {
            return;
        };
        let (verb, span, label) = match ctx {
            Ctx::Move(span) => ("move", span, "moved here"),
            _ => ("assign to", self.body.exprs[id].span, "assigned here"),
        };
        let all = ..self.borrows.len();
        self.check_not_held(id, &path, (verb, ""), span, label, all);
    }

    /// Reports doing `verb` to `path` while one of `borrows` holds it (E0414).
    fn check_not_held(
        &mut self,
        id: ExprId,
        path: &Path,
        (verb, how): (&str, &str),
        span: Span,
        label: &str,
        borrows: std::ops::RangeTo<usize>,
    ) {
        let path = path.clone();
        let Some((borrowed, holder)) = self.borrows[borrows]
            .iter()
            .find(|b| match b.holder {
                Holder::Assign { replace: true, .. } => b.path.within(&path) && b.path != path,
                Holder::Argument | Holder::Assign { .. } => {
                    path.within(&b.path) || b.path.within(&path)
                }
                // Bindings may write into the payload they alias.
                Holder::Match | Holder::Test | Holder::Guard | Holder::Loop => {
                    !into_payload(&path, &b.path) && (path.within(&b.path) || b.path.within(&path))
                }
            })
            .map(|b| (b.span, b.holder))
        else {
            return;
        };
        let name = self.display(id);
        let diagnostic = match holder {
            Holder::Match => Diagnostic::error(
                codes::CHANGED_WHILE_BORROWED,
                format!("cannot {verb} `{name}`{how} while a `match` binding refers to it"),
                span,
                label,
            )
            .with_secondary(borrowed, "matched here; the arm's bindings refer to it")
            .with_note("the bindings of a `match` on a place are aliases for its fields, so the place must not change until the arm ends"),
            Holder::Test => Diagnostic::error(
                codes::CHANGED_WHILE_BORROWED,
                format!("cannot {verb} `{name}`{how} while an `is` binding refers to it"),
                span,
                label,
            )
            .with_secondary(borrowed, "tested here; the bindings refer to it")
            .with_note("the bindings of an `is` test on a place are aliases for its fields, so the place must not change until the block ends"),
            Holder::Guard => Diagnostic::error(
                codes::CHANGED_WHILE_BORROWED,
                format!("cannot {verb} `{name}`{how} while the bindings of a `val` pattern refer to it"),
                span,
                label,
            )
            .with_secondary(borrowed, "matched here; the bindings refer to it until the block ends")
            .with_note("the bindings of `val pattern = place else { … }` are aliases for its fields, so the place must not change until the block ends"),
            Holder::Loop => Diagnostic::error(
                codes::CHANGED_WHILE_BORROWED,
                format!("cannot {verb} `{name}`{how} while a `for` loop walks it"),
                span,
                label,
            )
            .with_secondary(borrowed, "walked here; the loop's binding refers to its elements")
            .with_note("a `for` binding refers to the element itself, so what the loop walks must not change until the loop ends"),
            Holder::Argument => Diagnostic::error(
                codes::CHANGED_WHILE_BORROWED,
                format!("cannot {verb} `{name}`{how} while an earlier argument borrows it"),
                span,
                label,
            )
            .with_secondary(borrowed, "borrowed here, until the call returns")
            .with_note("arguments are evaluated left to right, and a reference argument must still be valid when the call starts"),
            Holder::Assign {
                target, compound, ..
            } => {
                let target_name = self.display(target);
                let note = if compound {
                    "`op=` finds its place and reads it before it computes the value, so the value must not change the place"
                } else {
                    "an assignment finds its place before it computes the value, so the value must not change what the place was found in"
                };
                Diagnostic::error(
                    codes::CHANGED_WHILE_BORROWED,
                    format!("cannot {verb} `{name}`{how} in the value assigned to `{target_name}`"),
                    span,
                    label,
                )
                .with_secondary(borrowed, "the place assigned, found before the value")
                .with_note(note)
            }
        };
        self.report(diagnostic);
    }

    fn through_shared_ref(&self, id: ExprId) -> bool {
        matches!(
            self.place(id),
            Some(Place::ThroughRef { reference })
                if matches!(self.program.types.kind(self.ty(reference)), TyKind::Ref(_, RefKind::Shared))
        )
    }

    /// Checks that every `&var` parameter holds a value at an exit (E0415).
    pub(super) fn check_var_params(&mut self, exit: Span, state: &State) {
        for param in self.var_params.clone() {
            let path = Path {
                local: param,
                projs: vec![Proj::Deref],
            };
            let Some((_, m)) = state.conflict(&path) else {
                continue;
            };
            let (moved_name, first, definite) = (m.name.clone(), m.spans[0], m.definite);
            if self.quiet > 0 || !self.reported.insert(first) {
                continue;
            }
            let name = self
                .interner
                .resolve(self.body.locals[param].name)
                .to_string();
            let verb = if definite { "is" } else { "may be" };
            let diagnostic = Diagnostic::error(
                codes::VAR_PARAMETER_LEFT_EMPTY,
                format!("`{name}` {verb} left without a value when `{}` returns", self.fn_name),
                exit,
                "returns here",
            )
            .with_secondary(first, format!("`{moved_name}` moved here"))
            .with_note("a `&var` parameter is the caller's variable, so it must hold a value again before the call returns")
            .with_help(format!("assign `{name}` a new value before this point"));
            self.report(diagnostic);
        }
    }
}
