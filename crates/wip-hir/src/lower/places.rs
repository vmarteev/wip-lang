//! Places: assignment, what may be written or borrowed as `&var`, fields
//! and indexing.

use super::*;

impl<'a> Lowerer<'a> {
    pub(super) fn place_root(&self, id: ExprId) -> PlaceRoot {
        match self.state.body.exprs[id].kind {
            ExprKind::Local(local) => PlaceRoot::Local(local),
            // `p[i]`: C's memory, reached through the pointer the place
            // holds.
            ExprKind::Index { base, .. }
                if matches!(self.kind(self.ty_of(base)), TyKind::Ptr(_)) =>
            {
                PlaceRoot::ThroughPointer
            }
            ExprKind::Field { base, .. }
            | ExprKind::Index { base, .. }
            | ExprKind::SubSlice { base, .. } => self.place_root(base),
            ExprKind::Deref(inner) => match self.kind(self.ty_of(inner)) {
                _ if let ExprKind::Table(id) = self.state.body.exprs[inner].kind => {
                    PlaceRoot::Table(id)
                }
                TyKind::Ref(_, crate::RefKind::Shared) => PlaceRoot::ThroughRef,
                TyKind::Ref(_, crate::RefKind::Var) => PlaceRoot::ThroughVarRef,
                TyKind::Ptr(_) => PlaceRoot::ThroughPointer,
                _ => self.place_root(inner),
            },
            _ => PlaceRoot::NotAPlace,
        }
    }

    /// Whether `place` is a value made where it is used, not a place that
    /// holds one: a call's answer, a literal, or a part of either. A `var
    /// fn` may be called on one, which it changes and which then ends. A
    /// variable C owns is read as a copy, and is not one: a change to the
    /// copy would leave C's as it was.
    pub(super) fn made_here(&self, place: ExprId) -> bool {
        if !matches!(self.place_root(place), PlaceRoot::NotAPlace) {
            return false;
        }
        let mut e = place;
        loop {
            match &self.state.body.exprs[e].kind {
                ExprKind::Field { base, .. }
                | ExprKind::Index { base, .. }
                | ExprKind::SubSlice { base, .. }
                | ExprKind::Deref(base) => e = *base,
                ExprKind::Len(_) | ExprKind::Error => return false,
                ExprKind::Call { callee, .. } => {
                    return !matches!(self.program.fns[*callee].accesses, Some(Access::Global(_)));
                }
                _ => return true,
            }
        }
    }

    /// Whether `place` may be assigned, or borrowed as `&var`.
    pub(super) fn writable(&self, place: ExprId) -> bool {
        // A field of another module's struct is written only where it
        // says `pub var`.
        if self.readonly_step(place).is_some() {
            return false;
        }
        match self.place_root(place) {
            PlaceRoot::Local(local) => self.state.body.locals[local].mutable(),
            PlaceRoot::ThroughVarRef | PlaceRoot::ThroughPointer => true,
            PlaceRoot::ThroughRef | PlaceRoot::Table(_) | PlaceRoot::NotAPlace => false,
        }
    }

    /// The field of this place that another module may read and not
    /// write, if it goes through one.
    pub(super) fn readonly_step(&self, id: ExprId) -> Option<(crate::StructId, usize, Span)> {
        match self.state.body.exprs[id].kind {
            ExprKind::Field { base, index } => {
                let ty = self.under_refs(self.ty_of(base));
                if let TyKind::Struct(owner, _) = self.kind(ty)
                    && !self.field_settable(owner, index as usize)
                {
                    return Some((owner, index as usize, self.state.body.exprs[id].span));
                }
                self.readonly_step(base)
            }
            ExprKind::Index { base, .. }
            | ExprKind::SubSlice { base, .. }
            | ExprKind::Deref(base) => self.readonly_step(base),
            _ => None,
        }
    }

    /// The `match` binding that `place` is reached through, if any. A binding
    /// of a matched place is a read-only alias.
    fn through_binding(&self, id: ExprId) -> Option<crate::LocalId> {
        match self.state.body.exprs[id].kind {
            ExprKind::Field { base, .. }
            | ExprKind::Index { base, .. }
            | ExprKind::SubSlice { base, .. } => self.through_binding(base),
            ExprKind::Deref(inner) => match self.state.body.exprs[inner].kind {
                ExprKind::Local(local) => {
                    (self.state.body.locals[local].kind == LocalKind::Binding).then_some(local)
                }
                _ => self.through_binding(inner),
            },
            _ => None,
        }
    }

    /// Reports a place that cannot be written, after giving a lending
    /// accessor the chance to be its writing twin.
    pub(super) fn require_writable(&mut self, place: ExprId, how: Writing) {
        // A `&var` closure keeps a `&var` to what it writes, and a `&` to
        // the rest.
        self.note_capture_written(place);
        // A `pub` field of another module's struct is read out here and
        // written where it is declared.
        if let Some((owner, index, span)) = self.readonly_step(place) {
            let what = match how {
                Writing::Assign => "assigned here",
                Writing::Borrow => "borrowed `&var` here",
            };
            self.readonly_field(owner, index, span, what);
            return;
        }
        if self.writable_for_writing(place) {
            return;
        }
        if let Some(diagnostic) = self.not_writable(place, how) {
            self.report(diagnostic);
        }
    }

    /// Whether a place may be written, giving a lending accessor the chance
    /// to be its writing twin first.
    pub(super) fn writable_for_writing(&mut self, place: ExprId) -> bool {
        self.writable(place) || (self.retarget_to_writer(place) && self.writable(place))
    }

    /// The call that lends a place, if one does.
    pub(super) fn lending_call(&self, place: ExprId) -> Option<ExprId> {
        let mut e = place;
        loop {
            match self.state.body.exprs[e].kind {
                ExprKind::Field { base, .. }
                | ExprKind::Index { base, .. }
                | ExprKind::SubSlice { base, .. } => e = base,
                ExprKind::Deref(base) => {
                    if matches!(self.state.body.exprs[base].kind, ExprKind::Call { .. }) {
                        return Some(base);
                    }
                    e = base;
                }
                _ => return None,
            }
        }
    }

    /// A place lent by an accessor that reads, where one that writes is
    /// needed: the writing half of the pair takes its place, and the
    /// receiver it was called on is lent `&var` instead.
    fn retarget_to_writer(&mut self, place: ExprId) -> bool {
        match self.lending_call(place) {
            Some(call) => self.swap_in_writer(call),
            None => false,
        }
    }

    /// An accessor that lends for reading where a write is needed: the
    /// writing half of the pair is missing, and the message says so.
    fn no_writing_twin(&self, place: ExprId, how: Writing) -> Option<Diagnostic> {
        let call = self.lending_call(place)?;
        let ExprKind::Call { callee, .. } = self.state.body.exprs[call].kind else {
            return None;
        };
        let def = &self.program.fns[callee];
        if def.receiver != Some(Receiver::Read) || self.writing_twin(callee).is_some() {
            return None;
        }
        let owner = self.type_name(def.owner?);
        let name = self.text(def.name).to_string();
        let params: Vec<String> = def
            .params
            .iter()
            .skip(1)
            .map(|p| {
                format!(
                    "{}: {}",
                    self.text(p.name),
                    self.program.ty_name(p.ty, self.interner)
                )
            })
            .collect();
        let lends = match self.kind(def.ret) {
            TyKind::Ref(pointee, _) => self.program.ty_name(pointee, self.interner),
            _ => return None,
        };
        let code = match how {
            Writing::Assign => codes::INVALID_ASSIGNMENT,
            Writing::Borrow => codes::CANNOT_BORROW_VAR,
        };
        let diagnostic = Diagnostic::error(
            code,
            format!("`{owner}::{name}` lends for reading"),
            self.state.body.exprs[place].span,
            "cannot be written through",
        );
        // A type of the program's own can be given the writing half; one of
        // another module's is read-only because that module says so, as
        // a set's keys are.
        let diagnostic = match def.module as usize == self.current {
            true => diagnostic.with_help(format!(
                "declare `var fn {name}({}): &var {lends}` beside it",
                params.join(", ")
            )),
            false => diagnostic.with_help(format!(
                "`{owner}` lends what `{name}` finds for reading only; its own methods are how it is changed"
            )),
        };
        Some(diagnostic.with_note(
            "one name is the accessor that reads and the one that writes, and where the call sits says which",
        ))
    }

    /// The call itself: its callee becomes the writing twin, and what it
    /// lends becomes `&var`.
    pub(super) fn swap_in_writer(&mut self, call: ExprId) -> bool {
        let ExprKind::Call {
            callee, ref args, ..
        } = self.state.body.exprs[call].kind
        else {
            return false;
        };
        let receiver = args.first().copied();
        let Some(writer) = self.writing_twin(callee) else {
            return false;
        };
        // The receiver is lent for the call; it must be writable itself,
        // and may be lent by an accessor in turn.
        if let Some(arg) = receiver {
            let ExprKind::Ref(inner) = self.state.body.exprs[arg].kind else {
                return false;
            };
            if !self.writable(inner) && !self.retarget_to_writer(inner) {
                return false;
            }
            if !self.writable(inner) {
                return false;
            }
            let inner_ty = self.ty_of(inner);
            let lent = self.intern(TyKind::Ref(inner_ty, crate::RefKind::Var));
            self.state.body.exprs[arg].ty = lent;
        }
        // What the call lends is the same place, written rather than read.
        let TyKind::Ref(pointee, crate::RefKind::Shared) = self.kind(self.ty_of(call)) else {
            return false;
        };
        let lent = self.intern(TyKind::Ref(pointee, crate::RefKind::Var));
        self.state.body.exprs[call].ty = lent;
        if let ExprKind::Call { callee, .. } = &mut self.state.body.exprs[call].kind {
            *callee = writer;
        }
        true
    }

    /// Why `place` cannot be assigned, or borrowed as `&var`, if it cannot.
    pub(super) fn not_writable(&self, place: ExprId, how: Writing) -> Option<Diagnostic> {
        if let Some(diagnostic) = self.no_writing_twin(place, how) {
            return Some(diagnostic);
        }
        // Lent by an accessor that has a writing twin, which needs the
        // container lent `&var`: why the container cannot be is the reason,
        // `v[0] = 3` with `val v`.
        if let Some(call) = self.lending_call(place)
            && let ExprKind::Call {
                callee, ref args, ..
            } = self.state.body.exprs[call].kind
            && self.writing_twin(callee).is_some()
            && let Some(&arg) = args.first()
            && let ExprKind::Ref(container) = self.state.body.exprs[arg].kind
            && !self.writable(container)
        {
            return self.not_writable(container, Writing::Borrow);
        }
        let span = self.state.body.exprs[place].span;
        let code = match how {
            Writing::Assign => codes::INVALID_ASSIGNMENT,
            Writing::Borrow => codes::CANNOT_BORROW_VAR,
        };
        match self.place_root(place) {
            PlaceRoot::Local(local) if self.state.body.locals[local].mutable() => None,
            PlaceRoot::ThroughVarRef | PlaceRoot::ThroughPointer => None,
            PlaceRoot::Table(id) => {
                let name = self.text(self.program.consts[id].name);
                let message = match how {
                    Writing::Assign => format!("cannot assign to `{name}`, a constant"),
                    Writing::Borrow => format!("cannot borrow `{name}`, a constant, as `&var`"),
                };
                Some(
                    Diagnostic::error(code, message, span, "a constant cannot change")
                        .with_secondary(self.program.consts[id].span, "declared here")
                        .with_note("a top-level `val` is kept once, in memory nothing writes")
                        .with_help(format!(
                            "to change a copy of it, copy it first: `var copy = {name}`"
                        )),
                )
            }
            PlaceRoot::Local(local) => {
                let copied = self.state.body.copied_bindings.contains(&local);
                let local = &self.state.body.locals[local];
                let name = self.text(local.name);
                let (message, label) = match how {
                    Writing::Assign => (format!("cannot assign to `{name}`"), "cannot be assigned"),
                    Writing::Borrow => (
                        format!("cannot borrow `{name}` as `&var`"),
                        "cannot be written",
                    ),
                };
                let diagnostic = Diagnostic::error(code, message, span, label);
                Some(match local.kind {
                    LocalKind::Let { keyword } => diagnostic
                        .with_secondary(local.span, "declared with `val`, so it cannot change")
                        .with_fix("declare it with `var`", [Edit::replace(keyword, "var")]),
                    LocalKind::Param => diagnostic
                        .with_secondary(local.span, "a parameter")
                        .with_help(format!("a parameter cannot change: copy it first (`var {name} = {name}`), or declare it `&var` so that the caller's variable changes")),
                    // A `val … else` binding of plain data holds a copy,
                    // so a write through it would go nowhere.
                    _ if copied => diagnostic
                        .with_secondary(local.span, "a copy, taken by the `val`")
                        .with_help(
                            "a `val … else` binding of plain data is a copy: take the value apart with `match`, whose bindings are the place itself",
                        ),
                    _ => diagnostic
                        .with_secondary(local.span, "bound by a pattern or a loop")
                        .with_help("the bindings of patterns and loops cannot be assigned"),
                })
            }
            PlaceRoot::ThroughRef => {
                // What an owned closure captured, which it only reads.
                if let ExprKind::Field { base, index } = self.state.body.exprs[place].kind
                    && let TyKind::Struct(id, _) = self.kind(self.ty_of(base))
                    && self.program.structs[id].is_env
                {
                    // The environment's fields are only filled in once the
                    // lambda's body has been checked, so the name comes from
                    // what it has captured so far: the drop function is
                    // before them.
                    let captured = self
                        .state
                        .captures
                        .get(index as usize - 1)
                        .map(|capture| capture.name);
                    let name = match captured {
                        Some(sym) => self.text(sym).to_string(),
                        None => "what this closure captured".to_string(),
                    };
                    let message = match how {
                        Writing::Assign => {
                            format!("cannot assign to `{name}`, which this closure captured")
                        }
                        Writing::Borrow => {
                            format!(
                                "cannot borrow `{name}`, which this closure captured, as `&var`"
                            )
                        }
                    };
                    return Some(
                        Diagnostic::error(code, message, span, "captured by value")
                            .with_note(
                                "an owned closure holds what it captured and reads it; whether it may change it is left for later",
                            )
                            .with_help("take what changes as a parameter of the closure"),
                    );
                }
                // What a closure that reads captured: one lent as `&(…) =>
                // R`, or kept in a `val`.
                if let ExprKind::Deref(field) = self.state.body.exprs[place].kind
                    && let ExprKind::Field { base, index } = self.state.body.exprs[field].kind
                    && let TyKind::Struct(id, _) = self.kind(self.ty_of(base))
                    && self.program.structs[id].is_env
                    && self.state.capture_kind
                        == Some(lambdas::CaptureKind::Lent(crate::RefKind::Shared))
                {
                    let name = self
                        .state
                        .captures
                        .get(index as usize)
                        .map_or("what this closure captured".to_string(), |capture| {
                            format!("`{}`", self.text(capture.name))
                        });
                    let message = match how {
                        Writing::Assign => {
                            format!("cannot assign to {name}, which this closure only reads")
                        }
                        Writing::Borrow => format!(
                            "cannot borrow {name}, which this closure only reads, as `&var`"
                        ),
                    };
                    return Some(
                        Diagnostic::error(code, message, span, "captured to read")
                            .with_note("a closure lent as `&(…) => R`, or kept in a `val`, reads what it captures; one that changes it is a `&var (…) => R`, lent for the call it is written in")
                            .with_help("write the lambda where a `&var (…) => R` is taken, or take what changes as a `&var` parameter of it"),
                    );
                }
                if let Some(binding) = self.through_binding(place) {
                    let local = &self.state.body.locals[binding];
                    let name = self.text(local.name);
                    let message = match how {
                        Writing::Assign => "cannot assign through a binding",
                        Writing::Borrow => "cannot borrow through a binding as `&var`",
                    };
                    return Some(
                        Diagnostic::error(
                            code,
                            message,
                            span,
                            format!("`{name}` refers to part of a place that cannot be written"),
                        )
                        .with_secondary(local.span, "bound here")
                        .with_help("bindings of `match` and `for` can be written when what they refer to can: a `var`, or a `&var` parameter"),
                    );
                }
                let message = match how {
                    Writing::Assign => "cannot assign through a `&` reference",
                    Writing::Borrow => "cannot borrow through a `&` reference as `&var`",
                };
                Some(
                    Diagnostic::error(
                        code,
                        message,
                        span,
                        "behind a `&` reference, which only reads",
                    )
                    .with_help(
                        "to write through it, declare the parameter `&var` and pass it with `&var`",
                    ),
                )
            }
            PlaceRoot::NotAPlace => {
                let message = match (how, &self.state.body.exprs[place].kind) {
                    (_, ExprKind::Len(_)) => "the length of an array or string cannot be changed",
                    (Writing::Assign, _) => "cannot assign to this expression",
                    (Writing::Borrow, _) => "`&var` needs a variable, field or element",
                };
                // A variable C owns is read as a copy, so there is nothing
                // here to point at.
                let global = match &self.state.body.exprs[place].kind {
                    ExprKind::Call { callee, .. } => self.program.fns[*callee].accesses,
                    _ => None,
                };
                let diagnostic =
                    Diagnostic::error(code, message, span, "not a variable, field or element");
                Some(match global {
                    Some(Access::Global(id)) => {
                        let name = self.text(self.program.globals[id].name).to_string();
                        diagnostic
                            .with_secondary(self.program.globals[id].span, "declared here")
                            .with_note(format!(
                                "`{name}` is C's variable, read as a copy of what C holds, so Wip has no address for it; pass the copy, or write it back with `{name} = …`"
                            ))
                    }
                    // A field of a struct C lays out, read the same way.
                    Some(Access::Field(id, index)) => {
                        let field = &self.program.structs[id].fields[index as usize];
                        let (field_name, at) = (field.name, field.span);
                        let name = self.text(field_name).to_string();
                        diagnostic.with_secondary(at, "declared here").with_note(format!(
                            "`{name}` is where C says it is, so Wip reads it through C rather than pointing at it; use the value, or write it back with `p.{name} = …`"
                        ))
                    }
                    None => diagnostic,
                })
            }
        }
    }

    /// The reference an assignment to `target` points somewhere else, where
    /// it is one: a `var` holding a `&`, or what a `&var &T` parameter
    /// refers to.
    fn repointed(&mut self, target: ast::ExprId) -> Option<ExprId> {
        let ast::ExprKind::Name(sym) = self.ast.exprs[target].kind else {
            return None;
        };
        let local = self.lookup(sym)?;
        let (kind, ty) = {
            let def = &self.state.body.locals[local];
            (def.kind, def.ty)
        };
        let span = self.ast.exprs[target].span;
        let shared_ref = |this: &Self, ty: Ty| {
            matches!(this.kind(ty), TyKind::Ref(inner, crate::RefKind::Shared)
                if !matches!(this.kind(inner), TyKind::Fn(..)))
        };
        match self.kind(ty) {
            _ if kind == LocalKind::Var && shared_ref(self, ty) => {
                Some(self.alloc(ExprKind::Local(local), ty, span))
            }
            TyKind::Ref(inner, crate::RefKind::Var)
                if kind == LocalKind::Param && shared_ref(self, inner) =>
            {
                let param = self.alloc(ExprKind::Local(local), ty, span);
                Some(self.alloc(ExprKind::Deref(param), inner, span))
            }
            _ => None,
        }
    }

    /// `place = value`, or `place op= value`, whose operands follow the
    /// operator's rules.
    pub(super) fn assign(
        &mut self,
        target: ast::ExprId,
        op: Option<(BinaryOp, Span)>,
        wrapping: bool,
        value: ast::ExprId,
        span: Span,
    ) -> ExprId {
        // A variable C owns is written through the accessor the compiler
        // declared for it, since it may be a macro rather than a place.
        if let ast::ExprKind::Name(sym) = self.ast.exprs[target].kind
            && self.lookup(sym).is_none()
            && let Some(id) = self.global_named(sym)
        {
            return self.write_global(id, target, op, wrapping, value, span);
        }
        // A field of a struct C lays out is written through C too.
        if let ast::ExprKind::Field { base, name } = self.ast.exprs[target].kind
            && let Some(written) = self.write_c_field(base, name, op, wrapping, value, span)
        {
            return written;
        }
        // A `var` that holds a `&` reference is pointed somewhere else, as a
        // `var` holding a `str` is: a reference parameter stands for what
        // it refers to, but this is a view of its own. So is the caller's
        // variable a `&var &Node` parameter refers to.
        let place = match self.repointed(target) {
            Some(place) => place,
            None => self.infer(target, None),
        };
        let place_ty = self.ty_of(place);
        if !self.is_poisoned(place_ty) {
            self.require_writable(place, Writing::Assign);
        }
        let Some((op, op_span)) = op else {
            let value = self.check(value, place_ty);
            return self.alloc(
                ExprKind::Assign {
                    place,
                    op: None,
                    wrapping: false,
                    value,
                },
                Types::UNIT,
                span,
            );
        };
        // `+%=` is the machine's, for its integers, as `+%` is.
        if wrapping && matches!(self.kind(place_ty), TyKind::Struct(..) | TyKind::Enum(..)) {
            self.report_wrapping_own_type(op, op_span, place_ty, "=");
            return self.error_expr(span);
        }
        // A number beside a type of a program's own is what its
        // implementation takes, as in `a * b`.
        let interface = KnownInterface::of_binary(op);
        let hint = match interface {
            Some(which) if !self.operator_applies(op, place_ty) => {
                let mut rhs = self.operator_rhs_tys(which, place_ty);
                rhs.retain(|&rhs| rhs != place_ty);
                match rhs.as_slice() {
                    [one] => *one,
                    _ => place_ty,
                }
            }
            _ => place_ty,
        };
        let value = self.operand(value, Some(hint));
        // `<<=` and `>>=` take an amount of any integer type, as `<<` does.
        let value = self.shift_amount(op, place, value);
        let value_ty = self.ty_of(value);
        // `a += b` on a type that implements `Add` is `a = a + b`: the
        // place is read for the call and then written. The
        // implementation is the one whose `Rhs` is the value's type, and it
        // must answer the place's.
        if !(place_ty == value_ty && self.operator_applies(op, place_ty))
            && let Some(which) = interface
            && let Some(found) = self.operator_impl(which, place_ty, value_ty)
        {
            if found.out != place_ty {
                let text = format!("{}=", op.text());
                let (place_name, out_name) = (self.ty_name(place_ty), self.ty_name(found.out));
                let interface_text = format!(
                    "{}<{}, {}>",
                    which.name(),
                    self.ty_name(value_ty).trim_matches('`'),
                    out_name.trim_matches('`')
                );
                let diagnostic = Diagnostic::error(
                    codes::INVALID_OPERANDS,
                    format!("`{text}` would put {out_name} in {place_name}"),
                    op_span,
                    format!("`{interface_text}` answers {out_name}"),
                )
                .with_note("`a op= b` is `a = a op b`, so the implementation it calls answers the type of `a`");
                self.report(diagnostic);
                return self.error_expr(span);
            }
            let read = self.infer(target, None);
            let result = self.operator_call(found, read, value, span);
            return self.alloc(
                ExprKind::Assign {
                    place,
                    op: None,
                    wrapping: false,
                    value: result,
                },
                Types::UNIT,
                span,
            );
        }
        let applies = place_ty == value_ty && self.operator_applies(op, place_ty);
        if !(applies || self.is_poisoned(place_ty) || self.is_poisoned(value_ty)) {
            let text = format!("{}{}=", op.text(), if wrapping { "%" } else { "" });
            self.report_operands(op, op_span, &text, place, value);
        }
        self.alloc(
            ExprKind::Assign {
                place,
                op: Some(op),
                wrapping,
                value,
            },
            Types::UNIT,
            span,
        )
    }

    /// `p.mode = 3` where C knows where `mode` is: the accessor that writes
    /// it, given what the pointer points at. `None` when
    /// this is an ordinary field of a struct Wip laid out.
    fn write_c_field(
        &mut self,
        base: ast::ExprId,
        name: ast::Name,
        op: Option<(BinaryOp, Span)>,
        wrapping: bool,
        value: ast::ExprId,
        span: Span,
    ) -> Option<ExprId> {
        let pointer = self.operand(base, None);
        let TyKind::Ptr(pointee) = self.kind(self.ty_of(pointer)) else {
            return None;
        };
        let TyKind::Struct(id, _) = self.kind(pointee) else {
            return None;
        };
        if !self.program.structs[id].is_opaque {
            return None;
        }
        let index = self.program.structs[id]
            .fields
            .iter()
            .position(|f| f.name == name.sym)?;
        let (get, set) = self.program.structs[id].accessors[index];
        let ty = self.program.structs[id].fields[index].ty;
        let read = |lowerer: &mut Self, at: Span| {
            lowerer.alloc(
                ExprKind::Call {
                    callee: get,
                    args: vec![pointer],
                    type_args: crate::TyList::EMPTY,
                    order: Vec::new(),
                },
                ty,
                at,
            )
        };
        let written = match op {
            None => self.check(value, ty),
            // `p.level += 1` reads it, applies the operator and writes it
            // back, since C's field is not a place Wip holds.
            Some((op, op_span)) => {
                let lhs = read(self, self.ast.exprs[base].span);
                let operand = self.operand(value, Some(ty));
                let operand_ty = self.ty_of(operand);
                let applies = ty == operand_ty && self.operator_applies(op, ty);
                if !(applies || self.is_poisoned(ty) || self.is_poisoned(operand_ty)) {
                    let text = format!("{}=", op.text());
                    self.report_operands(op, op_span, &text, lhs, operand);
                }
                self.alloc(
                    ExprKind::Binary {
                        op,
                        lhs,
                        rhs: operand,
                        wrapping,
                    },
                    ty,
                    span,
                )
            }
        };
        Some(self.alloc(
            ExprKind::Call {
                callee: set,
                args: vec![pointer, written],
                type_args: crate::TyList::EMPTY,
                order: Vec::new(),
            },
            Types::UNIT,
            span,
        ))
    }

    /// The variable C owns that a name stands for, if it is one and this
    /// module may see it.
    pub(super) fn global_named(&mut self, sym: Symbol) -> Option<GlobalId> {
        let (module, item) = match self.imported(ast::Name {
            sym,
            span: Span::at(0),
        }) {
            Some(ImportedItem::Broken) => return None,
            Some(ImportedItem::Item(module, item)) => (module, item.sym),
            None => (self.current, sym),
        };
        let &id = self.modules[module].globals.get(&item)?;
        let def = &self.program.globals[id];
        (def.is_pub || module == self.current).then_some(id)
    }

    /// `errno = 0`, and `count += 1`, which reads it and writes it back.
    fn write_global(
        &mut self,
        id: GlobalId,
        target: ast::ExprId,
        op: Option<(BinaryOp, Span)>,
        wrapping: bool,
        value: ast::ExprId,
        span: Span,
    ) -> ExprId {
        let def = &self.program.globals[id];
        let (ty, name, declared) = (def.ty, def.name, def.span);
        let Some(setter) = def.setter else {
            let text = self.text(name).to_string();
            let diagnostic = Diagnostic::error(
                codes::INVALID_ASSIGNMENT,
                format!("cannot assign to `{text}`, which C owns and Wip only reads"),
                self.ast.exprs[target].span,
                "declared `val`",
            )
            .with_secondary(declared, "declared here")
            .with_help("a variable Wip writes is declared `var`")
            .with_note("`val` in an extern block reads what C holds; `var` writes it as well");
            self.report(diagnostic);
            let value = self.infer(value, Some(ty));
            let _ = value;
            return self.error_expr(span);
        };
        let written = match op {
            None => self.check(value, ty),
            // `errno += 1` reads it, applies the operator and writes it
            // back, since C's variable is not a place Wip holds.
            Some((op, op_span)) => {
                let read = self.alloc(
                    ExprKind::Call {
                        callee: self.program.globals[id].getter,
                        args: Vec::new(),
                        type_args: crate::TyList::EMPTY,
                        order: Vec::new(),
                    },
                    ty,
                    self.ast.exprs[target].span,
                );
                let operand = self.operand(value, Some(ty));
                let operand_ty = self.ty_of(operand);
                let applies = ty == operand_ty && self.operator_applies(op, ty);
                if !(applies || self.is_poisoned(ty) || self.is_poisoned(operand_ty)) {
                    let text = format!("{}=", op.text());
                    self.report_operands(op, op_span, &text, read, operand);
                }
                self.alloc(
                    ExprKind::Binary {
                        op,
                        lhs: read,
                        rhs: operand,
                        wrapping,
                    },
                    ty,
                    span,
                )
            }
        };
        self.alloc(
            ExprKind::Call {
                callee: setter,
                args: vec![written],
                type_args: crate::TyList::EMPTY,
                order: Vec::new(),
            },
            Types::UNIT,
            span,
        )
    }

    pub(super) fn field(&mut self, base: ast::ExprId, name: ast::Name, span: Span) -> ExprId {
        if let Some(variant) = self.variant_with_dot(base, name, ItemUse::plain(None, None, span)) {
            return variant;
        }
        let b = self.operand(base, None);
        let ty = self.ty_of(b);
        // A method is not a value yet, so `x.name` is not one either.
        if let Some(owner) = self.owner_of(ty)
            && let Some(method) = self.method_of(owner, name.sym)
        {
            let type_name = self.type_name(owner).to_string();
            let text = self.text(name.sym).to_string();
            let is_static = self.program.fns[method].receiver == Some(Receiver::Static);
            let call = if is_static {
                format!("{type_name}::{text}(…)")
            } else {
                format!("{text}(…)")
            };
            // A method that takes nothing but its receiver is called by
            // adding `()`: `v.len` is `v.len()`.
            let takes_nothing = !is_static && self.program.fns[method].params.len() == 1;
            // The value's own type, not the one its methods are found on:
            // an array's are its slice's.
            let value_type = self.program.ty_name(ty, self.interner);
            let diagnostic = Diagnostic::error(
                codes::NOT_A_METHOD,
                format!("no field `{text}` on `{value_type}`"),
                name.span,
                "a method, not a field",
            )
            .with_note("`x.name` is not a value, since it would hold `x`");
            let diagnostic = match takes_nothing {
                true => diagnostic.with_fix(
                    format!("call it: `{text}()`"),
                    [Edit::insert(name.span.hi, "()")],
                ),
                false => diagnostic.with_help(format!("to call it, write `{call}`")),
            };
            // Where a function is wanted, the type's is one, where it can
            // be a value and its type has a name to write it with.
            let def = &self.program.fns[method];
            let a_value = !is_static
                && def.intrinsic.is_none()
                && !def.is_inline
                && !matches!(self.kind(def.ret), TyKind::Ref(..))
                && !matches!(
                    owner,
                    TypeDef::Builtin(BuiltinOwner::Slice | BuiltinOwner::Slots)
                );
            let diagnostic = match a_value {
                true => diagnostic.with_help(format!(
                    "where a function is wanted, `{type_name}::{text}` is one, taking the value first"
                )),
                false => diagnostic,
            };
            self.report(diagnostic);
            return self.error_expr(span);
        }
        // A block of slots says how many it has.
        if matches!(self.kind(ty), TyKind::Slots(_)) && self.text(name.sym) == "count" {
            return self.alloc(ExprKind::Len(b), Types::I64, span);
        }
        // A C pointer says whether it points at nothing.
        if let TyKind::Ptr(pointee) = self.kind(ty) {
            if self.text(name.sym) == "isNull" {
                return self.alloc(ExprKind::IsNull(b), Types::BOOL, span);
            }
            // `p.field`: the field of what it points at, which is the
            // deref C writes `p->field` for.
            if let TyKind::Struct(id, _) = self.kind(pointee)
                && let Some(index) = self.program.structs[id]
                    .fields
                    .iter()
                    .position(|f| f.name == name.sym)
            {
                // An `@opaque` struct is read through C, which is where
                // its fields are known to be.
                if self.program.structs[id].is_opaque {
                    let (get, _) = self.program.structs[id].accessors[index];
                    let field_ty = self.program.structs[id].fields[index].ty;
                    return self.alloc(
                        ExprKind::Call {
                            callee: get,
                            args: vec![b],
                            type_args: crate::TyList::EMPTY,
                            order: Vec::new(),
                        },
                        field_ty,
                        span,
                    );
                }
                let pointed = self.alloc(ExprKind::Deref(b), pointee, span);
                return self.field_of(pointed, pointee, name, span);
            }
            let field = self.text(name.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::NO_SUCH_FIELD,
                format!("no field `{field}` on {}", self.ty_name(ty)),
                name.span,
                "not a field",
            )
            .with_note("a `ptr<T>` answers `isNull`, and the fields of a `T` that is a struct");
            self.report(diagnostic);
            return self.error_expr(span);
        }
        self.field_of(b, ty, name, span)
    }

    /// The field `name` of a value of type `ty`, once what it is has been
    /// settled: a struct's, or one of the few a built-in type answers.
    fn field_of(&mut self, b: ExprId, ty: Ty, name: ast::Name, span: Span) -> ExprId {
        match self.kind(ty) {
            TyKind::Struct(id, _) => {
                let fields = &self.program.structs[id].fields;
                if let Some(i) = fields.iter().position(|f| f.name == name.sym) {
                    // Another module sees only what is `pub`.
                    if !self.field_visible(id, i) {
                        self.private_field(id, i, name.span);
                    }
                    let field_ty = self.struct_field_ty(ty, i);
                    return self.alloc(
                        ExprKind::Field {
                            base: b,
                            index: i as u32,
                        },
                        field_ty,
                        span,
                    );
                }
                let field = self.text(name.sym);
                let names: Vec<String> = fields
                    .iter()
                    .map(|f| format!("`{}`", self.text(f.name)))
                    .collect();
                let mut diagnostic = Diagnostic::error(
                    codes::NO_SUCH_FIELD,
                    format!("no field `{field}` on {}", self.ty_name(ty)),
                    name.span,
                    "unknown field",
                );
                if let Some(similar) = suggest(field, fields.iter().map(|f| self.text(f.name))) {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `{similar}`?"),
                        [Edit::replace(name.span, similar)],
                    );
                } else if !names.is_empty() {
                    diagnostic =
                        diagnostic.with_note(format!("the fields are {}", names.join(", ")));
                }
                self.report(diagnostic);
            }
            _ if self.is_poisoned(ty) => {}
            // A C string answers nothing by itself: what it holds is found by
            // walking it.
            TyKind::Cstring => {
                let field = self.text(name.sym).to_string();
                let diagnostic = Diagnostic::error(
                    codes::NO_SUCH_FIELD,
                    format!("`cstring` has no field `{field}`"),
                    name.span,
                    "not a field",
                )
                .with_note(
                    "a `cstring` is a pointer to bytes that end with a NUL, so its length is not known until it is walked",
                );
                let diagnostic = match field.as_str() {
                    "toStr" => diagnostic.with_fix(
                        "`toStr` is a method: call it",
                        [Edit::insert(name.span.hi, "()")],
                    ),
                    "len" => diagnostic.with_fix(
                        "walk it first, with `toStr()`, and ask it: `toStr().len()`",
                        [Edit::replace(name.span, "toStr().len()")],
                    ),
                    _ => diagnostic,
                };
                self.report(diagnostic);
            }
            _ => {
                let mut diagnostic = Diagnostic::error(
                    codes::NO_SUCH_FIELD,
                    format!("{} has no fields", self.ty_name(ty)),
                    name.span,
                    format!("field access on {}", self.ty_name(ty)),
                );
                // An interpolated literal appends its values through
                // `Text`, so this is where a value that has none is
                // reported.
                if name.sym == Symbol::append_to() {
                    diagnostic = diagnostic.with_note(format!(
                        "a value written between `\\(` and `)` is appended by `Text`, and {} cannot implement it",
                        self.ty_name(ty)
                    ));
                }
                self.report(diagnostic);
            }
        }
        self.error_expr(span)
    }

    /// Whether the type a pointer points at is one whose elements or
    /// fields the compiler can find: `ptr<void>` and a pointer to a C type
    /// whose contents Wip does not know are not.
    fn pointee_sized(&mut self, elem: Ty, ty: Ty, span: Span) -> bool {
        let unknown = match self.kind(elem) {
            TyKind::Opaque(_) => "a C type whose contents Wip does not know",
            _ if elem == Types::UNIT => "`void`",
            _ => return true,
        };
        if self.is_poisoned(elem) {
            return false;
        }
        let diagnostic = Diagnostic::error(
            codes::INVALID_INDEX,
            format!("{} points at {unknown}", self.ty_name(ty)),
            span,
            "how far apart its elements are is not known",
        )
        .with_help("cast it to a pointer whose type Wip knows, as in `p as ptr<u8>`")
        .with_note(
            "a pointer's elements are found by their size, which C does not give for either",
        );
        self.report(diagnostic);
        false
    }

    pub(super) fn index(
        &mut self,
        base: ast::ExprId,
        index: &'a ast::ExprId,
        span: Span,
    ) -> ExprId {
        let b = self.operand(base, None);
        // A `String` is read as the text it holds.
        let b = self.lend_string(b).unwrap_or(b);
        let ty = self.ty_of(b);
        // `v[0]`, `grid[point]`: a lookup the type answers, where it is
        // not the machine's own indexing.
        if let Some(call) = self.index_through_at(b, ty, index, span) {
            return call;
        }
        let i = self.index_value(*index);
        match self.kind(ty) {
            TyKind::Array(elem, len) => {
                // An index is an `i64`, so its bits are the literal's value.
                if let ExprKind::Int(bits) = self.state.body.exprs[i].kind
                    && let v = bits as u64 as i64
                    && (v < 0 || v as u64 >= len)
                {
                    let diagnostic = Diagnostic::error(
                        codes::INVALID_INDEX,
                        "index out of bounds",
                        self.state.body.exprs[i].span,
                        format!("index {v} is out of bounds for {}", self.ty_name(ty)),
                    );
                    self.report(diagnostic);
                }
                self.alloc(ExprKind::Index { base: b, index: i }, elem, span)
            }
            // Checked against the length when the program runs. A block of
            // slots is indexed the same way, and only the prelude has one.
            TyKind::Slice(elem) | TyKind::Slots(elem) => {
                self.alloc(ExprKind::Index { base: b, index: i }, elem, span)
            }
            // A `str` is bytes, and UTF-8 by convention: `s[i]` is one byte.
            TyKind::Str => self.alloc(ExprKind::Index { base: b, index: i }, Types::U8, span),
            // `p[i]`: the element C's pointer points at, counted from it.
            // Nothing is checked — a C pointer says nothing about how many
            // there are.
            TyKind::Ptr(elem) if self.pointee_sized(elem, ty, span) => {
                self.alloc(ExprKind::Index { base: b, index: i }, elem, span)
            }
            TyKind::Ptr(_) => self.error_expr(span),
            _ if self.is_poisoned(ty) => self.error_expr(span),
            _ => {
                let mut diagnostic = Diagnostic::error(
                    codes::INVALID_INDEX,
                    format!("cannot index into a value of type {}", self.ty_name(ty)),
                    self.state.body.exprs[b].span,
                    "not an array or slice",
                );
                // A type of a program's own says what `x[i]` means by
                // implementing `Index`.
                if matches!(self.kind(ty), TyKind::Struct(..) | TyKind::Enum(..)) {
                    let name = self.ty_name(ty).to_string();
                    diagnostic = diagnostic
                        .with_help(format!(
                            "write `extend {name}: Index<K, V>` to say what `x[i]` finds, or `extend {name}: Sequence<T>` where `i` is a position",
                            name = name.trim_matches('`')
                        ))
                        .with_note(
                            "an array, a slice and a `str` are indexed by the machine; everything else answers for itself",
                        );
                }
                self.report(diagnostic);
                self.error_expr(span)
            }
        }
    }

    /// `x[i]` where the type says what that means: the `at` of the
    /// prelude's `Index`, or of its `Sequence` for a position.
    /// `None` where the type is indexed by the machine, or by nothing.
    fn index_through_at(
        &mut self,
        base: ExprId,
        ty: Ty,
        index: &'a ast::ExprId,
        span: Span,
    ) -> Option<ExprId> {
        // An array, a slice and a `str` are indexed by the machine, with a
        // bounds check and nothing called, though a slice is a `Sequence`
        // too for generic code.
        if matches!(
            self.kind(ty),
            TyKind::Array(..) | TyKind::Slice(_) | TyKind::Str | TyKind::Slots(_) | TyKind::Ptr(_)
        ) {
            return None;
        }
        let interface = self
            .program
            .prelude_items
            .interface(KnownInterface::Index)?;
        let sequence = self
            .program
            .prelude_items
            .interface(KnownInterface::Sequence);
        let name = ast::Name {
            sym: Symbol::at(),
            span: self.ast.exprs[*index].span,
        };
        // A position is taken as an integer of any type, as the machine's
        // indexing takes it.
        self.state.index_operands.insert(*index);
        let item = ItemUse {
            args: Some(std::slice::from_ref(index)),
            names: &[],
            type_args: None,
            name_segment: 0,
            hint: None,
            receiver: None,
            self_ty: None,
            owner_ty: None,
            interface_args: crate::TyList::EMPTY,
            span,
        };
        match self.owner_of(ty) {
            // A type of its own, which says what `x[i]` means by
            // implementing `Index`.
            Some(owner)
                if self.implements_any(ty, interface)
                    || sequence.is_some_and(|sequence| self.implements_any(ty, sequence)) =>
            {
                self.dispatch(base, owner, name, item)
            }
            // A type parameter, where the constraint says it.
            None if matches!(self.kind(ty), TyKind::Param(_)) => {
                self.constrained_call(base, ty, name, item)
            }
            _ => None,
        }
    }

    /// An index, or a range's bound: an integer of any type, as the `i64`
    /// a position is. One of 32 bits or fewer is widened;
    /// one of 64 bits or more goes through the prelude's `indexOfUnsigned`
    /// or `indexOfSigned`, which panic, with the value, where it is more
    /// than an `i64` holds, so that it is never wrapped into a position.
    /// Anything else is checked as an `i64`, and reported as one.
    pub(super) fn index_value(&mut self, e: ast::ExprId) -> ExprId {
        let id = self.operand(e, Some(Types::I64));
        let ty = self.ty_of(id);
        let TyKind::Int(int) = self.kind(ty) else {
            return self.coerce(id, Types::I64, None);
        };
        if ty == Types::I64 {
            return id;
        }
        let span = self.state.body.exprs[id].span;
        if int.bits() < 64 {
            return self.alloc(ExprKind::Cast(id), Types::I64, span);
        }
        let [unsigned, signed] = Symbol::index_of();
        let (name, wide) = match int.signed() {
            true => (signed, Types::I128),
            false => (unsigned, Types::U128),
        };
        let Some(callee) = self
            .prelude
            .and_then(|prelude| self.modules[prelude].fns.get(&name).copied())
        else {
            return self.coerce(id, Types::I64, None);
        };
        let widened = match ty == wide {
            true => id,
            false => self.alloc(ExprKind::Cast(id), wide, span),
        };
        self.alloc(
            ExprKind::Call {
                callee,
                args: vec![widened],
                type_args: crate::TyList::EMPTY,
                order: vec![0],
            },
            Types::I64,
            span,
        )
    }

    /// An expression borrowed where it is written: the operand of `&`,
    /// what a `for` walks, and what a projection lends. A range of
    /// elements may be one, and nothing else may.
    pub(super) fn infer_borrowed(&mut self, e: ast::ExprId, hint: Option<Ty>) -> ExprId {
        let expr = &self.ast.exprs[e];
        match expr.kind {
            ast::ExprKind::SubSlice {
                base,
                lo,
                hi,
                inclusive,
            } => self.sub_slice(base, lo, hi, inclusive, expr.span, true),
            _ => self.infer(e, hint),
        }
    }

    /// The slice a container lends, where a range of it is asked for: a
    /// type whose `x[i]` is a position, a `Sequence<T>`, and which lends its
    /// elements as one slice, an `Items<T>`, of one `T`. Otherwise what is
    /// missing, for the message.
    fn ranged_items(&mut self, base: ExprId, ty: Ty) -> Result<ExprId, Option<&'static str>> {
        let known = |known| self.program.prelude_items.interface(known);
        let implements = |interface: Option<InterfaceId>| {
            interface.is_some_and(|interface| self.implements_any(ty, interface))
        };
        let sequence = known(KnownInterface::Sequence);
        let lends = implements(known(KnownInterface::Items));
        match (lends, implements(sequence)) {
            (true, true) => {
                // Of one element: the positions are those of the slice.
                let same = self.items_element(ty).is_some_and(|elem| {
                    sequence.is_some_and(|sequence| self.implements_args(ty, sequence, &[elem]))
                });
                match self.items_of(base, ty) {
                    super::body::Walked::Items(items, _) if same => Ok(items),
                    _ => Err(None),
                }
            }
            (false, true) => Err(Some(
                "its elements are not in one slice: it is a sequence, and lends no `Items`",
            )),
            (true, false) => Err(Some(
                "it lends its elements as a slice, and has no positions for a range to count: it is no `Sequence`",
            )),
            (false, false) => Err(None),
        }
    }

    /// `base[lo..hi]`: a range of elements of an array or slice, which must
    /// be `borrowed` where it is written.
    pub(super) fn sub_slice(
        &mut self,
        base: ast::ExprId,
        lo: Option<ast::ExprId>,
        hi: Option<ast::ExprId>,
        inclusive: bool,
        span: Span,
        borrowed: bool,
    ) -> ExprId {
        let b = self.operand(base, None);
        // A `String` is read as the text it holds.
        let b = self.lend_string(b).unwrap_or(b);
        let lo = lo.map(|e| self.index_value(e));
        let mut hi = hi.map(|e| self.index_value(e));
        // `lo..=hi` is `lo..hi + 1`, which only a range as long as there
        // are positions could make overflow.
        if inclusive && let Some(end) = hi {
            let end_span = self.state.body.exprs[end].span;
            let one = self.alloc(ExprKind::Int(1), Types::I64, end_span);
            hi = Some(self.alloc(
                ExprKind::Binary {
                    op: BinaryOp::Add,
                    lhs: end,
                    rhs: one,
                    wrapping: false,
                },
                Types::I64,
                end_span,
            ));
        }
        let mut b = b;
        let mut ty = self.ty_of(b);
        // A range of a `str` is a `str`: the same bytes, seen from further in.
        // It is a value, so it needs no borrow.
        if self.kind(ty) == TyKind::Str {
            return self.alloc(ExprKind::SubSlice { base: b, lo, hi }, Types::STR, span);
        }
        // A container whose `x[i]` is a position and which lends its
        // elements as one slice is ranged as that slice: `v[a..b]` is
        // `v.items()[a..b]`.
        if !matches!(
            self.kind(ty),
            TyKind::Array(..) | TyKind::Slice(_) | TyKind::Slots(_)
        ) && !self.is_poisoned(ty)
        {
            match self.ranged_items(b, ty) {
                Ok(items) => {
                    b = items;
                    ty = self.ty_of(items);
                }
                Err(note) => {
                    let mut diagnostic = Diagnostic::error(
                        codes::INVALID_INDEX,
                        format!(
                            "cannot take a range of a value of type {}",
                            self.ty_name(ty)
                        ),
                        self.state.body.exprs[b].span,
                        "not an array, a slice or a sequence that lends one",
                    );
                    if let Some(note) = note {
                        diagnostic = diagnostic.with_note(note);
                    }
                    self.report(diagnostic);
                    return self.error_expr(span);
                }
            }
        }
        let elem = match self.kind(ty) {
            TyKind::Array(elem, _) | TyKind::Slice(elem) | TyKind::Slots(elem) => elem,
            _ => return self.error_expr(span),
        };
        if !borrowed {
            let diagnostic = Diagnostic::error(
                codes::SLICE_NOT_BORROWED,
                "a range of elements must be borrowed",
                span,
                "a range of elements",
            )
            .with_note("a slice is a view of elements that belong to someone else, so it exists only as a `&` or `&var` argument, or as what a `for` loop walks")
            .with_fix("borrow it", [Edit::insert(span.lo, "&")]);
            self.report(diagnostic);
            return self.error_expr(span);
        }
        let ty = self.intern(TyKind::Slice(elem));
        self.alloc(ExprKind::SubSlice { base: b, lo, hi }, ty, span)
    }
}
