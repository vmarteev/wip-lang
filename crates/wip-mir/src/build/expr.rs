//! Expressions that produce scalars, operators and calls.

use super::*;

impl Builder<'_> {
    /// A scalar expression's value, read now: a place is copied into a
    /// temporary, so that a later operand cannot change what this one read.
    pub(super) fn scalar(&mut self, id: ExprId) -> Operand {
        match self.expr(id) {
            Value::Scalar(operand) => operand,
            _ => unreachable!("expected a scalar expression"),
        }
    }

    pub(super) fn expr(&mut self, id: ExprId) -> Value {
        let hir = self.hir;
        let expr = &hir.exprs[id];
        if self.is_aggregate(expr.ty) {
            return Value::Place(self.aggregate(id));
        }
        match &expr.kind {
            ExprKind::Int(bits) => Value::Scalar(Self::int(*bits, expr.ty)),
            ExprKind::Float(value) => Value::Scalar(Operand::Const(Const::Float {
                value: *value,
                ty: expr.ty,
            })),
            ExprKind::Bool(value) => Value::Scalar(Operand::Const(Const::Bool(*value))),
            // A null C pointer is the zero of its type.
            ExprKind::Null => Value::Scalar(Self::int(0, expr.ty)),
            // Every byte zero, as C leaves a field it is not given.
            ExprKind::Zeroed => {
                let zero = Place::local(self.temp(expr.ty));
                self.push(Statement::Zero(zero.clone()));
                Value::Scalar(Operand::Copy(zero))
            }
            // Two strings, compared by their bytes in the runtime.
            ExprKind::StrCmp { lhs, rhs } => {
                let left = self.aggregate(*lhs);
                let right = self.aggregate(*rhs);
                let order = Place::local(self.temp(Types::I64));
                self.push(Statement::Call {
                    callee: Callee::StrCmp,
                    args: vec![
                        Operand::Copy(left.clone().project(Projection::Field(0))),
                        Operand::Copy(left.project(Projection::Field(1))),
                        Operand::Copy(right.clone().project(Projection::Field(0))),
                        Operand::Copy(right.project(Projection::Field(1))),
                    ],
                    dest: Some(order.clone()),
                });
                Value::Scalar(Operand::Copy(order))
            }
            ExprKind::IsNull(inner) => {
                let ty = self.ty(*inner);
                let pointer = self.scalar(*inner);
                let null = Self::int(0, ty);
                Value::Scalar(self.value(Types::BOOL, Rvalue::Binary(BinaryOp::Eq, pointer, null)))
            }
            ExprKind::Str(sym) => Value::Scalar(Operand::Const(Const::CStr(*sym))),
            ExprKind::Table(id) => Value::Scalar(Operand::Const(Const::Table {
                id: *id,
                ty: expr.ty,
            })),
            ExprKind::Local(_)
            | ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Deref(_) => {
                if !self.is_scalar(expr.ty) {
                    return Value::Unit;
                }
                let place = self.place(id);
                Value::Scalar(self.value(expr.ty, Rvalue::Use(Operand::Copy(place))))
            }
            // A `@tailrec` function's call to itself: the arguments become
            // the parameters, and control goes back to the top.
            ExprKind::Call { args, order, .. } if self.hir.tail_calls.contains(&id) => {
                self.tail_call(args, order);
                Value::Unit
            }
            // A body the compiler writes.
            ExprKind::Call { callee, args, .. }
                if self.program.fns[*callee].intrinsic.is_some() =>
            {
                self.intrinsic_call(*callee, args, expr.ty, expr.span, None)
            }
            ExprKind::Call {
                callee,
                args,
                order,
                ..
            } => {
                let ret = self.program.fns[*callee].ret;
                self.call(Callee::Fn(*callee), ret, args, order, None)
            }
            // The function value is found first, then the arguments.
            ExprKind::CallValue { callee, args } => {
                let callee = self.scalar(*callee);
                self.call(Callee::Value(callee), expr.ty, args, &[], None)
            }
            ExprKind::FnRef { id, .. } => Value::Scalar(Operand::Const(Const::Fn {
                id: *id,
                ty: expr.ty,
            })),
            // A projection lends the place: its address is the result, and
            // it unwinds like a `return`.
            ExprKind::Lend(place) => {
                let ret = self.ret.expect("a projection has a result");
                // A slice is lent as what it already is, a pointer and a
                // length; anything else is lent by its address.
                if self.is_aggregate(self.local_ty(ret)) {
                    self.store_expr(*place, Place::local(ret));
                } else {
                    let place = self.place(*place);
                    self.assign(Place::local(ret), Rvalue::AddressOf(place));
                }
                self.drop_temps(0);
                self.drop_all_scopes();
                self.terminate(Terminator::Return);
                self.dead = true;
                Value::Unit
            }
            // `-x` of the most negative number has no answer that fits.
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } if self.checks_overflow(expr.ty) => {
                let at = self.span(*operand);
                let operand = self.scalar(*operand);
                if let TyKind::Int(int) = self.kind(expr.ty) {
                    let most_negative = if int.signed() {
                        1u128 << (int.bits() - 1)
                    } else {
                        // Nothing but zero may be negated unsigned.
                        1
                    };
                    let edge = Self::int(most_negative, expr.ty);
                    let fails = self.value(
                        Types::BOOL,
                        Rvalue::Binary(BinaryOp::Eq, operand.clone(), edge),
                    );
                    self.push(Statement::Check {
                        fails,
                        kind: CheckKind::Overflow(BinaryOp::Sub),
                        at,
                    });
                }
                Value::Scalar(self.value(expr.ty, Rvalue::Unary(UnaryOp::Neg, operand)))
            }
            ExprKind::Unary { op, operand } => {
                let operand = self.scalar(*operand);
                Value::Scalar(self.value(expr.ty, Rvalue::Unary(*op, operand)))
            }
            ExprKind::Binary {
                op,
                lhs,
                rhs,
                wrapping,
            } => Value::Scalar(self.binary(*op, *lhs, *rhs, *wrapping, expr.ty)),
            // The place is found first, then the value; the old value is
            // dropped just before it is replaced. `op=` reads the old value
            // before the value is computed.
            ExprKind::Assign {
                place,
                op,
                wrapping,
                value,
            } => {
                let place_ty = self.ty(*place);
                let dest = self.place(*place);
                if let Some(op) = op {
                    // Checked as the operator is: an answer that does not
                    // fit, and a division by zero, panic where they happen.
                    let at = self.span(*value);
                    let old = self.value(place_ty, Rvalue::Use(Operand::Copy(dest.clone())));
                    let value = self.scalar(*value);
                    let answer = self.arithmetic(*op, old, value, *wrapping, place_ty, at);
                    self.assign(dest, Rvalue::Use(answer));
                } else if self.is_aggregate(place_ty) {
                    let temp = Place::local(self.temp(place_ty));
                    self.store_expr(*value, temp.clone());
                    self.drop_replaced(*place, &dest, place_ty);
                    self.assign(dest, Rvalue::Use(Operand::Copy(temp)));
                    self.assigned_to(*place);
                } else {
                    let value = self.scalar(*value);
                    self.drop_replaced(*place, &dest, place_ty);
                    self.assign(dest, Rvalue::Use(value));
                    self.assigned_to(*place);
                }
                Value::Unit
            }
            ExprKind::Ref(inner) => {
                let place = self.place(*inner);
                Value::Scalar(self.value(expr.ty, Rvalue::AddressOf(place)))
            }
            // An owned closure lent for a call is the pair it already is.
            ExprKind::LendClosure(inner) => Value::Place(self.aggregate(*inner)),
            // The address of a drop function, which an owned closure's
            // environment carries.
            ExprKind::DropRef(ty) => Value::Scalar(Operand::Const(Const::DropFn(*ty))),
            ExprKind::Block(block) => self.block_expr(block, None),
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => self.if_expr(*cond, then_block, else_block.as_ref(), expr.ty, None),
            ExprKind::Cast(inner) => {
                let operand = self.scalar(*inner);
                // A constant integer or character made another integer type
                // is the constant it becomes: `'-' as u8` is `45_u8`, which
                // is how the standard library writes a byte.
                if let Some(folded) = self.folded_cast(&operand, expr.ty) {
                    return Value::Scalar(folded);
                }
                Value::Scalar(self.value(expr.ty, Rvalue::Cast(operand, expr.ty)))
            }
            // `colour as i64`: the tag, read where the value lies, and
            // widened to the type that was asked for.
            ExprKind::VariantIndex(inner) => {
                let place = self.aggregate(*inner);
                let index = self.value(Types::I32, Rvalue::Variant(place));
                Value::Scalar(self.value(expr.ty, Rvalue::Cast(index, expr.ty)))
            }
            ExprKind::Len(base) => Value::Scalar(self.len(*base)),
            // A panic ends the path: nothing after it runs.
            ExprKind::Panic { note, message } => {
                let note = note.map(|note| Operand::Copy(self.aggregate(note)));
                self.terminate(Terminator::Panic {
                    note,
                    message: *message,
                    at: expr.span,
                });
                self.dead = true;
                Value::Unit
            }
            // A call through a closure: what it captured goes first.
            ExprKind::CallClosure {
                callee,
                args,
                code_ty,
            } => {
                let closure = self.place(*callee);
                let code = self.value(
                    *code_ty,
                    Rvalue::Use(Operand::Copy(closure.clone().project(Projection::Field(1)))),
                );
                let captures = Operand::Copy(closure.project(Projection::Field(0)));
                self.closure_call(code, captures, args, expr.ty, None)
            }
            // A call through a `&dyn`: the method's address comes from the
            // table the reference carries.
            ExprKind::DynCall {
                index,
                fn_ty,
                args,
                order,
                ..
            } => {
                let receiver = self.place(args[0]);
                let table = Operand::Copy(receiver.clone().project(Projection::Field(1)));
                let function = self.value(
                    *fn_ty,
                    Rvalue::VTableFn {
                        table,
                        index: *index,
                        methods: true,
                    },
                );
                let pointer = Operand::Copy(receiver.project(Projection::Field(0)));
                self.dyn_call(function, pointer, args, order, expr.ty, None)
            }
            ExprKind::Move(operand) => {
                let place = self.place(*operand);
                let ty = self.ty(*operand);
                let value = self.value(ty, Rvalue::Use(Operand::Copy(place.clone())));
                self.vacate(place, ty);
                self.moved_from(*operand);
                Value::Scalar(value)
            }
            // A value that could jump out while it is built is built first,
            // so that nothing is allocated for it until it is complete.
            ExprKind::Own(inner) if self.may_jump(*inner) => {
                let ty = self.ty(*inner);
                let value = Place::local(self.temp(ty));
                self.store_expr(*inner, value.clone());
                let ptr = Place::local(self.temp(expr.ty));
                self.push(Statement::Alloc {
                    dest: ptr.clone(),
                    ty,
                });
                self.assign(
                    ptr.project(Projection::Deref),
                    Rvalue::Use(Operand::Copy(value)),
                );
                Value::Scalar(Operand::Copy(ptr))
            }
            ExprKind::Own(inner) => {
                let ptr = Place::local(self.temp(expr.ty));
                let ty = self.ty(*inner);
                self.push(Statement::Alloc {
                    dest: ptr.clone(),
                    ty,
                });
                self.store_expr(*inner, ptr.project(Projection::Deref));
                Value::Scalar(Operand::Copy(ptr))
            }
            ExprKind::Match { scrutinee, arms } => self.match_expr(*scrutinee, arms, expr.ty, None),
            ExprKind::Is { scrutinee, pattern } => Value::Scalar(self.is_test(*scrutinee, pattern)),
            // A `str` lives in memory: `place` builds it.
            ExprKind::CstrToStr(_)
            | ExprKind::Struct { .. }
            | ExprKind::Union { .. }
            | ExprKind::Array(_)
            | ExprKind::ArrayRepeat { .. }
            | ExprKind::SubSlice { .. }
            | ExprKind::Variant { .. }
            | ExprKind::OwnRepeat { .. }
            | ExprKind::Unsize(_)
            | ExprKind::Closure { .. }
            | ExprKind::DynRef { .. } => unreachable!("aggregates are handled above"),
            ExprKind::Places(_) => {
                unreachable!("a tuple matched in place is read only by its `match`")
            }
            ExprKind::Error => unreachable!("programs with errors are not lowered"),
        }
    }

    /// `xs.len`: the length in an array's type, or the second word of a
    /// `str` or a slice.
    fn len(&mut self, base: ExprId) -> Operand {
        match self.kind(self.ty(base)) {
            TyKind::Array(_, len) => {
                // The base is still evaluated, and a temporary one dropped.
                if !self.is_place_kind(base) {
                    self.place(base);
                }
                Self::int(u128::from(len), Types::I64)
            }
            _ => {
                let place = self.place(base);
                self.value(
                    Types::I64,
                    Rvalue::Use(Operand::Copy(place.project(Projection::Field(1)))),
                )
            }
        }
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        lhs: ExprId,
        rhs: ExprId,
        wrapping: bool,
        ty: Ty,
    ) -> Operand {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return self.short_circuit(op, lhs, rhs);
        }
        let at = self.span(rhs);
        let lhs = self.scalar(lhs);
        let rhs = self.scalar(rhs);
        self.arithmetic(op, lhs, rhs, wrapping, ty, at)
    }

    /// `lhs op rhs` of two operands already worked out, with the checks
    /// the operator has: what `a op b` and `a op= b` share.
    fn arithmetic(
        &mut self,
        op: BinaryOp,
        lhs: Operand,
        rhs: Operand,
        wrapping: bool,
        ty: Ty,
        at: Span,
    ) -> Operand {
        // Dividing by zero panics, with the place it happened.
        if matches!(op, BinaryOp::Div | BinaryOp::Rem) && matches!(self.kind(ty), TyKind::Int(_)) {
            let zero = Self::int(0, ty);
            let fails = self.value(Types::BOOL, Rvalue::Binary(BinaryOp::Eq, rhs.clone(), zero));
            self.push(Statement::Check {
                fails,
                kind: CheckKind::Division,
                at,
            });
            // The most negative number divided by -1 is one more than the
            // largest, which does not fit.
            if let TyKind::Int(int) = self.kind(ty)
                && int.signed()
                && int.bits() <= 64
            {
                let most_negative = Self::int(1u128 << (int.bits() - 1), ty);
                let minus_one = Self::int(u128::MAX, ty);
                let edge = self.value(
                    Types::BOOL,
                    Rvalue::Binary(BinaryOp::Eq, lhs.clone(), most_negative),
                );
                let by = self.value(
                    Types::BOOL,
                    Rvalue::Binary(BinaryOp::Eq, rhs.clone(), minus_one),
                );
                let fails = self.value(Types::BOOL, Rvalue::Binary(BinaryOp::BitAnd, edge, by));
                self.push(Statement::Check {
                    fails,
                    kind: CheckKind::Overflow(op),
                    at,
                });
            }
        }
        // `+`, `-` and `*` on integers panic if the answer does not fit.
        if matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
            && !wrapping
            && self.checks_overflow(ty)
        {
            let dest = Place::local(self.temp(ty));
            self.push(Statement::Arith {
                dest: dest.clone(),
                op,
                lhs,
                rhs,
                at,
            });
            return Operand::Copy(dest);
        }
        self.value(ty, Rvalue::Binary(op, lhs, rhs))
    }

    /// Whether an answer of this type is checked: the integers, up to the
    /// widest the backend has an instruction for.
    fn checks_overflow(&self, ty: Ty) -> bool {
        matches!(self.kind(ty), TyKind::Int(int) if int.bits() <= 64)
    }

    /// A call. An aggregate result is built at `dest`, or in a temporary.
    /// The arguments are evaluated first, in order; an aggregate one is
    /// passed by the address of its place, and the callee drops what it owns.
    pub(super) fn call(
        &mut self,
        callee: Callee,
        ret: Ty,
        args: &[ExprId],
        order: &[u32],
        dest: Option<Place>,
    ) -> Value {
        self.call_with(callee, ret, args, order, dest, None)
    }

    /// A `@tailrec` function's call to itself. The arguments
    /// are built in temporaries first, as any call's are, so a parameter that
    /// one of them reads keeps its value until every one is ready; then the
    /// parameters are assigned and control goes back to the top of the body.
    /// Nothing of this frame needs dropping: the checker has made sure of it.
    pub(super) fn tail_call(&mut self, args: &[ExprId], order: &[u32]) {
        let mut values = Vec::with_capacity(args.len());
        for i in evaluation_order(order, args.len()) {
            let ty = self.ty(args[i]);
            let temp = Place::local(self.temp(ty));
            self.store_expr(args[i], temp.clone());
            values.push((i, temp));
        }
        for (i, temp) in values {
            let param = Place::local(self.params[i]);
            self.assign(param, Rvalue::Use(Operand::Copy(temp)));
        }
        let top = self.top.expect("a body with a tail call has a top");
        self.terminate(Terminator::Goto(top));
        self.dead = true;
    }

    /// A call through a closure: the code from the pair, and what it
    /// captured before the arguments.
    pub(super) fn closure_call(
        &mut self,
        code: Operand,
        captures: Operand,
        args: &[ExprId],
        ret: Ty,
        dest: Option<Place>,
    ) -> Value {
        let mut operands = Vec::with_capacity(args.len() + 1);
        operands.push(captures);
        for &arg in args {
            let operand = match self.expr(arg) {
                Value::Scalar(operand) => operand,
                Value::Place(place) => Operand::Copy(place),
                // A `void` argument, as a generic function called with
                // `void` for a type parameter takes one — `send(work())`
                // where the work answers nothing — was
                // evaluated for what it does, and holds nothing: its
                // parameter, taken as an aggregate is, by address, gets a
                // null one it never reads.
                Value::Unit => Self::int(0, Types::PTR_U8),
            };
            operands.push(operand);
        }
        self.finish_call(Callee::Value(code), ret, operands, dest)
    }

    /// A call through a `&dyn`: the address from the table, the value's
    /// pointer as the receiver, and the rest of the arguments as written.
    pub(super) fn dyn_call(
        &mut self,
        function: Operand,
        pointer: Operand,
        args: &[ExprId],
        order: &[u32],
        ret: Ty,
        dest: Option<Place>,
    ) -> Value {
        self.call_with(
            Callee::Value(function),
            ret,
            args,
            order,
            dest,
            Some(pointer),
        )
    }

    /// A call whose first argument may already be an operand: the receiver of
    /// a call through a `&dyn`.
    fn call_with(
        &mut self,
        callee: Callee,
        ret: Ty,
        args: &[ExprId],
        order: &[u32],
        dest: Option<Place>,
        first: Option<Operand>,
    ) -> Value {
        let aggregate = self.is_aggregate(ret);
        let result = if aggregate {
            Some(dest.unwrap_or_else(|| Place::local(self.temp(ret))))
        } else {
            None
        };
        // Evaluated in the order written, passed in the order of the
        // parameters.
        let mut operands: Vec<Option<Operand>> = vec![None; args.len()];
        let mut pending = Vec::new();
        let skip = usize::from(first.is_some());
        operands[..skip].fill(first);
        for (n, i) in evaluation_order(order, args.len()).enumerate() {
            if i < skip {
                continue;
            }
            let mut operand = match self.expr(args[i]) {
                Value::Scalar(operand) => operand,
                Value::Place(place) => Operand::Copy(place),
                // A `void` argument, as a generic function called with
                // `void` for a type parameter takes one — `send(work())`
                // where the work answers nothing — was
                // evaluated for what it does, and holds nothing: its
                // parameter, taken as an aggregate is, by address, gets a
                // null one it never reads.
                Value::Unit => Self::int(0, Types::PTR_U8),
            };
            // An aggregate is passed by the address of its place, and a
            // parameter taken by value belongs to the callee, which may move
            // out of it. A place the caller keeps is therefore copied first;
            // what the caller moved, or built here, is passed as it is.
            let ty = self.ty(args[i]);
            if self.is_aggregate(ty)
                && self.hir.is_place(args[i])
                && let Operand::Copy(place) = &operand
            {
                let temp = Place::local(self.temp(ty));
                self.assign(temp.clone(), Rvalue::Use(Operand::Copy(place.clone())));
                operand = Operand::Copy(temp);
            }
            if n + 1 < args.len()
                && let Operand::Copy(place) = &operand
            {
                self.pending_part(place.clone(), self.ty(args[i]), &mut pending);
            }
            operands[i] = Some(operand);
        }
        self.forget_parts(pending);
        let operands: Vec<Operand> = operands
            .into_iter()
            .map(|operand| operand.expect("every argument was evaluated"))
            .collect();
        self.finish_call(callee, ret, operands, result)
    }

    /// The call itself, once its arguments are operands: where its result
    /// goes, and the path ending where it does not return.
    fn finish_call(
        &mut self,
        callee: Callee,
        ret: Ty,
        operands: Vec<Operand>,
        dest: Option<Place>,
    ) -> Value {
        let result = match dest {
            Some(place) => Some(place),
            None if self.is_aggregate(ret) => Some(Place::local(self.temp(ret))),
            None => None,
        };
        if self.ty_is_never(ret) {
            self.push(Statement::Call {
                callee,
                args: operands,
                dest: None,
            });
            self.terminate(Terminator::Unreachable);
            self.dead = true;
            return Value::Unit;
        }
        match result {
            Some(place) => {
                self.push(Statement::Call {
                    callee: callee.clone(),
                    args: operands,
                    dest: Some(place.clone()),
                });
                Value::Place(place)
            }
            None if self.is_scalar(ret) => {
                let temp = Place::local(self.temp(ret));
                self.push(Statement::Call {
                    callee: callee.clone(),
                    args: operands,
                    dest: Some(temp.clone()),
                });
                Value::Scalar(Operand::Copy(temp))
            }
            None => {
                self.push(Statement::Call {
                    callee,
                    args: operands,
                    dest: None,
                });
                Value::Unit
            }
        }
    }
}

impl Builder<'_> {
    /// A cast of a constant integer or character to an integer type, as the
    /// constant it makes: extended by the source's sign, then cut to the
    /// target's width, as the instruction would. Anything else is left to
    /// the instruction.
    fn folded_cast(&self, operand: &Operand, to: Ty) -> Option<Operand> {
        let Operand::Const(Const::Int { bits, ty: from }) = operand else {
            return None;
        };
        let width = |kind: TyKind| match kind {
            TyKind::Int(int) => Some((int.bits(), int.signed())),
            TyKind::Char => Some((32, false)),
            _ => None,
        };
        let (from_bits, signed) = width(self.kind(*from))?;
        let (to_bits, _) = match self.kind(to) {
            TyKind::Int(int) => (int.bits(), int.signed()),
            _ => return None,
        };
        let mut value = *bits;
        if signed && from_bits < 128 && (value >> (from_bits - 1)) & 1 == 1 {
            value |= u128::MAX << from_bits;
        }
        if to_bits < 128 {
            value &= (1u128 << to_bits) - 1;
        }
        Some(Operand::Const(Const::Int {
            bits: value,
            ty: to,
        }))
    }

    /// A body the compiler writes: the five operations on
    /// `Slots<T>`, the one type that holds memory holding nothing.
    pub(super) fn intrinsic_call(
        &mut self,
        callee: FnId,
        args: &[ExprId],
        ty: Ty,
        at: Span,
        dest: Option<Place>,
    ) -> Value {
        let which = self.program.fns[callee].intrinsic.expect("an intrinsic");
        match which {
            // `Slots::alloc(count)`: the block, with nothing in it.
            Intrinsic::SlotsAlloc => {
                let count = self.scalar(args[0]);
                let elem = element_ty(self.program, ty).expect("a block of slots has elements");
                let dest = dest.unwrap_or_else(|| Place::local(self.temp(ty)));
                self.push(Statement::AllocBuffer {
                    dest: dest.clone(),
                    elem,
                    count,
                });
                Value::Place(dest)
            }
            // `slots.moveIn(index, value)`: the slot held nothing, so
            // nothing is dropped.
            Intrinsic::SlotsMoveIn => {
                let slot = self.slot_place(args[0], args[1], at);
                self.store_expr(args[2], slot);
                Value::Unit
            }
            // `slots.moveOut(index)`: the value comes out and the slot is
            // left holding nothing, which its zeros say.
            Intrinsic::SlotsMoveOut => {
                let slot = self.slot_place(args[0], args[1], at);
                let value = match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Use(Operand::Copy(slot.clone())));
                        Value::Place(dest)
                    }
                    _ if self.is_aggregate(ty) => {
                        let temp = Place::local(self.temp(ty));
                        self.assign(temp.clone(), Rvalue::Use(Operand::Copy(slot.clone())));
                        Value::Place(temp)
                    }
                    _ => {
                        let value = self.value(ty, Rvalue::Use(Operand::Copy(slot.clone())));
                        Value::Scalar(value)
                    }
                };
                self.push(Statement::Zero(slot));
                value
            }
            // `assumed(&option)`: the address of what `.Some` holds, which a
            // projection lends. Not checked: the library calls it where what
            // it keeps says the option holds a value.
            Intrinsic::OptionAssumed => {
                let option = self.behind_reference(args[0]);
                let payload = option.project(Projection::VariantField {
                    variant: 0,
                    field: 0,
                });
                let address = Rvalue::AddressOf(payload);
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), address);
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, address)),
                }
            }
            // `slots.slot(index)`: the slot's address, which a projection
            // lends. Not checked: `Vec` checked the index against its length,
            // which is the bound that matters, and the slots' own count is
            // never less.
            Intrinsic::SlotsSlot => {
                let block = self.behind_reference(args[0]);
                let index = self.scalar(args[1]);
                let index = self.local_of(index, Types::I64);
                let address = Rvalue::AddressOf(block.project(Projection::Index(index)));
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), address);
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, address)),
                }
            }
            // `slots.takeBuffer(length)`: a block of slots and a buffer are
            // both a pointer and a count, so the buffer is the block's
            // pointer and the length of what it holds; the slots are
            // zeroed, and so free nothing when they are dropped.
            Intrinsic::SlotsTakeBuffer => {
                let block = self.behind_reference(args[0]);
                let length = self.scalar(args[1]);
                let dest = dest.unwrap_or_else(|| Place::local(self.temp(ty)));
                self.assign(
                    dest.clone().project(Projection::Field(0)),
                    Rvalue::Use(Operand::Copy(block.clone().project(Projection::Field(0)))),
                );
                self.assign(
                    dest.clone().project(Projection::Field(1)),
                    Rvalue::Use(length),
                );
                self.push(Statement::Zero(block));
                Value::Place(dest)
            }
            // `Slots::takeFrom(&var buffer)`: the buffer's pointer and
            // length are the block's, and the buffer, zeroed, frees nothing
            // when it is dropped.
            Intrinsic::SlotsTakeFrom => {
                let buffer = self.behind_reference(args[0]);
                let dest = dest.unwrap_or_else(|| Place::local(self.temp(ty)));
                self.assign(dest.clone(), Rvalue::Use(Operand::Copy(buffer.clone())));
                self.push(Statement::Zero(buffer));
                Value::Place(dest)
            }
            // `str::fromBytes(bytes)`: the same pointer and the same
            // length, read as text. Nothing is copied but the pair itself.
            Intrinsic::StrFromBytes => {
                let bytes = self.aggregate(args[0]);
                let dest = dest.unwrap_or_else(|| Place::local(self.temp(ty)));
                self.assign(
                    dest.clone().project(Projection::Field(0)),
                    Rvalue::Use(Operand::Copy(bytes.clone().project(Projection::Field(0)))),
                );
                self.assign(
                    dest.clone().project(Projection::Field(1)),
                    Rvalue::Use(Operand::Copy(bytes.project(Projection::Field(1)))),
                );
                Value::Place(dest)
            }
            // An `own` is its box's address, which it keeps.
            Intrinsic::BoxAddress => {
                let boxed = self.behind_reference(args[0]);
                let address = Rvalue::Cast(Operand::Copy(boxed), ty);
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), address);
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, address)),
                }
            }
            // A slice made of an address and a count: the pair it is.
            Intrinsic::SliceAt => {
                let first = self.scalar(args[0]);
                let count = self.scalar(args[1]);
                let dest = dest.unwrap_or_else(|| Place::local(self.temp(ty)));
                self.assign(
                    dest.clone().project(Projection::Field(0)),
                    Rvalue::Use(first),
                );
                self.assign(
                    dest.clone().project(Projection::Field(1)),
                    Rvalue::Use(count),
                );
                Value::Place(dest)
            }
            // A lent closure is a pair: what it captured, then its code;
            // each is the address a thread is given.
            Intrinsic::ClosureCode | Intrinsic::ClosureCaptures => {
                let closure = self.place(args[0]);
                let field = match which {
                    Intrinsic::ClosureCaptures => 0,
                    _ => 1,
                };
                let half = Rvalue::Use(Operand::Copy(closure.project(Projection::Field(field))));
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), half);
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, half)),
                }
            }
            // `text.items()`: the pointer and the length a `str` already
            // is, read as a slice.
            Intrinsic::StrBytes => {
                let text = self.behind_reference(args[0]);
                let dest = dest.unwrap_or_else(|| Place::local(self.temp(ty)));
                self.assign(
                    dest.clone().project(Projection::Field(0)),
                    Rvalue::Use(Operand::Copy(text.clone().project(Projection::Field(0)))),
                );
                self.assign(
                    dest.clone().project(Projection::Field(1)),
                    Rvalue::Use(Operand::Copy(text.project(Projection::Field(1)))),
                );
                Value::Place(dest)
            }
            // `values.swap(i, j)`: the two elements change places, through
            // a temporary. Nothing is dropped and no place is left
            // holding nothing, which is what makes a swap something a
            // move out of an element is not.
            Intrinsic::SliceSwap => {
                let slice = self.behind_reference(args[0]);
                let elem =
                    element_ty(self.program, self.ty(args[0])).expect("a slice has elements");
                let first = self.element_place(slice.clone(), args[1], at);
                let second = self.element_place(slice, args[2], at);
                let temp = Place::local(self.temp(elem));
                self.assign(temp.clone(), Rvalue::Use(Operand::Copy(first.clone())));
                self.assign(first.clone(), Rvalue::Use(Operand::Copy(second.clone())));
                self.assign(second, Rvalue::Use(Operand::Copy(temp)));
                Value::Unit
            }
            // `values.len()`, `text.len()`: the second half of the pair a
            // slice and a `str` are.
            Intrinsic::Len => {
                let pair = self.behind_reference(args[0]);
                let len = Operand::Copy(pair.project(Projection::Field(1)));
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Use(len));
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, Rvalue::Use(len))),
                }
            }
            // `char::fromScalar(code)`: the same 32 bits.
            // An `own` and a `ptr<u8>` are both one word: the address.
            // What moved into the call is the answer's now. A `&var` is
            // that word too.
            Intrinsic::OwnIntoAddress
            | Intrinsic::OwnFromAddress
            | Intrinsic::ReferenceAddress
            | Intrinsic::AddressReference => {
                let address = self.scalar(args[0]);
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Use(address));
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, Rvalue::Use(address))),
                }
            }
            // One step on a value other threads may touch: the runtime's
            // counters, and `std::sync::Atomic`.
            Intrinsic::AtomicAdd
            | Intrinsic::AtomicSubtract
            | Intrinsic::AtomicSwap
            | Intrinsic::AtomicCompareSwap
            | Intrinsic::AtomicLoad
            | Intrinsic::AtomicStore => {
                let address = self.scalar(args[0]);
                let (op, value, expected) = match which {
                    Intrinsic::AtomicAdd => (AtomicOp::Add, Some(self.scalar(args[1])), None),
                    Intrinsic::AtomicSubtract => {
                        (AtomicOp::Subtract, Some(self.scalar(args[1])), None)
                    }
                    Intrinsic::AtomicSwap => (AtomicOp::Swap, Some(self.scalar(args[1])), None),
                    Intrinsic::AtomicCompareSwap => {
                        let expected = self.scalar(args[1]);
                        (
                            AtomicOp::CompareSwap,
                            Some(self.scalar(args[2])),
                            Some(expected),
                        )
                    }
                    Intrinsic::AtomicLoad => (AtomicOp::Load, None, None),
                    _ => (AtomicOp::Store, Some(self.scalar(args[1])), None),
                };
                if op == AtomicOp::Store {
                    let stored = self.ty(args[1]);
                    self.push(Statement::Atomic {
                        op,
                        ty: stored,
                        address,
                        value,
                        expected,
                        dest: None,
                    });
                    return Value::Unit;
                }
                // A compare-and-swap answers what was there, and the call
                // whether that was what it expected: then it swapped.
                if op == AtomicOp::CompareSwap {
                    let held = self.ty(args[1]);
                    let before = Place::local(self.temp(held));
                    let wanted = expected.clone().expect("a compare-and-swap expects");
                    self.push(Statement::Atomic {
                        op,
                        ty: held,
                        address,
                        value,
                        expected,
                        dest: Some(before.clone()),
                    });
                    let swapped = Rvalue::Binary(BinaryOp::Eq, Operand::Copy(before), wanted);
                    return match dest {
                        Some(dest) => {
                            self.assign(dest.clone(), swapped);
                            Value::Place(dest)
                        }
                        None => Value::Scalar(self.value(ty, swapped)),
                    };
                }
                let place = dest.clone().unwrap_or_else(|| Place::local(self.temp(ty)));
                self.push(Statement::Atomic {
                    op,
                    ty,
                    address,
                    value,
                    expected,
                    dest: Some(place.clone()),
                });
                match dest {
                    Some(dest) => Value::Place(dest),
                    None => Value::Scalar(Operand::Copy(place)),
                }
            }
            // A pointer as a number, moved on, and a pointer again: what C
            // writes `p + n`.
            Intrinsic::OffsetBytes => {
                let pointer = self.scalar(args[0]);
                let bytes = self.scalar(args[1]);
                let number = self.value(Types::I64, Rvalue::Cast(pointer, Types::I64));
                let moved = self.value(Types::I64, Rvalue::Binary(BinaryOp::Add, number, bytes));
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Cast(moved, ty));
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, Rvalue::Cast(moved, ty))),
                }
            }
            // C's `sizeof` and `_Alignof` of the type the call names,
            // which the layout answers.
            Intrinsic::SizeOf | Intrinsic::AlignOf => {
                let (_, type_args) = self.program.fns[callee]
                    .instance_of
                    .expect("`sizeOf` and `alignOf` are generic");
                let of = self.program.types.list(type_args)[0];
                let answer = Operand::Const(match which {
                    Intrinsic::SizeOf => Const::SizeOf { of, ty },
                    _ => Const::AlignOf { of, ty },
                });
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Use(answer));
                        Value::Place(dest)
                    }
                    None => Value::Scalar(answer),
                }
            }
            // The checker reads the file and answers the call with the
            // constant that holds it.
            Intrinsic::EmbedBytes | Intrinsic::EmbedText => {
                unreachable!("a file embedded is a constant, never a call")
            }
            Intrinsic::FrameTables => {
                let answer = Operand::Const(Const::FrameTables);
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Use(answer));
                        Value::Place(dest)
                    }
                    None => Value::Scalar(answer),
                }
            }
            Intrinsic::RuntimeWords => {
                let words = Operand::Const(Const::RuntimeWords);
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Use(words));
                        Value::Place(dest)
                    }
                    None => Value::Scalar(words),
                }
            }
            // The value is copied out of the box, which is then freed
            // with nothing in it dropped: the value is the answer's.
            Intrinsic::OwnUnbox => {
                let boxed = self.scalar(args[0]);
                let boxed_ty = self.ty(args[0]);
                let pointer = self.local_of(boxed, boxed_ty);
                let inside = Place::local(pointer).project(Projection::Deref);
                let value = match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Use(Operand::Copy(inside)));
                        Value::Place(dest)
                    }
                    None if self.is_aggregate(ty) => {
                        let temp = Place::local(self.temp(ty));
                        self.assign(temp.clone(), Rvalue::Use(Operand::Copy(inside)));
                        Value::Place(temp)
                    }
                    None => Value::Scalar(self.value(ty, Rvalue::Use(Operand::Copy(inside)))),
                };
                self.push(Statement::Free(Operand::Copy(Place::local(pointer))));
                value
            }
            Intrinsic::CharFromScalar => {
                let code = self.scalar(args[0]);
                match dest {
                    // Where the caller has a place for it, such as a
                    // variant's payload, it goes there.
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Cast(code, ty));
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, Rvalue::Cast(code, ty))),
                }
            }
            // `cstring::fromBytes(bytes)`: the pointer, and nothing else.
            // What makes it a C string is the NUL the caller put there.
            // What a float is made of, what the machine does to one exactly,
            // and what it counts and turns of an integer's
            // bits.
            Intrinsic::FloatToBits
            | Intrinsic::FloatFromBits
            | Intrinsic::FloatSqrt
            | Intrinsic::FloatFloor
            | Intrinsic::FloatCeil
            | Intrinsic::FloatTrunc
            | Intrinsic::FloatMulAdd
            | Intrinsic::IntCountOnes
            | Intrinsic::IntLeadingZeros
            | Intrinsic::IntTrailingZeros
            | Intrinsic::IntSwapBytes
            | Intrinsic::IntReverseBits
            | Intrinsic::IntRotateLeft
            | Intrinsic::IntRotateRight => {
                // A method's receiver is lent; `fromBits` takes the bits.
                let operand = match which {
                    Intrinsic::FloatFromBits => self.scalar(args[0]),
                    _ => Operand::Copy(self.behind_reference(args[0])),
                };
                let rvalue = match which {
                    Intrinsic::FloatToBits | Intrinsic::FloatFromBits => Rvalue::Bits(operand),
                    Intrinsic::FloatSqrt => Rvalue::Float(FloatOp::Sqrt, operand),
                    Intrinsic::FloatFloor => Rvalue::Float(FloatOp::Floor, operand),
                    Intrinsic::FloatCeil => Rvalue::Float(FloatOp::Ceil, operand),
                    Intrinsic::FloatTrunc => Rvalue::Float(FloatOp::Trunc, operand),
                    Intrinsic::FloatMulAdd => {
                        let by = self.scalar(args[1]);
                        let plus = self.scalar(args[2]);
                        Rvalue::MulAdd(operand, by, plus)
                    }
                    Intrinsic::IntCountOnes => Rvalue::Integer(IntegerOp::CountOnes, operand),
                    Intrinsic::IntLeadingZeros => Rvalue::Integer(IntegerOp::LeadingZeros, operand),
                    Intrinsic::IntTrailingZeros => {
                        Rvalue::Integer(IntegerOp::TrailingZeros, operand)
                    }
                    Intrinsic::IntSwapBytes => Rvalue::Integer(IntegerOp::SwapBytes, operand),
                    Intrinsic::IntReverseBits => Rvalue::Integer(IntegerOp::ReverseBits, operand),
                    Intrinsic::IntRotateLeft => {
                        let amount = self.scalar(args[1]);
                        Rvalue::Rotate(Turn::Left, operand, amount)
                    }
                    _ => {
                        let amount = self.scalar(args[1]);
                        Rvalue::Rotate(Turn::Right, operand, amount)
                    }
                };
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), rvalue);
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, rvalue)),
                }
            }
            // Both are the slice's pointer, alone: a C string's first byte,
            // and the first element C is lent to read.
            Intrinsic::CstringFromBytes | Intrinsic::SlicePointer => {
                let bytes = self.aggregate(args[0]);
                let pointer = Operand::Copy(bytes.project(Projection::Field(0)));
                match dest {
                    Some(dest) => {
                        self.assign(dest.clone(), Rvalue::Use(pointer));
                        Value::Place(dest)
                    }
                    None => Value::Scalar(self.value(ty, Rvalue::Use(pointer))),
                }
            }
        }
    }

    /// The place of one element of a slice, checked against how many there
    /// are, as an index written in the program would be.
    fn element_place(&mut self, slice: Place, index: ExprId, at: Span) -> Place {
        let index = self.scalar(index);
        let length = Operand::Copy(slice.clone().project(Projection::Field(1)));
        self.check_unsigned(BinaryOp::Ge, index.clone(), length, at);
        let index = self.local_of(index, Types::I64);
        slice.project(Projection::Index(index))
    }

    /// The place of one slot, checked against how many there are. The first
    /// argument is the block, behind the `&var` a method takes.
    fn slot_place(&mut self, slots: ExprId, index: ExprId, at: Span) -> Place {
        let block = self.behind_reference(slots);
        let index = self.scalar(index);
        let length = Operand::Copy(block.clone().project(Projection::Field(1)));
        self.check_unsigned(BinaryOp::Ge, index.clone(), length, at);
        let index = self.local_of(index, Types::I64);
        block.project(Projection::Index(index))
    }

    /// The place a reference argument refers to: the place itself where the
    /// argument is `&x`, and a dereference otherwise.
    fn behind_reference(&mut self, id: ExprId) -> Place {
        if let ExprKind::Ref(inner) = self.hir.exprs[id].kind {
            return self.place(inner);
        }
        let ty = self.ty(id);
        let reference = self.scalar(id);
        let local = self.local_of(reference, ty);
        Place::local(local).project(Projection::Deref)
    }
}
