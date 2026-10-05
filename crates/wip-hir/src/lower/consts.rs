//! Top-level `val`s: constants.
//!
//! A constant's value is worked out where it is written, so nothing runs
//! before `main` and there is no order of initialisation to get wrong. A use
//! of one is the value itself, put in place by the checker, which is why a
//! constant may be a `str` as easily as an integer.
//!
//! What is written with literals, `-` and the arithmetic and bitwise
//! operators, comparisons, `as`, and other constants is folded here. What
//! else is written — a call, a loop, a block — becomes the body of a
//! function of its own, which the compiler runs once the program is checked;
//! a use of such a constant reads where it is kept.

use super::*;

/// Where a constant was written, so that a use of it can work it out even
/// where the pass over its own file has not reached it yet.
#[derive(Clone)]
pub(super) struct ConstSource<'a> {
    pub decl: &'a ast::ValDecl,
    /// Where `@comptime` was written on it, if it was.
    pub comptime: Option<Span>,
    pub ast: &'a Ast,
    pub module: usize,
    pub imports: FileImports,
    /// The lambdas and generators of its file, which its value may hold.
    pub lambdas: FxHashMap<ast::ExprId, FnId>,
    pub lambda_envs: FxHashMap<FnId, StructId>,
    pub generators: FxHashMap<ast::ExprId, StructId>,
}

impl<'a> Lowerer<'a> {
    /// Declares the file's constants, before any body is checked. Their
    /// values come later, once every type is known, and a constant that uses
    /// another is worked out through it, whatever order they are written in.
    pub(super) fn declare_consts(&mut self) {
        let ast = self.ast;
        for item in &ast.items {
            let ast::Item::Val(decl) = item else {
                continue;
            };
            let annotations = self.annotations(&decl.annotations, annotations::Target::Val);
            let id = self.program.consts.alloc(ConstDef {
                name: decl.name.sym,
                ty: Types::ERROR,
                value: None,
                code: None,
                module: self.current as u32,
                is_pub: decl.is_pub,
                span: decl.name.span,
            });
            if self.prelude_name(decl.name, "a constant named") {
                // Reported; it keeps its place in this module's scope.
            }
            match self.consts().get(&decl.name.sym) {
                Some(&first) => {
                    let previous = self.program.consts[first].span;
                    self.duplicate(decl.name, previous);
                }
                None => {
                    self.consts_mut().insert(decl.name.sym, id);
                }
            }
            // Where it was written, so that a use can work it out before the
            // pass over its own file reaches it.
            let source = ConstSource {
                decl,
                comptime: annotations.comptime,
                ast,
                module: self.current,
                imports: self.imports.clone(),
                lambdas: self.lambdas.clone(),
                lambda_envs: self.lambda_envs.clone(),
                generators: self.generators.clone(),
            };
            self.const_sources.insert(id, source);
        }
    }

    /// Works out the values of the file's constants, once every type is
    /// known.
    pub(super) fn check_consts(&mut self) {
        // In the order they were declared, not the order a hash map holds
        // them: two constants defined in terms of each other are reported
        // from the first of them, whatever else a program happens to name.
        let mut ids: Vec<ConstId> = self.consts().values().copied().collect();
        ids.sort_by_key(|id| id.into_raw());
        for id in ids {
            self.const_value(id);
        }
    }

    /// Works out one constant's value, if it has not been worked out
    /// already. Its initializer is checked as any expression is, with the
    /// written type expected, in the file that wrote it.
    pub(super) fn const_value(&mut self, id: ConstId) {
        if self.consts_done.contains(&id) {
            return;
        }
        let Some(source) = self.const_sources.get_mut(&id) else {
            return;
        };
        // Its file's lambdas and generators are wanted once, now; the rest
        // of where it was written is read again while it is worked out.
        let code = (
            std::mem::take(&mut source.lambdas),
            std::mem::take(&mut source.lambda_envs),
            std::mem::take(&mut source.generators),
        );
        let source = source.clone();
        self.consts_done.insert(id);
        self.const_stack.push(id);
        // A constant is checked in the file that wrote it, in a body of its
        // own that nothing keeps: only the value it works out to is.
        let here = (self.ast, self.current, self.imports.clone());
        let files = (
            std::mem::replace(&mut self.lambdas, code.0),
            std::mem::replace(&mut self.lambda_envs, code.1),
            std::mem::replace(&mut self.generators, code.2),
        );
        self.enter(source.module, source.ast, source.imports);
        self.outer.push(std::mem::take(&mut self.state));
        self.state.scopes = vec![FxHashMap::default()];
        let decl = source.decl;
        let (value, ty) = match decl.ty {
            Some(t) => {
                let mut ty = self.resolve_ty(t);
                // A file's bytes, embedded, are the one reference a
                // constant holds, alone or in an array.
                if (!self.bytes_refs_only(ty)
                    && self.no_ref(ty, t, "a constant cannot hold a reference", None))
                    || self.no_never(ty, t, "a constant")
                    || self.bare_slice(ty, t, false)
                {
                    ty = Types::ERROR;
                }
                let context = (self.ast.types[t].span, "expected because of this type");
                (self.check_in(decl.value, ty, Some(context)), ty)
            }
            None => {
                let value = self.infer(decl.value, None);
                (value, self.ty_of(value))
            }
        };
        let folded = if self.has_error(ty) {
            Err(Unfolded::Failed)
        } else if self.owns(ty) {
            // A use is a copy of the value, so what would be freed at each
            // use cannot be one.
            let span = self.state.body.exprs[value].span;
            let diagnostic = Diagnostic::error(
                codes::NOT_A_CONSTANT,
                "a top-level `val` cannot hold what owns memory",
                span,
                format!("{} owns memory", self.ty_name(ty)),
            )
            .with_note("each use of a constant is its value, made there; a value that owns memory would be made and freed at every use");
            self.report(diagnostic);
            Err(Unfolded::Failed)
        } else {
            self.fold(value)
        };
        // What folding cannot work out is run: the initializer is the body of
        // a function of its own, which the compiler runs once the program is
        // checked, where the constant says so with `@comptime`.
        let folded = self.comptime_mark(decl, source.comptime, folded, value);
        let code = match folded {
            Err(Unfolded::Runs) => {
                let mut body = std::mem::take(&mut self.state.body);
                body.value = Some(value);
                Some(self.constant_code(decl, ty, body))
            }
            _ => None,
        };
        self.state = self.outer.pop().expect("a constant put the body aside");
        self.enter(here.1, here.0, here.2);
        (self.lambdas, self.lambda_envs, self.generators) = files;
        self.const_stack.pop();
        let def = &mut self.program.consts[id];
        def.ty = ty;
        def.value = folded.ok();
        def.code = code;
    }

    /// The value of a checked expression, where folding can work it out; or
    /// that it is to be run, or that it failed, which is reported.
    fn fold(&mut self, id: ExprId) -> Result<ConstValue, Unfolded> {
        let expr = &self.state.body.exprs[id];
        let (kind, ty, span) = (expr.kind.clone(), expr.ty, expr.span);
        match kind {
            ExprKind::Int(value) => Ok(ConstValue::Int(value)),
            ExprKind::Float(value) => Ok(ConstValue::Float(value)),
            ExprKind::Bool(value) => Ok(ConstValue::Bool(value)),
            ExprKind::Str(sym) => Ok(ConstValue::Str(sym)),
            ExprKind::Error => Err(Unfolded::Failed),
            // A table's value, where it is known and holds a file's bytes,
            // which is read where it is kept rather than made again.
            ExprKind::Deref(address) => match self.state.body.exprs[address].kind {
                ExprKind::Table(id) => match &self.program.consts[id].value {
                    Some(value) if value.holds_bytes() => Ok(value.clone()),
                    _ => Err(Unfolded::Runs),
                },
                _ => Err(Unfolded::Runs),
            },
            // A table: an array, a struct or tuple, or a variant, of
            // constants.
            ExprKind::Array(parts) => Ok(ConstValue::Array(self.fold_all(&parts)?)),
            ExprKind::ArrayRepeat { elem, count } => {
                let value = self.fold(elem)?;
                Ok(ConstValue::Array(vec![value; count as usize]))
            }
            ExprKind::Struct { fields, .. } => Ok(ConstValue::Struct(self.fold_all(&fields)?)),
            ExprKind::Variant { variant, args, .. } => Ok(ConstValue::Variant {
                variant,
                fields: self.fold_all(&args)?,
            }),
            ExprKind::Unary { op, operand } => {
                let value = self.fold(operand)?;
                self.fold_unary(op, value, ty)
            }
            ExprKind::Binary { op, lhs, rhs, .. } => {
                let lhs_ty = self.ty_of(lhs);
                let (lhs, rhs) = (self.fold(lhs)?, self.fold(rhs)?);
                self.fold_binary(op, lhs, rhs, lhs_ty, span)
            }
            // `as` between numbers: the checker has already agreed to it.
            ExprKind::Cast(inner) => {
                let value = self.fold(inner)?;
                Ok(self.fold_cast(value, ty))
            }
            // A call, a block, a constant that is run itself: what folding
            // does not do is run.
            _ => Err(Unfolded::Runs),
        }
    }

    /// Every part of a table, where folding works each out.
    fn fold_all(&mut self, parts: &[ExprId]) -> Result<Vec<ConstValue>, Unfolded> {
        let mut values = Vec::with_capacity(parts.len());
        for &part in parts {
            values.push(self.fold(part)?);
        }
        Ok(values)
    }

    fn fold_unary(
        &mut self,
        op: UnaryOp,
        value: ConstValue,
        ty: Ty,
    ) -> Result<ConstValue, Unfolded> {
        match (op, value) {
            (UnaryOp::Neg, ConstValue::Int(v)) => {
                let bits = self.int_bits(ty);
                Ok(ConstValue::Int(truncate(0u128.wrapping_sub(v), bits)))
            }
            (UnaryOp::Neg, ConstValue::Float(v)) => Ok(ConstValue::Float(-v)),
            (UnaryOp::Not, ConstValue::Bool(v)) => Ok(ConstValue::Bool(!v)),
            (UnaryOp::Not, ConstValue::Int(v)) => {
                let bits = self.int_bits(ty);
                Ok(ConstValue::Int(truncate(!v, bits)))
            }
            _ => Err(Unfolded::Runs),
        }
    }

    fn fold_binary(
        &mut self,
        op: BinaryOp,
        lhs: ConstValue,
        rhs: ConstValue,
        operand_ty: Ty,
        span: Span,
    ) -> Result<ConstValue, Unfolded> {
        use BinaryOp::*;
        let signed = matches!(self.kind(operand_ty), TyKind::Int(t) if t.signed());
        let bits = self.int_bits(operand_ty);
        match (lhs, rhs) {
            (ConstValue::Int(a), ConstValue::Int(b)) => {
                // Arithmetic wraps; a division by zero is
                // the one thing a constant cannot do.
                let wrap = |v: u128| ConstValue::Int(truncate(v, bits));
                let value = match op {
                    Add => wrap(a.wrapping_add(b)),
                    Sub => wrap(a.wrapping_sub(b)),
                    Mul => wrap(a.wrapping_mul(b)),
                    Div | Rem if b == 0 => {
                        let diagnostic = Diagnostic::error(
                            codes::NOT_A_CONSTANT,
                            "this constant divides by zero",
                            span,
                            "a division by zero",
                        )
                        .with_note(
                            "a constant is worked out where it is written, so this would never be a value at all",
                        );
                        self.report(diagnostic);
                        return Err(Unfolded::Failed);
                    }
                    Div if signed => {
                        wrap(as_signed(a, bits).wrapping_div(as_signed(b, bits)) as u128)
                    }
                    Div => wrap(a / b),
                    Rem if signed => {
                        wrap(as_signed(a, bits).wrapping_rem(as_signed(b, bits)) as u128)
                    }
                    Rem => wrap(a % b),
                    BitAnd => wrap(a & b),
                    BitOr => wrap(a | b),
                    BitXor => wrap(a ^ b),
                    Shl => wrap(a.wrapping_shl(b as u32)),
                    Shr if signed => wrap((as_signed(a, bits) >> (b as u32 % 128)) as u128),
                    Shr => wrap(a >> (b as u32 % 128)),
                    Eq => ConstValue::Bool(a == b),
                    Ne => ConstValue::Bool(a != b),
                    Lt | Le | Gt | Ge => {
                        let ordering = if signed {
                            as_signed(a, bits).cmp(&as_signed(b, bits))
                        } else {
                            a.cmp(&b)
                        };
                        ConstValue::Bool(match op {
                            Lt => ordering.is_lt(),
                            Le => ordering.is_le(),
                            Gt => ordering.is_gt(),
                            _ => ordering.is_ge(),
                        })
                    }
                    And | Or => return Err(Unfolded::Runs),
                };
                Ok(value)
            }
            (ConstValue::Float(a), ConstValue::Float(b)) => Ok(match op {
                Add => ConstValue::Float(a + b),
                Sub => ConstValue::Float(a - b),
                Mul => ConstValue::Float(a * b),
                Div => ConstValue::Float(a / b),
                Rem => ConstValue::Float(a % b),
                Eq => ConstValue::Bool(a == b),
                Ne => ConstValue::Bool(a != b),
                Lt => ConstValue::Bool(a < b),
                Le => ConstValue::Bool(a <= b),
                Gt => ConstValue::Bool(a > b),
                Ge => ConstValue::Bool(a >= b),
                _ => return Err(Unfolded::Runs),
            }),
            (ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(match op {
                And => ConstValue::Bool(a && b),
                Or => ConstValue::Bool(a || b),
                Eq => ConstValue::Bool(a == b),
                Ne => ConstValue::Bool(a != b),
                BitAnd => ConstValue::Bool(a & b),
                BitOr => ConstValue::Bool(a | b),
                BitXor => ConstValue::Bool(a ^ b),
                _ => return Err(Unfolded::Runs),
            }),
            _ => Err(Unfolded::Runs),
        }
    }

    /// `as`, which the checker has already agreed to: between numbers, and
    /// between an integer and a `bool`.
    fn fold_cast(&mut self, value: ConstValue, ty: Ty) -> ConstValue {
        match (value, self.kind(ty)) {
            (ConstValue::Int(v), TyKind::Int(_)) => ConstValue::Int(truncate(v, self.int_bits(ty))),
            (ConstValue::Int(v), TyKind::Float(_)) => ConstValue::Float(v as f64),
            (ConstValue::Float(v), TyKind::Int(_)) => {
                ConstValue::Int(truncate(v as i128 as u128, self.int_bits(ty)))
            }
            (ConstValue::Bool(v), TyKind::Int(_)) => ConstValue::Int(u128::from(v)),
            (value, _) => value,
        }
    }

    /// The function that computes a constant folding cannot work out:
    /// its initializer, checked, as the body, and the constant's own name,
    /// which is how a failure while it runs names it.
    fn constant_code(&mut self, decl: &ast::ValDecl, ty: Ty, body: Body) -> FnId {
        self.program.fns.alloc(FnDef {
            name: decl.name.sym,
            name_span: decl.name.span,
            symbol: None,
            receiver: None,
            owner: None,
            interface: None,
            generics: Vec::new(),
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
            is_lambda: false,
            generator: None,
            is_tailrec: false,
            is_test: false,
            is_inline: false,
            generated: None,
            intrinsic: None,
            body: Some(body),
            projects: None,
            compile_time: true,
            module: self.current as u32,
            is_pub: false,
            span: decl.name.span,
        })
    }

    /// Holds a constant to its mark: one that runs code says so with
    /// `@comptime`, and one folding works out does not, since a mark that does
    /// nothing is worse than none. A constant that runs without the mark fails.
    fn comptime_mark(
        &mut self,
        decl: &ast::ValDecl,
        comptime: Option<Span>,
        folded: Result<ConstValue, Unfolded>,
        value: ExprId,
    ) -> Result<ConstValue, Unfolded> {
        let name = self.text(decl.name.sym).to_string();
        match (&folded, comptime) {
            (Err(Unfolded::Runs), None) => {
                let span = self.state.body.exprs[value].span;
                let at = decl
                    .annotations
                    .first()
                    .map_or(decl.span.lo, |annotation| annotation.span.lo);
                let diagnostic = Diagnostic::error(
                    codes::COMPTIME_MARK,
                    format!("`{name}` runs code while the program is compiled, and does not say so"),
                    span,
                    "runs while the program is compiled",
                )
                .with_fix("mark it `@comptime`", [Edit::insert(at, "@comptime\n")])
                .with_note(
                    "a constant written with a call, a loop or a block is worked out by running it, which `@comptime` says where it is declared",
                );
                self.report(diagnostic);
                Err(Unfolded::Failed)
            }
            (Ok(_), Some(mark)) => {
                let diagnostic = Diagnostic::error(
                    codes::COMPTIME_MARK,
                    format!("`{name}` runs nothing, so `@comptime` says nothing"),
                    mark,
                    "on a constant known where it is written",
                )
                .with_fix("remove `@comptime`", [Edit::replace(mark, "")])
                .with_note(
                    "`@comptime` marks a constant worked out by running code; one written with literals, operators and other such constants is known without it",
                );
                self.report(diagnostic);
                folded
            }
            _ => folded,
        }
    }

    /// `&[u8]`, what a file embedded as bytes is, or an array of them,
    /// which are the references a constant holds.
    fn bytes_refs_only(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Ref(inner, crate::RefKind::Shared) => {
                matches!(self.kind(inner), TyKind::Slice(elem) if elem == Types::U8)
            }
            TyKind::Array(elem, _) => self.bytes_refs_only(elem),
            _ => false,
        }
    }

    /// Whether a constant's value is known while types are checked, as an
    /// array's length and a pattern need it; one the compiler runs is known
    /// only after, which is reported.
    pub(super) fn known_now(&mut self, id: ConstId, span: Span, need: &str) -> bool {
        if self.program.consts[id].code.is_none() {
            return true;
        }
        let name = self.text(self.program.consts[id].name).to_string();
        let diagnostic = Diagnostic::error(
            codes::NOT_A_CONSTANT,
            format!("`{name}` is not known until the program is checked"),
            span,
            format!("{need} must be known while types are checked"),
        )
        .with_note(
            "a constant written with a call, a loop or a block is run once the program is checked; one written with literals, operators and other such constants is known before",
        );
        self.report(diagnostic);
        false
    }

    fn int_bits(&self, ty: Ty) -> u32 {
        match self.kind(ty) {
            TyKind::Int(t) => t.bits(),
            _ => 128,
        }
    }

    /// A use of a constant: the value itself, where it was worked out. One
    /// that has not been worked out yet is worked out here, so that a
    /// constant may use one written after it; one that is being worked out
    /// already is a cycle.
    pub(super) fn const_use(&mut self, id: ConstId, span: Span) -> ExprId {
        if !self.derived {
            self.program.names.push((span, Named::Const(id)));
        }
        if let Some(at) = self.const_stack.iter().position(|&on| on == id) {
            let cycle: Vec<String> = self.const_stack[at..]
                .iter()
                .map(|&on| format!("`{}`", self.text(self.program.consts[on].name)))
                .collect();
            let message = match cycle.len() {
                1 => format!("{} is defined in terms of itself", cycle[0]),
                2 => format!(
                    "{} and {} are defined in terms of each other",
                    cycle[0], cycle[1]
                ),
                _ => format!(
                    "{} is defined in terms of itself, through {}",
                    cycle[0],
                    cycle[1..].join(", ")
                ),
            };
            let diagnostic = Diagnostic::error(
                codes::NOT_A_CONSTANT,
                message,
                span,
                "used while it is being worked out",
            )
            .with_note(
                "a constant's value is worked out where it is written, so it cannot depend on itself",
            );
            self.report(diagnostic);
            return self.error_expr(span);
        }
        self.const_value(id);
        let def = self.program.consts[id].clone();
        // A constant the compiler runs is known only once the program is
        // checked, so a use reads where it is kept, as a table's does, and
        // another constant that uses it is run too.
        if def.code.is_some() {
            let reference = self.intern(TyKind::Ref(def.ty, crate::RefKind::Shared));
            let address = self.alloc(ExprKind::Table(id), reference, span);
            return self.alloc(ExprKind::Deref(address), def.ty, span);
        }
        let Some(value) = def.value else {
            // Its own error was reported where it was written.
            return self.error_expr(span);
        };
        // A table is kept once, and a use of it is the place it is kept in;
        // in another constant it is its value, which that constant folds,
        // unless that constant runs code, which reads the
        // table where it lies as a function does.
        let running = self.const_stack.last().is_none_or(|on| {
            self.const_sources
                .get(on)
                .is_some_and(|s| s.comptime.is_some())
        });
        // A file's bytes are never made again where they are used, even
        // in a constant being folded: that one reads them, and folds the
        // read.
        if value.is_table() && (running || value.holds_bytes()) {
            let reference = self.intern(TyKind::Ref(def.ty, crate::RefKind::Shared));
            let address = self.alloc(ExprKind::Table(id), reference, span);
            return self.alloc(ExprKind::Deref(address), def.ty, span);
        }
        self.const_expr(value, def.ty, span)
    }

    /// A constant's value, made where it is used, as a number is, and as a
    /// table is in another constant: of its parts, each of its own type.
    fn const_expr(&mut self, value: ConstValue, ty: Ty, span: Span) -> ExprId {
        let kind = match value {
            // A file's bytes are read where they are kept, and never made
            // again here.
            ConstValue::Bytes(_) => unreachable!("a use of a file's bytes reads its table"),
            ConstValue::Int(v) => ExprKind::Int(v),
            ConstValue::Float(v) => ExprKind::Float(v),
            ConstValue::Bool(v) => ExprKind::Bool(v),
            ConstValue::Str(sym) => ExprKind::Str(sym),
            ConstValue::Array(values) => {
                let TyKind::Array(elem, _) = self.kind(ty) else {
                    return self.error_expr(span);
                };
                let parts = values
                    .into_iter()
                    .map(|v| self.const_expr(v, elem, span))
                    .collect();
                ExprKind::Array(parts)
            }
            ConstValue::Struct(values) => {
                let TyKind::Struct(id, _) = self.kind(ty) else {
                    return self.error_expr(span);
                };
                let mut fields = Vec::with_capacity(values.len());
                for (i, v) in values.into_iter().enumerate() {
                    let field_ty = self.struct_field_ty(ty, i);
                    fields.push(self.const_expr(v, field_ty, span));
                }
                ExprKind::Struct {
                    id,
                    fields,
                    order: Vec::new(),
                }
            }
            ConstValue::Variant { variant, fields } => {
                let TyKind::Enum(id, _) = self.kind(ty) else {
                    return self.error_expr(span);
                };
                let mut args = Vec::with_capacity(fields.len());
                for (i, v) in fields.into_iter().enumerate() {
                    let field_ty = self.variant_field_ty(ty, variant as usize, i);
                    args.push(self.const_expr(v, field_ty, span));
                }
                ExprKind::Variant {
                    id,
                    variant,
                    args,
                    order: Vec::new(),
                }
            }
        };
        self.alloc(kind, ty, span)
    }
}

/// The low `bits` of a value, which is what an integer type holds.
fn truncate(value: u128, bits: u32) -> u128 {
    if bits >= 128 {
        value
    } else {
        value & ((1u128 << bits) - 1)
    }
}

/// The value as the signed integer of that width, for the operators that
/// treat it as one.
pub(super) fn as_signed(value: u128, bits: u32) -> i128 {
    if bits >= 128 {
        value as i128
    } else {
        let shift = 128 - bits;
        ((value << shift) as i128) >> shift
    }
}

/// Why folding did not work a constant out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unfolded {
    /// It is to be run, once the program is checked.
    Runs,
    /// It went wrong, and that was reported.
    Failed,
}
