//! Places, and writing aggregates into memory.

use super::*;

impl Builder<'_> {
    /// The place of an aggregate-typed expression. Locals, fields, elements
    /// and dereferences are places already; anything else is built in a
    /// temporary.
    pub(super) fn aggregate(&mut self, id: ExprId) -> Place {
        // A reference to a slice is the slice's pointer and length, which is
        // what the slice's place already holds.
        if let ExprKind::Ref(inner) = self.hir.exprs[id].kind
            && matches!(self.kind(self.ty(inner)), TyKind::Slice(_))
        {
            return self.place(inner);
        }
        if self.is_place_kind(id) {
            return self.place(id);
        }
        let temp = Place::local(self.temp(self.ty(id)));
        self.store_expr(id, temp.clone());
        temp
    }

    /// Locals, fields, elements, sub-slices and dereferences, which `place`
    /// finds without building anything.
    pub(super) fn is_place_kind(&self, id: ExprId) -> bool {
        matches!(
            self.hir.exprs[id].kind,
            ExprKind::Local(_)
                | ExprKind::Field { .. }
                | ExprKind::Index { .. }
                | ExprKind::SubSlice { .. }
                | ExprKind::Deref(_)
        )
    }

    /// The place a field or element is taken from. A base that is not a
    /// place, as in `make_car().seats`, is a temporary dropped when the
    /// statement ends.
    fn projection_base(&mut self, base: ExprId) -> Place {
        let place = self.aggregate(base);
        if !self.is_place_kind(base) {
            self.own_temp(place.clone(), self.ty(base));
        }
        place
    }

    /// The place an expression denotes. Anything that is not a place is
    /// evaluated into a temporary first, so `&f()` and `(Point { … }).x`
    /// work.
    pub(super) fn place(&mut self, id: ExprId) -> Place {
        let hir = self.hir;
        let expr = &hir.exprs[id];
        // Where a check that fails here says it happened.
        let at = expr.span;
        match &expr.kind {
            ExprKind::Local(local) => Place::local(self.vars[*local]),
            ExprKind::Field { base, index } => self
                .projection_base(*base)
                .project(Projection::Field(*index)),
            // `p[i]` through a C pointer: nothing says how many there
            // are, so nothing is checked.
            ExprKind::Index { base, index }
                if matches!(self.kind(self.ty(*base)), TyKind::Ptr(_)) =>
            {
                let elements = self.place(*base);
                let index = self.scalar(*index);
                let index = self.local_of(index, Types::I64);
                elements.project(Projection::Index(index))
            }
            ExprKind::Index { base, index } => {
                let (elements, len) = self.elements(*base);
                let index = self.scalar(*index);
                let index = self.local_of(index, Types::I64);
                self.check_unsigned(BinaryOp::Ge, Operand::Copy(Place::local(index)), len, at);
                elements.project(Projection::Index(index))
            }
            // `&xs[lo..hi]`: a new pointer and length, checked against the
            // elements they are taken from.
            ExprKind::SubSlice { base, lo, hi } => {
                let (elements, len) = self.elements(*base);
                let lo = match lo {
                    Some(lo) => self.scalar(*lo),
                    None => Self::int(0, Types::I64),
                };
                let hi = match hi {
                    Some(hi) => self.scalar(*hi),
                    None => len.clone(),
                };
                // `lo <= hi <= len`, unsigned so that a negative bound fails.
                // A start of 0 is no unsigned number's greater, so it is not
                // checked: `[..n]`, and what `Vec` lends, `[0..length]`.
                self.check_unsigned(BinaryOp::Gt, hi.clone(), len, at);
                let starts_at_zero = matches!(&lo, Operand::Const(Const::Int { bits: 0, .. }));
                if !starts_at_zero {
                    self.check_unsigned(BinaryOp::Gt, lo.clone(), hi.clone(), at);
                }
                let lo_local = self.local_of(lo.clone(), Types::I64);
                let start = self.value(
                    Types::PTR_U8,
                    Rvalue::AddressOf(elements.project(Projection::Index(lo_local))),
                );
                let count = self.value(Types::I64, Rvalue::Binary(BinaryOp::Sub, hi, lo));
                let slice = Place::local(self.temp(expr.ty));
                self.assign(slice.project(Projection::Field(0)), Rvalue::Use(start));
                self.assign(slice.project(Projection::Field(1)), Rvalue::Use(count));
                slice
            }
            // A slice parameter or a buffer: the place of the elements is the
            // pointer and length it holds.
            ExprKind::Deref(pointer) if is_slice_pointer(self.program, self.ty(*pointer)) => {
                self.place(*pointer)
            }
            ExprKind::Deref(pointer) => {
                let ptr = match self.scalar(*pointer) {
                    Operand::Copy(ptr) => ptr,
                    // A constant table's address.
                    constant => Place::local(self.local_of(constant, self.ty(*pointer))),
                };
                // `make().len`: the `own` the call returned is a temporary.
                let pointer_ty = self.ty(*pointer);
                if matches!(self.kind(pointer_ty), TyKind::Own(_)) && !self.is_place_kind(*pointer)
                {
                    self.own_temp(ptr.clone(), pointer_ty);
                }
                ptr.project(Projection::Deref)
            }
            // `&make()`: the value lives in a temporary until the statement
            // ends.
            _ => {
                let place = if self.is_aggregate(expr.ty) {
                    self.aggregate(id)
                } else {
                    let temp = Place::local(self.temp(expr.ty));
                    self.store_expr(id, temp.clone());
                    temp
                };
                self.own_temp(place.clone(), expr.ty);
                place
            }
        }
    }

    /// The place of an array's or slice's elements, and how many there are.
    pub(super) fn elements(&mut self, base: ExprId) -> (Place, Operand) {
        match self.kind(self.ty(base)) {
            TyKind::Array(_, len) => (
                self.projection_base(base),
                Self::int(u128::from(len), Types::I64),
            ),
            // A slice is a pointer and a length, and a block of slots is a
            // pointer and how many it has.
            TyKind::Slice(_) | TyKind::Slots(_) | TyKind::Str => {
                let slice = self.place(base);
                let len = self.value(
                    Types::I64,
                    Rvalue::Use(Operand::Copy(slice.project(Projection::Field(1)))),
                );
                (slice, len)
            }
            _ => unreachable!("elements of something that is not an array or slice"),
        }
    }

    /// Panics when `lhs op rhs` holds, comparing as unsigned numbers so that
    /// a negative index is out of bounds too. The message names the index
    /// and the length.
    pub(super) fn check_unsigned(&mut self, op: BinaryOp, lhs: Operand, rhs: Operand, at: Span) {
        let index = lhs.clone();
        let length = rhs.clone();
        let lhs = self.value(Types::U64, Rvalue::Cast(lhs, Types::U64));
        let rhs = self.value(Types::U64, Rvalue::Cast(rhs, Types::U64));
        let fails = self.value(Types::BOOL, Rvalue::Binary(op, lhs, rhs));
        self.push(Statement::Check {
            fails,
            kind: CheckKind::Bounds { index, length },
            at,
        });
    }

    /// Evaluates `id` into `dest`. Literals, calls, blocks, `if` and `match`
    /// build their aggregate there rather than in a temporary.
    pub(super) fn store_expr(&mut self, id: ExprId, dest: Place) {
        let hir = self.hir;
        let expr = &hir.exprs[id];
        let aggregate = self.is_aggregate(expr.ty);
        match &expr.kind {
            // Every byte zero, as C leaves a field it is not given.
            ExprKind::Zeroed => self.push(Statement::Zero(dest)),
            // `Event { key: … }`: every byte zero, and then the one field
            // it names, since they are all the same bytes.
            ExprKind::Union { field, .. } => {
                self.push(Statement::Zero(dest.clone()));
                if let Some((index, value)) = field {
                    self.store_expr(*value, dest.project(Projection::Field(*index)));
                }
            }
            ExprKind::Struct { fields, order, .. } => {
                let mut pending = Vec::new();
                for (n, i) in evaluation_order(order, fields.len()).enumerate() {
                    let part = dest.project(Projection::Field(i as u32));
                    self.store_expr(fields[i], part.clone());
                    if n + 1 < fields.len() {
                        self.pending_part(part, self.ty(fields[i]), &mut pending);
                    }
                }
                self.forget_parts(pending);
            }
            ExprKind::Array(elems) => {
                let mut pending = Vec::new();
                for (i, &elem) in elems.iter().enumerate() {
                    let part = dest.project(Projection::ConstIndex(i as u64));
                    self.store_expr(elem, part.clone());
                    if i + 1 < elems.len() {
                        self.pending_part(part, self.ty(elem), &mut pending);
                    }
                }
                self.forget_parts(pending);
            }
            ExprKind::ArrayRepeat { elem, count } => self.array_repeat(*elem, *count, dest),
            ExprKind::OwnRepeat { elem, count } => self.own_repeat(*elem, *count, dest),
            // What the closure captured, and the address of its code.
            ExprKind::Closure { id, env } => {
                let env_ty = self.ty(*env);
                // An owned closure's environment is on the heap already, and
                // the expression that builds it is its address; a lent one is
                // built in this frame.
                let captures = if matches!(self.kind(env_ty), TyKind::Own(..)) {
                    self.scalar(*env)
                } else {
                    let place = Place::local(self.temp(env_ty));
                    self.store_expr(*env, place.clone());
                    self.value(Types::PTR_U8, Rvalue::AddressOf(place))
                };
                self.assign(dest.project(Projection::Field(0)), Rvalue::Use(captures));
                let code = Operand::Const(Const::Fn {
                    id: *id,
                    ty: Types::PTR_U8,
                });
                self.assign(dest.project(Projection::Field(1)), Rvalue::Use(code));
            }
            // The reference stays, and the table of methods joins it.
            ExprKind::DynRef { value, interface } => {
                let TyKind::Ref(pointee, _) = self.kind(self.ty(*value)) else {
                    unreachable!("a `&dyn` is made from a reference")
                };
                let reference = self.scalar(*value);
                self.assign(dest.project(Projection::Field(0)), Rvalue::Use(reference));
                self.assign(
                    dest.project(Projection::Field(1)),
                    Rvalue::Use(Operand::Const(Const::VTable {
                        interface: *interface,
                        ty: pointee,
                    })),
                );
            }
            // The pointer stays, and the length joins it.
            ExprKind::Unsize(inner) => {
                let TyKind::Own(array) = self.kind(self.ty(*inner)) else {
                    unreachable!("only an `own` of an array becomes a buffer")
                };
                let TyKind::Array(_, len) = self.kind(array) else {
                    unreachable!("only an `own` of an array becomes a buffer")
                };
                let ptr = self.scalar(*inner);
                self.assign(dest.project(Projection::Field(0)), Rvalue::Use(ptr));
                self.assign(
                    dest.project(Projection::Field(1)),
                    Rvalue::Use(Self::int(u128::from(len), Types::I64)),
                );
            }
            // The tag, then the fields.
            ExprKind::Variant {
                variant,
                args,
                order,
                ..
            } => {
                self.push(Statement::SetVariant(dest.clone(), *variant));
                let mut pending = Vec::new();
                for (n, i) in evaluation_order(order, args.len()).enumerate() {
                    let field = Projection::VariantField {
                        variant: *variant,
                        field: i as u32,
                    };
                    let part = dest.project(field);
                    self.store_expr(args[i], part.clone());
                    if n + 1 < args.len() {
                        self.pending_part(part, self.ty(args[i]), &mut pending);
                    }
                }
                self.forget_parts(pending);
            }
            ExprKind::Match { scrutinee, arms } if aggregate => {
                self.match_expr(*scrutinee, arms, expr.ty, Some(dest));
            }
            // An owned closure lent for a call: the pair it already is.
            ExprKind::LendClosure(inner) => {
                let src = self.aggregate(*inner);
                self.assign(dest, Rvalue::Use(Operand::Copy(src)));
            }
            // A reference to a slice: its pointer and length.
            ExprKind::Ref(inner) if aggregate => {
                let src = self.place(*inner);
                self.assign(dest, Rvalue::Use(Operand::Copy(src)));
            }
            // `s.toStr()`: C's pointer, and the length found by walking to
            // the NUL.
            ExprKind::CstrToStr(inner) => {
                let pointer = self.scalar(*inner);
                let length = self.value(Types::I64, Rvalue::CstrLen(pointer.clone()));
                self.assign(dest.project(Projection::Field(0)), Rvalue::Use(pointer));
                self.assign(dest.project(Projection::Field(1)), Rvalue::Use(length));
            }
            // A `str` literal: a pointer to its bytes, which are also
            // NUL-terminated for C, and its length.
            ExprKind::Str(sym) if aggregate => {
                let len = self.interner.resolve(*sym).len() as u128;
                self.assign(
                    dest.project(Projection::Field(0)),
                    Rvalue::Use(Operand::Const(Const::CStr(*sym))),
                );
                self.assign(
                    dest.project(Projection::Field(1)),
                    Rvalue::Use(Self::int(len, Types::I64)),
                );
            }
            ExprKind::Block(block) => {
                self.block_expr(block, Some(dest));
            }
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } if aggregate => {
                self.if_expr(*cond, then_block, else_block.as_ref(), expr.ty, Some(dest));
            }
            // A tail call leaves nothing to store: control goes back to the
            // top of the body.
            ExprKind::Call { args, order, .. } if self.hir.tail_calls.contains(&id) => {
                self.tail_call(args, order);
            }
            // A body the compiler writes, whose value is an aggregate.
            ExprKind::Call { callee, args, .. }
                if self.program.fns[*callee].intrinsic.is_some() =>
            {
                self.intrinsic_call(*callee, args, expr.ty, expr.span, Some(dest));
            }
            ExprKind::Call {
                callee,
                args,
                order,
                ..
            } if aggregate => {
                let ret = self.program.fns[*callee].ret;
                self.call(Callee::Fn(*callee), ret, args, order, Some(dest));
            }
            ExprKind::CallValue { callee, args } if aggregate => {
                let callee = self.scalar(*callee);
                self.call(Callee::Value(callee), expr.ty, args, &[], Some(dest));
            }
            // A closure or a `dyn` method that answers an aggregate writes it
            // where it goes, as any call does.
            ExprKind::CallClosure {
                callee,
                args,
                code_ty,
            } if aggregate => {
                let closure = self.place(*callee);
                let code = self.value(
                    *code_ty,
                    Rvalue::Use(Operand::Copy(closure.clone().project(Projection::Field(1)))),
                );
                let captures = Operand::Copy(closure.project(Projection::Field(0)));
                self.closure_call(code, captures, args, expr.ty, Some(dest));
            }
            ExprKind::DynCall {
                index,
                fn_ty,
                args,
                order,
                ..
            } if aggregate => {
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
                self.dyn_call(function, pointer, args, order, expr.ty, Some(dest));
            }
            ExprKind::Move(operand) if aggregate => {
                let src = self.place(*operand);
                self.assign(dest, Rvalue::Use(Operand::Copy(src.clone())));
                let ty = self.ty(*operand);
                self.vacate(src, ty);
                self.moved_from(*operand);
            }
            // Any other aggregate would come back here through `aggregate`,
            // so only places may fall through to `expr`.
            kind if aggregate && !self.is_place_kind(id) => {
                unreachable!("aggregate expression without a lowering: {kind:?}")
            }
            _ => match self.expr(id) {
                Value::Scalar(operand) => self.assign(dest, Rvalue::Use(operand)),
                Value::Place(src) => self.assign(dest, Rvalue::Use(Operand::Copy(src))),
                Value::Unit => {}
            },
        }
    }

    /// `own [elem; count]` with a run-time count. `elem` is
    /// evaluated, then `count`; a negative count stops the program; then the
    /// buffer is allocated and every element gets a copy of `elem`, which
    /// owns nothing.
    fn own_repeat(&mut self, elem: ExprId, count: ExprId, dest: Place) {
        let elem_ty = self.ty(elem);
        let value = Place::local(self.temp(elem_ty));
        self.store_expr(elem, value.clone());
        let count_at = self.span(count);
        let count = self.scalar(count);
        let count = self.local_of(count, Types::I64);
        let negative = self.value(
            Types::BOOL,
            Rvalue::Binary(
                BinaryOp::Lt,
                Operand::Copy(Place::local(count)),
                Self::int(0, Types::I64),
            ),
        );
        self.push(Statement::Check {
            fails: negative,
            kind: CheckKind::Length {
                length: Operand::Copy(Place::local(count)),
            },
            at: count_at,
        });
        self.push(Statement::AllocBuffer {
            dest: dest.clone(),
            elem: elem_ty,
            count: Operand::Copy(Place::local(count)),
        });
        let i = self.temp(Types::I64);
        self.assign(Place::local(i), Rvalue::Use(Self::int(0, Types::I64)));
        let header = self.new_block();
        let body = self.new_block();
        let exit = self.new_block();
        self.terminate(Terminator::Goto(header));
        self.switch_to(header);
        let done = self.value(
            Types::BOOL,
            Rvalue::Binary(
                BinaryOp::Ge,
                Operand::Copy(Place::local(i)),
                Operand::Copy(Place::local(count)),
            ),
        );
        self.terminate(Terminator::Branch {
            cond: done,
            then: exit,
            otherwise: body,
        });
        self.switch_to(body);
        self.assign(
            dest.project(Projection::Index(i)),
            Rvalue::Use(Operand::Copy(value)),
        );
        let next = self.value(
            Types::I64,
            Rvalue::Binary(
                BinaryOp::Add,
                Operand::Copy(Place::local(i)),
                Self::int(1, Types::I64),
            ),
        );
        self.assign(Place::local(i), Rvalue::Use(next));
        self.terminate(Terminator::Goto(header));
        self.switch_to(exit);
    }

    /// `[elem; count]`: evaluates `elem` once, into the first element, then
    /// copies it into the rest.
    fn array_repeat(&mut self, elem: ExprId, count: u64, dest: Place) {
        if count == 0 {
            self.expr(elem);
            return;
        }
        let first = dest.project(Projection::ConstIndex(0));
        self.store_expr(elem, first.clone());
        if count <= 16 {
            for i in 1..count {
                self.assign(
                    dest.project(Projection::ConstIndex(i)),
                    Rvalue::Use(Operand::Copy(first.clone())),
                );
            }
            return;
        }
        let i = self.temp(Types::U64);
        self.assign(Place::local(i), Rvalue::Use(Self::int(1, Types::U64)));
        let header = self.new_block();
        let body = self.new_block();
        let exit = self.new_block();
        self.terminate(Terminator::Goto(header));
        self.switch_to(header);
        let done = self.value(
            Types::BOOL,
            Rvalue::Binary(
                BinaryOp::Ge,
                Operand::Copy(Place::local(i)),
                Self::int(u128::from(count), Types::U64),
            ),
        );
        self.terminate(Terminator::Branch {
            cond: done,
            then: exit,
            otherwise: body,
        });
        self.switch_to(body);
        self.assign(
            dest.project(Projection::Index(i)),
            Rvalue::Use(Operand::Copy(first)),
        );
        let next = self.value(
            Types::U64,
            Rvalue::Binary(
                BinaryOp::Add,
                Operand::Copy(Place::local(i)),
                Self::int(1, Types::U64),
            ),
        );
        self.assign(Place::local(i), Rvalue::Use(next));
        self.terminate(Terminator::Goto(header));
        self.switch_to(exit);
    }
}
