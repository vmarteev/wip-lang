//! MIR as text, one statement per line, for `wip mir` and for tests.

use std::fmt::Write;

use wip_hir::Program;
use wip_syntax::Interner;

use crate::*;

/// The text of a body named `name`.
pub fn dump(program: &Program, interner: &Interner, name: &str, body: &Body) -> String {
    let ty = |ty: Ty| program.ty_name(ty, interner);
    let mut out = String::new();
    let params: Vec<String> = body
        .params
        .iter()
        .map(|&p| format!("_{}: {}", p.0, ty(body.local(p).ty)))
        .collect();
    let ret = match body.ret {
        Some(r) => format!(" -> _{}: {}", r.0, ty(body.local(r).ty)),
        None => String::new(),
    };
    writeln!(out, "fn {name}({}){ret} {{", params.join(", ")).unwrap();
    for (i, decl) in body.locals.iter().enumerate() {
        if matches!(decl.kind, LocalKind::Var | LocalKind::Temp) {
            let kind = if decl.kind == LocalKind::Var {
                "var"
            } else {
                "temp"
            };
            writeln!(out, "    {kind} _{i}: {}", ty(decl.ty)).unwrap();
        }
    }
    for (i, block) in body.blocks.iter().enumerate() {
        writeln!(out, "  bb{i}:").unwrap();
        // Where code was written is for debug information, and would
        // bury what the code does.
        for statement in &block.statements {
            if matches!(statement, Statement::At(_)) {
                continue;
            }
            writeln!(out, "    {}", statement_text(program, interner, statement)).unwrap();
        }
        writeln!(out, "    {}", terminator_text(&block.terminator)).unwrap();
    }
    out.push_str("}\n");
    out
}

fn place_text(place: &Place) -> String {
    let mut text = format!("_{}", place.local.0);
    for projection in &place.projections {
        text = match projection {
            Projection::Deref => format!("(*{text})"),
            Projection::Field(i) => format!("{text}.{i}"),
            Projection::VariantField { variant, field } => {
                format!("({text} as {variant}).{field}")
            }
            Projection::Index(local) => format!("{text}[_{}]", local.0),
            Projection::ConstIndex(i) => format!("{text}[{i}]"),
        };
    }
    text
}

fn operand_text(program: &Program, interner: &Interner, operand: &Operand) -> String {
    match operand {
        Operand::Copy(place) => place_text(place),
        Operand::Const(Const::Int { bits, ty }) => {
            format!("{bits}_{}", program.ty_name(*ty, interner))
        }
        Operand::Const(Const::Float { value, ty }) => {
            format!("{value:?}_{}", program.ty_name(*ty, interner))
        }
        Operand::Const(Const::Bool(value)) => value.to_string(),
        Operand::Const(Const::CStr(sym)) => format!("c{:?}", interner.resolve(*sym)),
        Operand::Const(Const::Fn { id, .. }) => format!("fn {}", program.fn_name(*id, interner)),
        Operand::Const(Const::VTable { interface, ty }) => format!(
            "vtable {} for {}",
            interner.resolve(program.interfaces[*interface].name),
            program.ty_name(*ty, interner)
        ),
        Operand::Const(Const::DropFn(ty)) => {
            format!("drop_fn {}", program.ty_name(*ty, interner))
        }
        Operand::Const(Const::RuntimeWords) => "runtime_words".to_string(),
        Operand::Const(Const::FrameTables) => "frame_tables".to_string(),
        Operand::Const(Const::SizeOf { of, .. }) => {
            format!("size_of {}", program.ty_name(*of, interner))
        }
        Operand::Const(Const::AlignOf { of, .. }) => {
            format!("align_of {}", program.ty_name(*of, interner))
        }
        Operand::Const(Const::Table { id, .. }) => {
            format!("table {}", interner.resolve(program.consts[*id].name))
        }
    }
}

fn statement_text(program: &Program, interner: &Interner, statement: &Statement) -> String {
    let operand = |o: &Operand| operand_text(program, interner, o);
    let ty = |ty: Ty| program.ty_name(ty, interner);
    match statement {
        Statement::Assign(dest, rvalue) => {
            let value = match rvalue {
                Rvalue::Use(o) => operand(o),
                Rvalue::Unary(op, o) => {
                    let op = match op {
                        UnaryOp::Neg => "-",
                        UnaryOp::Not => "!",
                    };
                    format!("{op}{}", operand(o))
                }
                Rvalue::Binary(op, l, r) => format!("{} {} {}", operand(l), op.text(), operand(r)),
                Rvalue::Cast(o, to) => format!("{} as {}", operand(o), ty(*to)),
                Rvalue::AddressOf(p) => format!("&{}", place_text(p)),
                Rvalue::Variant(p) => format!("variant({})", place_text(p)),
                Rvalue::CstrLen(o) => format!("cstr_len({})", operand(o)),
                Rvalue::Float(op, o) => format!("{op:?}({})", operand(o)).to_lowercase(),
                Rvalue::MulAdd(first, second, third) => {
                    format!(
                        "muladd({}, {}, {})",
                        operand(first),
                        operand(second),
                        operand(third)
                    )
                }
                Rvalue::Integer(op, o) => format!("{op:?}({})", operand(o)).to_lowercase(),
                Rvalue::Rotate(turn, value, amount) => {
                    format!("rotate{turn:?}({}, {})", operand(value), operand(amount))
                        .to_lowercase()
                }
                Rvalue::Overflows(op, l, r) => {
                    format!("overflows({} {op:?} {})", operand(l), operand(r)).to_lowercase()
                }
                Rvalue::Bits(o) => format!("bits({})", operand(o)),
                Rvalue::VTableFn { table, index, .. } => {
                    format!("method {index} of {}", operand(table))
                }
            };
            format!("{} = {value}", place_text(dest))
        }
        Statement::SetVariant(p, variant) => format!("set_variant({}, {variant})", place_text(p)),
        Statement::Zero(p) => format!("zero({})", place_text(p)),
        Statement::Poison(p) => format!("poison({})", place_text(p)),
        Statement::CheckMoved { place, .. } => format!("check_moved({})", place_text(place)),
        Statement::At(span) => format!("at {}", span.lo),
        Statement::Call { callee, args, dest } => {
            let args: Vec<String> = args.iter().map(operand).collect();
            let name = match callee {
                Callee::Fn(id) => program.fn_name(*id, interner),
                Callee::Value(callee) => format!("({})", operand(callee)),
                Callee::StrCmp => "str_cmp".to_string(),
            };
            let call = format!("call {name}({})", args.join(", "));
            match dest {
                Some(dest) => format!("{} = {call}", place_text(dest)),
                None => call,
            }
        }
        Statement::Alloc { dest, ty: t } => format!("{} = alloc {}", place_text(dest), ty(*t)),
        Statement::AllocBuffer { dest, elem, count } => format!(
            "{} = alloc_buffer {}[{}]",
            place_text(dest),
            ty(*elem),
            operand(count)
        ),
        // `+`, `-` and `*` on integers, which panic if the answer does not
        // fit.
        Statement::Arith {
            dest, op, lhs, rhs, ..
        } => format!(
            "{} = {} {}? {}",
            place_text(dest),
            operand(lhs),
            op.text(),
            operand(rhs)
        ),
        Statement::Free(o) => format!("free({})", operand(o)),
        Statement::Atomic {
            op,
            ty: t,
            address,
            value,
            expected,
            dest,
        } => {
            let mut args = vec![operand(address)];
            args.extend(expected.iter().map(&operand));
            args.extend(value.iter().map(&operand));
            let call = format!("atomic_{op:?}<{}>({})", ty(*t), args.join(", "));
            match dest {
                Some(dest) => format!("{} = {call}", place_text(dest)),
                None => call,
            }
        }
        Statement::DropFn { ty: t, ptr } => format!("drop_fn<{}>({})", ty(*t), operand(ptr)),
        Statement::DropInPlace { ty: t, ptr } => {
            format!("drop_in_place<{}>({})", ty(*t), operand(ptr))
        }
        Statement::Check { fails, kind, .. } => {
            let what = match kind {
                CheckKind::Bounds { .. } => "Bounds",
                CheckKind::Length { .. } => "Length",
                CheckKind::Division => "Division",
                CheckKind::Overflow(_) => "Overflow",
            };
            format!("check {} ({what})", operand(fails))
        }
    }
}

fn terminator_text(terminator: &Terminator) -> String {
    let operand = |o: &Operand| match o {
        Operand::Copy(place) => place_text(place),
        Operand::Const(c) => format!("{c:?}"),
    };
    match terminator {
        Terminator::Goto(block) => format!("goto bb{}", block.0),
        Terminator::Panic { note: None, .. } => "panic".to_string(),
        Terminator::Panic {
            note: Some(note), ..
        } => format!("panic {}", operand(note)),
        Terminator::Branch {
            cond,
            then,
            otherwise,
        } => format!(
            "branch {} ? bb{} : bb{}",
            operand(cond),
            then.0,
            otherwise.0
        ),
        Terminator::Switch {
            value,
            cases,
            otherwise,
        } => {
            let cases: Vec<String> = cases
                .iter()
                .map(|(case, block)| format!("{case}: bb{}", block.0))
                .collect();
            format!(
                "switch {} [{}] else bb{}",
                operand(value),
                cases.join(", "),
                otherwise.0
            )
        }
        Terminator::Return => "return".to_string(),
        Terminator::Unreachable => "unreachable".to_string(),
    }
}
