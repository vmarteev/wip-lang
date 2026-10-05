//! Checks a body's structure. A malformed body is a bug in
//! the builder, so a problem panics with a description of it.

use crate::*;

/// Checks that every local and block `body` names exists, that its
/// parameters and result are locals of the right kind, and that no switch
/// lists a case twice.
pub fn validate(body: &Body) {
    let locals = body.locals.len();
    let blocks = body.blocks.len();
    let local = |local: Local, what: &str| {
        assert!(
            (local.0 as usize) < locals,
            "{what} names local _{} of {locals}",
            local.0
        );
    };
    let place = |place: &Place| {
        local(place.local, "a place");
        for projection in &place.projections {
            if let Projection::Index(index) = projection {
                local(*index, "an index");
            }
        }
    };
    let operand = |operand: &Operand| {
        if let Operand::Copy(p) = operand {
            place(p);
        }
    };
    let target = |block: BlockId| {
        assert!(
            (block.0 as usize) < blocks && block.0 != 0,
            "a jump to block {} of {blocks}, or to the entry",
            block.0
        );
    };

    for &param in &body.params {
        local(param, "a parameter");
        assert_eq!(body.local(param).kind, LocalKind::Param);
    }
    if let Some(ret) = body.ret {
        local(ret, "the result");
        assert_eq!(body.local(ret).kind, LocalKind::Return);
    }
    for block in &body.blocks {
        for statement in &block.statements {
            match statement {
                Statement::Assign(dest, rvalue) => {
                    place(dest);
                    match rvalue {
                        Rvalue::Use(o)
                        | Rvalue::Unary(_, o)
                        | Rvalue::Cast(o, _)
                        | Rvalue::CstrLen(o)
                        | Rvalue::Float(_, o)
                        | Rvalue::Integer(_, o)
                        | Rvalue::Bits(o) => operand(o),
                        Rvalue::Binary(_, l, r) | Rvalue::Rotate(_, l, r) => {
                            operand(l);
                            operand(r);
                        }
                        Rvalue::MulAdd(first, second, third) => {
                            operand(first);
                            operand(second);
                            operand(third);
                        }
                        Rvalue::AddressOf(p) | Rvalue::Variant(p) => place(p),
                        Rvalue::VTableFn { table, .. } => operand(table),
                    }
                }
                Statement::SetVariant(p, _)
                | Statement::Zero(p)
                | Statement::Poison(p)
                | Statement::CheckMoved { place: p, .. } => place(p),
                Statement::Arith { dest, lhs, rhs, .. } => {
                    place(dest);
                    operand(lhs);
                    operand(rhs);
                }
                Statement::Call { callee, args, dest } => {
                    if let Callee::Value(callee) = callee {
                        operand(callee);
                    }
                    args.iter().for_each(operand);
                    if let Some(dest) = dest {
                        place(dest);
                    }
                }
                Statement::Alloc { dest, .. } => place(dest),
                Statement::AllocBuffer { dest, count, .. } => {
                    place(dest);
                    operand(count);
                }
                Statement::Atomic {
                    address,
                    value,
                    expected,
                    dest,
                    ..
                } => {
                    operand(address);
                    if let Some(value) = value {
                        operand(value);
                    }
                    if let Some(expected) = expected {
                        operand(expected);
                    }
                    if let Some(dest) = dest {
                        place(dest);
                    }
                }
                Statement::Free(o)
                | Statement::DropFn { ptr: o, .. }
                | Statement::DropInPlace { ptr: o, .. } => operand(o),
                Statement::Check { fails, .. } => operand(fails),
                Statement::At(_) => {}
            }
        }
        match &block.terminator {
            Terminator::Goto(block) => target(*block),
            Terminator::Panic { .. } => {}
            Terminator::Branch {
                cond,
                then,
                otherwise,
            } => {
                operand(cond);
                target(*then);
                target(*otherwise);
            }
            Terminator::Switch {
                value,
                cases,
                otherwise,
            } => {
                operand(value);
                for (i, &(case, block)) in cases.iter().enumerate() {
                    assert!(
                        cases[..i].iter().all(|&(earlier, _)| earlier != case),
                        "a switch lists case {case} twice"
                    );
                    target(block);
                }
                target(*otherwise);
            }
            Terminator::Return | Terminator::Unreachable => {}
        }
    }
}
