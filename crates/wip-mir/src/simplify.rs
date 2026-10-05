//! What a place really names.
//!
//! MIR is written the way a body reads: a value is copied to a temporary,
//! an address is taken and then followed, a field is loaded and then read
//! through. Nothing folds any of that, so an inlined closure reads what it
//! captured through the environment the caller built rather than reading
//! the caller's own local.
//!
//! This pass follows two kinds of fact through a whole body:
//!
//! - `p = &x` makes `(*p)` mean `x`, and an address does not change while
//!   the pointer holds it.
//! - `a = b` makes a read of `a` a read of `b`, but only while `b` is what
//!   it was.
//!
//! Both are kept only for places written **once** in the body, so there is
//! no question of which write a read sees; a copy is kept only when what it
//! copied is written once too and its address is never taken, since a call
//! may write through an address it was given.
use rustc_hash::{FxHashMap, FxHashSet};

use crate::*;

/// Rewrites the places of `body` to the ones they stand for.
pub fn simplify(program: &Program, body: &mut Body) {
    let facts = facts(program, body);
    if facts.is_empty() {
        return;
    }
    for block in &mut body.blocks {
        for statement in &mut block.statements {
            rewrite_statement(statement, &facts);
        }
        rewrite_terminator(&mut block.terminator, &facts);
    }
}

/// What a place was given, where it was given it once.
#[derive(Clone)]
enum Value {
    Copied(Place),
    Address(Place),
}

/// A place a fact can be about: one reached by naming fields, with no `*`
/// and no index in it. An index names a local whose value may change, and
/// a `*` may be anywhere.
fn key(place: &Place) -> Option<(Local, Vec<Projection>)> {
    let plain = place
        .projections
        .iter()
        .all(|p| matches!(p, Projection::Field(_) | Projection::VariantField { .. }));
    plain.then(|| (place.local, place.projections.clone()))
}

type Facts = FxHashMap<(Local, Vec<Projection>), Value>;

/// Whether one path passes through the other: writing `p.0` writes part of
/// `p`, and writing `p` writes all of `p.0`.
fn overlaps(a: &[Projection], b: &[Projection]) -> bool {
    a.iter().zip(b).all(|(x, y)| x == y)
}

/// The facts a body makes, which is everything this pass knows.
fn facts(program: &Program, body: &Body) -> Facts {
    // Every write, so that a fact is kept only about a place written once.
    // A parameter arrives with a value, so a write to one is its second:
    // `x = p; zero(p)` is a move, and `x` is not another name for `p`.
    let mut writes: Vec<(Local, Vec<Projection>)> = body
        .params
        .iter()
        .map(|&param| (param, Vec::new()))
        .collect();
    // A local nothing can be known about: written through a pointer or an
    // index, or the target of an address the body then passes around.
    let mut unknown: FxHashSet<Local> = FxHashSet::default();
    let mut candidates: Vec<((Local, Vec<Projection>), Value)> = Vec::new();
    for block in &body.blocks {
        for statement in &block.statements {
            if let Some(dest) = written_place(statement) {
                match key(dest) {
                    Some(key) => writes.push(key),
                    // A write through a pointer or into an element: what it
                    // reached is not known.
                    None => {
                        unknown.insert(dest.local);
                        if dest.projections.contains(&Projection::Deref) {
                            // It may have been any place whose address was
                            // taken.
                            unknown.extend(addressed(body));
                        }
                    }
                }
            }
            // A call may write through any address it was given.
            if matches!(
                statement,
                Statement::Call { .. } | Statement::DropFn { .. } | Statement::DropInPlace { .. }
            ) {
                unknown.extend(addressed(body));
            }
            if let Statement::Assign(dest, rvalue) = statement
                && let Some(key) = key(dest)
            {
                let value = match rvalue {
                    // A copy names what it copied only where the two are of
                    // one type: a slice's pointer is a word, and the
                    // `ptr<T>` it is copied into is indexed as the elements.
                    Rvalue::Use(Operand::Copy(source))
                        if place_ty(program, body, source) == place_ty(program, body, dest) =>
                    {
                        Value::Copied(source.clone())
                    }
                    Rvalue::AddressOf(target) => Value::Address(target.clone()),
                    _ => continue,
                };
                candidates.push((key, value));
            }
        }
    }
    let written_once = |place: &Place| match key(place) {
        Some((local, projections)) => {
            writes
                .iter()
                .filter(|(l, p)| *l == local && overlaps(p, &projections))
                .count()
                <= 1
        }
        None => false,
    };
    let mut facts = Facts::default();
    for (dest, value) in candidates {
        let named = Place {
            local: dest.0,
            projections: dest.1.clone(),
        };
        if !written_once(&named) || unknown.contains(&dest.0) {
            continue;
        }
        // An address stays what it is; a copy is only a name for what it
        // copied while that is what it was.
        let keep = match &value {
            Value::Address(_) => true,
            // Only a scalar is a value a name can stand for. An aggregate
            // is an address, and a callee handed that address may write
            // through it — so a copy of an aggregate is a copy, not a
            // second name for what it copied.
            Value::Copied(source) => {
                written_once(source)
                    && !unknown.contains(&source.local)
                    && !crate::is_aggregate(program, crate::place_ty(program, body, &named))
            }
        };
        if keep {
            facts.insert(dest, value);
        }
    }
    facts
}

/// The locals whose address the body takes, which is what a write through a
/// pointer may have reached.
fn addressed(body: &Body) -> Vec<Local> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            Statement::Assign(_, Rvalue::AddressOf(target)) => Some(target.local),
            _ => None,
        })
        .collect()
}

/// The place a statement writes, if it writes one.
fn written_place(statement: &Statement) -> Option<&Place> {
    match statement {
        Statement::Assign(dest, _)
        | Statement::SetVariant(dest, _)
        | Statement::Zero(dest)
        | Statement::Poison(dest)
        | Statement::Arith { dest, .. }
        | Statement::Alloc { dest, .. }
        | Statement::AllocBuffer { dest, .. } => Some(dest),
        Statement::Call { dest, .. } | Statement::Atomic { dest, .. } => dest.as_ref(),
        _ => None,
    }
}

/// Follows what is known about `place` until nothing more is known.
fn rewrite_place(place: &mut Place, facts: &Facts) {
    for _ in 0..8 {
        // The longest part of the place a fact is about, with whatever it
        // is read through left on the end: a fact about `p.0` says what
        // `p.0.1` reads too.
        if let Some(folded) = copied(place, facts)
            && folded != *place
        {
            *place = folded;
            continue;
        }
        // `(*p).f`, where `p` holds `&x`, is `x.f`.
        if let Some((Projection::Deref, rest)) = place.projections.split_first()
            && let Some(Value::Address(target)) = facts.get(&(place.local, Vec::new()))
        {
            let mut folded = target.clone();
            folded.projections.extend_from_slice(rest);
            if folded != *place {
                *place = folded;
                continue;
            }
        }
        break;
    }
}

/// What a place stands for, when a fact is about it or about a part of it.
fn copied(place: &Place, facts: &Facts) -> Option<Place> {
    let deref = place
        .projections
        .iter()
        .position(|p| *p == Projection::Deref)
        .unwrap_or(place.projections.len());
    for taken in (0..=deref).rev() {
        let key = (place.local, place.projections[..taken].to_vec());
        if let Some(Value::Copied(source)) = facts.get(&key) {
            let mut folded = source.clone();
            folded
                .projections
                .extend_from_slice(&place.projections[taken..]);
            return Some(folded);
        }
    }
    None
}

fn rewrite_operand(operand: &mut Operand, facts: &Facts) {
    if let Operand::Copy(place) = operand {
        rewrite_place(place, facts);
    }
}

fn rewrite_rvalue(rvalue: &mut Rvalue, facts: &Facts) {
    match rvalue {
        Rvalue::Use(o)
        | Rvalue::Unary(_, o)
        | Rvalue::Cast(o, _)
        | Rvalue::CstrLen(o)
        | Rvalue::Float(_, o)
        | Rvalue::Integer(_, o)
        | Rvalue::Bits(o)
        | Rvalue::VTableFn { table: o, .. } => rewrite_operand(o, facts),
        Rvalue::Binary(_, l, r) | Rvalue::Rotate(_, l, r) => {
            rewrite_operand(l, facts);
            rewrite_operand(r, facts);
        }
        Rvalue::MulAdd(first, second, third) => {
            rewrite_operand(first, facts);
            rewrite_operand(second, facts);
            rewrite_operand(third, facts);
        }
        // The address of a place is the address of what it stands for.
        Rvalue::AddressOf(place) | Rvalue::Variant(place) => rewrite_place(place, facts),
    }
}

fn rewrite_statement(statement: &mut Statement, facts: &Facts) {
    match statement {
        Statement::Assign(dest, rvalue) => {
            rewrite_through(dest, facts);
            rewrite_rvalue(rvalue, facts);
        }
        Statement::SetVariant(place, _)
        | Statement::Zero(place)
        | Statement::Poison(place)
        | Statement::CheckMoved { place, .. } => rewrite_through(place, facts),
        Statement::Arith { dest, lhs, rhs, .. } => {
            rewrite_through(dest, facts);
            rewrite_operand(lhs, facts);
            rewrite_operand(rhs, facts);
        }
        Statement::Call { args, dest, .. } => {
            // What is called is left alone: a closure's code address is
            // read from the pair, and the pair is what says its type.
            for arg in args {
                rewrite_operand(arg, facts);
            }
            if let Some(dest) = dest {
                rewrite_through(dest, facts);
            }
        }
        Statement::Alloc { dest, .. } => rewrite_through(dest, facts),
        Statement::AllocBuffer { dest, count, .. } => {
            rewrite_through(dest, facts);
            rewrite_operand(count, facts);
        }
        Statement::Atomic {
            address,
            value,
            expected,
            dest,
            ..
        } => {
            rewrite_operand(address, facts);
            if let Some(value) = value {
                rewrite_operand(value, facts);
            }
            if let Some(expected) = expected {
                rewrite_operand(expected, facts);
            }
            if let Some(dest) = dest {
                rewrite_through(dest, facts);
            }
        }
        Statement::Free(o)
        | Statement::DropFn { ptr: o, .. }
        | Statement::DropInPlace { ptr: o, .. } => rewrite_operand(o, facts),
        Statement::At(_) => {}
        Statement::Check { fails, kind, .. } => {
            rewrite_operand(fails, facts);
            match kind {
                CheckKind::Bounds { index, length } => {
                    rewrite_operand(index, facts);
                    rewrite_operand(length, facts);
                }
                CheckKind::Length { length } => rewrite_operand(length, facts),
                CheckKind::Division | CheckKind::Overflow(_) => {}
            }
        }
    }
}

/// A place that is written stays where it is; only the pointer it is
/// reached through is followed.
fn rewrite_through(place: &mut Place, facts: &Facts) {
    if matches!(place.projections.first(), Some(Projection::Deref)) {
        rewrite_place(place, facts);
    }
}

fn rewrite_terminator(terminator: &mut Terminator, facts: &Facts) {
    match terminator {
        Terminator::Branch { cond, .. } => rewrite_operand(cond, facts),
        Terminator::Switch { value, .. } => rewrite_operand(value, facts),
        Terminator::Panic {
            note: Some(note), ..
        } => rewrite_operand(note, facts),
        _ => {}
    }
}
