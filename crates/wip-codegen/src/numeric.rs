//! Integer constants, casts, and the 128-bit operations that go through the
//! runtime.

use super::*;

impl FnCodegen<'_, '_, '_> {
    /// An integer constant of type `ty`, from the bits of a literal. An
    /// `i128` is built from two 64-bit halves, since a Cranelift immediate
    /// holds at most 64 bits.
    pub(super) fn int_const(&mut self, ty: ir::Type, bits: u128) -> Value {
        if ty == types::I128 {
            let lo = self.b.ins().iconst(types::I64, bits as u64 as i64);
            let hi = self.b.ins().iconst(types::I64, (bits >> 64) as u64 as i64);
            return self.b.ins().iconcat(lo, hi);
        }
        // The literal's bits are already truncated to the type's width.
        self.b.ins().iconst(ty, bits as u64 as i64)
    }

    /// 128-bit division or remainder through the runtime's `wip_int128_div`,
    /// since Cranelift does not lower them on every target. The operands go
    /// as 64-bit halves and the result comes back through a stack slot, so no
    /// 128-bit calling convention is involved.
    pub(super) fn int128_div(
        &mut self,
        remainder: bool,
        signed: bool,
        l: Value,
        r: Value,
    ) -> Value {
        let (l_lo, l_hi) = self.b.ins().isplit(l);
        let (r_lo, r_hi) = self.b.ins().isplit(r);
        // 0: unsigned division, 1: unsigned remainder, 2: signed division,
        // 3: signed remainder.
        let which = i64::from(signed) * 2 + i64::from(remainder);
        let which = self.b.ins().iconst(types::I64, which);
        self.call_for_int128(self.cg.int128_div_fn, &[l_lo, l_hi, r_lo, r_hi, which])
    }

    /// A 128-bit integer converted to a float by the runtime. An `f32` result
    /// is rounded to `f32` there, so it is not rounded twice.
    pub(super) fn int128_to_float(&mut self, value: Value, signed: bool, to: FloatTy) -> Value {
        let (lo, hi) = self.b.ins().isplit(value);
        let flags = i64::from(signed) | (i64::from(to == FloatTy::F32) << 1);
        let flags = self.b.ins().iconst(types::I64, flags);
        let callee = self.runtime_ref(self.cg.int128_to_float_fn);
        let call = self.b.ins().call(callee, &[lo, hi, flags]);
        let result = self.b.inst_results(call)[0];
        match to {
            FloatTy::F32 => self.b.ins().fdemote(types::F32, result),
            FloatTy::F64 => result,
        }
    }

    /// A float converted to a 128-bit integer by the runtime: toward zero,
    /// saturating, NaN becoming 0.
    pub(super) fn float_to_int128(&mut self, value: Value, from: FloatTy, signed: bool) -> Value {
        let value = match from {
            FloatTy::F32 => self.b.ins().fpromote(types::F64, value),
            FloatTy::F64 => value,
        };
        let signed = self.b.ins().iconst(types::I64, i64::from(signed));
        self.call_for_int128(self.cg.float_to_int128_fn, &[value, signed])
    }

    /// Calls a runtime helper that writes a 128-bit result through a pointer
    /// passed after `args`, and returns that result.
    pub(super) fn call_for_int128(&mut self, helper: FuncId, args: &[Value]) -> Value {
        let slot =
            self.b
                .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 4));
        let out = self.b.ins().stack_addr(types::I64, slot, 0);
        let mut args = args.to_vec();
        args.push(out);
        let callee = self.runtime_ref(helper);
        self.b.ins().call(callee, &args);
        let lo = self
            .b
            .ins()
            .load(types::I64, MemFlagsData::trusted(), out, 0);
        let hi = self
            .b
            .ins()
            .load(types::I64, MemFlagsData::trusted(), out, 8);
        self.b.ins().iconcat(lo, hi)
    }

    /// `operand as to`.
    pub(super) fn cast(&mut self, operand: &Operand, to: Ty) -> Value {
        // A character converts as the `u32` it is held in.
        let as_number = |kind: TyKind| match kind {
            TyKind::Char => TyKind::Int(wip_hir::IntTy::U32),
            other => other,
        };
        let from = as_number(self.kind(self.operand_ty(operand)));
        let target = self
            .cg
            .scalar_type(to)
            .expect("casts are between numeric types");
        let value = self.operand(operand);
        let to = as_number(self.kind(to));
        // Cranelift does not convert between 128-bit integers and floats on
        // every target; the runtime does.
        match (from, to) {
            (TyKind::Int(a), TyKind::Float(f)) if a.bits() == 128 => {
                return self.int128_to_float(value, a.signed(), f);
            }
            (TyKind::Float(f), TyKind::Int(b)) if b.bits() == 128 => {
                return self.float_to_int128(value, f, b.signed());
            }
            // Cranelift's x86-64 backend converts a float to a 32- or a
            // 64-bit integer and to nothing narrower, so a narrower one is
            // converted to 32 bits, held to its own range, and cut down.
            // Saturating is what `as` promises, and
            // clamping before the cut is what keeps it: 300.0 as u8 is
            // 255, not the low byte of 300.
            (TyKind::Float(_), TyKind::Int(b)) if b.bits() < 32 => {
                return self.float_to_narrow_int(value, b, target);
            }
            _ => {}
        }
        let ins = self.b.ins();
        match (from, to) {
            (from, to) if from == to => value,
            // One C pointer as another: the same word.
            (TyKind::Ptr(_), TyKind::Ptr(_)) => value,
            // `intoAddress(own x) as ptr<T>`, where the simplifier has
            // put the `own` in place of the address it is: one word.
            (TyKind::Own(_), TyKind::Ptr(_)) => value,
            // A pointer as the number it is, and back: only the runtime's
            // `offsetBytes` asks for this.
            (TyKind::Ptr(_), TyKind::Int(_)) | (TyKind::Int(_), TyKind::Ptr(_)) => value,
            // Widening extends by the source's signedness; narrowing keeps
            // the low bits; the same width reinterprets them.
            (TyKind::Int(a), TyKind::Int(b)) => match a.bits().cmp(&b.bits()) {
                std::cmp::Ordering::Less if a.signed() => ins.sextend(target, value),
                std::cmp::Ordering::Less => ins.uextend(target, value),
                std::cmp::Ordering::Greater => ins.ireduce(target, value),
                std::cmp::Ordering::Equal => value,
            },
            (TyKind::Int(a), TyKind::Float(_)) if a.signed() => ins.fcvt_from_sint(target, value),
            (TyKind::Int(_), TyKind::Float(_)) => ins.fcvt_from_uint(target, value),
            // A float converts toward zero, saturating at the ends of the
            // range.
            (TyKind::Float(_), TyKind::Int(b)) if b.signed() => ins.fcvt_to_sint_sat(target, value),
            (TyKind::Float(_), TyKind::Int(_)) => ins.fcvt_to_uint_sat(target, value),
            (TyKind::Float(FloatTy::F32), TyKind::Float(_)) => ins.fpromote(target, value),
            (TyKind::Float(_), TyKind::Float(_)) => ins.fdemote(target, value),
            (from, to) => {
                unreachable!("the type checker allows only numeric conversions: {from:?} to {to:?}")
            }
        }
    }

    /// A float to an 8- or 16-bit integer: through 32 bits, clamped to
    /// what the narrow type holds, then cut down. Only x86-64 needs the
    /// detour, and doing it everywhere keeps one answer on every target.
    fn float_to_narrow_int(
        &mut self,
        value: Value,
        to: wip_hir::IntTy,
        target: types::Type,
    ) -> Value {
        let wide = types::I32;
        let (low, high) = match to.signed() {
            true => (-(1i64 << (to.bits() - 1)), (1i64 << (to.bits() - 1)) - 1),
            false => (0, (1i64 << to.bits()) - 1),
        };
        let ins = self.b.ins();
        let wide_value = match to.signed() {
            true => ins.fcvt_to_sint_sat(wide, value),
            false => ins.fcvt_to_uint_sat(wide, value),
        };
        let high = self.b.ins().iconst(wide, high);
        let held = match to.signed() {
            true => {
                let low = self.b.ins().iconst(wide, low);
                let held = self.b.ins().smin(wide_value, high);
                self.b.ins().smax(held, low)
            }
            false => self.b.ins().umin(wide_value, high),
        };
        self.b.ins().ireduce(target, held)
    }

    pub(super) fn binary(&mut self, op: BinaryOp, lhs: &Operand, rhs: &Operand) -> Value {
        use BinaryOp::*;
        let operand = self.kind(self.operand_ty(lhs));
        let l = self.operand(lhs);
        let r = self.operand(rhs);
        if let TyKind::Int(t) = operand
            && t.bits() == 128
            && matches!(op, Div | Rem)
        {
            return self.int128_div(op == Rem, t.signed(), l, r);
        }
        let ins = self.b.ins();
        if let TyKind::Float(_) = operand {
            return match op {
                Add => ins.fadd(l, r),
                Sub => ins.fsub(l, r),
                Mul => ins.fmul(l, r),
                Div => ins.fdiv(l, r),
                Eq => ins.fcmp(FloatCC::Equal, l, r),
                Ne => ins.fcmp(FloatCC::NotEqual, l, r),
                Lt => ins.fcmp(FloatCC::LessThan, l, r),
                Le => ins.fcmp(FloatCC::LessThanOrEqual, l, r),
                Gt => ins.fcmp(FloatCC::GreaterThan, l, r),
                Ge => ins.fcmp(FloatCC::GreaterThanOrEqual, l, r),
                Rem | And | Or | BitAnd | BitOr | BitXor | Shl | Shr => {
                    unreachable!("the type checker rejects `{op:?}` on floats")
                }
            };
        }
        // Unsigned integers divide and compare as unsigned;
        // `bool` and pointers compare as unsigned too.
        let unsigned = !matches!(operand, TyKind::Int(t) if t.signed());
        let order = |signed: IntCC, unsigned_cc: IntCC| if unsigned { unsigned_cc } else { signed };
        match op {
            Add => ins.iadd(l, r),
            Sub => ins.isub(l, r),
            Mul => ins.imul(l, r),
            // Division by zero traps, and so does `MIN / -1` for signed types.
            Div if unsigned => ins.udiv(l, r),
            Div => ins.sdiv(l, r),
            Rem if unsigned => ins.urem(l, r),
            Rem => ins.srem(l, r),
            BitAnd => ins.band(l, r),
            BitOr => ins.bor(l, r),
            BitXor => ins.bxor(l, r),
            // Cranelift takes the shift amount modulo the width, which is
            // Wip's rule too. `>>` fills with the sign bit for a
            // signed type, and with zeros for an unsigned one.
            Shl => ins.ishl(l, r),
            Shr if unsigned => ins.ushr(l, r),
            Shr => ins.sshr(l, r),
            Eq => ins.icmp(IntCC::Equal, l, r),
            Ne => ins.icmp(IntCC::NotEqual, l, r),
            Lt => ins.icmp(order(IntCC::SignedLessThan, IntCC::UnsignedLessThan), l, r),
            Le => ins.icmp(
                order(IntCC::SignedLessThanOrEqual, IntCC::UnsignedLessThanOrEqual),
                l,
                r,
            ),
            Gt => ins.icmp(
                order(IntCC::SignedGreaterThan, IntCC::UnsignedGreaterThan),
                l,
                r,
            ),
            Ge => ins.icmp(
                order(
                    IntCC::SignedGreaterThanOrEqual,
                    IntCC::UnsignedGreaterThanOrEqual,
                ),
                l,
                r,
            ),
            And | Or => unreachable!("`&&` and `||` are branches in MIR"),
        }
    }
}
