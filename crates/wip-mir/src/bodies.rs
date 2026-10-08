//! A function's body as every consumer of the MIR takes it: lowered, with
//! each `@inline` call spliced in, and its places made to
//! name what they stand for — the code a backend compiles, and the code
//! the compiler runs for a constant, which are one.

use wip_hir::{FnId, Generated, Program, TyList, TypeDef};
use wip_syntax::Interner;

use crate::build::{lower_all_variants_fn, lower_count_fn, lower_fn, lower_from_index_fn};
use crate::{Body, convert_c_bools, inline, simplify, validate};

/// The body of function `id`: the program's, or the one the compiler writes
/// for it; `None` for a function with neither, as C's are.
pub fn function_body(program: &Program, interner: &Interner, id: FnId) -> Option<Body> {
    let def = &program.fns[id];
    let mut body = match &def.body {
        Some(hir) => {
            let mut body = lower_fn(program, interner, def, hir);
            inline(program, interner, &mut body);
            convert_c_bools(program, id, &mut body);
            body
        }
        None => generated_body(program, interner, id)?,
    };
    simplify(program, &mut body);
    if cfg!(debug_assertions) {
        validate(&body);
    }
    Some(body)
}

/// The body the compiler writes for a method a type is given: an enum's
/// `count`, `fromIndex` and `all`.
fn generated_body(program: &Program, interner: &Interner, id: FnId) -> Option<Body> {
    let def = &program.fns[id];
    let generated = def.generated?;
    let owner = program
        .owner_of_fn(id)
        .expect("a generated method belongs to a type");
    let args = def.instance_of.map_or(TyList::EMPTY, |(_, a)| a);
    // `count` answers a number and never names its own type, so the program
    // may never have made that type; everything else here is written from
    // it.
    let ty = program.type_of(owner, args);
    let of_type = |ty: Option<wip_hir::Ty>| {
        ty.expect("a type with a generated method is one the program has")
    };
    Some(match generated {
        // What an enum whose variants carry nothing gets.
        Generated::VariantCount => {
            let TypeDef::Enum(enum_id) = owner else {
                unreachable!("only an enum counts its variants")
            };
            let count = program.enums[enum_id].variants.len() as u64;
            lower_count_fn(program, interner, count)
        }
        Generated::FromIndex => lower_from_index_fn(program, interner, of_type(ty), def.ret),
        Generated::AllVariants => lower_all_variants_fn(program, interner, of_type(ty), def.ret),
    })
}
