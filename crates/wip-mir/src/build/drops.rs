//! Unwinding and drops: the cleanups of scopes and temporaries, drops
//! elaborated into statements, and drop functions.

use super::*;

/// A part of a value that needs dropping, and the projection that reaches it.
type Part = (Projection, Ty);

impl Builder<'_> {
    /// The fields of a struct, or the elements of an array, that need
    /// dropping, in declaration order.
    fn parts(&mut self, ty: Ty) -> Vec<Part> {
        let program = self.program;
        let parts: Vec<Part> = match self.kind(ty) {
            TyKind::Struct(..) => program
                .field_tys(ty)
                .into_iter()
                .enumerate()
                .map(|(i, field)| (Projection::Field(i as u32), field))
                .collect(),
            TyKind::Array(elem, len) => (0..len)
                .map(|i| (Projection::ConstIndex(i), elem))
                .collect(),
            _ => Vec::new(),
        };
        parts
            .into_iter()
            .filter(|&(_, part)| self.needs_drop(part))
            .collect()
    }

    /// The fields of each variant of an enum that need dropping, in
    /// declaration order.
    fn variant_parts(&mut self, ty: Ty) -> Vec<Vec<Part>> {
        let variants = self.program.variant_field_tys(ty);
        variants
            .into_iter()
            .enumerate()
            .map(|(v, fields)| {
                fields
                    .into_iter()
                    .enumerate()
                    .filter(|&(_, field)| self.needs_drop(field))
                    .map(|(i, field)| {
                        let projection = Projection::VariantField {
                            variant: v as u32,
                            field: i as u32,
                        };
                        (projection, field)
                    })
                    .collect()
            })
            .collect()
    }

    /// Drops the value at `place`: fields and elements in reverse order, an
    /// enum's payload by a switch on its variant, and an `own` by freeing
    /// what it points to, after which the pointer is zeroed.
    pub(super) fn drop_place(&mut self, place: &Place, ty: Ty) {
        if !self.needs_drop(ty) {
            return;
        }
        // A value moved away is never dropped; a build that checks moves
        // says so where it would be.
        if self.check_moves && self.poisons(ty) {
            self.push(Statement::CheckMoved {
                place: place.clone(),
                at: self.drop_at,
            });
        }
        // A generator drops what it holds where it stopped, which only its
        // `next` knows.
        if let TyKind::Struct(id, _) = self.kind(ty)
            && self.program.structs[id].generator.is_some()
        {
            self.drop_generator(place, ty);
            return;
        }
        self.call_drop(place, ty);
        match self.kind(ty) {
            // A block of slots frees itself and nothing else: it does not
            // know which slots hold values.
            TyKind::Slots(_) => {
                let pointer = self.value(
                    Types::PTR_U8,
                    Rvalue::Use(Operand::Copy(place.project(Projection::Field(0)))),
                );
                let not_null = self.value(
                    Types::BOOL,
                    Rvalue::Binary(BinaryOp::Ne, pointer.clone(), Self::int(0, Types::PTR_U8)),
                );
                let free = self.new_block();
                let next = self.new_block();
                self.terminate(Terminator::Branch {
                    cond: not_null,
                    then: free,
                    otherwise: next,
                });
                self.switch_to(free);
                self.push(Statement::Free(pointer));
                self.terminate(Terminator::Goto(next));
                self.switch_to(next);
                self.push(Statement::Zero(place.clone()));
            }
            TyKind::Own(inner) => {
                if let TyKind::Slice(elem) = self.kind(inner) {
                    self.drop_buffer(place, elem);
                    return;
                }
                // An owned closure's type says nothing about which lambda
                // made it, so its environment carries its drop function.
                if matches!(self.kind(inner), TyKind::Fn(..)) {
                    self.drop_closure(place);
                    return;
                }
                let ptr = self.value(ty, Rvalue::Use(Operand::Copy(place.clone())));
                self.drop_owned(ptr, ty, inner);
                self.push(Statement::Zero(place.clone()));
            }
            TyKind::Struct(..) | TyKind::Array(..) => {
                for (projection, part) in self.parts(ty).into_iter().rev() {
                    self.drop_place(&place.project(projection), part);
                }
            }
            // The fields of whichever variant the value holds.
            TyKind::Enum(..) => {
                let variants = self.variant_parts(ty);
                let variant = self.value(Types::I32, Rvalue::Variant(place.clone()));
                let done = self.new_block();
                let mut cases = Vec::new();
                for (v, parts) in variants.iter().enumerate() {
                    if !parts.is_empty() {
                        cases.push((v as u32, self.new_block()));
                    }
                }
                self.terminate(Terminator::Switch {
                    value: variant,
                    cases: cases.clone(),
                    otherwise: done,
                });
                for (v, block) in cases {
                    self.switch_to(block);
                    for &(projection, part) in variants[v as usize].iter().rev() {
                        self.drop_place(&place.project(projection), part);
                    }
                    self.terminate(Terminator::Goto(done));
                }
                self.switch_to(done);
            }
            _ => {}
        }
    }

    /// Drops a buffer: its elements, last first, then its
    /// allocation. A moved-out buffer holds a null pointer and no elements.
    fn drop_buffer(&mut self, place: &Place, elem: Ty) {
        if self.needs_drop(elem) {
            let i = self.temp(Types::I64);
            self.assign(
                Place::local(i),
                Rvalue::Use(Operand::Copy(place.project(Projection::Field(1)))),
            );
            let header = self.new_block();
            let body = self.new_block();
            let exit = self.new_block();
            self.terminate(Terminator::Goto(header));
            self.switch_to(header);
            let done = self.value(
                Types::BOOL,
                Rvalue::Binary(
                    BinaryOp::Le,
                    Operand::Copy(Place::local(i)),
                    Self::int(0, Types::I64),
                ),
            );
            self.terminate(Terminator::Branch {
                cond: done,
                then: exit,
                otherwise: body,
            });
            self.switch_to(body);
            let previous = self.value(
                Types::I64,
                Rvalue::Binary(
                    BinaryOp::Sub,
                    Operand::Copy(Place::local(i)),
                    Self::int(1, Types::I64),
                ),
            );
            self.assign(Place::local(i), Rvalue::Use(previous));
            // Out of line: the element may hold a buffer of its own type.
            let element = place.project(Projection::Index(i));
            let address = self.value(Types::PTR_U8, Rvalue::AddressOf(element));
            self.push(Statement::DropInPlace {
                ty: elem,
                ptr: address,
            });
            self.terminate(Terminator::Goto(header));
            self.switch_to(exit);
        }
        let ptr = self.value(
            Types::PTR_U8,
            Rvalue::Use(Operand::Copy(place.project(Projection::Field(0)))),
        );
        let not_null = self.value(
            Types::BOOL,
            Rvalue::Binary(BinaryOp::Ne, ptr.clone(), Self::int(0, Types::PTR_U8)),
        );
        let free = self.new_block();
        let next = self.new_block();
        self.terminate(Terminator::Branch {
            cond: not_null,
            then: free,
            otherwise: next,
        });
        self.switch_to(free);
        self.push(Statement::Free(ptr));
        self.terminate(Terminator::Goto(next));
        self.switch_to(next);
        self.push(Statement::Zero(place.clone()));
    }

    /// Drops an owned closure: the first word of the pair is
    /// its environment, whose first word is the drop function of that
    /// environment's type, which drops what was captured and frees it. A
    /// closure that was moved out of holds a null environment.
    fn drop_closure(&mut self, place: &Place) {
        let env = self.value(
            Types::PTR_U8,
            Rvalue::Use(Operand::Copy(place.project(Projection::Field(0)))),
        );
        let not_null = self.value(
            Types::BOOL,
            Rvalue::Binary(BinaryOp::Ne, env.clone(), Self::int(0, Types::PTR_U8)),
        );
        let drop = self.new_block();
        let next = self.new_block();
        self.terminate(Terminator::Branch {
            cond: not_null,
            then: drop,
            otherwise: next,
        });
        self.switch_to(drop);
        let ty = self.drop_fn_ty();
        let code = self.value(
            ty,
            Rvalue::VTableFn {
                table: env.clone(),
                index: 0,
                methods: false,
            },
        );
        self.push(Statement::Call {
            callee: Callee::Value(code),
            args: vec![env],
            dest: None,
        });
        self.terminate(Terminator::Goto(next));
        self.switch_to(next);
        self.push(Statement::Zero(place.clone()));
    }

    /// The type of a drop function, `(ptr<u8>) => void`: an owned closure's
    /// environment holds one, so it is interned wherever one is created.
    fn drop_fn_ty(&self) -> Ty {
        let types = &self.program.types;
        let list = types
            .find_list(&[Types::PTR_U8])
            .expect("an owned closure's environment holds a drop function");
        types
            .find(TyKind::Fn(list, Types::UNIT))
            .expect("an owned closure's environment holds a drop function")
    }

    /// Frees the allocation `ptr` points to, after dropping what it holds. A
    /// null pointer, from a place that was moved out of, is skipped.
    fn drop_owned(&mut self, ptr: Operand, pointer_ty: Ty, pointee: Ty) {
        if self.needs_drop(pointee) {
            // Out of line, since the pointee may hold an `own` of its own
            // type. The drop function skips null and frees.
            self.push(Statement::DropFn { ty: pointee, ptr });
            return;
        }
        let not_null = self.value(
            Types::BOOL,
            Rvalue::Binary(BinaryOp::Ne, ptr.clone(), Self::int(0, pointer_ty)),
        );
        let free = self.new_block();
        let next = self.new_block();
        self.terminate(Terminator::Branch {
            cond: not_null,
            then: free,
            otherwise: next,
        });
        self.switch_to(free);
        self.push(Statement::Free(ptr));
        self.terminate(Terminator::Goto(next));
        self.switch_to(next);
    }

    /// Drops what an assignment replaces. A whole local that was moved
    /// away holds nothing to drop, which its flag says: without looking,
    /// the zeroes a move leaves were dropped as a value — a `destroy` run
    /// on nothing.
    pub(super) fn drop_replaced(&mut self, place: ExprId, dest: &Place, ty: Ty) {
        self.drop_at = self.hir.exprs[place].span;
        let flag = match self.hir.exprs[place].kind {
            ExprKind::Local(local) => self.flags.get(local).copied(),
            ExprKind::Field { .. } => self
                .field_of_local(place)
                .and_then(|field| self.field_flags.get(&field).copied())
                .or_else(|| {
                    self.field_of_referent(place)
                        .and_then(|field| self.referent_field_flags.get(&field).copied())
                }),
            _ => self
                .referent_of(place)
                .and_then(|reference| self.referent_flags.get(&reference).copied()),
        };
        if let Some(flag) = flag {
            let live = Operand::Copy(Place::local(flag));
            let drop = self.new_block();
            let next = self.new_block();
            self.terminate(Terminator::Branch {
                cond: live,
                then: drop,
                otherwise: next,
            });
            self.switch_to(drop);
            self.drop_place(dest, ty);
            self.terminate(Terminator::Goto(next));
            self.switch_to(next);
            return;
        }
        // A field with a field moved out of it holds only what is left.
        if let Some((local, path)) = self.field_of_local(place) {
            self.drop_remains_at(local, &path, dest, ty);
            return;
        }
        self.drop_place(dest, ty);
    }

    fn drop_local(&mut self, local: LocalId) {
        let ty = self.hir.locals[local].ty;
        if !self.needs_drop(ty) {
            return;
        }
        self.drop_at = self.hir.locals[local].span;
        let Some(&var) = self.vars.get(local) else {
            return;
        };
        // A local that was moved somewhere may hold nothing by now. An `own`
        // says so itself, with a null pointer; anything else — a type that
        // cleans up after itself, and holds no `own` — needs a flag.
        let Some(&flag) = self.flags.get(local) else {
            self.drop_remains(local, &Place::local(var), ty);
            return;
        };
        let live = Operand::Copy(Place::local(flag));
        let drop = self.new_block();
        let next = self.new_block();
        self.terminate(Terminator::Branch {
            cond: live,
            then: drop,
            otherwise: next,
        });
        self.switch_to(drop);
        self.drop_remains(local, &Place::local(var), ty);
        self.terminate(Terminator::Goto(next));
        self.switch_to(next);
    }

    /// Drops a local, or, where the body moves fields out of it, what is
    /// left of it: each field moved out only where its flag says it is
    /// still there. Without this, the zeroes a move leaves were dropped as
    /// a value — a `destroy` run on nothing.
    fn drop_remains(&mut self, local: LocalId, place: &Place, ty: Ty) {
        self.drop_remains_at(local, &[], place, ty);
    }

    /// What is left of the part of a local at `prefix`, a path of field
    /// indices: a field moved out is dropped where its flag says it is
    /// still there, and one with a field moved out of it is taken apart
    /// in turn.
    pub(super) fn drop_remains_at(
        &mut self,
        local: LocalId,
        prefix: &[u32],
        place: &Place,
        ty: Ty,
    ) {
        let moved = self.moved_fields(local);
        let below = |path: &[u32]| {
            moved
                .iter()
                .any(|m| m.len() > path.len() && m.starts_with(path))
        };
        if !below(prefix) {
            self.drop_place(place, ty);
            return;
        }
        for (projection, part) in self.parts(ty).into_iter().rev() {
            let Projection::Field(index) = projection else {
                self.drop_place(&place.project(projection), part);
                continue;
            };
            let mut path = prefix.to_vec();
            path.push(index);
            let part_place = place.project(projection);
            if let Some(&flag) = self.field_flags.get(&(local, path.clone())) {
                let live = Operand::Copy(Place::local(flag));
                let drop = self.new_block();
                let next = self.new_block();
                self.terminate(Terminator::Branch {
                    cond: live,
                    then: drop,
                    otherwise: next,
                });
                self.switch_to(drop);
                self.drop_place(&part_place, part);
                self.terminate(Terminator::Goto(next));
                self.switch_to(next);
            } else if below(&path) {
                self.drop_remains_at(local, &path, &part_place, part);
            } else {
                self.drop_place(&part_place, part);
            }
        }
    }

    /// The fields the body moves out of a local's struct, each as its path
    /// of field indices — `all.row.entries` is two deep — where each
    /// struct on the way cleans up after itself field by field, and the
    /// field cleans up after itself without an `own` to zero. None where
    /// the local's struct has a `destroy` of its own, which drops it whole.
    pub(super) fn moved_fields(&mut self, local: LocalId) -> Vec<Vec<u32>> {
        let ty = self.hir.locals[local].ty;
        if !matches!(self.kind(ty), TyKind::Struct(..)) || self.program.has_drop(ty) {
            return Vec::new();
        }
        let hir = self.hir;
        let mut moved: Vec<Vec<u32>> = hir
            .exprs
            .iter()
            .filter_map(|(_, expr)| match expr.kind {
                ExprKind::Move(inner) => match self.field_of_local(inner) {
                    Some((l, path)) if l == local => Some(path),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        moved.sort_unstable();
        moved.dedup();
        moved.retain(|path| {
            let mut at = ty;
            for (depth, &index) in path.iter().enumerate() {
                // Every struct on the way is one that is dropped field by
                // field.
                if depth > 0
                    && (!matches!(self.kind(at), TyKind::Struct(..)) || self.program.has_drop(at))
                {
                    return false;
                }
                at = self.program.field_ty(at, index);
            }
            self.needs_drop(at) && !self.zeroes_itself(at)
        });
        moved
    }

    /// Whether a reference parameter needs a flag for its referent: the
    /// body moves the referent out, as `move self` does, and it cleans up
    /// after itself without an `own` to zero.
    pub(super) fn needs_referent_flag(&mut self, param: LocalId) -> bool {
        let TyKind::Ref(referent, _) = self.kind(self.hir.locals[param].ty) else {
            return false;
        };
        if !self.needs_drop(referent) || self.zeroes_itself(referent) {
            return false;
        }
        let hir = self.hir;
        hir.exprs.iter().any(|(_, expr)| {
            matches!(expr.kind, ExprKind::Move(inner)
                if matches!(hir.exprs[inner].kind, ExprKind::Deref(r)
                    if matches!(hir.exprs[r].kind, ExprKind::Local(l) if l == param)))
        })
    }

    /// The fields of what a reference parameter refers to that the body
    /// moves out, and that clean up after themselves without an `own` to
    /// zero.
    pub(super) fn moved_referent_fields(&mut self, param: LocalId) -> Vec<u32> {
        let TyKind::Ref(referent, _) = self.kind(self.hir.locals[param].ty) else {
            return Vec::new();
        };
        if !matches!(self.kind(referent), TyKind::Struct(..)) {
            return Vec::new();
        }
        let fields = self.program.field_tys(referent);
        let hir = self.hir;
        let mut moved: Vec<u32> = hir
            .exprs
            .iter()
            .filter_map(|(_, expr)| match expr.kind {
                ExprKind::Move(inner) => match hir.exprs[inner].kind {
                    ExprKind::Field { base, index }
                        if matches!(hir.exprs[base].kind, ExprKind::Deref(r)
                            if matches!(hir.exprs[r].kind, ExprKind::Local(l) if l == param)) =>
                    {
                        Some(index)
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        moved.sort_unstable();
        moved.dedup();
        moved.retain(|&index| {
            let field = fields[index as usize];
            self.needs_drop(field) && !self.zeroes_itself(field)
        });
        moved
    }

    /// Whether a local needs a flag: its type cleans up after itself without
    /// an `own` to zero, and the body moves it somewhere.
    pub(super) fn needs_flag(&mut self, local: LocalId) -> bool {
        let ty = self.hir.locals[local].ty;
        if !self.needs_drop(ty) || self.zeroes_itself(ty) {
            return false;
        }
        let hir = self.hir;
        hir.exprs.iter().any(|(_, expr)| {
            matches!(expr.kind, ExprKind::Move(inner)
                if matches!(hir.exprs[inner].kind, ExprKind::Local(l) if l == local))
        })
    }

    /// Whether a moved-out value of this type can be told from a live one by
    /// its own bits: an `own` holds null, and a buffer holds null and zero.
    pub(super) fn zeroes_itself(&self, ty: Ty) -> bool {
        matches!(self.kind(ty), TyKind::Own(_))
    }

    /// Drops a block's locals and runs its `defer`s, newest first.
    pub(super) fn unwind(&mut self, scope: Vec<Cleanup>) {
        for cleanup in scope.into_iter().rev() {
            match cleanup {
                Cleanup::Drop(local) => self.drop_local(local),
                // Like an expression statement written at this exit.
                Cleanup::Defer(expr) => {
                    let mark = self.temps.len();
                    self.expr_stmt(expr, mark);
                }
            }
        }
    }

    /// Unwinds the innermost block.
    pub(super) fn drop_scope(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            self.unwind(scope);
        }
    }

    /// On `return`: unwinds every block of the function, innermost first,
    /// without forgetting them, since the paths that do not return still
    /// unwind them where their blocks end.
    pub(super) fn drop_all_scopes(&mut self) {
        for scope in self.scopes.clone().into_iter().rev() {
            self.unwind(scope);
        }
    }

    /// Registers a temporary to drop at the end of the current statement, if
    /// it owns memory.
    pub(super) fn own_temp(&mut self, place: Place, ty: Ty) {
        if !self.needs_drop(ty) {
            return;
        }
        // A `match` or a `?` may take the value out of it, and a type that
        // cleans up after itself cannot say so by its bits, so a flag says
        // it.
        if place.projections.is_empty() && !self.zeroes_itself(ty) && self.has_own_drop(ty) {
            let flag = self.new_local(Types::BOOL, LocalKind::Temp);
            // False from the start: a temporary made in one branch of what
            // its statement evaluates is dropped where the branches join,
            // and on the others there is nothing to drop.
            self.blocks[0].0.insert(
                0,
                Statement::Assign(
                    Place::local(flag),
                    Rvalue::Use(Operand::Const(Const::Bool(false))),
                ),
            );
            self.assign(
                Place::local(flag),
                Rvalue::Use(Operand::Const(Const::Bool(true))),
            );
            self.temp_flags.insert(place.local, flag);
        }
        self.temps.push((place, ty));
    }

    /// Whether dropping a value of this type runs a `destroy` a program wrote,
    /// anywhere inside it. Everything else that a drop does — freeing an `own`,
    /// and the zeroing that follows — can tell a value that was moved away from
    /// one that was not.
    fn has_own_drop(&mut self, ty: Ty) -> bool {
        if self.program.has_drop(ty) {
            return true;
        }
        match self.kind(ty) {
            TyKind::Struct(..) | TyKind::Array(..) => self
                .parts(ty)
                .into_iter()
                .any(|(_, part)| self.has_own_drop(part)),
            TyKind::Enum(..) => self
                .variant_parts(ty)
                .into_iter()
                .flatten()
                .any(|(_, part)| self.has_own_drop(part)),
            // An `own` says for itself that it was moved away: its pointer
            // is null, and nothing below it is reached. That is also what
            // keeps this from following a type that holds itself.
            TyKind::Own(_) => false,
            _ => false,
        }
    }

    /// The value at `place` is no longer the temporary's to drop: a `match`
    /// arm or a `?` has taken it.
    pub(super) fn taken_from_temp(&mut self, place: &Place) {
        let Some(&flag) = self.temp_flags.get(&place.local) else {
            return;
        };
        self.assign(
            Place::local(flag),
            Rvalue::Use(Operand::Const(Const::Bool(false))),
        );
    }

    /// A finished part of something still being built, such as an argument
    /// or a field, that owns memory. Until the whole is complete, the part is
    /// a temporary: a later part may `return`, `break` or `continue`, which
    /// must drop it.
    pub(super) fn pending_part(&mut self, place: Place, ty: Ty, pending: &mut Vec<usize>) {
        if self.needs_drop(ty) {
            pending.push(self.temps.len());
            self.temps.push((place, ty));
        }
    }

    /// The whole is complete and owns its parts. Temporaries registered
    /// after them, by later parts, still end with their statement.
    pub(super) fn forget_parts(&mut self, pending: Vec<usize>) {
        for index in pending.into_iter().rev() {
            self.temps.remove(index);
        }
    }

    /// Whether evaluating `id` could leave it by `return`, `break` or
    /// `continue`: only a block holds statements, and only `if`, `match` and
    /// blocks hold blocks.
    pub(super) fn may_jump(&self, id: ExprId) -> bool {
        let kind = &self.hir.exprs[id].kind;
        match kind {
            ExprKind::Block(_) | ExprKind::If { .. } | ExprKind::Match { .. } => true,
            _ => kind.children().into_iter().any(|e| self.may_jump(e)),
        }
    }

    /// Drops the temporaries registered since `mark`, newest first. On a
    /// path that has ended there is nothing left to drop: each `break`,
    /// `continue` or `return` that ended it dropped them as it left.
    pub(super) fn drop_temps(&mut self, mark: usize) {
        if self.dead {
            self.temps.truncate(mark);
            return;
        }
        while self.temps.len() > mark {
            let (place, ty) = self.temps.pop().expect("length checked");
            self.drop_temp(&place, ty);
        }
    }

    /// A temporary, which a `match` arm or a `?` may have taken the value
    /// out of.
    pub(super) fn drop_temp(&mut self, place: &Place, ty: Ty) {
        let Some(&flag) = self.temp_flags.get(&place.local) else {
            self.drop_place(place, ty);
            return;
        };
        let live = Operand::Copy(Place::local(flag));
        let drop = self.new_block();
        let next = self.new_block();
        self.terminate(Terminator::Branch {
            cond: live,
            then: drop,
            otherwise: next,
        });
        self.switch_to(drop);
        self.drop_place(place, ty);
        self.terminate(Terminator::Goto(next));
        self.switch_to(next);
    }

    /// On `return`: drops the temporaries of the statements around it
    /// without forgetting them, since the paths that do not return still
    /// drop them when their statements end.
    pub(super) fn drop_pending_temps(&mut self) {
        for (place, ty) in self.temps.clone().into_iter().rev() {
            self.drop_temp(&place, ty);
        }
    }

    /// The body of the drop function of an `own<ty>`: a loop over a chain
    /// of `own` values. Each iteration frees one allocation,
    /// and when what the value drops last is an `own<ty>` again, continues
    /// with that pointer.
    /// The body of [`lower_drop_in_place_fn`](super::lower_drop_in_place_fn).
    pub(super) fn drop_in_place_fn(&mut self, ty: Ty) {
        let pointer = self
            .program
            .types
            .find(TyKind::Own(ty))
            .expect("interning a buffer of the type interns a pointer to it");
        let param = self.new_local(pointer, LocalKind::Param);
        self.params.push(param);
        self.drop_place(&Place::local(param).project(Projection::Deref), ty);
        self.terminate(Terminator::Return);
    }

    pub(super) fn drop_fn(&mut self, ty: Ty) {
        let own = self
            .program
            .types
            .find(TyKind::Own(ty))
            .expect("something drops an `own` of the type, so the type exists");
        let param = self.new_local(own, LocalKind::Param);
        self.params.push(param);
        let node = self.new_local(own, LocalKind::Var);
        self.assign(
            Place::local(node),
            Rvalue::Use(Operand::Copy(Place::local(param))),
        );
        let again = self.new_block();
        let body = self.new_block();
        let exit = self.new_block();
        self.terminate(Terminator::Goto(again));
        self.switch_to(again);
        let is_null = self.value(
            Types::BOOL,
            Rvalue::Binary(
                BinaryOp::Eq,
                Operand::Copy(Place::local(node)),
                Self::int(0, own),
            ),
        );
        self.terminate(Terminator::Branch {
            cond: is_null,
            then: exit,
            otherwise: body,
        });
        self.switch_to(body);
        let chain = Chain {
            target: ty,
            node,
            again,
        };
        self.drop_last(Place::local(node).project(Projection::Deref), ty, chain);
        self.switch_to(exit);
        self.terminate(Terminator::Return);
    }

    /// Drops the value of type `ty` at `place`, the last part of the chain's
    /// node to be dropped, and then frees the node. If that last part is an
    /// `own` of the chain's type, the loop continues with it instead of
    /// calling the drop function again. Ends the current block.
    /// A type that cleans up after itself runs its own drop first; its
    /// fields are dropped after it returns.
    fn call_drop(&mut self, place: &Place, ty: Ty) {
        let Some(&drop) = self.program.drop_fns.get(&ty) else {
            return;
        };
        let address = self.value(Types::PTR_U8, Rvalue::AddressOf(place.clone()));
        self.push(Statement::Call {
            callee: Callee::Fn(drop),
            args: vec![address],
            dest: None,
        });
    }

    fn drop_last(&mut self, place: Place, ty: Ty, chain: Chain) {
        self.call_drop(&place, ty);
        // A generator drops what it holds through its `next`.
        if let TyKind::Struct(id, _) = self.kind(ty)
            && self.program.structs[id].generator.is_some()
        {
            self.drop_place(&place, ty);
            self.push(Statement::Free(Operand::Copy(Place::local(chain.node))));
            self.terminate(Terminator::Return);
            return;
        }
        match self.kind(ty) {
            TyKind::Own(inner) if inner == chain.target => {
                let next = self.value(ty, Rvalue::Use(Operand::Copy(place)));
                self.push(Statement::Free(Operand::Copy(Place::local(chain.node))));
                self.assign(Place::local(chain.node), Rvalue::Use(next));
                self.terminate(Terminator::Goto(chain.again));
            }
            TyKind::Struct(..) | TyKind::Array(..) => {
                let parts = self.parts(ty);
                self.drop_parts_last(place, &parts, chain);
            }
            TyKind::Enum(..) => {
                let variants = self.variant_parts(ty);
                let variant = self.value(Types::I32, Rvalue::Variant(place.clone()));
                // A variant that owns nothing only frees the node.
                let owns_nothing = self.new_block();
                let mut cases = Vec::new();
                let mut bodies = Vec::new();
                for (v, parts) in variants.into_iter().enumerate() {
                    if parts.is_empty() {
                        continue;
                    }
                    let block = self.new_block();
                    cases.push((v as u32, block));
                    bodies.push((block, parts));
                }
                self.terminate(Terminator::Switch {
                    value: variant,
                    cases,
                    otherwise: owns_nothing,
                });
                for (block, parts) in bodies {
                    self.switch_to(block);
                    self.drop_parts_last(place.clone(), &parts, chain);
                }
                self.switch_to(owns_nothing);
                self.push(Statement::Free(Operand::Copy(Place::local(chain.node))));
                self.terminate(Terminator::Return);
            }
            _ => {
                self.drop_place(&place, ty);
                self.push(Statement::Free(Operand::Copy(Place::local(chain.node))));
                self.terminate(Terminator::Return);
            }
        }
    }

    /// Drops the given parts of the value at `place`, listed in declaration
    /// order, in reverse order. The first part is dropped
    /// last, through `drop_last`. Ends the current block.
    fn drop_parts_last(&mut self, place: Place, parts: &[Part], chain: Chain) {
        let Some((&(first, first_ty), rest)) = parts.split_first() else {
            self.push(Statement::Free(Operand::Copy(Place::local(chain.node))));
            self.terminate(Terminator::Return);
            return;
        };
        for &(projection, ty) in rest.iter().rev() {
            self.drop_place(&place.project(projection), ty);
        }
        self.drop_last(place.project(first), first_ty, chain);
    }
}

/// An iteration of a drop function's loop: the local that
/// holds the allocation being dropped, of type `own<target>`, and the block
/// that takes the next one.
#[derive(Clone, Copy)]
struct Chain {
    target: Ty,
    node: Local,
    again: BlockId,
}
