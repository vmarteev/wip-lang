//! Calls to a C function that takes more than it declares:
//! each is pointed at a declaration of exactly what it
//! passes, which the C the compiler writes for itself then calls.

use crate::hir::*;
use crate::ty::Ty;

/// Points each call to a C function that takes more than it declares at a
/// declaration of exactly what that call passes. This is
/// done once every body is checked, since bodies are checked on threads
/// that share no arena to declare it in. One declaration
/// serves every call that passes the same types, so two `printf`s of one
/// format share a wrapper.
pub fn resolve_variadic_calls(program: &mut Program) {
    if !program.fns.iter().any(|(_, def)| def.is_variadic) {
        return;
    }
    let mut calls: Vec<(FnId, ExprId, FnId, Vec<Ty>)> = Vec::new();
    for (owner, def) in program.fns.iter() {
        let Some(body) = &def.body else { continue };
        for (id, expr) in body.exprs.iter() {
            if let ExprKind::Call { callee, args, .. } = &expr.kind
                && program.fns[*callee].is_variadic
            {
                let types = args.iter().map(|&arg| body.exprs[arg].ty).collect();
                calls.push((owner, id, *callee, types));
            }
        }
    }
    let mut made: rustc_hash::FxHashMap<(FnId, crate::TyList), FnId> = Default::default();
    let mut redirects: Vec<(FnId, ExprId, FnId)> = Vec::with_capacity(calls.len());
    for (owner, id, callee, types) in calls {
        let list = program.types.intern_list(&types);
        let concrete = match made.get(&(callee, list)) {
            Some(&made) => made,
            None => {
                let declared = program.fns[callee].params.clone();
                let mut def = program.fns[callee].clone();
                def.params = types
                    .iter()
                    .enumerate()
                    .map(|(i, &ty)| match declared.get(i) {
                        Some(param) => ParamDef {
                            ty,
                            ..param.clone()
                        },
                        None => ParamDef {
                            ty,
                            ..declared[0].clone()
                        },
                    })
                    .collect();
                def.is_variadic = false;
                def.variadic_of = Some(callee);
                def.body = None;
                let made_id = program.fns.alloc(def);
                made.insert((callee, list), made_id);
                made_id
            }
        };
        redirects.push((owner, id, concrete));
    }
    for (owner, id, concrete) in redirects {
        if let Some(body) = &mut program.fns[owner].body
            && let ExprKind::Call { callee, .. } = &mut body.exprs[id].kind
        {
            *callee = concrete;
        }
    }
}
