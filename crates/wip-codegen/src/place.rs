//! Places and operands: addresses, loads and stores, enum tags and
//! constants.

use super::*;

impl FnCodegen<'_, '_, '_> {
    pub(super) fn kind(&self, ty: Ty) -> TyKind {
        self.cg.kind(ty)
    }

    pub(super) fn place_ty(&self, place: &Place) -> Ty {
        mir::place_ty(self.cg.program, self.body, place)
    }

    pub(super) fn operand_ty(&self, operand: &Operand) -> Ty {
        mir::operand_ty(self.cg.program, self.body, operand)
    }

    fn storage(&self, local: Local) -> Storage {
        self.storage[local.0 as usize]
    }

    /// The address of a place.
    pub(super) fn place_addr(&mut self, place: &Place) -> Value {
        let decl_ty = self.body.local(place.local).ty;
        let mut projections = place.projections.iter();
        let (mut addr, mut ty) = match self.storage(place.local) {
            Storage::Slot(slot) => (self.b.ins().stack_addr(types::I64, slot, 0), decl_ty),
            Storage::Ptr(ptr) => (ptr, decl_ty),
            Storage::Empty => (self.b.ins().iconst(types::I64, 0), decl_ty),
            // A variable has no address, so a place starting at one goes
            // through the pointer it holds.
            Storage::Var(var) => {
                let pointer = self.b.use_var(var);
                match projections.next() {
                    Some(&Projection::Deref) => (
                        pointer,
                        mir::project_ty(self.cg.program, decl_ty, Projection::Deref),
                    ),
                    // `p[i]` through a C pointer: the elements are where it
                    // points, so the pointer itself is the first of them.
                    Some(&Projection::Index(index)) => {
                        let elem =
                            mir::project_ty(self.cg.program, decl_ty, Projection::Index(index));
                        let stride = i64::from(self.cg.layout(elem).stride());
                        let index = self.local_value(index);
                        let offset = self.b.ins().imul_imm_s(index, stride);
                        (self.b.ins().iadd(pointer, offset), elem)
                    }
                    _ => unreachable!("the address of a local that has none"),
                }
            }
        };
        for &projection in projections {
            addr = self.project(addr, ty, projection);
            ty = mir::project_ty(self.cg.program, ty, projection);
        }
        addr
    }

    /// The address `projection` reaches from a place of type `ty` at `addr`.
    fn project(&mut self, addr: Value, ty: Ty, projection: Projection) -> Value {
        match projection {
            Projection::Deref => self
                .b
                .ins()
                .load(types::I64, MemFlagsData::trusted(), addr, 0),
            Projection::Field(index) => {
                let offset = match self.kind(ty) {
                    TyKind::Struct(..) => self.cg.layouts.field_offset(self.cg.program, ty, index),
                    // A `str`'s or a slice's pointer, then its length.
                    _ => index * 8,
                };
                self.b.ins().iadd_imm_s(addr, i64::from(offset))
            }
            Projection::VariantField { variant, field } => {
                let offset = self
                    .cg
                    .layouts
                    .variant_offset(self.cg.program, ty, variant, field);
                self.b.ins().iadd_imm_s(addr, i64::from(offset))
            }
            Projection::Index(_) | Projection::ConstIndex(_) => {
                let elem = mir::element_ty(self.cg.program, ty).expect("elements");
                // An array's elements are where the array is; a slice's are
                // where its pointer points.
                let first = match self.kind(ty) {
                    TyKind::Array(..) => addr,
                    _ => self
                        .b
                        .ins()
                        .load(types::I64, MemFlagsData::trusted(), addr, 0),
                };
                let stride = i64::from(self.cg.layout(elem).stride());
                match projection {
                    Projection::Index(index) => {
                        let index = self.local_value(index);
                        let offset = self.b.ins().imul_imm_s(index, stride);
                        self.b.ins().iadd(first, offset)
                    }
                    Projection::ConstIndex(index) => self
                        .b
                        .ins()
                        .iadd_imm_s(first, stride.wrapping_mul(index as i64)),
                    _ => unreachable!(),
                }
            }
        }
    }

    /// The value of a scalar local.
    fn local_value(&mut self, local: Local) -> Value {
        self.operand(&Operand::Copy(Place::local(local)))
    }

    /// The value of a scalar operand.
    pub(super) fn operand(&mut self, operand: &Operand) -> Value {
        match operand {
            Operand::Const(constant) => self.constant(constant),
            Operand::Copy(place) => {
                if place.projections.is_empty()
                    && let Storage::Var(var) = self.storage(place.local)
                {
                    return self.b.use_var(var);
                }
                let ty = self.place_ty(place);
                let scalar = self
                    .cg
                    .scalar_type(ty)
                    .expect("an aggregate operand is passed by address");
                let addr = self.place_addr(place);
                self.b.ins().load(scalar, MemFlagsData::trusted(), addr, 0)
            }
        }
    }

    fn constant(&mut self, constant: &Const) -> Value {
        match *constant {
            Const::Int { bits, ty } => {
                let scalar = self
                    .cg
                    .scalar_type(ty)
                    .expect("integers have a machine type");
                self.int_const(scalar, bits)
            }
            Const::Float { value, ty } => match self.kind(ty) {
                TyKind::Float(FloatTy::F32) => self.b.ins().f32const(value as f32),
                _ => self.b.ins().f64const(value),
            },
            Const::Bool(value) => self.b.ins().iconst(types::I8, i64::from(value)),
            Const::CStr(sym) => self.string(sym),
            Const::Fn { id, .. } => {
                let func_ref = self.func_ref(id);
                self.b.ins().func_addr(types::I64, func_ref)
            }
            // The address of a type's table of methods.
            Const::VTable { interface, ty } => {
                let data = self.cg.vtable_data(interface, ty);
                let global = match self.data_refs.get(&data) {
                    Some(&global) => global,
                    None => {
                        let global = self.cg.module.declare_data_in_func(data, self.b.func);
                        self.data_refs.insert(data, global);
                        global
                    }
                };
                self.b.ins().symbol_value(types::I64, global)
            }
            // A constant table, kept once.
            Const::Table { id, .. } => {
                let data = self.cg.table_data(id);
                self.data_addr(data)
            }
            // C's `sizeof` and `_Alignof`, from the layout.
            Const::SizeOf { of, ty } | Const::AlignOf { of, ty } => {
                let layout = self.cg.layout(of);
                let bytes = match *constant {
                    Const::SizeOf { .. } => layout.stride(),
                    _ => layout.align,
                };
                let scalar = self
                    .cg
                    .scalar_type(ty)
                    .expect("integers have a machine type");
                self.int_const(scalar, u128::from(bytes))
            }
            // What a panic reads to say its calls.
            Const::FrameTables => {
                let data = self.cg.frame_tables();
                self.data_addr(data)
            }
            // The runtime's words.
            Const::RuntimeWords => {
                let data = self.cg.runtime_words();
                self.data_addr(data)
            }
            // The address of a type's drop function, which an owned closure's
            // environment carries.
            Const::DropFn(ty) => {
                let drop_fn = self.cg.drop_fn(ty);
                let func_ref = self.runtime_ref(drop_fn);
                self.b.ins().func_addr(types::I64, func_ref)
            }
        }
    }

    /// Stores a scalar in a place.
    pub(super) fn write(&mut self, place: &Place, value: Value) {
        if place.projections.is_empty() {
            match self.storage(place.local) {
                Storage::Var(var) => {
                    self.b.def_var(var, value);
                    return;
                }
                Storage::Empty => return,
                Storage::Slot(_) | Storage::Ptr(_) => {}
            }
        }
        let addr = self.place_addr(place);
        self.b.ins().store(MemFlagsData::trusted(), value, addr, 0);
    }

    pub(super) fn assign(&mut self, place: &Place, rvalue: &Rvalue) {
        let ty = self.place_ty(place);
        if self.cg.is_aggregate(ty) {
            let Rvalue::Use(Operand::Copy(src)) = rvalue else {
                unreachable!("an aggregate is assigned by copying a place")
            };
            let dest = self.place_addr(place);
            let src = self.place_addr(src);
            self.copy(dest, src, ty);
            return;
        }
        // A value with no size is not stored, and rvalues have no effects.
        if self.cg.scalar_type(ty).is_none() {
            return;
        }
        let value = self.rvalue(rvalue, ty);
        self.write(place, value);
    }

    fn rvalue(&mut self, rvalue: &Rvalue, ty: Ty) -> Value {
        match rvalue {
            Rvalue::Use(operand) => self.operand(operand),
            Rvalue::Unary(op, operand) => {
                let value = self.operand(operand);
                match (op, self.kind(ty)) {
                    (UnaryOp::Neg, TyKind::Float(_)) => self.b.ins().fneg(value),
                    (UnaryOp::Neg, _) => self.b.ins().ineg(value),
                    // A `bool` is a byte holding 0 or 1, so `!` flips its low
                    // bit; on an integer it flips every bit.
                    (UnaryOp::Not, TyKind::Bool) => self.b.ins().bxor_imm_s(value, 1),
                    (UnaryOp::Not, _) => self.b.ins().bnot(value),
                }
            }
            Rvalue::Binary(op, lhs, rhs) => self.binary(*op, lhs, rhs),
            Rvalue::Cast(operand, to) => self.cast(operand, *to),
            Rvalue::AddressOf(place) => self.place_addr(place),
            // The method at `index` of a table of methods.
            Rvalue::VTableFn { table, index, .. } => {
                let table = self.operand(table);
                let offset = i32::try_from(index * 8).expect("a table has few methods");
                self.b
                    .ins()
                    .load(types::I64, MemFlagsData::trusted(), table, offset)
            }
            // One instruction each, exact on every machine.
            Rvalue::Float(op, operand) => {
                let value = self.operand(operand);
                match op {
                    mir::FloatOp::Sqrt => self.b.ins().sqrt(value),
                    mir::FloatOp::Floor => self.b.ins().floor(value),
                    mir::FloatOp::Ceil => self.b.ins().ceil(value),
                    mir::FloatOp::Trunc => self.b.ins().trunc(value),
                }
            }
            // One rounding: the processor's fused multiply-add, or C's
            // `fma` where it has none, which answers the same.
            Rvalue::MulAdd(first, second, third) => {
                let first = self.operand(first);
                let second = self.operand(second);
                let third = self.operand(third);
                self.b.ins().fma(first, second, third)
            }
            // An integer's bits, one instruction each.
            Rvalue::Integer(op, operand) => {
                let value = self.operand(operand);
                match op {
                    mir::IntegerOp::CountOnes => self.b.ins().popcnt(value),
                    mir::IntegerOp::LeadingZeros => self.b.ins().clz(value),
                    mir::IntegerOp::TrailingZeros => self.b.ins().ctz(value),
                    // One byte has no other order.
                    mir::IntegerOp::SwapBytes if self.b.func.dfg.value_type(value) == types::I8 => {
                        value
                    }
                    mir::IntegerOp::SwapBytes => self.b.ins().bswap(value),
                    mir::IntegerOp::ReverseBits => self.b.ins().bitrev(value),
                }
            }
            // The amount is taken modulo the width, as the instruction
            // takes it; its low bits say all of that, whatever its width.
            Rvalue::Rotate(turn, value, amount) => {
                let value = self.operand(value);
                let mut amount = self.operand(amount);
                if self.b.func.dfg.value_type(amount) == types::I128 {
                    amount = self.b.ins().ireduce(types::I64, amount);
                }
                match turn {
                    mir::Turn::Left => self.b.ins().rotl(value, amount),
                    mir::Turn::Right => self.b.ins().rotr(value, amount),
                }
            }
            // Whether the answer would not fit: the flag of the instruction a
            // checked `+` panics on.
            Rvalue::Overflows(op, lhs, rhs) => {
                let signed =
                    matches!(self.kind(self.operand_ty(lhs)), TyKind::Int(int) if int.signed());
                let l = self.operand(lhs);
                let r = self.operand(rhs);
                let (_, overflowed) = match (op, signed) {
                    (BinaryOp::Add, true) => self.b.ins().sadd_overflow(l, r),
                    (BinaryOp::Add, false) => self.b.ins().uadd_overflow(l, r),
                    (BinaryOp::Sub, true) => self.b.ins().ssub_overflow(l, r),
                    (BinaryOp::Sub, false) => self.b.ins().usub_overflow(l, r),
                    (BinaryOp::Mul, true) => self.b.ins().smul_overflow(l, r),
                    (BinaryOp::Mul, false) => self.b.ins().umul_overflow(l, r),
                    (op, _) => unreachable!("{op:?} does not overflow"),
                };
                overflowed
            }
            // A float's bits as the integer of its width, and back.
            Rvalue::Bits(operand) => {
                let value = self.operand(operand);
                let target = self.cg.scalar_type(ty).expect("bits are a scalar's");
                self.b.ins().bitcast(target, MemFlagsData::new(), value)
            }
            // The length of a C string, which the runtime walks to find.
            Rvalue::CstrLen(operand) => {
                let pointer = self.operand(operand);
                let cstr_len = self.cg.cstr_len_fn();
                let func = self.runtime_ref(cstr_len);
                let call = self.b.ins().call(func, &[pointer]);
                self.b.inst_results(call)[0]
            }
            Rvalue::Variant(place) => {
                let ty = self.place_ty(place);
                let addr = self.place_addr(place);
                self.variant_index(addr, ty)
            }
        }
    }

    /// The tag, or the null that stands for the empty variant of a niche.
    pub(super) fn set_variant(&mut self, place: &Place, variant: u32) {
        let ty = self.place_ty(place);
        match self.cg.layouts.enum_tag(self.cg.program, ty) {
            Tag::Byte { size } => {
                let ty = if size == 1 { types::I8 } else { types::I32 };
                let tag = self.b.ins().iconst(ty, i64::from(variant));
                let addr = self.place_addr(place);
                self.b.ins().store(MemFlagsData::trusted(), tag, addr, 0);
            }
            Tag::Niche { empty, offset, .. } if variant == empty => {
                let null = self.b.ins().iconst(types::I64, 0);
                let addr = self.place_addr(place);
                self.b
                    .ins()
                    .store(MemFlagsData::trusted(), null, addr, offset as i32);
            }
            Tag::Niche { .. } => {}
        }
    }

    /// Zeros: what a move leaves behind.
    pub(super) fn zero_place(&mut self, place: &Place) {
        let ty = self.place_ty(place);
        if place.projections.is_empty() {
            match self.storage(place.local) {
                Storage::Var(var) => {
                    let scalar = self.cg.scalar_type(ty).expect("a variable is a scalar");
                    let zero = match scalar {
                        types::F32 => self.b.ins().f32const(0.0),
                        types::F64 => self.b.ins().f64const(0.0),
                        scalar => self.int_const(scalar, 0),
                    };
                    self.b.def_var(var, zero);
                    return;
                }
                Storage::Empty => return,
                Storage::Slot(_) | Storage::Ptr(_) => {}
            }
        }
        let size = self.cg.layout(ty).size;
        if size == 0 {
            return;
        }
        let addr = self.place_addr(place);
        self.zero(addr, size);
    }

    /// Fills a place with [`POISON`], byte after byte: what a move leaves in
    /// a build that checks moves.
    pub(super) fn poison_place(&mut self, place: &Place) {
        let ty = self.place_ty(place);
        if place.projections.is_empty() {
            match self.storage(place.local) {
                Storage::Var(var) => {
                    let scalar = self.cg.scalar_type(ty).expect("a variable is a scalar");
                    if scalar.is_int() {
                        let poison = self.int_const(scalar, u128::from(POISON));
                        self.b.def_var(var, poison);
                    }
                    return;
                }
                Storage::Empty => return,
                Storage::Slot(_) | Storage::Ptr(_) => {}
            }
        }
        let size = self.cg.layout(ty).size;
        let addr = self.place_addr(place);
        let mut offset = 0;
        while offset + 8 <= size {
            let word = self.b.ins().iconst(types::I64, POISON as i64);
            self.b
                .ins()
                .store(MemFlagsData::trusted(), word, addr, offset as i32);
            offset += 8;
        }
        while offset < size {
            let byte = self.b.ins().iconst(types::I8, i64::from(POISON as u8));
            self.b
                .ins()
                .store(MemFlagsData::trusted(), byte, addr, offset as i32);
            offset += 1;
        }
    }

    /// Whether a place begins with what [`Statement::Poison`] leaves: its
    /// first word, or as much of one as it has.
    pub(super) fn poisoned(&mut self, place: &Place) -> Value {
        let ty = self.place_ty(place);
        if place.projections.is_empty()
            && let Storage::Var(var) = self.storage(place.local)
        {
            let scalar = self.cg.scalar_type(ty).expect("a variable is a scalar");
            let value = self.b.use_var(var);
            if !scalar.is_int() {
                return self.b.ins().iconst(types::I8, 0);
            }
            let pattern = self
                .b
                .ins()
                .iconst(scalar, POISON as i64 & mask(scalar.bits()));
            return self.b.ins().icmp(IntCC::Equal, value, pattern);
        }
        let size = self.cg.layout(ty).size;
        let width = match size {
            0 => return self.b.ins().iconst(types::I8, 0),
            8.. => types::I64,
            4..=7 => types::I32,
            2..=3 => types::I16,
            _ => types::I8,
        };
        let addr = self.place_addr(place);
        let first = self.b.ins().load(width, MemFlagsData::trusted(), addr, 0);
        let pattern = self
            .b
            .ins()
            .iconst(width, POISON as i64 & mask(width.bits()));
        self.b.ins().icmp(IntCC::Equal, first, pattern)
    }

    /// Which variant the enum at `addr` holds, as an `i32` index.
    pub(super) fn variant_index(&mut self, addr: Value, ty: Ty) -> Value {
        match self.cg.layouts.enum_tag(self.cg.program, ty) {
            Tag::Byte { size: 1 } => {
                let tag = self
                    .b
                    .ins()
                    .load(types::I8, MemFlagsData::trusted(), addr, 0);
                self.b.ins().uextend(types::I32, tag)
            }
            Tag::Byte { .. } => self
                .b
                .ins()
                .load(types::I32, MemFlagsData::trusted(), addr, 0),
            Tag::Niche {
                payload,
                empty,
                offset,
            } => {
                let ptr =
                    self.b
                        .ins()
                        .load(types::I64, MemFlagsData::trusted(), addr, offset as i32);
                let is_null = self.b.ins().icmp_imm_u(IntCC::Equal, ptr, 0);
                let empty = self.b.ins().iconst(types::I32, i64::from(empty));
                let payload = self.b.ins().iconst(types::I32, i64::from(payload));
                self.b.ins().select(is_null, empty, payload)
            }
        }
    }

    /// The address of a data object, declared in this function on first use.
    pub(super) fn data_addr(&mut self, data: DataId) -> Value {
        let global = match self.data_refs.get(&data) {
            Some(&global) => global,
            None => {
                let global = self.cg.module.declare_data_in_func(data, self.b.func);
                self.data_refs.insert(data, global);
                global
            }
        };
        self.b.ins().symbol_value(types::I64, global)
    }

    pub(super) fn string(&mut self, sym: Symbol) -> Value {
        let data = self.cg.string_data(sym);
        let global = match self.data_refs.get(&data) {
            Some(&global) => global,
            None => {
                let global = self.cg.module.declare_data_in_func(data, self.b.func);
                self.data_refs.insert(data, global);
                global
            }
        };
        self.b.ins().symbol_value(types::I64, global)
    }
}

/// What a move leaves in a build that checks moves: a
/// pattern no pointer, length or tag of a live value holds.
const POISON: u64 = 0xA5A5_A5A5_A5A5_A5A5;

/// The low `bits` of a word.
fn mask(bits: u32) -> i64 {
    if bits >= 64 { -1 } else { (1i64 << bits) - 1 }
}
