//! A function C calls with a struct by value, or that answers C one:
//! C's side is a function of its own with the signature C's ABI gives it, which
//! puts the structs back together from what C passed, calls the function as Wip
//! calls it, and answers as C's ABI says. LLVM inlines the one into the other.

use std::fmt::Write as _;

use wip_hir::{FnId, Ty};
use wip_mir::c_abi::{Answer, CCall, Part, Pass, Reg};

use crate::{Llvm, parts_type, reg_lt};

/// The text of C's side of `id`, whose structs travel as `c_call` says,
/// named `symbol` and calling `body`, the function as Wip calls it.
pub(crate) fn define(
    m: &mut Llvm<'_>,
    id: FnId,
    symbol: &str,
    body: &str,
    c_call: &CCall,
) -> String {
    let def = &m.program.fns[id];
    let params: Vec<Ty> = def.params.iter().map(|p| p.ty).collect();
    let ret = def.ret;
    let c_sig = m.c_signature(id, c_call);
    let wip_sig = m.signature_with(&params, ret, true);
    let name = m.program.fn_name(id, m.interner);
    let subprogram = m.debug.subprogram(&name, None);
    let place = m.debug.no_line(subprogram);

    let mut out = String::new();
    let mut next = 0;
    let mut temp = || {
        next += 1;
        format!("%w{next}")
    };
    // C's arguments, in order, as `%p0`, `%p1`, …
    let mut incoming = 0usize;
    let mut take = || {
        incoming += 1;
        format!("%p{}", incoming - 1)
    };
    let mut args: Vec<String> = Vec::new();
    // Where the struct the function answers is written: C's address, or a
    // slot its pieces are read from.
    let mut result = None;
    if c_call.ret == Answer::Hidden {
        let address = take();
        args.push(address);
    } else if let Answer::Parts(parts) = &c_call.ret {
        let slot = temp();
        let size = reach(m.layout(ret).size, parts);
        let _ = writeln!(out, "  {slot} = alloca [{size} x i8], align 8");
        args.push(slot.clone());
        result = Some(slot);
    }
    for (&ty, pass) in params.iter().zip(&c_call.params) {
        match pass {
            Pass::Plain if m.is_c_pair(ty) => {
                let (pointer, length) = (take(), take());
                args.push(pointer);
                args.push(length);
            }
            Pass::Plain => {
                args.push(take());
            }
            // A struct in pieces: stored to a slot as large as all of them,
            // whose address the function takes.
            Pass::Parts(parts) | Pass::Spilled { parts, .. } => {
                if let Pass::Spilled { pad, .. } = pass {
                    for _ in 0..pad.ints + pad.floats {
                        take();
                    }
                }
                let slot = temp();
                let size = reach(m.layout(ty).size, parts);
                let _ = writeln!(out, "  {slot} = alloca [{size} x i8], align 8");
                for part in parts {
                    let piece = take();
                    let at = temp();
                    let _ = writeln!(
                        out,
                        "  {at} = getelementptr inbounds i8, ptr {slot}, i64 {}",
                        part.offset
                    );
                    let _ = writeln!(
                        out,
                        "  store {} {piece}, ptr {at}, align 1",
                        reg_lt(part.reg).text()
                    );
                }
                args.push(slot);
            }
            // C's copy, or the bytes C put on the stack: the struct's own
            // address, which the function takes as Wip passes a struct.
            Pass::Copy | Pass::Stack(_) => {
                let address = take();
                args.push(address);
            }
        }
    }
    // As the function takes them, which says what C's ABI asks of each.
    let typed: Vec<String> = args
        .iter()
        .zip(&wip_sig.params)
        .map(|(value, slot)| format!("{} {value}", slot.text()))
        .collect();
    let call = format!("call {} {body}({})", wip_sig.ret_text(), typed.join(", "));
    match &c_call.ret {
        Answer::Plain => match &wip_sig.ret {
            Some(slot) => {
                let answer = temp();
                let _ = writeln!(out, "  {answer} = {call}");
                let _ = writeln!(out, "  ret {} {answer}", slot.lt.text());
            }
            None => {
                let _ = writeln!(out, "  {call}");
                out.push_str("  ret void\n");
            }
        },
        Answer::Hidden => {
            let _ = writeln!(out, "  {call}");
            out.push_str("  ret void\n");
        }
        // The pieces, read from where the function wrote the struct.
        Answer::Parts(parts) => {
            let _ = writeln!(out, "  {call}");
            let slot = result.expect("a slot for the struct answered");
            let ty = parts_type(parts);
            let mut both = "undef".to_string();
            for (i, part) in parts.iter().enumerate() {
                let at = temp();
                let _ = writeln!(
                    out,
                    "  {at} = getelementptr inbounds i8, ptr {slot}, i64 {}",
                    part.offset
                );
                let piece = temp();
                let lt = reg_lt(part.reg).text();
                let _ = writeln!(out, "  {piece} = load {lt}, ptr {at}, align 1");
                let next = temp();
                let _ = writeln!(out, "  {next} = insertvalue {ty} {both}, {lt} {piece}, {i}");
                both = next;
            }
            let _ = writeln!(out, "  ret {ty} {both}");
        }
    }

    let params: Vec<String> = c_sig
        .params
        .iter()
        .enumerate()
        .map(|(i, slot)| format!("{} %p{i}", slot.text()))
        .collect();
    let mut text = format!(
        "define {} {symbol}({}) #0 !dbg !{} {{\nstart:\n",
        c_sig.ret_text(),
        params.join(", "),
        subprogram.node
    );
    for line in out.lines() {
        let _ = writeln!(text, "{line}, !dbg !{place}");
    }
    text.push_str("}\n\n");
    text
}

/// The bytes a struct's slot takes: the struct's, or as far as its pieces
/// reach, which may be past them.
fn reach(size: u32, parts: &[Part]) -> u32 {
    let end = parts
        .iter()
        .map(|part| {
            part.offset
                + match part.reg {
                    Reg::F32 => 4,
                    _ => 8,
                }
        })
        .max()
        .unwrap_or(0);
    size.max(end).max(1)
}
