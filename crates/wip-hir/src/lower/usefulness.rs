//! Whether a `match` covers every value, and whether an arm can be
//! reached.
//!
//! Both questions are one question: is a pattern *useful* against the arms
//! before it — is there a value that it matches and they do not. A `match`
//! is exhaustive where `_` is not useful against all of its arms, and an
//! arm is unreachable where it is not useful against the arms before it.
//! What comes back where a pattern is useful is a value that shows why,
//! which the message prints as the pattern that would match it.
//!
//! This is Maranget's algorithm ("Warnings for pattern matching", JFP
//! 2007), over a matrix whose columns carry their types, since a pattern
//! alone does not say how many fields a constructor has here.

use crate::hir::{Binder, Pattern, Program};
use crate::ty::{Ty, TyKind};
use wip_syntax::Symbol;

/// What a pattern tests for: one shape a value of its type may have.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Ctor {
    Variant(u32),
    Bool(bool),
    /// The values of an integer or a character type from one key of its
    /// order to another, both included: a value is the
    /// range of one. A column of them is complete where its ranges cover
    /// the type.
    Range(u128, u128),
    /// A value of a type with no order to cover, so a column of them is
    /// never complete.
    Value(u128),
    Text(Symbol),
    /// A struct or a tuple: the one shape every value of the type has.
    Only,
    /// An array or a slice of exactly this many elements.
    Length(u64),
    /// Of at least `prefix + suffix` elements, the first `prefix` and the
    /// last `suffix` of which are its fields. A column of
    /// slices is cut into the lengths below the longest a row names, and
    /// this for the rest, as rustc does.
    AtLeast(u64, u64),
}

/// A pattern, as the algorithm needs it: a constructor with the patterns
/// of its fields, or anything at all.
#[derive(Clone, Debug)]
pub(super) enum Shape {
    Wild,
    Ctor(Ctor, Vec<Shape>),
    /// `a | b`: rows are split on these before anything else.
    Any(Vec<Shape>),
}

impl Shape {
    /// The pattern as the algorithm sees it. `ty` says how many fields each
    /// constructor has, which the pattern itself does not.
    pub(super) fn of(program: &Program, pattern: &Pattern, ty: Ty) -> Shape {
        match pattern {
            Pattern::Wildcard | Pattern::Binding(_) | Pattern::Error => Shape::Wild,
            // A reference is seen through: its column is its referent's.
            Pattern::Deref(inner) => Shape::of(program, inner, referent(program, ty)),
            Pattern::Bool(value) => Shape::Ctor(Ctor::Bool(*value), Vec::new()),
            Pattern::Int(bits) => {
                let ctor = match program.types.order_key(ty, *bits) {
                    Some(key) => Ctor::Range(key, key),
                    None => Ctor::Value(*bits),
                };
                Shape::Ctor(ctor, Vec::new())
            }
            // An end left off runs to the type's least or greatest value.
            Pattern::Range { lo, hi } => {
                let types = &program.types;
                // Refused where it was checked: only an ordered type has a
                // range.
                let Some((least, greatest)) = types.order_range(ty) else {
                    return Shape::Wild;
                };
                let key = |bits: u128| types.order_key(ty, bits).unwrap_or(least);
                let lo = lo.map_or(least, key);
                let hi = hi.map_or(greatest, key);
                Shape::Ctor(Ctor::Range(lo, hi), Vec::new())
            }
            Pattern::Str(text) => Shape::Ctor(Ctor::Text(*text), Vec::new()),
            // The elements it names from either end.
            Pattern::Slice {
                prefix,
                rest,
                suffix,
            } => {
                let named: Vec<Binder> = prefix.iter().chain(suffix).cloned().collect();
                let tys = vec![referent(program, program.element_ty(ty)); named.len()];
                let ctor = match rest {
                    Some(_) => Ctor::AtLeast(prefix.len() as u64, suffix.len() as u64),
                    None => Ctor::Length(named.len() as u64),
                };
                Shape::Ctor(ctor, binder_shapes(program, &named, &tys))
            }
            Pattern::Any { alternatives, .. } => Shape::Any(
                alternatives
                    .iter()
                    .map(|p| Shape::of(program, p, ty))
                    .collect(),
            ),
            Pattern::Fields(binders) => {
                let tys = field_tys(program, ty);
                Shape::Ctor(Ctor::Only, binder_shapes(program, binders, &tys))
            }
            Pattern::Variant { variant, binders } => {
                // A variant pattern on a struct is a struct pattern written
                // with the type's name.
                let tys = match program.types.kind(ty) {
                    TyKind::Enum(..) => variant_field_tys(program, ty, *variant),
                    _ => field_tys(program, ty),
                };
                Shape::Ctor(
                    Ctor::Variant(*variant),
                    binder_shapes(program, binders, &tys),
                )
            }
        }
    }
}

/// What a column of `ty` is matched as: a reference is seen through, at
/// any depth, since a pattern tests what it refers to.
fn referent(program: &Program, mut ty: Ty) -> Ty {
    while let TyKind::Ref(inner, _) = program.types.kind(ty) {
        ty = inner;
    }
    ty
}

fn binder_shapes(program: &Program, binders: &[Binder], tys: &[Ty]) -> Vec<Shape> {
    binders
        .iter()
        .zip(tys)
        .map(|(binder, &ty)| match binder {
            Binder::Ignored | Binder::Bind(_) => Shape::Wild,
            Binder::Nested(pattern) => Shape::of(program, pattern, ty),
        })
        .collect()
}

/// The types of a struct's fields, and of a variant's. A type the checker
/// has not interned — a substitution it never needed — is left unknown:
/// nothing can be listed for it, so a column of them is never complete,
/// which is what the algorithm does with a number or a text anyway.
fn field_tys(program: &Program, ty: Ty) -> Vec<Ty> {
    let TyKind::Struct(id, args) = program.types.kind(ty) else {
        return Vec::new();
    };
    let args = program.types.list(args);
    program.structs[id]
        .fields
        .iter()
        .map(|field| {
            let ty = program
                .types
                .try_subst_find(field.ty, args)
                .unwrap_or(crate::ty::Types::ERROR);
            referent(program, ty)
        })
        .collect()
}

/// The lengths an array or a slice may have, as a column of patterns cuts
/// them: an array's one; a slice's lengths below the
/// longest a pattern of `matrix`, or `head`, names, each on its own, and
/// every length from there on as one, whose fields are the most any
/// pattern names from the start and from the end. Nothing for any other
/// type.
fn lengths(
    program: &Program,
    ty: Ty,
    matrix: &[Vec<Shape>],
    head: Option<Ctor>,
) -> Option<Vec<Ctor>> {
    match program.types.kind(ty) {
        TyKind::Array(_, len) => return Some(vec![Ctor::Length(len)]),
        TyKind::Slice(_) => {}
        _ => return None,
    }
    let (mut prefix, mut suffix, mut longest) = (0, 0, None);
    for ctor in matrix.iter().filter_map(|row| head_ctor(row)).chain(head) {
        match ctor {
            Ctor::Length(n) => longest = longest.max(Some(n)),
            Ctor::AtLeast(p, s) => {
                prefix = prefix.max(p);
                suffix = suffix.max(s);
            }
            _ => {}
        }
    }
    // Every closed length is below where the open one starts.
    if let Some(n) = longest
        && prefix + suffix <= n
    {
        prefix = n + 1 - suffix;
    }
    let mut ctors: Vec<Ctor> = (0..prefix + suffix).map(Ctor::Length).collect();
    ctors.push(Ctor::AtLeast(prefix, suffix));
    Some(ctors)
}

fn variant_field_tys(program: &Program, ty: Ty, variant: u32) -> Vec<Ty> {
    let TyKind::Enum(id, args) = program.types.kind(ty) else {
        return field_tys(program, ty);
    };
    let args = program.types.list(args);
    program.enums[id].variants[variant as usize]
        .fields
        .iter()
        .map(|field| {
            let ty = program
                .types
                .try_subst_find(field.ty, args)
                .unwrap_or(crate::ty::Types::ERROR);
            referent(program, ty)
        })
        .collect()
}

/// The constructors a type has, where they can be listed: an enum's
/// variants, and `true` and `false`. A struct has the one. Numbers and text,
/// a `str` or a `String`, have too many, and a column of them is complete
/// only with a `_`.
fn every_ctor(program: &Program, ty: Ty) -> Option<Vec<Ctor>> {
    match program.types.kind(ty) {
        TyKind::Enum(id, _) => Some(
            (0..program.enums[id].variants.len() as u32)
                .map(Ctor::Variant)
                .collect(),
        ),
        TyKind::Bool => Some(vec![Ctor::Bool(false), Ctor::Bool(true)]),
        // A `String` is matched by the text it holds, of which there are as
        // many as of a `str`'s.
        TyKind::Struct(..) if program.is_string(ty) => None,
        TyKind::Struct(..) => Some(vec![Ctor::Only]),
        TyKind::Array(_, len) => Some(vec![Ctor::Length(len)]),
        _ => None,
    }
}

/// How many fields a constructor has here, and what their types are.
fn ctor_tys(program: &Program, ty: Ty, ctor: Ctor) -> Vec<Ty> {
    match ctor {
        Ctor::Variant(variant) => variant_field_tys(program, ty, variant),
        Ctor::Only => field_tys(program, ty),
        Ctor::Length(n) => vec![referent(program, program.element_ty(ty)); n as usize],
        Ctor::AtLeast(p, s) => vec![referent(program, program.element_ty(ty)); (p + s) as usize],
        _ => Vec::new(),
    }
}

/// A value no arm matches, as the pattern that would match it.
pub(super) struct Witness(Vec<Shape>);

impl Witness {
    /// The witness written out: `.Some(.None)`, `(true, _)`, `_`.
    pub(super) fn text(
        &self,
        program: &Program,
        ty: Ty,
        interner: &wip_syntax::Interner,
    ) -> String {
        match self.0.first() {
            Some(shape) => write_shape(program, shape, ty, interner),
            None => "_".to_string(),
        }
    }
}

fn write_shape(
    program: &Program,
    shape: &Shape,
    ty: Ty,
    interner: &wip_syntax::Interner,
) -> String {
    let Shape::Ctor(ctor, fields) = shape else {
        return "_".to_string();
    };
    let inner = |fields: &[Shape], tys: Vec<Ty>| -> Vec<String> {
        fields
            .iter()
            .zip(tys)
            .map(|(field, ty)| write_shape(program, field, ty, interner))
            .collect()
    };
    match *ctor {
        Ctor::Bool(value) => value.to_string(),
        Ctor::Range(lo, hi) => write_range(program, ty, lo, hi),
        Ctor::Value(bits) => bits.to_string(),
        Ctor::Text(text) => format!("\"{}\"", interner.resolve(text)),
        // `[_, _]`, `[_, ..]`, `[]`.
        Ctor::Length(_) | Ctor::AtLeast(..) => {
            let mut parts = inner(fields, ctor_tys(program, ty, *ctor));
            if let Ctor::AtLeast(p, _) = *ctor {
                parts.insert((p as usize).min(parts.len()), "..".to_string());
            }
            format!("[{}]", parts.join(", "))
        }
        Ctor::Only => match program.types.kind(ty) {
            // A tuple prints as one.
            TyKind::Struct(id, _) if program.structs[id].is_tuple => {
                format!("({})", inner(fields, field_tys(program, ty)).join(", "))
            }
            TyKind::Struct(id, _) => {
                let name = interner.resolve(program.structs[id].name);
                match fields.is_empty() {
                    true => name.to_string(),
                    false => format!(
                        "{name}({})",
                        inner(fields, field_tys(program, ty)).join(", ")
                    ),
                }
            }
            _ => "_".to_string(),
        },
        Ctor::Variant(variant) => {
            let TyKind::Enum(id, _) = program.types.kind(ty) else {
                return "_".to_string();
            };
            let name = interner.resolve(program.enums[id].variants[variant as usize].name);
            match fields.is_empty() {
                true => format!(".{name}"),
                false => format!(
                    ".{name}({})",
                    inner(fields, ctor_tys(program, ty, *ctor)).join(", ")
                ),
            }
        }
    }
}

/// The values from `lo` to `hi` of an ordered type, as the pattern that
/// matches them: `_` for all of them, one value, `lo..`, `..hi` (`..='z'`
/// for characters) or `lo..=hi`.
fn write_range(program: &Program, ty: Ty, lo: u128, hi: u128) -> String {
    let Some((least, greatest)) = program.types.order_range(ty) else {
        return "_".to_string();
    };
    let value = |key| write_value(program, ty, key);
    match (lo == least, hi == greatest) {
        (true, true) => "_".to_string(),
        _ if lo == hi => value(lo),
        (false, true) => format!("{}..", value(lo)),
        // `..0` says "below zero" as a `match` would write it.
        (true, false) if program.types.is_integer(ty) => format!("..{}", value(hi + 1)),
        (true, false) => format!("..={}", value(hi)),
        (false, false) => format!("{}..={}", value(lo), value(hi)),
    }
}

/// The value at `key` in a type's order, as a literal: a signed number
/// with its sign, a character in quotes.
fn write_value(program: &Program, ty: Ty, key: u128) -> String {
    let bits = program.types.from_order_key(ty, key);
    match program.types.kind(ty) {
        TyKind::Int(t) if t.signed() => {
            let shift = 128 - t.bits();
            (((bits << shift) as i128) >> shift).to_string()
        }
        TyKind::Char => match char::from_u32(bits as u32) {
            Some(c) => write_char(c),
            None => bits.to_string(),
        },
        _ => bits.to_string(),
    }
}

/// A character as its literal is written, escaped where it would not be
/// seen.
fn write_char(c: char) -> String {
    match c {
        '\n' => r"'\n'".to_string(),
        '\t' => r"'\t'".to_string(),
        '\r' => r"'\r'".to_string(),
        '\0' => r"'\0'".to_string(),
        '\\' => r"'\\'".to_string(),
        '\'' => r"'\''".to_string(),
        ' ' => "' '".to_string(),
        c if c.is_ascii_graphic() => format!("'{c}'"),
        c => format!("'\\u{{{:X}}}'", c as u32),
    }
}

/// The runs of a type's order that its values fill: an integer's one, a
/// `char`'s two, either side of the surrogates. Nothing
/// for a type with no order.
fn domain(program: &Program, ty: Ty) -> Option<Vec<(u128, u128)>> {
    match program.types.kind(ty) {
        TyKind::Char => Some(vec![(0, 0xD7FF), (0xE000, 0x10_FFFF)]),
        _ => program.types.order_range(ty).map(|range| vec![range]),
    }
}

/// The part of `range` that holds values of the type.
fn clip(range: (u128, u128), domain: &[(u128, u128)]) -> Vec<(u128, u128)> {
    domain
        .iter()
        .filter_map(|&(start, end)| {
            let lo = range.0.max(start);
            let hi = range.1.min(end);
            (lo <= hi).then_some((lo, hi))
        })
        .collect()
}

/// `within` cut wherever a range at the head of a row starts or ends: the
/// pieces each of those ranges holds whole or not at all, in order.
fn split(matrix: &[Vec<Shape>], within: &[(u128, u128)]) -> Vec<Ctor> {
    let mut cuts: Vec<u128> = Vec::new();
    for row in matrix {
        if let Some(Ctor::Range(lo, hi)) = head_ctor(row) {
            cuts.push(lo);
            cuts.extend(hi.checked_add(1));
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut pieces = Vec::new();
    for &(start, end) in within {
        let mut from = start;
        for &cut in cuts.iter().filter(|&&cut| cut > start && cut <= end) {
            pieces.push(Ctor::Range(from, cut - 1));
            from = cut;
        }
        pieces.push(Ctor::Range(from, end));
    }
    pieces
}

/// Whether a pattern testing `outer` matches every value `inner` stands
/// for: a range the piece lies in, or the one constructor.
fn covers(outer: Ctor, inner: Ctor) -> bool {
    match (outer, inner) {
        (Ctor::Range(lo, hi), Ctor::Range(from, to)) => lo <= from && to <= hi,
        // A slice of at least so many: every length from there, and the
        // longer pieces whose fields hold its own.
        (Ctor::AtLeast(p, s), Ctor::Length(n)) => n >= p + s,
        (Ctor::AtLeast(p, s), Ctor::AtLeast(q, t)) => q >= p && t >= s,
        _ => outer == inner,
    }
}

/// Whether `row` matches a value the rows of `matrix` do not, and one such
/// value where it does. Every row, and `row` itself, has one pattern per
/// column of `tys`.
pub(super) fn useful(
    program: &Program,
    matrix: &[Vec<Shape>],
    row: &[Shape],
    tys: &[Ty],
) -> Option<Witness> {
    // No columns left: the row matches what is left where nothing above it
    // does.
    let Some((head, rest)) = row.split_first() else {
        return matrix.is_empty().then(|| Witness(Vec::new()));
    };
    let ty = referent(program, tys[0]);
    // `a | b` is useful where either is.
    if let Shape::Any(alternatives) = head {
        for alternative in alternatives {
            let mut row = row.to_vec();
            row[0] = alternative.clone();
            if let Some(witness) = useful(program, matrix, &row, tys) {
                return Some(witness);
            }
        }
        return None;
    }
    let matrix = flatten(matrix);
    if let Shape::Ctor(ctor, fields) = head {
        // A range is useful where one of the pieces the column cuts it
        // into is.
        let pieces = match *ctor {
            Ctor::Range(lo, hi) => split(&matrix, &clip((lo, hi), &domain(program, ty)?)),
            // So is a slice's, where one of the lengths it matches is.
            Ctor::Length(_) | Ctor::AtLeast(..) => lengths(program, ty, &matrix, Some(*ctor))?
                .into_iter()
                .filter(|&piece| covers(*ctor, piece))
                .collect(),
            ctor => vec![ctor],
        };
        for piece in pieces {
            let mut narrowed = Vec::new();
            for r in &matrix {
                if let Some(specialized) = specialize(program, r, piece, ty) {
                    narrowed.push(specialized);
                }
            }
            let mut row: Vec<Shape> = fields.clone();
            row.extend_from_slice(rest);
            let mut tys_below = ctor_tys(program, ty, piece);
            tys_below.extend_from_slice(&tys[1..]);
            if let Some(witness) = useful(program, &narrowed, &row, &tys_below) {
                return Some(rebuild(witness, piece, fields.len()));
            }
        }
        return None;
    }
    // A `_`: useful where any constructor the column does not cover is, or,
    // where the column covers them all, where it is useful under one of
    // them. An ordered type's constructors are the pieces its column's
    // ranges cut it into.
    let all = match domain(program, ty) {
        Some(domain) => Some(split(&matrix, &domain)),
        None => lengths(program, ty, &matrix, None).or_else(|| every_ctor(program, ty)),
    };
    let covered = |ctor: &Ctor| {
        matrix
            .iter()
            .any(|r| head_ctor(r).is_some_and(|head| covers(head, *ctor)))
    };
    let complete = all
        .as_ref()
        .is_some_and(|all| !all.is_empty() && all.iter().all(covered));
    if complete {
        for ctor in all.unwrap_or_default() {
            let mut narrowed = Vec::new();
            for r in &matrix {
                if let Some(specialized) = specialize(program, r, ctor, ty) {
                    narrowed.push(specialized);
                }
            }
            let arity = ctor_tys(program, ty, ctor);
            let mut row: Vec<Shape> = vec![Shape::Wild; arity.len()];
            row.extend_from_slice(rest);
            let mut tys_below = arity.clone();
            tys_below.extend_from_slice(&tys[1..]);
            if let Some(witness) = useful(program, &narrowed, &row, &tys_below) {
                return Some(rebuild(witness, ctor, arity.len()));
            }
        }
        return None;
    }
    // The rows that say nothing about this column.
    let default: Vec<Vec<Shape>> = matrix
        .iter()
        .filter(|r| head_ctor(r).is_none())
        .map(|r| r[1..].to_vec())
        .collect();
    let witness = useful(program, &default, rest, &tys[1..])?;
    // A constructor no row has, where one can be named, says more than `_`:
    // of an ordered type, the first values no row has, as far as they run.
    let all = all.unwrap_or_default();
    let missing = all.iter().position(|c| !covered(c)).map(|first| {
        match all[first..].iter().take_while(|c| !covered(c)).last() {
            Some(&Ctor::Range(_, hi)) => match all[first] {
                Ctor::Range(lo, _) => Ctor::Range(lo, hi),
                ctor => ctor,
            },
            _ => all[first],
        }
    });
    let mut shapes = vec![match missing {
        Some(ctor) => Shape::Ctor(ctor, vec![Shape::Wild; ctor_tys(program, ty, ctor).len()]),
        None => Shape::Wild,
    }];
    shapes.extend(witness.0);
    Some(Witness(shapes))
}

/// The runs of an ordered type's values that no row of a one-column
/// `matrix` matches, each written as the pattern that would: what a
/// `match` of numbers or characters leaves out. Nothing
/// for a type with no order.
pub(super) fn missing_ranges(program: &Program, matrix: &[Vec<Shape>], ty: Ty) -> Vec<String> {
    let Some(domain) = domain(program, ty) else {
        return Vec::new();
    };
    let matrix = flatten(matrix);
    let covered = |piece: Ctor| {
        matrix.iter().any(|r| match r.first() {
            Some(Shape::Ctor(head, _)) => covers(*head, piece),
            _ => true,
        })
    };
    // Pieces left out one after another are one run, across the
    // surrogates too.
    let mut runs: Vec<(u128, u128)> = Vec::new();
    let mut open = false;
    for piece in split(&matrix, &domain) {
        let Ctor::Range(lo, hi) = piece else {
            continue;
        };
        if covered(piece) {
            open = false;
            continue;
        }
        match runs.last_mut() {
            Some(run) if open => run.1 = hi,
            _ => runs.push((lo, hi)),
        }
        open = true;
    }
    runs.into_iter()
        .map(|(lo, hi)| write_range(program, ty, lo, hi))
        .collect()
}

/// Rows with `a | b` in the first column, split into one row each.
fn flatten(matrix: &[Vec<Shape>]) -> Vec<Vec<Shape>> {
    let mut out = Vec::new();
    for row in matrix {
        match row.first() {
            Some(Shape::Any(alternatives)) => {
                for alternative in alternatives {
                    let mut split = row.clone();
                    split[0] = alternative.clone();
                    out.extend(flatten(&[split]));
                }
            }
            _ => out.push(row.clone()),
        }
    }
    out
}

fn head_ctor(row: &[Shape]) -> Option<Ctor> {
    match row.first() {
        Some(Shape::Ctor(ctor, _)) => Some(*ctor),
        _ => None,
    }
}

/// The row as it stands under `ctor`: its fields, then the other columns,
/// or nothing where the row tests another constructor.
fn specialize(program: &Program, row: &[Shape], ctor: Ctor, ty: Ty) -> Option<Vec<Shape>> {
    let (head, rest) = row.split_first()?;
    let mut out = match head {
        Shape::Wild => vec![Shape::Wild; ctor_tys(program, ty, ctor).len()],
        // A slice's first and last elements, with what is between them
        // anything.
        Shape::Ctor(Ctor::AtLeast(p, s), fields) if covers(Ctor::AtLeast(*p, *s), ctor) => {
            let arity = ctor_tys(program, ty, ctor).len();
            let between = arity.saturating_sub((p + s) as usize);
            let (first, last) = fields.split_at((*p as usize).min(fields.len()));
            let mut out = first.to_vec();
            out.extend(vec![Shape::Wild; between]);
            out.extend_from_slice(last);
            out
        }
        Shape::Ctor(c, fields) if covers(*c, ctor) => fields.clone(),
        _ => return None,
    };
    out.extend_from_slice(rest);
    Some(out)
}

/// The witness of the columns under `ctor`, written as one of `ctor`.
fn rebuild(witness: Witness, ctor: Ctor, arity: usize) -> Witness {
    let mut shapes = witness.0;
    let fields: Vec<Shape> = shapes.drain(..arity.min(shapes.len())).collect();
    let mut out = vec![Shape::Ctor(ctor, fields)];
    out.extend(shapes);
    Witness(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TyList;
    use crate::ty::Types;
    use wip_syntax::Interner;

    /// A program that declares what `text` does, and the interner that
    /// reads its names.
    fn program(text: &str) -> (Program, Interner) {
        let mut interner = Interner::new();
        let lexed = wip_syntax::lex(text, &mut interner);
        let ast = wip_syntax::parse(text, &lexed).ast;
        let lowered = crate::lower_file(&ast, &interner);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        (lowered.program, interner)
    }

    /// The type called `name` that `program` declares.
    fn ty(program: &mut Program, interner: &Interner, name: &str) -> Ty {
        let named = |sym| interner.resolve(sym) == name;
        if let Some((id, _)) = program.enums.iter().find(|(_, e)| named(e.name)) {
            return program.types.intern(TyKind::Enum(id, TyList::EMPTY));
        }
        let (id, _) = program
            .structs
            .iter()
            .find(|(_, s)| named(s.name))
            .expect("the program declares it");
        program.types.intern(TyKind::Struct(id, TyList::EMPTY))
    }

    fn variant(index: u32, fields: Vec<Shape>) -> Shape {
        Shape::Ctor(Ctor::Variant(index), fields)
    }

    fn boolean(value: bool) -> Shape {
        Shape::Ctor(Ctor::Bool(value), Vec::new())
    }

    /// What `arms`, one column of `ty`, leave uncovered, as the message
    /// writes it; nothing where they cover every value.
    fn uncovered(
        program: &Program,
        interner: &Interner,
        arms: Vec<Shape>,
        ty: Ty,
    ) -> Option<String> {
        let matrix: Vec<Vec<Shape>> = arms.into_iter().map(|arm| vec![arm]).collect();
        useful(program, &matrix, &[Shape::Wild], &[ty])
            .map(|found| found.text(program, ty, interner))
    }

    const SHAPES: &str = "enum Shape {\n    Dot\n    Round(radius: i64)\n    Square\n}\n";

    #[test]
    fn a_missing_variant_is_named() {
        let (mut program, interner) = program(SHAPES);
        let shape = ty(&mut program, &interner, "Shape");
        let arms = vec![variant(0, vec![]), variant(1, vec![Shape::Wild])];
        assert_eq!(
            uncovered(&program, &interner, arms, shape).as_deref(),
            Some(".Square")
        );
    }

    #[test]
    fn every_variant_covers_the_enum() {
        let (mut program, interner) = program(SHAPES);
        let shape = ty(&mut program, &interner, "Shape");
        let arms = vec![
            variant(0, vec![]),
            variant(1, vec![Shape::Wild]),
            variant(2, vec![]),
        ];
        assert_eq!(uncovered(&program, &interner, arms, shape), None);
    }

    #[test]
    fn an_arm_after_a_wildcard_is_unreachable() {
        let (mut program, interner) = program(SHAPES);
        let shape = ty(&mut program, &interner, "Shape");
        let above = vec![vec![Shape::Wild]];
        assert!(useful(&program, &above, &[variant(0, vec![])], &[shape]).is_none());
        // And one the arms above leave something to is not.
        let above = vec![vec![variant(0, vec![])]];
        assert!(useful(&program, &above, &[variant(2, vec![])], &[shape]).is_some());
    }

    #[test]
    fn either_of_two_patterns_covers_both() {
        let (mut program, interner) = program(SHAPES);
        let shape = ty(&mut program, &interner, "Shape");
        let arms = vec![Shape::Any(vec![variant(0, vec![]), variant(2, vec![])])];
        assert_eq!(
            uncovered(&program, &interner, arms, shape).as_deref(),
            Some(".Round(_)")
        );
    }

    #[test]
    fn what_is_missing_inside_a_variant_is_named_there() {
        let (mut program, interner) = program(
            "enum Inner {\n    X\n    Y\n}\n\nenum Outer {\n    Some(value: Inner)\n    None\n}\n",
        );
        let outer = ty(&mut program, &interner, "Outer");
        let arms = vec![variant(0, vec![variant(0, vec![])]), variant(1, vec![])];
        assert_eq!(
            uncovered(&program, &interner, arms, outer).as_deref(),
            Some(".Some(.Y)")
        );
    }

    #[test]
    fn a_struct_of_bools_is_covered_field_by_field() {
        let (mut program, interner) = program("struct Pair {\n    a: bool\n    b: bool\n}\n");
        let pair = ty(&mut program, &interner, "Pair");
        let only = |a: Shape, b: Shape| Shape::Ctor(Ctor::Only, vec![a, b]);
        let arms = vec![
            only(boolean(true), Shape::Wild),
            only(boolean(false), boolean(true)),
        ];
        let left = uncovered(&program, &interner, arms, pair).expect("one pair is left");
        assert!(left.contains("false") && !left.contains("true"), "{left}");
        let arms = vec![
            only(boolean(true), Shape::Wild),
            only(boolean(false), Shape::Wild),
        ];
        assert_eq!(uncovered(&program, &interner, arms, pair), None);
    }

    /// The range of `ty`'s values from `lo` to `hi`, as a pattern would
    /// write it; an end left off runs to the least or greatest value.
    fn range(program: &Program, ty: Ty, lo: Option<i128>, hi: Option<i128>) -> Shape {
        let (least, greatest) = program.types.order_range(ty).expect("an ordered type");
        let key = |n: i128| {
            program
                .types
                .order_key(ty, n as u128)
                .expect("an ordered type")
        };
        Shape::Ctor(
            Ctor::Range(lo.map_or(least, key), hi.map_or(greatest, key)),
            Vec::new(),
        )
    }

    fn value(program: &Program, ty: Ty, n: i128) -> Shape {
        range(program, ty, Some(n), Some(n))
    }

    #[test]
    fn numbers_are_covered_by_ranges_that_meet() {
        let (program, interner) = program("");
        let i64 = Types::I64;
        let arms = vec![
            range(&program, i64, None, Some(-1)),
            value(&program, i64, 0),
            range(&program, i64, Some(1), None),
        ];
        assert_eq!(uncovered(&program, &interner, arms, i64), None);
        let arms = vec![
            range(&program, Types::U8, Some(0), Some(127)),
            range(&program, Types::U8, Some(128), Some(255)),
        ];
        assert_eq!(uncovered(&program, &interner, arms, Types::U8), None);
    }

    #[test]
    fn what_no_range_covers_is_written_as_one() {
        let (program, interner) = program("");
        let i64 = Types::I64;
        let left = |arms| uncovered(&program, &interner, arms, i64);
        let arms = vec![range(&program, i64, None, Some(9))];
        assert_eq!(left(arms).as_deref(), Some("10.."));
        let arms = vec![range(&program, i64, Some(0), None)];
        assert_eq!(left(arms).as_deref(), Some("..0"));
        let arms = vec![
            range(&program, i64, None, Some(-1)),
            range(&program, i64, Some(3), None),
        ];
        assert_eq!(left(arms).as_deref(), Some("0..=2"));
        let arms = vec![value(&program, i64, 1), value(&program, i64, 2)];
        assert_eq!(left(arms).as_deref(), Some("..1"));
        let arms = vec![Shape::Wild];
        assert_eq!(left(arms), None);
    }

    #[test]
    fn a_character_range_skips_the_surrogates() {
        let (program, interner) = program("");
        let char = Types::CHAR;
        let arms = vec![
            range(&program, char, None, Some(0xD7FF)),
            range(&program, char, Some(0xE000), None),
        ];
        assert_eq!(uncovered(&program, &interner, arms, char), None);
        let arms = vec![range(&program, char, None, Some('a' as i128))];
        assert_eq!(
            uncovered(&program, &interner, arms, char).as_deref(),
            Some("'b'..")
        );
    }

    fn length(n: u64) -> Shape {
        Shape::Ctor(Ctor::Length(n), vec![Shape::Wild; n as usize])
    }

    fn at_least(prefix: u64, suffix: u64) -> Shape {
        Shape::Ctor(
            Ctor::AtLeast(prefix, suffix),
            vec![Shape::Wild; (prefix + suffix) as usize],
        )
    }

    #[test]
    fn a_slice_is_covered_by_lengths_and_what_is_longer() {
        let (mut program, interner) = program("");
        let slice = program.types.intern(TyKind::Slice(Types::I64));
        // `[]`, `[x]`, `[first, ..rest]`.
        let arms = vec![length(0), length(1), at_least(1, 0)];
        assert_eq!(uncovered(&program, &interner, arms, slice), None);
        // `[]`, `[x]`: two or more are left.
        let arms = vec![length(0), length(1)];
        assert_eq!(
            uncovered(&program, &interner, arms, slice).as_deref(),
            Some("[_, _, ..]")
        );
        // `[first, ..]` and `[.., last]` both need one: none is left.
        let arms = vec![at_least(1, 0), at_least(0, 1)];
        assert_eq!(
            uncovered(&program, &interner, arms, slice).as_deref(),
            Some("[]")
        );
    }

    #[test]
    fn a_slice_pattern_inside_a_longer_one_above_is_unreachable() {
        let (mut program, _) = program("");
        let slice = program.types.intern(TyKind::Slice(Types::I64));
        // `[..]` above `[a, ..]`, and `[a, ..]` above `[a, b, c]`.
        assert!(
            useful(
                &program,
                &[vec![at_least(0, 0)]],
                &[at_least(1, 0)],
                &[slice]
            )
            .is_none()
        );
        assert!(useful(&program, &[vec![at_least(1, 0)]], &[length(3)], &[slice]).is_none());
        // `[a, b]` above `[a, ..]` leaves it one and three and more.
        assert!(useful(&program, &[vec![length(2)]], &[at_least(1, 0)], &[slice]).is_some());
    }

    #[test]
    fn an_array_has_its_one_length() {
        let (mut program, interner) = program("");
        let array = program.types.intern(TyKind::Array(Types::BOOL, 2));
        let pair = |a: Shape, b: Shape| Shape::Ctor(Ctor::Length(2), vec![a, b]);
        let arms = vec![pair(boolean(true), Shape::Wild), at_least(0, 1)];
        assert_eq!(uncovered(&program, &interner, arms, array), None);
        let arms = vec![pair(boolean(true), Shape::Wild)];
        assert_eq!(
            uncovered(&program, &interner, arms, array).as_deref(),
            Some("[false, false]")
        );
    }

    #[test]
    fn a_range_inside_one_above_is_unreachable() {
        let (program, _) = program("");
        let char = Types::CHAR;
        let letters = range(&program, char, Some('a' as i128), Some('z' as i128));
        let hex = range(&program, char, Some('a' as i128), Some('f' as i128));
        let upper = range(&program, char, Some('A' as i128), Some('Z' as i128));
        assert!(useful(&program, &[vec![letters.clone()]], &[hex], &[char]).is_none());
        let overlapping = range(&program, char, Some('X' as i128), Some('c' as i128));
        let above = vec![vec![letters], vec![upper]];
        assert!(useful(&program, &above, &[overlapping], &[char]).is_some());
    }
}
