//! A generator's frame: the locals of its `next` live in
//! the generator, after its declared fields, so that each call finds them
//! where the last one left them.
//!
//! Every local but the generator itself and the result is kept there, and
//! every place of the body that names one is rewritten to reach it
//! through the generator: `_3.1` becomes `(*_0).4.1`. An index names a
//! local, not a place, so a kept index is copied into a local of its own
//! just before the statement that reads it.

use rustc_hash::FxHashMap;

use crate::*;

/// Which field of the generator each local of `body` is kept in, counted
/// from `base`, its first field after the declared ones: `None` for the
/// generator itself and the result, which stay where they are.
fn fields(body: &Body, base: u32) -> Vec<Option<u32>> {
    let mut next = base;
    (0..body.locals.len() as u32)
        .map(|i| {
            let local = Local(i);
            if body.params.contains(&local) || body.ret == Some(local) {
                return None;
            }
            next += 1;
            Some(next - 1)
        })
        .collect()
}

/// The types of what a generator keeps between calls: its frame, in the
/// order of its fields.
pub(crate) fn frame_tys(body: &Body) -> Vec<Ty> {
    fields(body, 0)
        .iter()
        .zip(&body.locals)
        .filter(|(field, _)| field.is_some())
        .map(|(_, decl)| decl.ty)
        .collect()
}

/// Moves the locals of a generator's `next` into the generator, whose
/// declared fields are the first `base`.
pub(crate) fn keep_in_frame(body: &mut Body, base: u32) {
    let fields = fields(body, base);
    let this = body.params[0];
    let mut rewrite = Rewrite {
        this,
        fields,
        shadows: FxHashMap::default(),
        locals: std::mem::take(&mut body.locals),
    };
    for block in &mut body.blocks {
        let mut statements = Vec::with_capacity(block.statements.len());
        for mut statement in std::mem::take(&mut block.statements) {
            let mut copies = Vec::new();
            rewrite.statement(&mut statement, &mut copies);
            statements.extend(copies);
            statements.push(statement);
        }
        let mut copies = Vec::new();
        rewrite.terminator(&mut block.terminator, &mut copies);
        statements.extend(copies);
        block.statements = statements;
    }
    body.locals = rewrite.locals;
}

struct Rewrite {
    this: Local,
    fields: Vec<Option<u32>>,
    /// For each kept local that is read as an index, the local its value
    /// is copied into first.
    shadows: FxHashMap<Local, Local>,
    locals: Vec<LocalDecl>,
}

impl Rewrite {
    /// Where a kept local is: a field of the generator.
    fn kept(&self, local: Local) -> Option<Place> {
        let field = self.fields.get(local.0 as usize).copied().flatten()?;
        Some(
            Place::local(self.this)
                .project(Projection::Deref)
                .project(Projection::Field(field)),
        )
    }

    fn place(&mut self, place: &mut Place, copies: &mut Vec<Statement>) {
        for projection in &mut place.projections {
            if let Projection::Index(index) = projection
                && let Some(kept) = self.kept(*index)
            {
                let shadow = match self.shadows.get(index) {
                    Some(&shadow) => shadow,
                    None => {
                        let ty = self.locals[index.0 as usize].ty;
                        self.locals.push(LocalDecl::unnamed(ty, LocalKind::Temp));
                        let shadow = Local(self.locals.len() as u32 - 1);
                        self.shadows.insert(*index, shadow);
                        shadow
                    }
                };
                copies.push(Statement::Assign(
                    Place::local(shadow),
                    Rvalue::Use(Operand::Copy(kept)),
                ));
                *index = shadow;
            }
        }
        if let Some(mut kept) = self.kept(place.local) {
            kept.projections.append(&mut place.projections);
            *place = kept;
        }
    }

    fn operand(&mut self, operand: &mut Operand, copies: &mut Vec<Statement>) {
        if let Operand::Copy(place) = operand {
            self.place(place, copies);
        }
    }

    fn rvalue(&mut self, rvalue: &mut Rvalue, copies: &mut Vec<Statement>) {
        match rvalue {
            Rvalue::Use(operand)
            | Rvalue::Unary(_, operand)
            | Rvalue::Cast(operand, _)
            | Rvalue::CstrLen(operand)
            | Rvalue::Float(_, operand)
            | Rvalue::Integer(_, operand)
            | Rvalue::Bits(operand)
            | Rvalue::VTableFn { table: operand, .. } => self.operand(operand, copies),
            Rvalue::Binary(_, lhs, rhs) | Rvalue::Rotate(_, lhs, rhs) => {
                self.operand(lhs, copies);
                self.operand(rhs, copies);
            }
            Rvalue::MulAdd(first, second, third) => {
                self.operand(first, copies);
                self.operand(second, copies);
                self.operand(third, copies);
            }
            Rvalue::AddressOf(place) | Rvalue::Variant(place) => self.place(place, copies),
        }
    }

    fn statement(&mut self, statement: &mut Statement, copies: &mut Vec<Statement>) {
        match statement {
            Statement::Assign(place, rvalue) => {
                self.rvalue(rvalue, copies);
                self.place(place, copies);
            }
            Statement::SetVariant(place, _)
            | Statement::Zero(place)
            | Statement::Poison(place)
            | Statement::CheckMoved { place, .. } => self.place(place, copies),
            Statement::Arith { dest, lhs, rhs, .. } => {
                self.operand(lhs, copies);
                self.operand(rhs, copies);
                self.place(dest, copies);
            }
            Statement::Call { callee, args, dest } => {
                if let Callee::Value(operand) = callee {
                    self.operand(operand, copies);
                }
                for arg in args {
                    self.operand(arg, copies);
                }
                if let Some(dest) = dest {
                    self.place(dest, copies);
                }
            }
            Statement::Alloc { dest, .. } => self.place(dest, copies),
            Statement::AllocBuffer { dest, count, .. } => {
                self.operand(count, copies);
                self.place(dest, copies);
            }
            Statement::Atomic {
                address,
                value,
                expected,
                dest,
                ..
            } => {
                self.operand(address, copies);
                if let Some(value) = value {
                    self.operand(value, copies);
                }
                if let Some(expected) = expected {
                    self.operand(expected, copies);
                }
                if let Some(dest) = dest {
                    self.place(dest, copies);
                }
            }
            Statement::Free(operand)
            | Statement::DropFn { ptr: operand, .. }
            | Statement::DropInPlace { ptr: operand, .. } => self.operand(operand, copies),
            Statement::At(_) => {}
            Statement::Check { fails, kind, .. } => {
                self.operand(fails, copies);
                match kind {
                    CheckKind::Bounds { index, length } => {
                        self.operand(index, copies);
                        self.operand(length, copies);
                    }
                    CheckKind::Length { length } => self.operand(length, copies),
                    CheckKind::Division | CheckKind::Overflow(_) => {}
                }
            }
        }
    }

    fn terminator(&mut self, terminator: &mut Terminator, copies: &mut Vec<Statement>) {
        match terminator {
            Terminator::Branch { cond, .. } => self.operand(cond, copies),
            Terminator::Switch { value, .. } => self.operand(value, copies),
            Terminator::Panic {
                note: Some(note), ..
            } => self.operand(note, copies),
            Terminator::Goto(_)
            | Terminator::Return
            | Terminator::Unreachable
            | Terminator::Panic { .. } => {}
        }
    }
}
