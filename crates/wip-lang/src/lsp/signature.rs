//! A call's signature as its arguments are written.

use serde_json::{Value, json};
use wip_hir::{Program, Receiver, Types};

use super::navigate::{self, Target};
use crate::Loaded;

/// A call the cursor is in the arguments of, as the text says.
#[derive(Debug, PartialEq)]
pub struct Call {
    /// The byte offset where the called name ends: its `(`.
    pub paren: usize,
    /// Which argument the cursor is at, from zero.
    pub argument: usize,
    /// Called as a method, `value.name(…)`, whose receiver is not an
    /// argument.
    pub method: bool,
}

/// The call whose arguments hold `cursor`: the `(` before it that is not
/// closed, after a name. The arguments before the cursor are counted by
/// their commas, not those inside brackets or strings of their own.
pub fn call_at(text: &str, cursor: usize) -> Option<Call> {
    let before = text.get(..cursor)?;
    let mut depth = 0usize;
    let mut argument = 0;
    let mut in_string = false;
    let bytes = before.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        let c = bytes[i];
        if in_string {
            if c == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b')' | b']' | b'}' => depth += 1,
            b'[' | b'{' if depth == 0 => return None,
            b'(' | b'[' | b'{' if depth > 0 => depth -= 1,
            b'(' => {
                let name = before[..i].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
                if name.len() == i {
                    // `(` after no name: a tuple or a grouping.
                    return None;
                }
                let method = name.ends_with('.');
                return Some(Call {
                    paren: i,
                    argument,
                    method,
                });
            }
            b',' if depth == 0 => argument += 1,
            _ => {}
        }
    }
    None
}

/// The signature help for `call`, from a program checked with the file
/// at `base`: `None` where the name before the `(` is not a function.
pub fn help(loaded: &Loaded, program: &Program, base: u32, call: &Call) -> Option<Value> {
    let hit = navigate::at(program, &loaded.interner, base + call.paren as u32 - 1)?;
    let Target::Fn(id) = hit.target else {
        return None;
    };
    let def = &program.fns[id];
    let interner = &loaded.interner;
    let takes_receiver = !matches!(def.receiver, None | Some(Receiver::Static));
    let skip = usize::from(call.method && takes_receiver);
    let params: Vec<String> = def
        .params
        .iter()
        .skip(skip)
        .map(|p| {
            format!(
                "{}: {}",
                interner.resolve(p.name),
                program.ty_name(p.ty, interner)
            )
        })
        .collect();
    let mut label = format!("{}({})", interner.resolve(def.name), params.join(", "));
    if def.ret != Types::UNIT {
        label.push_str(&format!(": {}", program.ty_name(def.ret, interner)));
    }
    let docs = loaded
        .sources
        .of(def.name_span)
        .map(|(_, text, base)| navigate::documentation(text, (def.name_span.lo - base) as usize))
        .unwrap_or_default();
    let mut signature = json!({
        "label": label,
        "parameters": params.iter().map(|p| json!({ "label": p })).collect::<Vec<_>>(),
    });
    if !docs.is_empty() {
        signature["documentation"] = json!({ "kind": "markdown", "value": docs });
    }
    Some(json!({
        "signatures": [signature],
        "activeSignature": 0,
        "activeParameter": call.argument.min(params.len().saturating_sub(1)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_call_and_the_argument_are_read_from_before_the_cursor() {
        let at_end = |text: &str| call_at(text, text.len());
        assert_eq!(
            at_end("\tval a = area(1, g(2, 3), "),
            Some(Call {
                paren: 13,
                argument: 2,
                method: false
            })
        );
        assert_eq!(
            at_end("\tball.step("),
            Some(Call {
                paren: 10,
                argument: 0,
                method: true
            })
        );
        assert_eq!(
            at_end("\tf(\"a, b\", "),
            Some(Call {
                paren: 2,
                argument: 1,
                method: false
            })
        );
        assert_eq!(at_end("\tval t = (1, "), None);
        assert_eq!(at_end("\tf(Point { x: 1, "), None);
    }
}
