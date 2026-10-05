//! Which locals a body may read before anything writes them: a forward
//! analysis of what is written on every path to each point.
//!
//! A write to any part of a local counts as writing it — a field assigned,
//! a variant set, its address taken for a call to fill — so what this
//! answers is a local nothing on some path writes at all before a read.
//! Cranelift's code reads such a local as zero, and LLVM's as anything,
//! so a body that has one says nothing definite.

use crate::*;

/// The locals `body` may read before it writes them, each with the block
/// where it is read, once each.
pub fn reads_before_writes(body: &Body) -> Vec<(Local, BlockId)> {
    let count = body.locals.len();
    let blocks = body.blocks.len();
    // What is written on every path into each block; `None` where no path
    // has been followed yet.
    let mut entry: Vec<Option<Vec<bool>>> = vec![None; blocks];
    let mut start = vec![false; count];
    for &param in &body.params {
        start[param.0 as usize] = true;
    }
    entry[0] = Some(start);
    let mut changed = true;
    while changed {
        changed = false;
        for b in 0..blocks {
            let Some(written) = entry[b].clone() else {
                continue;
            };
            let mut written = written;
            for statement in &body.blocks[b].statements {
                statement_effect(statement, &mut written, &mut |_| {});
            }
            for next in body.blocks[b].terminator.successors() {
                let slot = &mut entry[next.0 as usize];
                match slot {
                    None => {
                        *slot = Some(written.clone());
                        changed = true;
                    }
                    Some(old) => {
                        for (o, n) in old.iter_mut().zip(&written) {
                            if *o && !*n {
                                *o = false;
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut found: Vec<(Local, BlockId)> = Vec::new();
    for (b, block) in body.blocks.iter().enumerate() {
        let Some(mut written) = entry[b].clone() else {
            continue;
        };
        let at = BlockId(b as u32);
        let report = |local: Local, written: &[bool], found: &mut Vec<(Local, BlockId)>| {
            if !written[local.0 as usize] && !found.iter().any(|(l, _)| *l == local) {
                found.push((local, at));
            }
        };
        for statement in &block.statements {
            let before = written.clone();
            let mut reads = Vec::new();
            statement_effect(statement, &mut written, &mut |local| reads.push(local));
            for local in reads {
                report(local, &before, &mut found);
            }
        }
        let mut reads = Vec::new();
        terminator_reads(body, &block.terminator, &mut |local| reads.push(local));
        for local in reads {
            report(local, &written, &mut found);
        }
    }
    found
}

/// The locals reading `place` reads: its own, and those its indices name.
fn place_reads(place: &Place, read: &mut impl FnMut(Local)) {
    read(place.local);
    index_reads(place, read);
}

fn index_reads(place: &Place, read: &mut impl FnMut(Local)) {
    for projection in &place.projections {
        if let Projection::Index(index) = projection {
            read(*index);
        }
    }
}

fn operand_reads(operand: &Operand, read: &mut impl FnMut(Local)) {
    if let Operand::Copy(place) = operand {
        place_reads(place, read);
    }
}

/// Writing `place`: through a pointer it holds, a read of the local; into
/// the local itself, or a part of it, a write.
fn place_write(place: &Place, written: &mut [bool], read: &mut impl FnMut(Local)) {
    index_reads(place, read);
    if place.projections.contains(&Projection::Deref) {
        read(place.local);
    } else {
        written[place.local.0 as usize] = true;
    }
}

/// What a statement reads, given to `read`, and what it writes, marked.
fn statement_effect(statement: &Statement, written: &mut [bool], read: &mut impl FnMut(Local)) {
    match statement {
        Statement::Assign(dest, rvalue) => {
            match rvalue {
                Rvalue::Use(o)
                | Rvalue::Unary(_, o)
                | Rvalue::Cast(o, _)
                | Rvalue::CstrLen(o)
                | Rvalue::Float(_, o)
                | Rvalue::Integer(_, o)
                | Rvalue::Bits(o) => operand_reads(o, read),
                Rvalue::Binary(_, a, b) | Rvalue::Rotate(_, a, b) => {
                    operand_reads(a, read);
                    operand_reads(b, read);
                }
                Rvalue::MulAdd(a, b, c) => {
                    operand_reads(a, read);
                    operand_reads(b, read);
                    operand_reads(c, read);
                }
                Rvalue::VTableFn { table, .. } => operand_reads(table, read),
                Rvalue::Variant(place) => place_reads(place, read),
                // An address taken may be where something is written, so
                // the local counts as written from here.
                Rvalue::AddressOf(place) => {
                    index_reads(place, read);
                    if place.projections.contains(&Projection::Deref) {
                        read(place.local);
                    } else {
                        written[place.local.0 as usize] = true;
                    }
                }
            }
            place_write(dest, written, read);
        }
        Statement::SetVariant(place, _) | Statement::Zero(place) | Statement::Poison(place) => {
            place_write(place, written, read);
        }
        Statement::CheckMoved { place, .. } => place_reads(place, read),
        Statement::Call { callee, args, dest } => {
            if let Callee::Value(value) = callee {
                operand_reads(value, read);
            }
            for arg in args {
                operand_reads(arg, read);
            }
            if let Some(dest) = dest {
                place_write(dest, written, read);
            }
        }
        Statement::Alloc { dest, .. } => place_write(dest, written, read),
        Statement::AllocBuffer { dest, count, .. } => {
            operand_reads(count, read);
            place_write(dest, written, read);
        }
        Statement::Free(o) => operand_reads(o, read),
        Statement::DropFn { ptr, .. } | Statement::DropInPlace { ptr, .. } => {
            operand_reads(ptr, read);
        }
        Statement::Arith { dest, lhs, rhs, .. } => {
            operand_reads(lhs, read);
            operand_reads(rhs, read);
            place_write(dest, written, read);
        }
        Statement::Atomic {
            address,
            value,
            expected,
            dest,
            ..
        } => {
            operand_reads(address, read);
            for o in value.iter().chain(expected) {
                operand_reads(o, read);
            }
            if let Some(dest) = dest {
                place_write(dest, written, read);
            }
        }
        Statement::Check { fails, kind, .. } => {
            operand_reads(fails, read);
            match kind {
                CheckKind::Bounds { index, length } => {
                    operand_reads(index, read);
                    operand_reads(length, read);
                }
                CheckKind::Length { length } => operand_reads(length, read),
                CheckKind::Division | CheckKind::Overflow(_) => {}
            }
        }
        Statement::At(_) => {}
    }
}

fn terminator_reads(body: &Body, terminator: &Terminator, read: &mut impl FnMut(Local)) {
    match terminator {
        Terminator::Branch { cond, .. } => operand_reads(cond, read),
        Terminator::Switch { value, .. } => operand_reads(value, read),
        Terminator::Return => {
            if let Some(ret) = body.ret {
                read(ret);
            }
        }
        Terminator::Panic { note, .. } => {
            if let Some(note) = note {
                operand_reads(note, read);
            }
        }
        Terminator::Goto(_) | Terminator::Unreachable => {}
    }
}
