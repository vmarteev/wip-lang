//! What the interfaces a type implements give it by a name: each one's
//! method, as the implementation has it, and the methods of each one's
//! extensions whose condition the implementation's types meet.
//! `extend Iterator<T: Ord> { move fn max() … }` gives `max` to every
//! iterator of an ordered element, and to no other.

use super::*;

/// A method a call may mean, and the interface it comes from.
#[derive(Clone, Copy)]
pub(super) struct Offered {
    pub interface: InterfaceId,
    pub method: FnId,
    /// The interface's types for the value it is called on, which the call
    /// is given as a default's call is.
    pub args: crate::TyList,
}

/// An extension the value's types do not meet the condition of: which
/// parameter, and what it does not implement.
#[derive(Clone, Copy)]
pub(super) struct Unmet {
    pub method: FnId,
    pub interface: InterfaceId,
    pub args: crate::TyList,
    pub param: usize,
    pub missing: crate::Constraint,
}

impl Lowerer<'_> {
    /// What a call of `name` on a value of `ty`, whose type is `owner`,
    /// may mean through the interfaces `owner` implements, and the
    /// extensions of that name it does not meet the condition of.
    pub(super) fn offered(
        &mut self,
        owner: TypeDef,
        ty: Ty,
        name: Symbol,
    ) -> (Vec<Offered>, Vec<Unmet>) {
        let value = self.under_refs(ty);
        let own = self.owner_args(value);
        let mut offered = Vec::new();
        let mut unmet = Vec::new();
        let implementations: Vec<crate::ImplDef> = self
            .program
            .impls
            .iter()
            .filter(|i| i.ty == owner)
            .cloned()
            .collect();
        for implementation in implementations {
            // The implementation's types, read in the value's own.
            let written = self.program.types.list(implementation.args).to_vec();
            let args: Vec<Ty> = written
                .into_iter()
                .map(|arg| self.program.types.subst(arg, &own))
                .collect();
            let list = self.program.types.intern_list(&args);
            let interface = self.program.interfaces[implementation.interface].clone();
            for (at, method) in interface.methods.iter().enumerate() {
                if self.program.fns[method.id].name != name {
                    continue;
                }
                if let Some(&method) = implementation.methods.get(at) {
                    offered.push(Offered {
                        interface: implementation.interface,
                        method,
                        args: list,
                    });
                }
            }
            for &method in &interface.extensions {
                if self.program.fns[method].name != name {
                    continue;
                }
                match self.extension_unmet(method, value, &args) {
                    None => offered.push(Offered {
                        interface: implementation.interface,
                        method,
                        args: list,
                    }),
                    Some((param, missing)) => unmet.push(Unmet {
                        method,
                        interface: implementation.interface,
                        args: list,
                        param,
                        missing,
                    }),
                }
            }
        }
        (offered, unmet)
    }

    /// The same for a type parameter, through what its constraints say.
    pub(super) fn offered_by_constraints(
        &mut self,
        param: Ty,
        constraints: &[crate::Constraint],
        name: Symbol,
    ) -> (Vec<Offered>, Vec<Unmet>) {
        let mut offered = Vec::new();
        let mut unmet = Vec::new();
        for &constraint in constraints {
            let interface = self.program.interfaces[constraint.interface].clone();
            if let Some(method) = interface
                .methods
                .iter()
                .find(|m| self.program.fns[m.id].name == name)
            {
                offered.push(Offered {
                    interface: constraint.interface,
                    method: method.id,
                    args: constraint.args,
                });
            }
            let args = self.program.types.list(constraint.args).to_vec();
            for &method in &interface.extensions {
                if self.program.fns[method].name != name {
                    continue;
                }
                match self.extension_unmet(method, param, &args) {
                    None => offered.push(Offered {
                        interface: constraint.interface,
                        method,
                        args: constraint.args,
                    }),
                    Some((at, missing)) => unmet.push(Unmet {
                        method,
                        interface: constraint.interface,
                        args: constraint.args,
                        param: at,
                        missing,
                    }),
                }
            }
        }
        (offered, unmet)
    }

    /// Which of the interface's types, `args`, does not meet extension
    /// `method`'s condition for a value of `value`, and what it lacks;
    /// nothing where all do.
    fn extension_unmet(
        &mut self,
        method: FnId,
        value: Ty,
        args: &[Ty],
    ) -> Option<(usize, crate::Constraint)> {
        let generics = self.program.fns[method].generics.clone();
        // `Self`, then the interface's types: what the condition is read in.
        let mut known = vec![value];
        known.extend_from_slice(args);
        for (at, &arg) in args.iter().enumerate() {
            let Some(param) = generics.get(at + 1) else {
                continue;
            };
            for &constraint in &param.interfaces {
                let written = self.program.types.list(constraint.args).to_vec();
                let wanted: Vec<Ty> = written
                    .into_iter()
                    .map(|ty| self.program.types.subst(ty, &known))
                    .collect();
                if !self.implements_args(arg, constraint.interface, &wanted) {
                    return Some((at, constraint));
                }
            }
        }
        None
    }

    /// A call of a name two interfaces give the type: reported, since which
    /// one is meant is in question. `subject` is how the type is named.
    pub(super) fn ambiguous_method(&mut self, offered: &[Offered], subject: &str, name: ast::Name) {
        let text = self.text(name.sym).to_string();
        let mut interfaces: Vec<String> = Vec::new();
        for offer in offered {
            let interface = format!(
                "`{}`",
                self.text(self.program.interfaces[offer.interface].name)
            );
            if !interfaces.contains(&interface) {
                interfaces.push(interface);
            }
        }
        let mut diagnostic = Diagnostic::error(
            codes::AMBIGUOUS_METHOD,
            format!("`{text}` of `{subject}` is in question"),
            name.span,
            format!("{} each give it", interfaces.join(" and ")),
        )
        .with_note("a method that two interfaces give a type is not chosen between: a type's own method of the name would be the one");
        for offer in offered {
            diagnostic =
                diagnostic.with_secondary(self.program.fns[offer.method].name_span, "one of them");
        }
        self.report(diagnostic);
    }

    /// An extension of that name whose condition the value's types do not
    /// meet: the call says so, rather than that no method exists.
    pub(super) fn unmet_extension(&mut self, unmet: Unmet, subject: &str, name: ast::Name) {
        let text = self.text(name.sym).to_string();
        let declared = self.program.fns[unmet.method].name_span;
        let interface = self
            .text(self.program.interfaces[unmet.interface].name)
            .to_string();
        // The block's own names: `Iterator<T>`, where `T: Add + Zero`.
        let count = self.program.interfaces[unmet.interface].generics.len();
        let params: Vec<GenericParamDef> = self.program.fns[unmet.method]
            .generics
            .iter()
            .skip(1)
            .take(count)
            .cloned()
            .collect();
        let names: Vec<String> = params
            .iter()
            .map(|p| self.text(p.name).to_string())
            .collect();
        let param = params[unmet.param].clone();
        let subject_param = self.intern_param(unmet.param + 1, &param);
        let condition: Vec<String> = param
            .interfaces
            .iter()
            .map(|&c| self.constraint_name(c, subject_param))
            .collect();
        let actual = self.program.types.list(unmet.args)[unmet.param];
        let lacking = self.text(self.program.interfaces[unmet.missing.interface].name);
        let generic = format!("{interface}<{}>", names.join(", "));
        let given: Vec<String> = self
            .program
            .types
            .list(unmet.args)
            .iter()
            .map(|&ty| self.program.ty_name(ty, self.interner))
            .collect();
        let given = format!("{interface}<{}>", given.join(", "));
        let mut diagnostic = Diagnostic::error(
            codes::NOT_A_METHOD,
            format!("no method `{text}` on `{subject}`"),
            name.span,
            format!(
                "`{text}` is {} `{generic}`'s where `{}: {}`",
                article(&generic),
                names[unmet.param],
                condition.join(" + ")
            ),
        )
        .with_secondary(declared, "declared here");
        let actual_name = self.ty_name(actual);
        diagnostic = match self.kind(actual) {
            // A type parameter has what its constraints promise.
            TyKind::Param(_) => diagnostic
                .with_note(format!(
                    "`{subject}` is {} `{given}`, and nothing says {actual_name} implements `{lacking}`",
                    article(&given)
                ))
                .with_help(format!("constrain it: `{}: {lacking}`", actual_name.trim_matches('`'))),
            _ => diagnostic.with_note(format!(
                "`{subject}` is {} `{given}`, and {actual_name} does not implement `{lacking}`",
                article(&given)
            )),
        };
        self.report(diagnostic);
    }

    /// The type parameter a generic parameter's place stands for.
    fn intern_param(&mut self, index: usize, param: &GenericParamDef) -> Ty {
        self.intern(TyKind::Param(crate::TyParam {
            index: index as u32,
            name: param.name,
            copy: param.copy,
        }))
    }
}
