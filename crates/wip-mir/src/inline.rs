//! `@inline`: the callee's body, spliced into its caller.
//!
//! A call in MIR is a statement rather than a terminator, so splicing is
//! cutting the block in two around it: what came before keeps its place,
//! the callee's blocks go between, and what came after becomes the block
//! the callee's `return` goes to. Its locals and blocks are renumbered into
//! the caller, and its parameters become the places the caller passed —
//! so an aggregate argument is not copied, it is simply used where it is.
use rustc_hash::{FxHashMap, FxHashSet};
use wip_syntax::Interner;

use crate::*;

/// Splices every call in `body` to a function that promised to be inlined.
/// The callee is inlined first, so a chain of them is spliced once through.
pub fn inline(program: &Program, interner: &Interner, body: &mut Body) {
    let mut bodies = Bodies {
        program,
        interner,
        done: FxHashMap::default(),
        lambdas: FxHashSet::default(),
        open: Vec::new(),
    };
    bodies.splice_all(body);
}

struct Bodies<'a> {
    program: &'a Program,
    interner: &'a Interner,
    /// The MIR of each `@inline` function, already inlined itself, so a
    /// function called from several places is lowered once.
    done: FxHashMap<FnId, Body>,
    /// The lambdas an `@inline` function was handed, whose calls were made
    /// direct when its body was spliced: they are inlined too, which is
    /// what makes internal iteration cost what a loop costs.
    lambdas: FxHashSet<FnId>,
    /// What is being spliced, so that a body never splices itself.
    open: Vec<FnId>,
}

impl Bodies<'_> {
    /// The body to splice for `id`, if the compiler has one to splice.
    fn of(&mut self, id: FnId) -> Option<&Body> {
        if !self.done.contains_key(&id) {
            let def = &self.program.fns[id];
            let hir = def.body.as_ref()?;
            let mut body = crate::build::lower_fn(self.program, self.interner, def, hir);
            self.open.push(id);
            self.splice_all(&mut body);
            self.open.pop();
            self.done.insert(id, body);
        }
        self.done.get(&id)
    }

    /// Splices the calls of one body, from the last to the first, so that
    /// the statements a splice moves are ones this pass has already seen.
    fn splice_all(&mut self, body: &mut Body) {
        let mut block = 0;
        while block < body.blocks.len() {
            let mut statement = 0;
            while statement < body.blocks[block].statements.len() {
                let Statement::Call {
                    callee: Callee::Fn(id),
                    ..
                } = &body.blocks[block].statements[statement]
                else {
                    statement += 1;
                    continue;
                };
                let id = *id;
                let wanted = self.program.fns[id].is_inline || self.lambdas.contains(&id);
                if !wanted || self.open.contains(&id) {
                    statement += 1;
                    continue;
                }
                let Some(callee) = self.of(id).cloned() else {
                    statement += 1;
                    continue;
                };
                self.open.push(id);
                let handed = splice(
                    self.program,
                    body,
                    BlockId(block as u32),
                    statement,
                    &callee,
                );
                self.open.pop();
                self.lambdas.extend(handed);
                // What followed the call is now in a block of its own, so
                // this one is finished.
                break;
            }
            block += 1;
        }
    }
}

/// Puts `callee` in place of the call at `statement` of `block`, and says
/// which lambdas the call handed it: the calls through those are direct
/// now, so they are spliced in their turn.
fn splice(
    program: &Program,
    body: &mut Body,
    block: BlockId,
    statement: usize,
    callee: &Body,
) -> Vec<FnId> {
    let Statement::Call { args, dest, .. } =
        body.blocks[block.0 as usize].statements[statement].clone()
    else {
        unreachable!("the caller found a call here");
    };
    // Where the call was written. The callee's own positions become this
    // one, so that debug information and a panic's frames put the caller
    // where it called, as they did before the call was spliced; the
    // callee's blocks follow the caller's, and would otherwise lend their
    // lines to what comes after them.
    let called_at = body.blocks[block.0 as usize].statements[..statement]
        .iter()
        .rev()
        .find_map(|statement| match statement {
            Statement::At(span) => Some(*span),
            _ => None,
        });
    // What comes after the call waits in a block of its own.
    let rest: Vec<Statement> = body.blocks[block.0 as usize]
        .statements
        .drain(statement..)
        .skip(1)
        .collect();
    let after = BlockId(body.blocks.len() as u32);
    let terminator = std::mem::replace(
        &mut body.blocks[block.0 as usize].terminator,
        Terminator::Goto(after),
    );
    let mut rest = rest;
    if let Some(span) = called_at {
        rest.insert(0, Statement::At(span));
    }
    body.blocks.push(Block {
        statements: rest,
        terminator,
    });

    // A closure argument written at the call is a pair of a code address
    // and an environment, and the code address is a constant the caller
    // just wrote down. Finding it is what lets the call through the pair
    // become a call to the lambda.
    let mut handed: FxHashMap<Local, FnId> = FxHashMap::default();
    for (index, arg) in args.iter().enumerate() {
        if let Operand::Copy(pair) = arg
            && pair.projections.is_empty()
            && let Some(lambda) = code_address(body, block, statement, pair.local)
            && let Some(&param) = callee.params.get(index)
        {
            handed.insert(param, lambda);
        }
    }

    // Each of the callee's locals becomes a place in the caller: a
    // parameter is the place the argument named, and everything else is a
    // local of its own.
    let first_block = BlockId(body.blocks.len() as u32);
    let mut places: Vec<Place> = Vec::with_capacity(callee.locals.len());
    let mut prologue: Vec<Statement> = Vec::new();
    for (i, decl) in callee.locals.iter().enumerate() {
        let local = Local(i as u32);
        let place = if let Some(index) = callee.params.iter().position(|&p| p == local) {
            match &args[index] {
                // An aggregate is passed by its address, so the callee
                // reads and writes the caller's own place, as the call
                // did. A scalar is a copy, and stays one: a local that is
                // read as an index or a count must be a local.
                Operand::Copy(place)
                    if crate::is_aggregate(program, decl.ty) && !writes_back(callee, local) =>
                {
                    place.clone()
                }
                operand => {
                    let fresh = fresh_local(body, decl.ty, LocalKind::Temp);
                    prologue.push(Statement::Assign(
                        Place::local(fresh),
                        Rvalue::Use(operand.clone()),
                    ));
                    Place::local(fresh)
                }
            }
        } else if Some(local) == callee.ret {
            match &dest {
                // The result goes where the call's did, unless the caller
                // is also reading that place as an argument.
                Some(place) if !aliases(place, &args) => place.clone(),
                _ => Place::local(fresh_local(body, decl.ty, LocalKind::Temp)),
            }
        } else {
            Place::local(fresh_local(body, decl.ty, decl.kind))
        };
        places.push(place);
    }

    // A call through one of those pairs is a call to the lambda itself.
    let direct: Vec<(Place, FnId)> = handed
        .iter()
        .map(|(&param, &lambda)| {
            let mut code = places[param.0 as usize].clone();
            code.projections.push(Projection::Field(1));
            (code, lambda)
        })
        .collect();

    // The callee's blocks, renumbered, with `return` going to what follows
    // the call.
    for (i, original) in callee.blocks.iter().enumerate() {
        let mut moved = Block {
            statements: original.statements.clone(),
            terminator: original.terminator.clone(),
        };
        // A body reads the code address into a temporary before it calls
        // through it, so what each temporary last held is followed here.
        let mut copies: Vec<(Local, Place)> = Vec::new();
        // With no position for the call, the callee's are dropped, and its
        // instructions keep whatever the caller's last one was.
        if called_at.is_none() {
            moved
                .statements
                .retain(|statement| !matches!(statement, Statement::At(_)));
        }
        for statement in &mut moved.statements {
            if let (Statement::At(span), Some(call)) = (&mut *statement, called_at) {
                *span = call;
            }
            map_statement(statement, &places, first_block);
            make_direct(statement, &direct, &copies);
            follow_copy(statement, &mut copies);
        }
        map_terminator(&mut moved.terminator, &places, first_block, after);
        if i == 0 {
            let mut statements = std::mem::take(&mut prologue);
            if let Some(span) = called_at {
                statements.insert(0, Statement::At(span));
            }
            statements.append(&mut moved.statements);
            moved.statements = statements;
        }
        body.blocks.push(moved);
    }
    body.blocks[block.0 as usize].terminator = Terminator::Goto(first_block);

    let lambdas: Vec<FnId> = handed.into_values().collect();

    // The result, when it could not be written where the call's went.
    if let Some(dest) = dest
        && let Some(ret) = callee.ret
        && places[ret.0 as usize] != dest
    {
        let value = Operand::Copy(places[ret.0 as usize].clone());
        body.blocks[after.0 as usize]
            .statements
            .insert(0, Statement::Assign(dest, Rvalue::Use(value)));
    }
    lambdas
}

/// The lambda a closure pair holds, when the caller wrote its code address
/// down as a constant before the call. A pair built anywhere else — one
/// passed on from elsewhere, or held in a variable — has no constant to
/// find, and the call through it stays a call.
fn code_address(body: &Body, block: BlockId, before: usize, pair: Local) -> Option<FnId> {
    let statements = &body.blocks[block.0 as usize].statements[..before];
    statements.iter().rev().find_map(|statement| {
        let Statement::Assign(place, Rvalue::Use(Operand::Const(Const::Fn { id, .. }))) = statement
        else {
            return None;
        };
        (place.local == pair && place.projections == [Projection::Field(1)]).then_some(*id)
    })
}

/// Turns a call through a closure pair into a call to the lambda it holds.
fn make_direct(statement: &mut Statement, direct: &[(Place, FnId)], copies: &[(Local, Place)]) {
    let Statement::Call {
        callee: callee @ Callee::Value(_),
        ..
    } = statement
    else {
        return;
    };
    let Callee::Value(Operand::Copy(code)) = &*callee else {
        return;
    };
    // What the call names, and what that was a copy of.
    let held = copies
        .iter()
        .rev()
        .find(|(local, _)| *local == code.local && code.projections.is_empty())
        .map(|(_, place)| place);
    let Some(&(_, lambda)) = direct
        .iter()
        .find(|(place, _)| Some(place) == held || place == code)
    else {
        return;
    };
    // The environment stays the first argument: what the lambda captured
    // it still reads through it.
    *callee = Callee::Fn(lambda);
}

/// Remembers `x = y` for a local, and forgets what a local held when it is
/// written any other way.
fn follow_copy(statement: &Statement, copies: &mut Vec<(Local, Place)>) {
    let written = match statement {
        Statement::Assign(place, rvalue) => {
            if place.projections.is_empty()
                && let Rvalue::Use(Operand::Copy(source)) = rvalue
            {
                copies.push((place.local, source.clone()));
                return;
            }
            Some(place.local)
        }
        Statement::SetVariant(place, _) | Statement::Zero(place) | Statement::Poison(place) => {
            Some(place.local)
        }
        Statement::CheckMoved { .. } => None,
        Statement::Arith { dest, .. } => Some(dest.local),
        Statement::Call { dest, .. } | Statement::Atomic { dest, .. } => {
            dest.as_ref().map(|dest| dest.local)
        }
        Statement::Alloc { dest, .. } | Statement::AllocBuffer { dest, .. } => Some(dest.local),
        _ => None,
    };
    if let Some(local) = written {
        copies.retain(|(held, _)| *held != local);
    }
}

/// Whether the callee assigns to one of its own parameters, in which case
/// the caller's place cannot stand in for it.
fn writes_back(callee: &Body, param: Local) -> bool {
    callee.blocks.iter().any(|block| {
        block.statements.iter().any(|statement| match statement {
            Statement::Assign(place, _) | Statement::Zero(place) | Statement::Poison(place) => {
                place.local == param
            }
            Statement::Call { dest, .. } => dest.as_ref().is_some_and(|dest| dest.local == param),
            Statement::Alloc { dest, .. } | Statement::AllocBuffer { dest, .. } => {
                dest.local == param
            }
            _ => false,
        })
    })
}

/// Whether the place the result goes to is one of the arguments, which the
/// callee may still be reading when it writes its result.
fn aliases(dest: &Place, args: &[Operand]) -> bool {
    args.iter().any(|arg| match arg {
        Operand::Copy(place) => place.local == dest.local,
        Operand::Const(_) => false,
    })
}

fn fresh_local(body: &mut Body, ty: Ty, kind: LocalKind) -> Local {
    let local = Local(body.locals.len() as u32);
    // A callee's return local is an ordinary local of the caller.
    let kind = match kind {
        LocalKind::Param | LocalKind::Return => LocalKind::Temp,
        kind => kind,
    };
    // Its name is the callee's; in the caller it is a temporary.
    body.locals.push(LocalDecl::unnamed(ty, kind));
    local
}

/// The caller's place for one of the callee's: what its local became, with
/// the projections it was read through. An index is a local too, and it is
/// the caller's now.
fn map_place(place: &mut Place, places: &[Place]) {
    // The callee's own projections first, since an index among them names
    // one of its locals; the place it was passed may have projections of
    // its own, which are the caller's already.
    for projection in &mut place.projections {
        if let Projection::Index(index) = projection {
            let mapped = &places[index.0 as usize];
            assert!(
                mapped.projections.is_empty(),
                "an index is a scalar, and a scalar argument is copied to a local"
            );
            *index = mapped.local;
        }
    }
    let mapped = &places[place.local.0 as usize];
    place.local = mapped.local;
    let mut projections = mapped.projections.clone();
    projections.append(&mut place.projections);
    place.projections = projections;
}

fn map_operand(operand: &mut Operand, places: &[Place]) {
    if let Operand::Copy(place) = operand {
        map_place(place, places);
    }
}

fn map_rvalue(rvalue: &mut Rvalue, places: &[Place]) {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Unary(_, operand)
        | Rvalue::Cast(operand, _)
        | Rvalue::CstrLen(operand)
        | Rvalue::Float(_, operand)
        | Rvalue::Integer(_, operand)
        | Rvalue::Bits(operand)
        | Rvalue::VTableFn { table: operand, .. } => map_operand(operand, places),
        Rvalue::Binary(_, lhs, rhs) | Rvalue::Rotate(_, lhs, rhs) => {
            map_operand(lhs, places);
            map_operand(rhs, places);
        }
        Rvalue::MulAdd(first, second, third) => {
            map_operand(first, places);
            map_operand(second, places);
            map_operand(third, places);
        }
        Rvalue::AddressOf(place) | Rvalue::Variant(place) => map_place(place, places),
    }
}

fn map_statement(statement: &mut Statement, places: &[Place], first: BlockId) {
    let _ = first;
    match statement {
        Statement::Assign(place, rvalue) => {
            map_place(place, places);
            map_rvalue(rvalue, places);
        }
        Statement::SetVariant(place, _)
        | Statement::Zero(place)
        | Statement::Poison(place)
        | Statement::CheckMoved { place, .. } => map_place(place, places),
        Statement::Arith { dest, lhs, rhs, .. } => {
            map_place(dest, places);
            map_operand(lhs, places);
            map_operand(rhs, places);
        }
        Statement::Call { callee, args, dest } => {
            if let Callee::Value(operand) = callee {
                map_operand(operand, places);
            }
            for arg in args {
                map_operand(arg, places);
            }
            if let Some(dest) = dest {
                map_place(dest, places);
            }
        }
        Statement::Alloc { dest, .. } => map_place(dest, places),
        Statement::AllocBuffer { dest, count, .. } => {
            map_place(dest, places);
            map_operand(count, places);
        }
        Statement::Atomic {
            address,
            value,
            expected,
            dest,
            ..
        } => {
            map_operand(address, places);
            if let Some(value) = value {
                map_operand(value, places);
            }
            if let Some(expected) = expected {
                map_operand(expected, places);
            }
            if let Some(dest) = dest {
                map_place(dest, places);
            }
        }
        Statement::Free(operand)
        | Statement::DropFn { ptr: operand, .. }
        | Statement::DropInPlace { ptr: operand, .. } => map_operand(operand, places),
        Statement::At(_) => {}
        Statement::Check { fails, kind, .. } => {
            map_operand(fails, places);
            match kind {
                CheckKind::Bounds { index, length } => {
                    map_operand(index, places);
                    map_operand(length, places);
                }
                CheckKind::Length { length } => map_operand(length, places),
                CheckKind::Division | CheckKind::Overflow(_) => {}
            }
        }
    }
}

fn map_terminator(terminator: &mut Terminator, places: &[Place], first: BlockId, after: BlockId) {
    let moved = |block: &mut BlockId| *block = BlockId(block.0 + first.0);
    match terminator {
        Terminator::Goto(target) => moved(target),
        Terminator::Branch {
            cond,
            then,
            otherwise,
        } => {
            map_operand(cond, places);
            moved(then);
            moved(otherwise);
        }
        Terminator::Switch {
            value,
            cases,
            otherwise,
        } => {
            map_operand(value, places);
            for (_, target) in cases.iter_mut() {
                moved(target);
            }
            moved(otherwise);
        }
        // What the callee returns to is what follows the call.
        Terminator::Return => *terminator = Terminator::Goto(after),
        Terminator::Panic { note, .. } => {
            if let Some(note) = note {
                map_operand(note, places);
            }
        }
        Terminator::Unreachable => {}
    }
}
