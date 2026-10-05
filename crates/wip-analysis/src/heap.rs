//! Whether a place lies on the heap: behind an `own`, in
//! a block of slots or a buffer, or lent by a projection that lends from
//! one. A generator that holds the value such a place is part of may be
//! moved while it waits, and the place stays where it is, so a loop may
//! walk it across a `yield`. What lies in the value itself moves with it.

use super::*;
use wip_hir::FnId;

/// How many projections deep the answer is looked for: one that forwards
/// to another, as a map's `items` does to its vector's.
const DEPTH: u32 = 8;

impl Checker<'_> {
    /// Whether the elements `id` names lie on the heap.
    pub(super) fn on_heap(&self, id: ExprId) -> bool {
        heap(self.program, self.body, id, DEPTH)
    }
}

fn heap(program: &Program, body: &Body, id: ExprId, depth: u32) -> bool {
    let kind = |e: ExprId| program.types.kind(body.exprs[e].ty);
    let buffer = |e: ExprId| {
        matches!(kind(e), TyKind::Slots(_))
            || matches!(kind(e), TyKind::Own(inner) if matches!(program.types.kind(inner), TyKind::Slice(_)))
    };
    // A buffer's elements are where its pointer points.
    if buffer(id) {
        return true;
    }
    match &body.exprs[id].kind {
        ExprKind::Deref(inner) => match kind(*inner) {
            TyKind::Own(_) => true,
            // Where a projection lends from.
            _ => heap(program, body, *inner, depth),
        },
        ExprKind::Field { base, .. } => heap(program, body, *base, depth),
        ExprKind::Index { base, .. } | ExprKind::SubSlice { base, .. } => {
            buffer(*base) || heap(program, body, *base, depth)
        }
        ExprKind::Call { callee, .. } => depth > 0 && lends_from_heap(program, *callee, depth - 1),
        _ => false,
    }
}

/// Whether every place a projection lends lies on the heap.
fn lends_from_heap(program: &Program, callee: FnId, depth: u32) -> bool {
    let def = &program.fns[callee];
    // A projection's place is its reference result's.
    if !matches!(program.types.kind(def.ret), TyKind::Ref(..)) {
        return false;
    }
    let Some(body) = &def.body else {
        return false;
    };
    let mut lent = body
        .exprs
        .iter()
        .filter_map(|(_, e)| match e.kind {
            ExprKind::Lend(place) => Some(place),
            _ => None,
        })
        .peekable();
    lent.peek().is_some() && lent.all(|place| heap(program, body, place, depth))
}
