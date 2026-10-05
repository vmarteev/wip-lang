//! Translating one MIR body: where each local lives, and each block's
//! statements and terminator.

use super::*;
use crate::module::reg_type;

impl<'c, 'p, 'f> FnCodegen<'c, 'p, 'f> {
    pub(super) fn new(
        cg: &'c mut Codegen<'p>,
        b: FunctionBuilder<'f>,
        body: &'f mir::Body,
    ) -> Self {
        FnCodegen {
            cg,
            b,
            body,
            storage: Vec::with_capacity(body.locals.len()),
            homes: Vec::new(),
            blocks: Vec::with_capacity(body.blocks.len()),
            runtime_refs: FxHashMap::default(),
            func_refs: FxHashMap::default(),
            data_refs: FxHashMap::default(),
            c_call: None,
        }
    }

    /// `c_abi` says C calls this one, so a `str` and a `&[T]` arrive as a
    /// pointer and a length, and `c_call` says how its structs arrive, where
    /// one crosses by value.
    /// Answers where the variables and parameters a debugger is told of
    /// are kept.
    pub(super) fn lower(mut self, c_abi: bool, c_call: Option<CCall>) -> Vec<Home> {
        let body = self.body;
        for _ in &body.blocks {
            let block = self.b.create_block();
            self.blocks.push(block);
        }
        let entry = self.blocks[0];
        self.b.append_block_params_for_function_params(entry);
        self.b.switch_to_block(entry);
        let mut incoming = self.b.block_params(entry).to_vec().into_iter();
        // A result C takes in registers is built in a slot of this
        // function's own, and loaded from it at the return.
        let answer = c_call.as_ref().map(|c_call| c_call.ret.clone());
        let mut ret_slot = None;
        let ret_ptr = match body.ret {
            Some(ret) if self.cg.is_aggregate(body.local(ret).ty) => match &answer {
                Some(Answer::Parts(parts)) => {
                    let slot = self.scratch(body.local(ret).ty, parts);
                    ret_slot = Some(slot);
                    Some(self.b.ins().stack_addr(types::I64, slot, 0))
                }
                _ => incoming.next(),
            },
            _ => None,
        };
        let mut params = FxHashMap::default();
        for (i, &param) in body.params.iter().enumerate() {
            let ty = body.local(param).ty;
            // A struct C passed in registers is put back into memory, which
            // is where the body reads it from; one C passed by a pointer to
            // a copy, or on the stack, is at the address that arrives.
            let pass = c_call.as_ref().map(|c_call| &c_call.params[i]);
            // The zeros a caller put before a spilled struct say only where
            // the struct is; there is nothing in them to read.
            if let Some(Pass::Spilled { pad, .. }) = pass {
                for _ in 0..pad.ints + pad.floats {
                    incoming.next().expect("a register of padding");
                }
            }
            match pass {
                Some(Pass::Parts(parts) | Pass::Spilled { parts, .. }) => {
                    let slot = self.scratch(ty, parts);
                    let addr = self.b.ins().stack_addr(types::I64, slot, 0);
                    for part in parts {
                        let value = incoming.next().expect("one value per piece");
                        self.b.ins().store(
                            MemFlagsData::trusted(),
                            value,
                            addr,
                            part.offset as i32,
                        );
                    }
                    params.insert(param, addr);
                    continue;
                }
                Some(Pass::Copy | Pass::Stack(_)) => {
                    let addr = incoming.next().expect("an address");
                    params.insert(param, addr);
                    continue;
                }
                Some(Pass::Plain) | None => {}
            }
            // A `str` and a `&[T]` arrive from C as a pointer and a length,
            // and the body reads the pair as one place, so it is put back
            // together here.
            if c_abi && self.cg.is_c_pair(ty) {
                let pointer = incoming.next().expect("a pointer");
                let length = incoming.next().expect("a length");
                let slot = self.slot(ty);
                let addr = self.b.ins().stack_addr(types::I64, slot, 0);
                self.b
                    .ins()
                    .store(MemFlagsData::trusted(), pointer, addr, 0);
                self.b.ins().store(MemFlagsData::trusted(), length, addr, 8);
                params.insert(param, addr);
                continue;
            }
            let value = incoming.next().expect("one block parameter per parameter");
            params.insert(param, value);
        }

        let addressed = addressed_locals(body);
        for (i, decl) in body.locals.iter().enumerate() {
            let local = Local(i as u32);
            let aggregate = self.cg.is_aggregate(decl.ty);
            // A debug build keeps each variable and parameter of the source
            // in memory, where the debugger finds it for as long as it is
            // in scope.
            let homed = self.cg.homes_locals && decl.source.is_some();
            let storage = if aggregate {
                match decl.kind {
                    LocalKind::Param => {
                        let address = params[&local];
                        if homed {
                            let slot = self.address_slot();
                            self.b.ins().stack_store(types::I64, address, slot, 0);
                            self.homes.push(Home {
                                local,
                                slot,
                                indirect: true,
                            });
                        }
                        Storage::Ptr(address)
                    }
                    LocalKind::Return if ret_slot.is_some() => {
                        Storage::Slot(ret_slot.expect("a slot for the result"))
                    }
                    LocalKind::Return => {
                        Storage::Ptr(ret_ptr.expect("an aggregate result has a pointer"))
                    }
                    LocalKind::Var | LocalKind::Temp => {
                        let slot = self.slot(decl.ty);
                        if homed {
                            self.homes.push(Home {
                                local,
                                slot,
                                indirect: false,
                            });
                        }
                        Storage::Slot(slot)
                    }
                }
            } else if let Some(scalar) = self.cg.scalar_type(decl.ty) {
                if addressed[i] || homed {
                    let slot = self.slot(decl.ty);
                    if homed {
                        // A binding that aliases what it matched holds its
                        // address, and the debugger shows what is there.
                        let indirect = decl.source.is_some_and(|source| source.by_address);
                        self.homes.push(Home {
                            local,
                            slot,
                            indirect,
                        });
                    }
                    Storage::Slot(slot)
                } else {
                    Storage::Var(self.b.declare_var(scalar))
                }
            } else {
                Storage::Empty
            };
            self.storage.push(storage);
            if decl.kind == LocalKind::Param && !aggregate {
                self.write(&Place::local(local), params[&local]);
            }
        }

        // A block is sealed once every branch into it has been emitted, which
        // lets Cranelift build SSA form as it goes.
        self.c_call = c_call;
        let mut remaining = vec![0u32; body.blocks.len()];
        for block in &body.blocks {
            for target in block.terminator.successors() {
                remaining[target.0 as usize] += 1;
            }
        }
        for (i, &count) in remaining.iter().enumerate() {
            if count == 0 {
                self.b.seal_block(self.blocks[i]);
            }
        }
        for (i, block) in body.blocks.iter().enumerate() {
            if i > 0 {
                self.b.switch_to_block(self.blocks[i]);
            }
            for statement in &block.statements {
                self.statement(statement);
            }
            self.terminator(&block.terminator);
            for target in block.terminator.successors() {
                let count = &mut remaining[target.0 as usize];
                *count -= 1;
                if *count == 0 {
                    self.b.seal_block(self.blocks[target.0 as usize]);
                }
            }
        }
        self.b.seal_all_blocks();
        let config = self.cg.module.target_config();
        self.b.finalize(config);
        self.homes
    }

    fn statement(&mut self, statement: &Statement) {
        match statement {
            // The instructions that follow were written here.
            Statement::At(span) => self.b.set_srcloc(SourceLoc::new(span.lo)),
            Statement::Assign(place, rvalue) => self.assign(place, rvalue),
            Statement::SetVariant(place, variant) => self.set_variant(place, *variant),
            Statement::Zero(place) => self.zero_place(place),
            Statement::Poison(place) => self.poison_place(place),
            Statement::CheckMoved { place, at } => {
                let moved = self.poisoned(place);
                let panics = self.b.create_block();
                let carries_on = self.b.create_block();
                self.b.ins().brif(moved, panics, &[], carries_on, &[]);
                self.b.switch_to_block(panics);
                self.b.seal_block(panics);
                self.call_panic("wip_panic_moved", Vec::new(), *at);
                self.b.switch_to_block(carries_on);
                self.b.seal_block(carries_on);
            }
            Statement::Call { callee, args, dest } => self.call(callee, args, dest.as_ref()),
            Statement::Alloc { dest, ty } => {
                let size = self.cg.layout(*ty).size.max(1);
                let size = self.b.ins().iconst(types::I64, i64::from(size));
                let alloc = self.runtime_ref(self.cg.alloc_fn);
                let call = self.b.ins().call(alloc, &[size]);
                let ptr = self.b.inst_results(call)[0];
                self.write(dest, ptr);
            }
            Statement::AllocBuffer { dest, elem, count } => {
                let count = self.operand(count);
                let stride = i64::from(self.cg.layout(*elem).stride());
                let stride = self.b.ins().iconst(types::I64, stride);
                // More bytes than an `i64` counts is more than memory holds.
                let (size, overflow) = self.b.ins().smul_overflow(count, stride);
                self.b.ins().trapnz(overflow, TrapCode::HEAP_OUT_OF_BOUNDS);
                let alloc = self.runtime_ref(self.cg.alloc_fn);
                let call = self.b.ins().call(alloc, &[size]);
                let ptr = self.b.inst_results(call)[0];
                let addr = self.place_addr(dest);
                self.b.ins().store(MemFlagsData::trusted(), ptr, addr, 0);
                self.b.ins().store(MemFlagsData::trusted(), count, addr, 8);
            }
            // One step on an `i64` other threads may touch.
            Statement::Atomic {
                op,
                ty,
                address,
                value,
                expected,
                dest,
            } => {
                let address = self.operand(address);
                let flags = MemFlagsData::trusted();
                let width = self
                    .cg
                    .scalar_type(*ty)
                    .expect("an atomic's value is a scalar");
                let rmw = |this: &mut Self, operation| {
                    let operand = this.operand(value.as_ref().expect("an atomic's value"));
                    this.b
                        .ins()
                        .atomic_rmw(width, flags, operation, address, operand)
                };
                let answer = match op {
                    AtomicOp::Add => Some(rmw(self, ir::AtomicRmwOp::Add)),
                    AtomicOp::Subtract => Some(rmw(self, ir::AtomicRmwOp::Sub)),
                    AtomicOp::Swap => Some(rmw(self, ir::AtomicRmwOp::Xchg)),
                    AtomicOp::CompareSwap => {
                        let wanted = self.operand(expected.as_ref().expect("an expected value"));
                        let stored = self.operand(value.as_ref().expect("an atomic's value"));
                        Some(self.b.ins().atomic_cas(flags, address, wanted, stored))
                    }
                    AtomicOp::Load => Some(self.b.ins().atomic_load(width, flags, address)),
                    AtomicOp::Store => {
                        let stored = self.operand(value.as_ref().expect("a store has a value"));
                        self.b.ins().atomic_store(flags, stored, address);
                        None
                    }
                };
                if let (Some(dest), Some(answer)) = (dest, answer) {
                    self.write(dest, answer);
                }
            }
            Statement::Free(ptr) => {
                let ptr = self.operand(ptr);
                let free = self.runtime_ref(self.cg.free_fn);
                self.b.ins().call(free, &[ptr]);
            }
            Statement::DropFn { ty, ptr } => {
                let ptr = self.operand(ptr);
                let drop_fn = self.cg.drop_fn(*ty);
                let drop_fn = self.runtime_ref(drop_fn);
                self.b.ins().call(drop_fn, &[ptr]);
            }
            Statement::DropInPlace { ty, ptr } => {
                let ptr = self.operand(ptr);
                let drop_fn = self.cg.drop_in_place_fn(*ty);
                let drop_fn = self.runtime_ref(drop_fn);
                self.b.ins().call(drop_fn, &[ptr]);
            }
            // A check that fails panics, with what its kind says and where
            // it was written.
            // `a + b`, `a - b`, `a * b` on integers: the answer and whether
            // it fits come from one instruction, and not fitting panics
            // where it was written.
            Statement::Arith {
                dest,
                op,
                lhs,
                rhs,
                at,
            } => {
                let ty = self.place_ty(dest);
                let signed = matches!(self.kind(ty), TyKind::Int(int) if int.signed());
                let lhs = self.operand(lhs);
                let rhs = self.operand(rhs);
                let (value, overflowed) = match (op, signed) {
                    (BinaryOp::Add, true) => self.b.ins().sadd_overflow(lhs, rhs),
                    (BinaryOp::Add, false) => self.b.ins().uadd_overflow(lhs, rhs),
                    (BinaryOp::Sub, true) => self.b.ins().ssub_overflow(lhs, rhs),
                    (BinaryOp::Sub, false) => self.b.ins().usub_overflow(lhs, rhs),
                    (BinaryOp::Mul, true) => self.b.ins().smul_overflow(lhs, rhs),
                    (BinaryOp::Mul, false) => self.b.ins().umul_overflow(lhs, rhs),
                    (op, _) => unreachable!("{op:?} does not overflow"),
                };
                let panics = self.b.create_block();
                let carries_on = self.b.create_block();
                self.b.ins().brif(overflowed, panics, &[], carries_on, &[]);
                self.b.switch_to_block(panics);
                self.b.seal_block(panics);
                let which = self
                    .b
                    .ins()
                    .iconst(types::I64, i64::from(overflow_code(*op)));
                self.call_panic("wip_panic_overflow", vec![which], *at);
                self.b.switch_to_block(carries_on);
                self.b.seal_block(carries_on);
                self.write(dest, value);
            }
            Statement::Check { fails, kind, at } => {
                let fails = self.operand(fails);
                let panics = self.b.create_block();
                let carries_on = self.b.create_block();
                self.b.ins().brif(fails, panics, &[], carries_on, &[]);
                self.b.switch_to_block(panics);
                self.b.seal_block(panics);
                let (name, numbers) = match kind {
                    mir::CheckKind::Bounds { index, length } => {
                        let index = self.operand(index);
                        let length = self.operand(length);
                        ("wip_panic_index", vec![index, length])
                    }
                    mir::CheckKind::Length { length } => {
                        let length = self.operand(length);
                        ("wip_panic_length", vec![length])
                    }
                    mir::CheckKind::Division => ("wip_panic_division", Vec::new()),
                    mir::CheckKind::Overflow(op) => {
                        let which = self
                            .b
                            .ins()
                            .iconst(types::I64, i64::from(overflow_code(*op)));
                        ("wip_panic_overflow", vec![which])
                    }
                };
                self.call_panic(name, numbers, *at);
                self.b.switch_to_block(carries_on);
                self.b.seal_block(carries_on);
            }
        }
    }

    /// A call of one of the runtime's panic functions: its own numbers, then
    /// the file, the line and the column. It does not return, so the block
    /// ends.
    fn call_panic(&mut self, name: &'static str, numbers: Vec<Value>, at: wip_syntax::Span) {
        let (file, line, column) = (self.cg.locations)(at);
        let file_data = self.cg.text_data(&file);
        let file_ptr = self.data_addr(file_data);
        let file_len = self.b.ins().iconst(types::I64, file.len() as i64);
        let line = self.b.ins().iconst(types::I64, i64::from(line));
        let column = self.b.ins().iconst(types::I64, i64::from(column));
        let count = numbers.len();
        let mut args = numbers;
        args.extend([file_ptr, file_len, line, column]);
        let func = self.cg.panic_fn(name, count);
        let func_ref = self.runtime_ref(func);
        self.b.ins().call(func_ref, &args);
        self.b
            .ins()
            .trap(TrapCode::user(2).expect("non-zero trap code"));
    }

    fn terminator(&mut self, terminator: &Terminator) {
        match terminator {
            Terminator::Goto(target) => {
                self.b.ins().jump(self.blocks[target.0 as usize], &[]);
            }
            Terminator::Branch {
                cond,
                then,
                otherwise,
            } => {
                let cond = self.operand(cond);
                let (then, otherwise) = (
                    self.blocks[then.0 as usize],
                    self.blocks[otherwise.0 as usize],
                );
                self.b.ins().brif(cond, then, &[], otherwise, &[]);
            }
            // Cranelift's switch: a jump table where the cases are dense,
            // a search where they are not, and a comparison or two where
            // there are few. An interpreter's dispatch on its opcodes is
            // one, as a `match` on a large enum is.
            Terminator::Switch {
                value,
                cases,
                otherwise,
            } => {
                let value = self.operand(value);
                let otherwise = self.blocks[otherwise.0 as usize];
                let mut switch = Switch::new();
                for &(case, target) in cases {
                    switch.set_entry(u128::from(case), self.blocks[target.0 as usize]);
                }
                switch.emit(&mut self.b, value, otherwise);
            }
            Terminator::Return => match self.body.ret {
                // A struct C takes back in registers, loaded from where the
                // body built it.
                Some(ret)
                    if matches!(
                        self.c_call.as_ref().map(|c_call| &c_call.ret),
                        Some(Answer::Parts(_))
                    ) =>
                {
                    let Some(Answer::Parts(parts)) = self.c_call.as_ref().map(|c| c.ret.clone())
                    else {
                        unreachable!("matched above")
                    };
                    let addr = self.place_addr(&Place::local(ret));
                    let values: Vec<Value> = parts
                        .iter()
                        .map(|part| {
                            self.b.ins().load(
                                reg_type(part.reg),
                                MemFlagsData::trusted(),
                                addr,
                                part.offset as i32,
                            )
                        })
                        .collect();
                    self.b.ins().return_(&values);
                }
                Some(ret) if self.cg.scalar_type(self.body.local(ret).ty).is_some() => {
                    let value = self.operand(&Operand::Copy(Place::local(ret)));
                    self.b.ins().return_(&[value]);
                }
                _ => {
                    self.b.ins().return_(&[]);
                }
            },
            // The message and the place go to the runtime, which prints them
            // and ends the program.
            Terminator::Panic { note, message, at } => {
                let text =
                    message.map_or(String::new(), |m| self.cg.interner.resolve(m).to_string());
                let data = self.cg.text_data(&text);
                let ptr = self.data_addr(data);
                let len = self.b.ins().iconst(types::I64, text.len() as i64);
                match note {
                    None => self.call_panic("wip_panic", vec![ptr, len], *at),
                    // A message built when the program ran, and then what is
                    // written in the program: a `str` is its pointer and its
                    // length.
                    Some(Operand::Copy(place)) => {
                        let addr = self.place_addr(place);
                        let flags = MemFlagsData::trusted();
                        let note_ptr = self.b.ins().load(types::I64, flags, addr, 0);
                        let note_len = self.b.ins().load(types::I64, flags, addr, 8);
                        self.call_panic("wip_panic_noted", vec![note_ptr, note_len, ptr, len], *at);
                    }
                    Some(Operand::Const(_)) => unreachable!("a note is a place"),
                }
            }
            Terminator::Unreachable => {
                self.b
                    .ins()
                    .trap(TrapCode::user(1).expect("non-zero trap code"));
            }
        }
    }

    /// A call. An aggregate argument is passed by the address of its place;
    /// an aggregate result is written to the address of `dest`.
    fn call(&mut self, callee: &mir::Callee, args: &[Operand], dest: Option<&Place>) {
        let program = self.cg.program;
        if let mir::Callee::Fn(id) = callee
            && let Some(original) = program.fns[*id].variadic_of
            && !self.cg.shimmed.contains(id)
        {
            return self.variadic_call(*id, original, args, dest);
        }
        // A variable C owns: a load, or a store.
        if let mir::Callee::Fn(id) = callee
            && let Some(wip_hir::Access::Global(global)) = program.fns[*id].accesses
            && !self.cg.shimmed.contains(id)
        {
            let data = self.cg.c_global(global);
            let addr = self.data_addr(data);
            let ty = program.globals[global].ty;
            let scalar = self.cg.scalar_type(ty).expect("C's variable is a scalar");
            match args {
                [value] => {
                    let value = self.operand(value);
                    self.b.ins().store(MemFlagsData::trusted(), value, addr, 0);
                }
                _ => {
                    let value = self.b.ins().load(scalar, MemFlagsData::trusted(), addr, 0);
                    let dest = dest.expect("a read has a destination");
                    self.write(dest, value);
                }
            }
            return;
        }
        // Only C takes a `str` or a `&[T]` as two arguments.
        let c_abi = matches!(callee, mir::Callee::Fn(id)
            if crate::module::uses_c_abi(&self.cg.program.fns[*id]));
        let (params, ret) = match callee {
            // The runtime compares two strings' bytes: two
            // pointers and two lengths in, an order out.
            mir::Callee::StrCmp => (
                vec![Types::PTR_U8, Types::I64, Types::PTR_U8, Types::I64],
                Types::I64,
            ),
            mir::Callee::Fn(id) => {
                let def = &program.fns[*id];
                (def.params.iter().map(|p| p.ty).collect(), def.ret)
            }
            mir::Callee::Value(value) => {
                let TyKind::Fn(params, ret) = self.kind(self.operand_ty(value)) else {
                    unreachable!("a value that is called has a function type")
                };
                (program.types.list(params).to_vec(), ret)
            }
        };
        // How C passes this callee's structs, where one crosses by value
        // and the call is made here rather than in a wrapper.
        let c_call = match callee {
            mir::Callee::Fn(id) => self.cg.c_call(*id),
            _ => None,
        };
        let mut values = Vec::with_capacity(args.len() + 1);
        let answer_parts = match c_call.as_ref().map(|c_call| &c_call.ret) {
            Some(Answer::Parts(parts)) => Some(parts.clone()),
            _ => None,
        };
        if self.cg.is_aggregate(ret) && answer_parts.is_none() {
            let dest = dest.expect("an aggregate result has a destination");
            values.push(self.place_addr(dest));
        }
        for (i, arg) in args.iter().enumerate() {
            let ty = self.operand_ty(arg);
            let pass = c_call.as_ref().map(|c_call| &c_call.params[i]);
            // A struct the registers left cannot hold: zeros fill the rest of
            // its class's registers, so that its pieces go on the stack.
            if let Some(Pass::Spilled { pad, .. }) = pass {
                for _ in 0..pad.ints {
                    values.push(self.b.ins().iconst(types::I64, 0));
                }
                for _ in 0..pad.floats {
                    values.push(self.b.ins().f64const(0.0));
                }
            }
            match pass {
                // Its bytes, a piece to a register or, spilled, to the
                // stack. It is copied to a slot at least as large as the
                // pieces first, since the last may reach past the struct's
                // own bytes.
                Some(Pass::Parts(parts) | Pass::Spilled { parts, .. }) => {
                    let Operand::Copy(place) = arg else {
                        unreachable!("an aggregate operand is a place")
                    };
                    let src = self.place_addr(place);
                    let slot = self.scratch(ty, parts);
                    let addr = self.b.ins().stack_addr(types::I64, slot, 0);
                    self.copy(addr, src, ty);
                    for part in parts {
                        let value = self.b.ins().load(
                            reg_type(part.reg),
                            MemFlagsData::trusted(),
                            addr,
                            part.offset as i32,
                        );
                        values.push(value);
                    }
                    continue;
                }
                // A copy of its own, which C may write to.
                Some(Pass::Copy) => {
                    let Operand::Copy(place) = arg else {
                        unreachable!("an aggregate operand is a place")
                    };
                    let src = self.place_addr(place);
                    let slot = self.slot(ty);
                    let addr = self.b.ins().stack_addr(types::I64, slot, 0);
                    self.copy(addr, src, ty);
                    values.push(addr);
                    continue;
                }
                // Cranelift copies the bytes onto the stack from here.
                Some(Pass::Stack(_)) => {
                    let Operand::Copy(place) = arg else {
                        unreachable!("an aggregate operand is a place")
                    };
                    values.push(self.place_addr(place));
                    continue;
                }
                Some(Pass::Plain) | None => {}
            }
            // A `str` and a `&[T]` are two arguments: the bytes or the
            // elements, and how many.
            if c_abi && self.cg.is_c_pair(ty) {
                let Operand::Copy(place) = arg else {
                    unreachable!("a pointer-and-length operand is a place")
                };
                let addr = self.place_addr(place);
                let pointer = self
                    .b
                    .ins()
                    .load(types::I64, MemFlagsData::trusted(), addr, 0);
                let length = self
                    .b
                    .ins()
                    .load(types::I64, MemFlagsData::trusted(), addr, 8);
                values.push(pointer);
                values.push(length);
                continue;
            }
            if self.cg.is_aggregate(ty) {
                let Operand::Copy(place) = arg else {
                    unreachable!("an aggregate operand is a place")
                };
                values.push(self.place_addr(place));
            } else {
                values.push(self.operand(arg));
            }
        }
        let call = match callee {
            mir::Callee::StrCmp => {
                let func = self.cg.str_cmp_fn();
                let func = self.runtime_ref(func);
                self.b.ins().call(func, &values)
            }
            mir::Callee::Fn(id) => {
                let func_ref = self.func_ref(*id);
                self.b.ins().call(func_ref, &values)
            }
            // Through a function's address, with the signature its type
            // gives.
            mir::Callee::Value(value) => {
                let addr = self.operand(value);
                let sig = self.cg.signature_of(&params, ret);
                let sig = self.b.import_signature(sig);
                self.b.ins().call_indirect(sig, addr, &values)
            }
        };
        // A struct C answered in registers: each piece stored where it
        // belongs, through a slot large enough for all of them.
        if let Some(parts) = answer_parts {
            let results = self.b.inst_results(call).to_vec();
            let slot = self.scratch(ret, &parts);
            let addr = self.b.ins().stack_addr(types::I64, slot, 0);
            for (part, value) in parts.iter().zip(results) {
                self.b
                    .ins()
                    .store(MemFlagsData::trusted(), value, addr, part.offset as i32);
            }
            let dest = dest.expect("an aggregate result has a destination");
            let dest = self.place_addr(dest);
            self.copy(dest, addr, ret);
            return;
        }
        if self.cg.scalar_type(ret).is_some() {
            let result = self.b.inst_results(call)[0];
            let dest = dest.expect("a scalar result has a destination");
            self.write(dest, result);
        }
    }

    /// A call that passes more than the declaration names, made here
    /// rather than in a wrapper. The named arguments go as
    /// they always do. The rest are promoted as C promotes them, and on
    /// Apple arm64 go on the stack, eight bytes each, once the argument
    /// registers are filled; elsewhere they go where a named argument
    /// would.
    fn variadic_call(&mut self, id: FnId, original: FnId, args: &[Operand], dest: Option<&Place>) {
        let program = self.cg.program;
        let def = &program.fns[id];
        let named = program.fns[original].params.len();
        let apple = self.cg.call_conv == CallConv::AppleAarch64;
        let mut sig = Signature::new(self.cg.call_conv);
        let mut values = Vec::with_capacity(args.len() + 16);
        let (mut ints, mut floats) = (0, 0);
        for (arg, param) in args[..named].iter().zip(&def.params) {
            let ty = param.ty;
            if self.cg.is_c_pair(ty) {
                let Operand::Copy(place) = arg else {
                    unreachable!("a pointer-and-length operand is a place")
                };
                let addr = self.place_addr(place);
                for offset in [0, 8] {
                    let value =
                        self.b
                            .ins()
                            .load(types::I64, MemFlagsData::trusted(), addr, offset);
                    values.push(value);
                    sig.params.push(AbiParam::new(types::I64));
                }
                ints += 2;
                continue;
            }
            match self.kind(ty) {
                TyKind::Float(_) => floats += 1,
                _ => ints += 1,
            }
            values.push(self.operand(arg));
            sig.params.push(self.cg.abi_param(ty));
        }
        // Apple arm64: every extra argument is on the stack. Filling the
        // registers the named arguments left puts them there, in order.
        if apple {
            for _ in ints..8 {
                values.push(self.b.ins().iconst(types::I64, 0));
                sig.params.push(AbiParam::new(types::I64));
            }
            for _ in floats..8 {
                values.push(self.b.ins().f64const(0.0));
                sig.params.push(AbiParam::new(types::F64));
            }
        }
        for arg in &args[named..] {
            let ty = self.operand_ty(arg);
            let value = self.operand(arg);
            let promoted = mir::c_abi::promoted(program, ty).expect("a scalar extra argument");
            let (value, param) = match promoted {
                Promoted::Double => {
                    let value = match self.kind(ty) {
                        TyKind::Float(FloatTy::F32) => self.b.ins().fpromote(types::F64, value),
                        _ => value,
                    };
                    (value, AbiParam::new(types::F64))
                }
                Promoted::Int { bits, signed } => {
                    // On Apple each takes a slot of eight bytes; elsewhere an
                    // `int` is an `int`.
                    let bits = if apple { 64 } else { bits };
                    let target = int_type(bits);
                    let current = self.b.func.dfg.value_type(value);
                    let value = if current.bits() < target.bits() {
                        if signed {
                            self.b.ins().sextend(target, value)
                        } else {
                            self.b.ins().uextend(target, value)
                        }
                    } else {
                        value
                    };
                    (value, AbiParam::new(target))
                }
            };
            values.push(value);
            sig.params.push(param);
        }
        let ret = def.ret;
        if self.cg.scalar_type(ret).is_some() {
            sig.returns.push(self.cg.abi_param(ret));
        }
        let func_ref = self.func_ref(original);
        let addr = self.b.ins().func_addr(types::I64, func_ref);
        let sig = self.b.import_signature(sig);
        let call = self.b.ins().call_indirect(sig, addr, &values);
        if self.cg.scalar_type(ret).is_some() {
            let result = self.b.inst_results(call)[0];
            let dest = dest.expect("a scalar result has a destination");
            self.write(dest, result);
        }
    }

    /// A slot for a struct that travels in registers: as large as the
    /// struct, and as the pieces it is loaded as, which may reach past
    /// its last byte.
    fn scratch(&mut self, ty: Ty, parts: &[mir::c_abi::Part]) -> StackSlot {
        let layout = self.cg.layout(ty);
        let reach = parts
            .iter()
            .map(|part| part.offset + reg_type(part.reg).bytes())
            .max()
            .unwrap_or(0);
        self.b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            layout.size.max(reach),
            3,
        ))
    }

    /// A reference to a Wip or C function in this function.
    pub(super) fn func_ref(&mut self, id: FnId) -> ir::FuncRef {
        if let Some(&func_ref) = self.func_refs.get(&id) {
            return func_ref;
        }
        let func = self.cg.fn_id(id);
        let func_ref = self.cg.module.declare_func_in_func(func, self.b.func);
        self.func_refs.insert(id, func_ref);
        func_ref
    }

    /// A stack slot for an address, kept where a debugger reads it.
    fn address_slot(&mut self) -> StackSlot {
        self.b
            .create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3))
    }

    /// A stack slot for a local of type `ty`. Aggregates always get at least
    /// one byte, so that they have an address.
    fn slot(&mut self, ty: Ty) -> StackSlot {
        let layout = self.cg.layout(ty);
        let align_shift = layout.align.trailing_zeros() as u8;
        self.b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            layout.size.max(1),
            align_shift,
        ))
    }

    pub(super) fn copy(&mut self, dest: Value, src: Value, ty: Ty) {
        let layout = self.cg.layout(ty);
        if layout.size == 0 {
            return;
        }
        let config = self.cg.module.target_config();
        let align = u8::try_from(layout.align).unwrap_or(u8::MAX);
        self.b.emit_small_memory_copy(
            config,
            dest,
            src,
            u64::from(layout.size),
            align,
            align,
            false,
            MemFlagsData::trusted(),
        );
    }

    pub(super) fn zero(&mut self, addr: Value, size: u32) {
        let mut offset = 0;
        while offset + 8 <= size {
            let zero = self.b.ins().iconst(types::I64, 0);
            self.b
                .ins()
                .store(MemFlagsData::trusted(), zero, addr, offset as i32);
            offset += 8;
        }
        while offset < size {
            let zero = self.b.ins().iconst(types::I8, 0);
            self.b
                .ins()
                .store(MemFlagsData::trusted(), zero, addr, offset as i32);
            offset += 1;
        }
    }

    pub(super) fn runtime_ref(&mut self, func: FuncId) -> ir::FuncRef {
        if let Some(&func_ref) = self.runtime_refs.get(&func) {
            return func_ref;
        }
        let func_ref = self.cg.module.declare_func_in_func(func, self.b.func);
        self.runtime_refs.insert(func, func_ref);
        func_ref
    }
}

/// Which scalar locals a body takes the address of, so that they need a
/// stack slot rather than an SSA variable.
fn addressed_locals(body: &mir::Body) -> Vec<bool> {
    let mut addressed = vec![false; body.locals.len()];
    for block in &body.blocks {
        for statement in &block.statements {
            if let Statement::Assign(_, Rvalue::AddressOf(place)) = statement
                && place.projections.is_empty()
            {
                addressed[place.local.0 as usize] = true;
            }
        }
    }
    addressed
}
