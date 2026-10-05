//! A generator's `next`: its loop, run on from the `yield`
//! it stopped at.
//!
//! The body is lowered as any other, with each `yield` a place it stops:
//! it writes `.Some(value)` to the result, notes where it is in the
//! generator's `state#` and returns. The entry block reads `state#` and
//! goes on from there. [`crate::frame`] then moves the body's locals into
//! the generator, after its declared fields, so they are where they were
//! when it is asked again.
//!
//! `state#` is 0 before the first call, `2k` while it waits at the `k`th
//! `yield`, and [`DONE`] once it has ended. A drop adds one and calls
//! `next`: `2k + 1` goes to what drops the locals in scope at the `k`th
//! `yield`, 1 to what drops what it was given and never started on, and
//! `DONE + 1` to nothing.

use super::*;

/// The state of a generator that has ended, or was dropped. One more is
/// still no case, so dropping it does nothing.
pub(crate) const DONE: i32 = -2;

/// A generator's `next` being lowered.
pub(super) struct Generator {
    /// The parameter: the generator itself, `&var generator#`.
    this: Local,
    /// Where the body starts, and where it goes when it has ended.
    start: BlockId,
    done: BlockId,
    /// The blocks the entry goes to for each state.
    cases: Vec<(u32, BlockId)>,
}

impl Builder<'_> {
    /// The body of a generator's `next`.
    pub(super) fn generator(&mut self, def: &FnDef) {
        let hir = self.hir;
        let this_hir = hir.params[0];
        let this = self.new_local(hir.locals[this_hir].ty, LocalKind::Param);
        self.params.push(this);
        self.vars.insert(this_hir, this);
        self.ret = Some(self.new_local(def.ret, LocalKind::Return));
        let start = self.new_block();
        let done = self.new_block();
        self.generator = Some(Generator {
            this,
            start,
            done,
            cases: Vec::new(),
        });

        // The body, from its start, into `done` at its end. What a function
        // that yields was given is in the generator's fields after
        // `state#`, and is taken out of them when it starts: from then on
        // it is the body's, and dropped as its parameters would be.
        self.switch_to(start);
        let given: Vec<(LocalId, u32)> = hir
            .params
            .iter()
            .skip(1)
            .enumerate()
            .map(|(i, &param)| (param, i as u32 + 1))
            .collect();
        for &(param, field) in &given {
            let ty = hir.locals[param].ty;
            let var = self.source_local(param, LocalKind::Var);
            self.vars.insert(param, var);
            let held = self.field_of_this(field);
            self.assign(Place::local(var), Rvalue::Use(Operand::Copy(held.clone())));
            if self.needs_drop(ty) {
                self.push(Statement::Zero(held));
            }
            self.set_flag(param, true);
        }
        self.scopes
            .push(given.iter().map(|&(p, _)| Cleanup::Drop(p)).collect());
        let value = hir.value();
        self.expr(value);
        if !self.dead {
            self.drop_temps(0);
            self.drop_scope();
            self.terminate(Terminator::Goto(done));
        }
        self.switch_to(done);
        self.stopped();

        // Dropped before it started: what it was given is still where the
        // function put it.
        let unstarted = self.new_block();
        self.switch_to(unstarted);
        for &(param, field) in given.iter().rev() {
            let ty = hir.locals[param].ty;
            let held = self.field_of_this(field);
            self.drop_place(&held, ty);
        }
        self.stopped();

        // The entry: on from where it stopped.
        self.switch_to(BlockId(0));
        let state = self.state_place();
        let now = self.value(Types::I32, Rvalue::Use(Operand::Copy(state)));
        let finished = self.new_block();
        let generator = self.generator.as_ref().expect("set above");
        let mut cases = vec![(0, generator.start), (1, unstarted)];
        cases.extend(generator.cases.iter().copied());
        self.terminate(Terminator::Switch {
            value: now,
            cases,
            otherwise: finished,
        });
        self.switch_to(finished);
        self.answer_none();
        self.terminate(Terminator::Return);
    }

    /// `(*this).state#`.
    fn state_place(&self) -> Place {
        self.field_of_this(0)
    }

    /// A declared field of the generator.
    fn field_of_this(&self, field: u32) -> Place {
        let this = self.generator.as_ref().expect("in a generator").this;
        Place::local(this)
            .project(Projection::Deref)
            .project(Projection::Field(field))
    }

    fn set_state(&mut self, state: i32) {
        let place = self.state_place();
        let bits = u128::from(state as u32);
        self.assign(place, Rvalue::Use(Self::int(bits, Types::I32)));
    }

    /// `None`, as the result.
    fn answer_none(&mut self) {
        let ret = self.ret.expect("a generator answers an `Option`");
        let option = self.program.types.kind(self.local_ty(ret));
        let TyKind::Enum(id, _) = option else {
            unreachable!("a generator answers an `Option`")
        };
        let none = self.program.enums[id]
            .variants
            .iter()
            .position(|v| self.interner.resolve(v.name) == "None")
            .expect("`Option` has `None`") as u32;
        self.push(Statement::SetVariant(Place::local(ret), none));
    }

    /// The generator has ended: it answers nothing, now and from here on.
    fn stopped(&mut self) {
        self.set_state(DONE);
        self.answer_none();
        self.terminate(Terminator::Return);
    }

    /// `yield value`: `.Some(value)` is the result, and the generator
    /// stops here until it is asked again, or dropped.
    pub(super) fn yield_stmt(&mut self, value: ExprId, mark: usize) {
        let ret = self.ret.expect("a generator answers an `Option`");
        self.store_expr(value, Place::local(ret));
        self.drop_temps(mark);
        if self.dead {
            return;
        }
        let k = self.generator.as_ref().expect("in a generator").cases.len() as u32 / 2 + 1;
        let resume = self.new_block();
        let drop = self.new_block();
        self.set_state(2 * k as i32);
        self.terminate(Terminator::Return);

        // Dropped while it waits here: what is in scope here is dropped,
        // as a `return` from here would drop it.
        self.switch_to(drop);
        self.drop_pending_temps();
        self.drop_all_scopes();
        self.stopped();

        let generator = self.generator.as_mut().expect("in a generator");
        generator.cases.push((2 * k, resume));
        generator.cases.push((2 * k + 1, drop));
        self.switch_to(resume);
    }

    /// `return` in a generator: it ends, having dropped what is in scope.
    pub(super) fn generator_return(&mut self) {
        self.drop_pending_temps();
        self.drop_all_scopes();
        let done = self.generator.as_ref().expect("in a generator").done;
        self.terminate(Terminator::Goto(done));
        self.dead = true;
    }

    /// Drops a generator: its `next`, told to drop what it holds where it
    /// stopped rather than to go on.
    pub(super) fn drop_generator(&mut self, place: &Place, ty: Ty) {
        let next = *self
            .program
            .generator_next
            .get(&ty)
            .expect("the monomorphizer made the `next` of every generator a program drops");
        let state = place.project(Projection::Field(0));
        let now = self.value(Types::I32, Rvalue::Use(Operand::Copy(state.clone())));
        let asked = self.value(
            Types::I32,
            Rvalue::Binary(BinaryOp::Add, now, Self::int(1, Types::I32)),
        );
        self.assign(state, Rvalue::Use(asked));
        let def = &self.program.fns[next];
        let (this_ty, ret_ty) = (def.params[0].ty, def.ret);
        let address = self.value(this_ty, Rvalue::AddressOf(place.clone()));
        let result = self.temp(ret_ty);
        self.push(Statement::Call {
            callee: Callee::Fn(next),
            args: vec![address],
            dest: Some(Place::local(result)),
        });
    }
}
