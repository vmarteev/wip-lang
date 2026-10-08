//! Matching the arguments of a call, or the values of a variant, to the
//! parameters or fields they are for: by position, by name, or from a
//! default.

use super::*;

/// A parameter or field that an argument can give a value to.
#[derive(Clone)]
pub(super) struct Slot {
    pub name: Symbol,
    pub ty: Ty,
    /// Where it is declared.
    pub span: Span,
    pub default: Option<Box<DefaultValue>>,
}

/// Where the arguments of one call went.
pub(super) struct Matched {
    /// For each slot, the argument that gives it a value, if one does.
    pub slots: Vec<Option<usize>>,
    /// Arguments that give no slot a value: an unknown name, a slot given
    /// twice, or more positional arguments than slots. They are still
    /// checked, so that the mistakes inside them are reported.
    pub unmatched: Vec<usize>,
    /// How many positional arguments had no slot.
    pub extra: usize,
    /// Whether an argument named no slot. The slot it was probably meant
    /// for is then not reported as missing too.
    pub unknown: bool,
}

impl Matched {
    /// The slots that get neither an argument nor a default, unless an
    /// unknown name was probably meant for one of them.
    pub fn missing(&self, slots: &[Slot]) -> Vec<usize> {
        if self.unknown {
            return Vec::new();
        }
        (0..slots.len())
            .filter(|&i| self.slots[i].is_none() && slots[i].default.is_none())
            .collect()
    }
}

impl<'a> Lowerer<'a> {
    /// Matches arguments to slots: positional arguments to the first slots,
    /// named ones to the slots of their names. Reports unknown names and
    /// slots given twice; the caller reports what is missing or extra.
    /// `what` is "parameter" or "field", and `owner` the function or variant.
    pub(super) fn match_arguments(
        &mut self,
        slots: &[Slot],
        args: &[ast::ExprId],
        names: &[Option<ast::Name>],
        what: &str,
        owner: &str,
    ) -> Matched {
        let mut matched = Matched {
            slots: vec![None; slots.len()],
            unmatched: Vec::new(),
            extra: 0,
            unknown: false,
        };
        let mut positional = 0;
        for (i, name) in names.iter().enumerate() {
            // `f(style = x)`, meant as `f(style: x)`: an assignment to a
            // parameter's name, where nothing of that name is in scope.
            let meant = match (name, self.ast.exprs[args[i]].kind.clone()) {
                (
                    None,
                    ast::ExprKind::Assign {
                        target,
                        op: None,
                        value,
                        ..
                    },
                ) => match self.ast.exprs[target].kind {
                    ast::ExprKind::Name(sym)
                        if self.lookup(sym).is_none() && slots.iter().any(|s| s.name == sym) =>
                    {
                        let between = Span::new(
                            self.ast.exprs[target].span.hi,
                            self.ast.exprs[value].span.lo,
                        );
                        let diagnostic = Diagnostic::error(
                            codes::ARGUMENT_NAMED_WITH_EQUALS,
                            format!("an argument is named with `:`, as `{}: …`", self.text(sym)),
                            self.ast.exprs[args[i]].span,
                            "an assignment",
                        )
                        .with_fix("name it", [Edit::replace(between, ": ")])
                        .with_note("`=` assigns, and an assignment is an expression, so `f(x = 1)` would assign to `x`; an argument is named as a struct literal's field is");
                        self.report(diagnostic);
                        self.state.named_by_equals.insert(args[i]);
                        Some(sym)
                    }
                    _ => None,
                },
                _ => None,
            };
            let slot = match (name, meant) {
                (None, Some(sym)) => slots
                    .iter()
                    .position(|s| s.name == sym)
                    .expect("found just now"),
                (None, None) => {
                    let slot = positional;
                    positional += 1;
                    if slot >= slots.len() {
                        matched.extra += 1;
                        matched.unmatched.push(i);
                        continue;
                    }
                    slot
                }
                (Some(name), _) => match slots.iter().position(|s| s.name == name.sym) {
                    Some(slot) => slot,
                    None => {
                        let text = self.text(name.sym);
                        let mut diagnostic = Diagnostic::error(
                            codes::UNKNOWN_ARGUMENT_NAME,
                            format!("`{owner}` has no {what} named `{text}`"),
                            name.span,
                            format!("no such {what}"),
                        );
                        let candidates = slots.iter().map(|s| self.text(s.name));
                        if let Some(similar) = suggest(text, candidates) {
                            diagnostic = diagnostic.with_fix(
                                format!("did you mean `{similar}`?"),
                                [Edit::replace(name.span, similar)],
                            );
                        } else {
                            let names: Vec<String> = slots
                                .iter()
                                .map(|s| format!("`{}`", self.text(s.name)))
                                .collect();
                            if !names.is_empty() {
                                diagnostic = diagnostic
                                    .with_note(format!("the {what}s are {}", names.join(", ")));
                            }
                        }
                        self.report(diagnostic);
                        matched.unmatched.push(i);
                        matched.unknown = true;
                        continue;
                    }
                },
            };
            if let Some(first) = matched.slots[slot] {
                let text = self.text(slots[slot].name);
                let this = name.map_or(self.ast.exprs[args[i]].span, |n| n.span);
                let diagnostic = Diagnostic::error(
                    codes::ARGUMENT_TWICE,
                    format!("the {what} `{text}` is given twice"),
                    this,
                    "given again here",
                )
                .with_secondary(self.ast.exprs[args[first]].span, "given here first");
                self.report(diagnostic);
                matched.unmatched.push(i);
                continue;
            }
            matched.slots[slot] = Some(i);
        }
        matched
    }

    /// A warning for a call whose arguments by position include two or more
    /// that are the parameters' own names, each in another's place:
    /// `draw(height, width)` for `draw(width, height)`. One named as
    /// another parameter whose own argument is something else — `rhs` in
    /// `lhs` and `size` in `rhs` — is a chain a program writes on purpose,
    /// and is not one. Naming the arguments says the order is meant.
    pub(super) fn swapped_arguments(
        &mut self,
        params: &[ParamDef],
        args: &[ast::ExprId],
        names: &[Option<ast::Name>],
        callee: &str,
    ) {
        let given: Vec<Option<Symbol>> = args
            .iter()
            .zip(names)
            .take_while(|(_, name)| name.is_none())
            .map(|(&arg, _)| self.argument_name(arg))
            .collect();
        let param = |sym: Symbol| params.iter().position(|p| p.name == sym);
        // A parameter given a value named as another parameter, not itself.
        let elsewhere = |at: usize| {
            given
                .get(at)
                .copied()
                .flatten()
                .and_then(param)
                .filter(|&other| other != at)
        };
        let swapped: Vec<usize> = (0..given.len())
            .filter(|&i| elsewhere(i).is_some_and(|j| elsewhere(j).is_some()))
            .collect();
        if swapped.len() < 2 {
            return;
        }
        let quoted: Vec<String> = swapped
            .iter()
            .filter_map(|&i| given[i])
            .map(|sym| format!("`{}`", self.text(sym)))
            .collect();
        let message = match quoted.as_slice() {
            [a, b] => format!("{a} and {b} are passed in each other's places"),
            _ => format!(
                "{} and {} are passed in one another's places",
                quoted[..quoted.len() - 1].join(", "),
                quoted[quoted.len() - 1]
            ),
        };
        let mut diagnostic = Diagnostic::warning(
            codes::ARGUMENTS_SWAPPED,
            message,
            self.ast.exprs[args[swapped[0]]].span,
            format!("given as `{}`", self.text(params[swapped[0]].name)),
        );
        for &i in &swapped[1..] {
            diagnostic = diagnostic.with_secondary(
                self.ast.exprs[args[i]].span,
                format!("given as `{}`", self.text(params[i].name)),
            );
        }
        // Named from the first of them on, since nothing by position may
        // follow a named argument.
        let edits: Vec<Edit> = (swapped[0]..given.len())
            .map(|i| {
                Edit::insert(
                    self.ast.exprs[args[i]].span.lo,
                    format!("{}: ", self.text(params[i].name)),
                )
            })
            .collect();
        let order: Vec<String> = params[..given.len()]
            .iter()
            .map(|p| format!("`{}`", self.text(p.name)))
            .collect();
        diagnostic = diagnostic
            .with_fix("if that is meant, name them", edits)
            .with_note(format!(
                "`{callee}` takes {}, in that order",
                order.join(", ")
            ));
        self.report(diagnostic);
    }

    /// The name an argument is written by: a variable's, or a field's that
    /// is read, through `&`, `&var`, `move` and parentheses.
    fn argument_name(&self, arg: ast::ExprId) -> Option<Symbol> {
        match self.ast.exprs[arg].kind {
            ast::ExprKind::Name(sym) => Some(sym),
            ast::ExprKind::Field { name, .. } => Some(name.sym),
            ast::ExprKind::Paren(inner)
            | ast::ExprKind::Unary {
                op: ast::UnaryOp::Ref | ast::UnaryOp::RefVar | ast::UnaryOp::Move,
                operand: inner,
                ..
            } => self.argument_name(inner),
            _ => None,
        }
    }

    /// The checked values of matched arguments, in slot order, with defaults
    /// copied in and error expressions for what is missing, and the order in
    /// which they are evaluated: the written arguments as written, then the
    /// defaults.
    pub(super) fn arrange_arguments(
        &mut self,
        slots: &[Slot],
        matched: &Matched,
        checked: &[Option<ExprId>],
        span: Span,
    ) -> (Vec<ExprId>, Vec<u32>) {
        let mut values = Vec::with_capacity(slots.len());
        let mut written: Vec<(usize, u32)> = Vec::new();
        let mut rest = Vec::new();
        for (slot, arg) in matched.slots.iter().enumerate() {
            let value = match (arg, &slots[slot].default) {
                (Some(arg), _) => {
                    written.push((*arg, slot as u32));
                    checked[*arg].expect("matched arguments were checked")
                }
                (None, Some(default)) => {
                    rest.push(slot as u32);
                    self.use_default(default, span)
                }
                (None, None) => {
                    rest.push(slot as u32);
                    self.error_expr(span)
                }
            };
            values.push(value);
        }
        written.sort_unstable();
        let order = written
            .into_iter()
            .map(|(_, slot)| slot)
            .chain(rest)
            .collect();
        (values, order_of(order))
    }

    /// What a literal or a call that leaves a field or a parameter out puts
    /// there: a copy of a constant, or a call of the default's own function. A
    /// generic one's call is given its type arguments when the literal's or the
    /// call's are known: [`Lowerer::settle_defaults`].
    pub(super) fn use_default(&mut self, default: &DefaultValue, span: Span) -> ExprId {
        match default {
            DefaultValue::Constant { exprs, root } => self.copy_default(exprs, *root, span),
            &DefaultValue::Code(code) => {
                let ret = self.program.fns[code].ret;
                let call = self.alloc(
                    ExprKind::Call {
                        callee: code,
                        args: Vec::new(),
                        type_args: crate::TyList::EMPTY,
                        order: Vec::new(),
                    },
                    ret,
                    span,
                );
                if !self.program.fns[code].generics.is_empty() {
                    self.state.unsettled_defaults.push(call);
                }
                call
            }
        }
    }

    /// Gives the calls of generic defaults made since `mark` the type
    /// arguments of the literal or the call that uses them, which are the
    /// defaults' own.
    pub(super) fn settle_defaults(&mut self, mark: usize, args: &[Ty]) {
        let calls: Vec<ExprId> = self.state.unsettled_defaults.drain(mark..).collect();
        if calls.is_empty() {
            return;
        }
        let list = self.program.types.intern_list(args);
        for call in calls {
            let ExprKind::Call { callee, .. } = self.state.body.exprs[call].kind else {
                continue;
            };
            let ret = self.program.types.subst(self.program.fns[callee].ret, args);
            let expr = &mut self.state.body.exprs[call];
            if let ExprKind::Call { type_args, .. } = &mut expr.kind {
                *type_args = list;
            }
            expr.ty = ret;
        }
    }

    /// A copy of a constant default's expressions in the body being
    /// checked, at the literal or the call that uses it.
    fn copy_default(&mut self, exprs: &la_arena::Arena<Expr>, id: ExprId, span: Span) -> ExprId {
        let expr = &exprs[id];
        let ty = expr.ty;
        let kind = match &expr.kind {
            ExprKind::Unary { op, operand } => ExprKind::Unary {
                op: *op,
                operand: self.copy_default(exprs, *operand, span),
            },
            ExprKind::Variant {
                id: enum_id,
                variant,
                args,
                order,
            } => ExprKind::Variant {
                id: *enum_id,
                variant: *variant,
                args: args
                    .iter()
                    .map(|&a| self.copy_default(exprs, a, span))
                    .collect(),
                order: order.clone(),
            },
            ExprKind::Struct {
                id: struct_id,
                fields,
                order,
            } => ExprKind::Struct {
                id: *struct_id,
                fields: fields
                    .iter()
                    .map(|&f| self.copy_default(exprs, f, span))
                    .collect(),
                order: order.clone(),
            },
            ExprKind::Array(elems) => ExprKind::Array(
                elems
                    .iter()
                    .map(|&e| self.copy_default(exprs, e, span))
                    .collect(),
            ),
            ExprKind::ArrayRepeat { elem, count } => ExprKind::ArrayRepeat {
                elem: self.copy_default(exprs, *elem, span),
                count: *count,
            },
            other => other.clone(),
        };
        self.alloc(kind, ty, span)
    }

    /// Whether a parameter may have a default at all: a C function's may
    /// not, nor may a reference parameter, which a value would not be a
    /// place for. What is refused is reported.
    pub(super) fn default_allowed(
        &mut self,
        value: ast::ExprId,
        ty: Ty,
        param: &ast::Param,
        is_extern: bool,
    ) -> bool {
        let span = self.ast.exprs[value].span;
        let refusal = if is_extern {
            Some("a C function's parameters have no defaults")
        } else if matches!(self.kind(ty), TyKind::Ref(..)) {
            Some("a reference parameter has no default: a value is not a place")
        } else {
            None
        };
        if let Some(message) = refusal {
            let diagnostic = Diagnostic::error(codes::INVALID_DEFAULT, message, span, "a default")
                .with_secondary(param.name.span, "for this parameter");
            self.report(diagnostic);
            return false;
        }
        true
    }

    /// A default, checked as a value of the type it is for, once every
    /// function is declared, with the type parameters of what it belongs
    /// to in scope. A constant is kept to be copied in where it is used; a
    /// default that is code is the body of a function of its own, which is
    /// called there.
    pub(super) fn default_of(&mut self, value: ast::ExprId, ty: Ty) -> Option<Box<DefaultValue>> {
        let span = self.ast.exprs[value].span;
        let outer = std::mem::take(&mut self.state.body);
        let scopes = std::mem::replace(&mut self.state.scopes, vec![FxHashMap::default()]);
        let (ret, ret_span) = (self.state.ret, self.state.ret_span);
        // A `return` in a default leaves its function, with a value of the
        // type the default is.
        self.state.ret = ty;
        self.state.ret_span = None;
        let root = self.check(value, ty);
        let constant = self.is_constant(root);
        let mut body = std::mem::replace(&mut self.state.body, outer);
        self.state.scopes = scopes;
        self.state.ret = ret;
        self.state.ret_span = ret_span;
        if constant {
            return Some(Box::new(DefaultValue::Constant {
                exprs: body.exprs,
                root,
            }));
        }
        body.value = Some(root);
        let code = self.program.fns.alloc(FnDef {
            name: Symbol::default_code(),
            name_span: span,
            symbol: None,
            receiver: None,
            owner: None,
            interface: None,
            generics: self.type_params.clone(),
            instance_of: None,
            params: Vec::new(),
            ret: ty,
            ret_span: None,
            is_extern: false,
            exports_c: false,
            header: None,
            accesses: None,
            is_variadic: false,
            variadic_of: None,
            lends_from: None,
            is_lambda: false,
            generator: None,
            is_tailrec: false,
            is_test: false,
            is_inline: false,
            generated: None,
            intrinsic: None,
            body: Some(body),
            projects: None,
            compile_time: false,
            module: self.current as u32,
            is_pub: false,
            span,
        });
        Some(Box::new(DefaultValue::Code(code)))
    }

    /// Whether a checked expression is a constant.
    fn is_constant(&self, id: ExprId) -> bool {
        match &self.state.body.exprs[id].kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Error => true,
            ExprKind::Unary { operand, .. } => self.is_constant(*operand),
            ExprKind::Variant { args: parts, .. }
            | ExprKind::Struct { fields: parts, .. }
            | ExprKind::Array(parts) => parts.iter().all(|&p| self.is_constant(p)),
            ExprKind::ArrayRepeat { elem, .. } => self.is_constant(*elem),
            _ => false,
        }
    }
}
