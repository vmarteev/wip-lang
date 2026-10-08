//! Small aggregates as scalars.
//!
//! A struct, a tuple or an enum lives in a stack slot, written and read
//! field by field through memory, even where it is only ever taken apart:
//! the `Option<i64>` an inlined lookup answers, the pair of numbers a
//! helper hands back. Cranelift keeps a value in a register only where the
//! MIR made it a local of its own, so this pass makes one: each field of
//! such a local becomes a local, and an enum's tag one more. A field that is
//! an aggregate itself becomes a local that a later round may split again.
//!
//! A local is split only where nothing needs it whole in memory: its
//! address is never taken, it is not handed to a call or taken back from
//! one, and it owns nothing that would be dropped through its address. A
//! struct may be copied whole, to or from another place, which becomes a
//! copy of each field; an enum may not, since writing it back would write
//! its tag from a value rather than a constant. What it does is otherwise
//! exactly what it did: the same fields, written and read in the same
//! order.
//!
//! It runs on a release build's bodies alone. A debug build keeps each
//! variable whole, where the debugger looks for it.
use rustc_hash::FxHashMap;
use wip_hir::{Program, Ty, TyKind, Types};

use crate::*;

/// The most fields a local may have to be split: a value larger than that
/// is cheaper copied as the block of memory it is.
const MOST_FIELDS: usize = 8;

/// How many times a split is tried again, for the aggregates the last one
/// made locals of.
const ROUNDS: usize = 4;

/// Splits the small aggregate locals of `body` into one local per field.
pub fn scalars(program: &Program, body: &mut Body) {
    for _ in 0..ROUNDS {
        let splits = candidates(program, body);
        if splits.is_empty() {
            return;
        }
        let parts = split_locals(program, body, &splits);
        for block in &mut body.blocks {
            let statements = std::mem::take(&mut block.statements);
            for statement in statements {
                expand(statement, &parts, &mut block.statements);
            }
            for statement in &mut block.statements {
                remap_statement(statement, &parts);
            }
            remap_terminator(&mut block.terminator, &parts);
        }
    }
}

/// What a split local became.
enum Parts {
    /// A struct's: a local for each field.
    Fields(Vec<Local>),
    /// An enum's: a local for its tag, an `i32`, and one for each field of
    /// each variant.
    Variants { tag: Local, fields: Vec<Vec<Local>> },
}

impl Parts {
    fn place(&self, projection: Projection) -> Option<Local> {
        match (self, projection) {
            (Parts::Fields(fields), Projection::Field(i)) => fields.get(i as usize).copied(),
            (Parts::Variants { fields, .. }, Projection::VariantField { variant, field }) => fields
                .get(variant as usize)
                .and_then(|variant| variant.get(field as usize))
                .copied(),
            _ => None,
        }
    }
}

/// What kind of aggregate a local is, where it is one that may be split.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Struct,
    Enum,
}

fn shape(program: &Program, ty: Ty) -> Option<Shape> {
    if program.owns_memory(ty) {
        return None;
    }
    match program.types.kind(ty) {
        TyKind::Struct(id, _) => {
            let def = &program.structs[id];
            // A union's fields share their bytes, an opaque struct's are
            // C's, a generator's frame has fields beyond its declared ones,
            // and an `@intrinsic` type is reached only through its address.
            let plain = !def.is_union
                && !def.is_opaque
                && !def.is_intrinsic
                && !def.is_env
                && def.generator.is_none();
            let fields = program.field_tys(ty).len();
            (plain && fields <= MOST_FIELDS).then_some(Shape::Struct)
        }
        TyKind::Enum(..) => {
            let fields: usize = program.variant_field_tys(ty).iter().map(Vec::len).sum();
            (fields <= MOST_FIELDS).then_some(Shape::Enum)
        }
        _ => None,
    }
}

/// The locals of `body` that may be split, with what kind each is.
fn candidates(program: &Program, body: &Body) -> FxHashMap<Local, Shape> {
    let mut found: FxHashMap<Local, Shape> = FxHashMap::default();
    for (i, decl) in body.locals.iter().enumerate() {
        if !matches!(decl.kind, LocalKind::Var | LocalKind::Temp) {
            continue;
        }
        if let Some(shape) = shape(program, decl.ty) {
            found.insert(Local(i as u32), shape);
        }
    }
    // Every place a candidate is named, which may rule it out.
    let mut ruled_out: Vec<Local> = Vec::new();
    let see = |place: &Place, whole_allowed: bool, ruled_out: &mut Vec<Local>| {
        if !found.contains_key(&place.local) {
            return;
        }
        let shape = found[&place.local];
        let fits = match place.projections.first() {
            None => whole_allowed,
            Some(Projection::Field(_)) => shape == Shape::Struct,
            Some(Projection::VariantField { .. }) => shape == Shape::Enum,
            Some(_) => false,
        };
        if !fits {
            ruled_out.push(place.local);
        }
        // An index into a candidate's field names another local, which is
        // only read: it is seen where it is written.
    };
    for block in &body.blocks {
        for statement in &block.statements {
            match statement {
                // A whole copy of a struct, in or out, is a copy of each
                // field; anything else assigned to a whole local is not.
                Statement::Assign(dest, Rvalue::Use(Operand::Copy(source))) => {
                    let whole_copy = |place: &Place| {
                        place.projections.is_empty()
                            && found.get(&place.local) == Some(&Shape::Struct)
                    };
                    see(dest, whole_copy(dest), &mut ruled_out);
                    see(source, whole_copy(source), &mut ruled_out);
                }
                Statement::Assign(dest, rvalue) => {
                    see(dest, false, &mut ruled_out);
                    match rvalue {
                        // The tag of a whole enum is the tag's local.
                        Rvalue::Variant(place) => {
                            let tag = place.projections.is_empty()
                                && found.get(&place.local) == Some(&Shape::Enum);
                            see(place, tag, &mut ruled_out);
                        }
                        // An address taken of anything a candidate holds
                        // is an address of a field of it, which stays a
                        // place of its own: allowed, but not of the whole.
                        Rvalue::AddressOf(place) => see(place, false, &mut ruled_out),
                        _ => each_operand(rvalue, &mut |operand| {
                            if let Operand::Copy(place) = operand {
                                see(place, false, &mut ruled_out);
                            }
                        }),
                    }
                }
                Statement::SetVariant(place, _) => {
                    let tag = place.projections.is_empty()
                        && found.get(&place.local) == Some(&Shape::Enum);
                    see(place, tag, &mut ruled_out);
                }
                Statement::Zero(place) => see(place, true, &mut ruled_out),
                other => each_place(other, &mut |place| see(place, false, &mut ruled_out)),
            }
        }
        each_terminator_operand(&block.terminator, &mut |operand| {
            if let Operand::Copy(place) = operand {
                see(place, false, &mut ruled_out);
            }
        });
    }
    // An index local inside a projection is a local read, and a candidate
    // is never one: indices are integers.
    for local in ruled_out {
        found.remove(&local);
    }
    found
}

/// Makes the locals each split local becomes.
fn split_locals(
    program: &Program,
    body: &mut Body,
    splits: &FxHashMap<Local, Shape>,
) -> FxHashMap<Local, Parts> {
    let mut parts = FxHashMap::default();
    let mut ordered: Vec<(&Local, &Shape)> = splits.iter().collect();
    ordered.sort_by_key(|(local, _)| local.0);
    for (&local, &shape) in ordered {
        let ty = body.locals[local.0 as usize].ty;
        let made = match shape {
            Shape::Struct => Parts::Fields(
                program
                    .field_tys(ty)
                    .into_iter()
                    .map(|field| new_local(body, field))
                    .collect(),
            ),
            Shape::Enum => Parts::Variants {
                tag: new_local(body, Types::I32),
                fields: program
                    .variant_field_tys(ty)
                    .into_iter()
                    .map(|variant| variant.into_iter().map(|f| new_local(body, f)).collect())
                    .collect(),
            },
        };
        parts.insert(local, made);
        // Nothing names the whole any more: it needs no storage.
        body.locals[local.0 as usize].ty = Types::UNIT;
    }
    parts
}

fn new_local(body: &mut Body, ty: Ty) -> Local {
    body.locals.push(LocalDecl::unnamed(ty, LocalKind::Temp));
    Local(body.locals.len() as u32 - 1)
}

/// Writes `statement` as what it is once its split locals are fields: a
/// whole copy becomes a copy of each field, a tag written or read becomes
/// the tag's local, and zeros become zeros in each part.
fn expand(statement: Statement, parts: &FxHashMap<Local, Parts>, out: &mut Vec<Statement>) {
    let whole = |place: &Place| place.projections.is_empty() && parts.contains_key(&place.local);
    match statement {
        Statement::Assign(dest, Rvalue::Use(Operand::Copy(source)))
            if whole(&dest) || whole(&source) =>
        {
            let split = if whole(&dest) { &dest } else { &source };
            let Parts::Fields(fields) = &parts[&split.local] else {
                unreachable!("an enum copied whole is not split");
            };
            for i in 0..fields.len() as u32 {
                out.push(Statement::Assign(
                    dest.project(Projection::Field(i)),
                    Rvalue::Use(Operand::Copy(source.project(Projection::Field(i)))),
                ));
            }
        }
        Statement::SetVariant(place, variant) if whole(&place) => {
            let Parts::Variants { tag, .. } = &parts[&place.local] else {
                unreachable!("only an enum has a variant");
            };
            out.push(Statement::Assign(
                Place::local(*tag),
                Rvalue::Use(Operand::Const(Const::Int {
                    bits: u128::from(variant),
                    ty: Types::I32,
                })),
            ));
        }
        Statement::Assign(dest, Rvalue::Variant(place)) if whole(&place) => {
            let Parts::Variants { tag, .. } = &parts[&place.local] else {
                unreachable!("only an enum has a variant");
            };
            out.push(Statement::Assign(
                dest,
                Rvalue::Use(Operand::Copy(Place::local(*tag))),
            ));
        }
        Statement::Zero(place) if whole(&place) => match &parts[&place.local] {
            Parts::Fields(fields) => {
                out.extend(fields.iter().map(|&f| Statement::Zero(Place::local(f))));
            }
            Parts::Variants { tag, fields } => {
                out.push(Statement::Zero(Place::local(*tag)));
                out.extend(
                    fields
                        .iter()
                        .flatten()
                        .map(|&f| Statement::Zero(Place::local(f))),
                );
            }
        },
        other => out.push(other),
    }
}

/// A place whose first projection is a field of a split local, as the
/// field's own local.
fn remap_place(place: &mut Place, parts: &FxHashMap<Local, Parts>) {
    if let Some(split) = parts.get(&place.local)
        && let Some(&first) = place.projections.first()
        && let Some(local) = split.place(first)
    {
        place.local = local;
        place.projections.remove(0);
    }
}

fn remap_operand(operand: &mut Operand, parts: &FxHashMap<Local, Parts>) {
    if let Operand::Copy(place) = operand {
        remap_place(place, parts);
    }
}

fn remap_statement(statement: &mut Statement, parts: &FxHashMap<Local, Parts>) {
    each_place_mut(statement, &mut |place| remap_place(place, parts));
}

fn remap_terminator(terminator: &mut Terminator, parts: &FxHashMap<Local, Parts>) {
    each_terminator_operand_mut(terminator, &mut |operand| remap_operand(operand, parts));
}

/// Each operand an rvalue reads.
fn each_operand(rvalue: &Rvalue, f: &mut impl FnMut(&Operand)) {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Unary(_, operand)
        | Rvalue::Cast(operand, _)
        | Rvalue::CstrLen(operand)
        | Rvalue::Float(_, operand)
        | Rvalue::Integer(_, operand)
        | Rvalue::Bits(operand)
        | Rvalue::VTableFn { table: operand, .. } => f(operand),
        Rvalue::Binary(_, lhs, rhs)
        | Rvalue::Rotate(_, lhs, rhs)
        | Rvalue::Overflows(_, lhs, rhs) => {
            f(lhs);
            f(rhs);
        }
        Rvalue::MulAdd(first, second, third) => {
            f(first);
            f(second);
            f(third);
        }
        Rvalue::AddressOf(_) | Rvalue::Variant(_) => {}
    }
}

/// Each place a statement names, read or written, as a whole value: what
/// rules a candidate out where it is named whole, and what is remapped.
fn each_place(statement: &Statement, f: &mut impl FnMut(&Place)) {
    let mut clone = statement.clone();
    each_place_mut(&mut clone, &mut |place| f(place));
}

fn each_place_mut(statement: &mut Statement, f: &mut impl FnMut(&mut Place)) {
    let operand = |operand: &mut Operand, f: &mut dyn FnMut(&mut Place)| {
        if let Operand::Copy(place) = operand {
            f(place);
        }
    };
    match statement {
        Statement::Assign(place, rvalue) => {
            f(place);
            match rvalue {
                Rvalue::AddressOf(place) | Rvalue::Variant(place) => f(place),
                Rvalue::Use(o)
                | Rvalue::Unary(_, o)
                | Rvalue::Cast(o, _)
                | Rvalue::CstrLen(o)
                | Rvalue::Float(_, o)
                | Rvalue::Integer(_, o)
                | Rvalue::Bits(o)
                | Rvalue::VTableFn { table: o, .. } => operand(o, f),
                Rvalue::Binary(_, lhs, rhs)
                | Rvalue::Rotate(_, lhs, rhs)
                | Rvalue::Overflows(_, lhs, rhs) => {
                    operand(lhs, f);
                    operand(rhs, f);
                }
                Rvalue::MulAdd(a, b, c) => {
                    operand(a, f);
                    operand(b, f);
                    operand(c, f);
                }
            }
        }
        Statement::SetVariant(place, _)
        | Statement::Zero(place)
        | Statement::Poison(place)
        | Statement::CheckMoved { place, .. } => f(place),
        Statement::Arith { dest, lhs, rhs, .. } => {
            f(dest);
            operand(lhs, f);
            operand(rhs, f);
        }
        Statement::Call { callee, args, dest } => {
            if let Callee::Value(o) = callee {
                operand(o, f);
            }
            for arg in args {
                operand(arg, f);
            }
            if let Some(dest) = dest {
                f(dest);
            }
        }
        Statement::Alloc { dest, .. } => f(dest),
        Statement::AllocBuffer { dest, count, .. } => {
            f(dest);
            operand(count, f);
        }
        Statement::Atomic {
            address,
            value,
            expected,
            dest,
            ..
        } => {
            operand(address, f);
            if let Some(value) = value {
                operand(value, f);
            }
            if let Some(expected) = expected {
                operand(expected, f);
            }
            if let Some(dest) = dest {
                f(dest);
            }
        }
        Statement::Free(o)
        | Statement::DropFn { ptr: o, .. }
        | Statement::DropInPlace { ptr: o, .. } => operand(o, f),
        Statement::At(_) => {}
        Statement::Check { fails, kind, .. } => {
            operand(fails, f);
            match kind {
                CheckKind::Bounds { index, length } => {
                    operand(index, f);
                    operand(length, f);
                }
                CheckKind::Length { length } => operand(length, f),
                CheckKind::Division | CheckKind::Overflow(_) => {}
            }
        }
    }
}

fn each_terminator_operand(terminator: &Terminator, f: &mut impl FnMut(&Operand)) {
    let mut clone = terminator.clone();
    each_terminator_operand_mut(&mut clone, &mut |operand| f(operand));
}

fn each_terminator_operand_mut(terminator: &mut Terminator, f: &mut impl FnMut(&mut Operand)) {
    match terminator {
        Terminator::Branch { cond, .. } => f(cond),
        Terminator::Switch { value, .. } => f(value),
        Terminator::Panic {
            note: Some(note), ..
        } => f(note),
        Terminator::Goto(_)
        | Terminator::Return
        | Terminator::Unreachable
        | Terminator::Panic { note: None, .. } => {}
    }
}
