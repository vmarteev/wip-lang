//! A `bool` that C hands over, read as C reads one.
//!
//! Wip writes a `bool` as 0 or 1, and its `!` and `==` count on that. A byte
//! C wrote may be anything: a struct filled by `fread`, a field C left
//! unset, a `uint8_t` flag declared as `bool`. Read as it is, a 2 would be
//! true, and its `!` true too, and an `Option<bool>` holding it would be
//! empty. So where a `bool` comes from C, it is compared with zero, as C
//! converts a byte to `_Bool`: zero is false and anything else is true.
//!
//! A `bool` comes from C where a declaration of C's says so:
//!
//! - a field of an `extern struct` or an `extern union`, or an element of a
//!   `bool` array in one, wherever the value lies: it is converted where it
//!   is read, and lent converted;
//! - what a C function answers, and what a C variable's getter answers,
//!   converted after the call; and what a function value answers, since the
//!   function may be C's;
//! - a parameter of a function C may call: one exported to C, and one the
//!   program uses as a value, which may be handed to C as a function
//!   pointer, converted where the function begins.
//!
//! What a `ptr<bool>` points at is not converted: a pointer reaches what the
//! program says it does, a `Vec<bool>`'s elements among them.

use crate::*;

/// Converts every `bool` of `body` that comes from C, `id` being the
/// function it is the body of.
pub fn convert_c_bools(program: &Program, id: FnId, body: &mut Body) {
    let def = &program.fns[id];
    let entered_by_c = def.exports_c || program.fn_values.contains(&id);
    let mut c = Converter { program, body };
    if entered_by_c {
        c.parameters();
    }
    for index in 0..c.body.blocks.len() {
        c.block(index);
    }
}

struct Converter<'a> {
    program: &'a Program,
    body: &'a mut Body,
}

impl Converter<'_> {
    /// Each `bool` parameter, compared with zero before anything reads it.
    fn parameters(&mut self) {
        let converted: Vec<Statement> = self
            .body
            .params
            .iter()
            .filter(|&&param| self.body.local(param).ty == Types::BOOL)
            .map(|&param| in_place(Place::local(param)))
            .collect();
        let entry = &mut self.body.blocks[0].statements;
        entry.splice(0..0, converted);
    }

    fn block(&mut self, index: usize) {
        let statements = std::mem::take(&mut self.body.blocks[index].statements);
        let mut out = Vec::with_capacity(statements.len());
        for mut statement in statements {
            self.statement(&mut statement, &mut out);
            let after = self.after_call(&statement);
            out.push(statement);
            out.extend(after);
        }
        let mut terminator = std::mem::replace(
            &mut self.body.blocks[index].terminator,
            Terminator::Unreachable,
        );
        match &mut terminator {
            Terminator::Branch { cond: o, .. }
            | Terminator::Switch { value: o, .. }
            | Terminator::Panic { note: Some(o), .. } => self.operand(o, &mut out),
            _ => {}
        }
        let block = &mut self.body.blocks[index];
        block.statements = out;
        block.terminator = terminator;
    }

    /// The reads of C's `bool`s in `statement`, each made a read of a
    /// temporary converted before it, in `out`.
    fn statement(&mut self, statement: &mut Statement, out: &mut Vec<Statement>) {
        match statement {
            Statement::Assign(dest, Rvalue::AddressOf(place)) if self.is_c_bool(place) => {
                let dest_ty = place_ty(self.program, self.body, dest);
                match self.program.types.kind(dest_ty) {
                    // Lent to be written: converted where it lies, so that
                    // what reads it through the reference reads 0 or 1.
                    TyKind::Ref(_, wip_hir::RefKind::Var) | TyKind::Ptr(_) => {
                        out.push(in_place(place.clone()));
                    }
                    // Lent to be read: a converted copy is lent, and C's
                    // memory, which may be read-only, is left alone.
                    _ => {
                        let copy = self.converted(place.clone(), out);
                        *place = copy;
                    }
                }
            }
            Statement::Assign(_, rvalue) => self.rvalue(rvalue, out),
            Statement::Arith { lhs, rhs, .. } => {
                self.operand(lhs, out);
                self.operand(rhs, out);
            }
            Statement::Call { callee, args, .. } => {
                if let Callee::Value(o) = callee {
                    self.operand(o, out);
                }
                for arg in args {
                    self.operand(arg, out);
                }
            }
            Statement::AllocBuffer { count, .. } => self.operand(count, out),
            Statement::Atomic {
                address,
                value,
                expected,
                ..
            } => {
                self.operand(address, out);
                for o in [value, expected].into_iter().flatten() {
                    self.operand(o, out);
                }
            }
            Statement::Free(o)
            | Statement::DropFn { ptr: o, .. }
            | Statement::DropInPlace { ptr: o, .. } => self.operand(o, out),
            Statement::Check { fails, kind, .. } => {
                self.operand(fails, out);
                match kind {
                    CheckKind::Bounds { index, length } => {
                        self.operand(index, out);
                        self.operand(length, out);
                    }
                    CheckKind::Length { length } => self.operand(length, out),
                    CheckKind::Division | CheckKind::Overflow(_) => {}
                }
            }
            Statement::SetVariant(..)
            | Statement::Zero(_)
            | Statement::Poison(_)
            | Statement::CheckMoved { .. }
            | Statement::Alloc { .. }
            | Statement::At(_) => {}
        }
    }

    fn rvalue(&mut self, rvalue: &mut Rvalue, out: &mut Vec<Statement>) {
        match rvalue {
            Rvalue::Use(o)
            | Rvalue::Unary(_, o)
            | Rvalue::Cast(o, _)
            | Rvalue::CstrLen(o)
            | Rvalue::Float(_, o)
            | Rvalue::Integer(_, o)
            | Rvalue::Bits(o)
            | Rvalue::VTableFn { table: o, .. } => self.operand(o, out),
            Rvalue::Binary(_, l, r) | Rvalue::Rotate(_, l, r) | Rvalue::Overflows(_, l, r) => {
                self.operand(l, out);
                self.operand(r, out);
            }
            Rvalue::MulAdd(first, second, third) => {
                self.operand(first, out);
                self.operand(second, out);
                self.operand(third, out);
            }
            // A `bool` is lent by the statement, above; an enum's variant is
            // not a `bool`.
            Rvalue::AddressOf(_) | Rvalue::Variant(_) => {}
        }
    }

    /// A read of C's `bool`, made a read of the converted value.
    fn operand(&mut self, operand: &mut Operand, out: &mut Vec<Statement>) {
        if let Operand::Copy(place) = operand
            && self.is_c_bool(place)
        {
            let copy = self.converted(place.clone(), out);
            *operand = Operand::Copy(copy);
        }
    }

    /// A temporary holding `place` compared with zero, assigned in `out`.
    fn converted(&mut self, place: Place, out: &mut Vec<Statement>) -> Place {
        let temp = Local(self.body.locals.len() as u32);
        self.body.locals.push(LocalDecl {
            ty: Types::BOOL,
            kind: LocalKind::Temp,
            source: None,
        });
        out.push(Statement::Assign(Place::local(temp), nonzero(place)));
        Place::local(temp)
    }

    /// What a call answers, converted where C may have answered it.
    fn after_call(&self, statement: &Statement) -> Option<Statement> {
        let Statement::Call {
            callee,
            dest: Some(dest),
            ..
        } = statement
        else {
            return None;
        };
        let from_c = match callee {
            Callee::Fn(id) => {
                let def = &self.program.fns[*id];
                def.is_extern || def.accesses.is_some()
            }
            Callee::Value(_) => true,
            Callee::StrCmp => false,
        };
        (from_c && place_ty(self.program, self.body, dest) == Types::BOOL)
            .then(|| in_place(dest.clone()))
    }

    /// Whether `place` is a `bool` C declares: a field of an `extern
    /// struct` or an `extern union`, or an element of a `bool` array that
    /// is one.
    fn is_c_bool(&self, place: &Place) -> bool {
        if place_ty(self.program, self.body, place) != Types::BOOL {
            return false;
        }
        let elements = place
            .projections
            .iter()
            .rev()
            .take_while(|p| matches!(p, Projection::Index(_) | Projection::ConstIndex(_)))
            .count();
        let Some(end) = place.projections.len().checked_sub(elements + 1) else {
            return false;
        };
        if !matches!(place.projections[end], Projection::Field(_)) {
            return false;
        }
        let owner = Place {
            local: place.local,
            projections: place.projections[..end].to_vec(),
        };
        let owner_ty = place_ty(self.program, self.body, &owner);
        matches!(
            self.program.types.kind(owner_ty),
            TyKind::Struct(id, _) if self.program.structs[id].is_extern
        )
    }
}

/// `place != false`: zero is false, and any other byte true.
fn nonzero(place: Place) -> Rvalue {
    Rvalue::Binary(
        BinaryOp::Ne,
        Operand::Copy(place),
        Operand::Const(Const::Bool(false)),
    )
}

/// `place = place != false`.
fn in_place(place: Place) -> Statement {
    Statement::Assign(place.clone(), nonzero(place))
}
