//! Instances of generic functions.
//!
//! A generic function's body is checked once, with its type parameters as
//! types. Before code is generated, each use of it with new type arguments
//! makes an instance: a copy of the function whose types have the arguments
//! in place of the parameters. Calls are pointed at their instances, so the
//! MIR and the backends never see a type parameter.

use rustc_hash::{FxHashMap, FxHashSet};
use wip_syntax::{Diagnostic, Interner, Symbol};

use crate::hir::*;
use crate::ty::{Ty, TyKind, TyList};
use crate::{KnownEnum, KnownFn, KnownInterface};
use wip_syntax::codes;

/// How deeply instances may be made from instances before they are taken to
/// be an endless series, as with `f<T>` calling `f<Pair<T, T>>`.
const MAX_INSTANCE_DEPTH: u32 = 64;

/// How deeply types may nest, likewise: `S<T>` holding `own<S<own<T>>>`.
pub(crate) const MAX_TYPE_DEPTH: u32 = 64;

/// Makes every instance a program needs, starting from its functions that are
/// not generic, and points every call and function value at its instance. A
/// module makes the instances it uses. Instances are made in the order they are
/// found, so the result does not depend on threads.
pub fn instantiate(program: &mut Program, interner: &Interner) -> Vec<Diagnostic> {
    let diagnostics = make_instances(program, interner);
    program.fn_values = fn_values(program);
    diagnostics
}

/// The functions some body uses as a value, by the instance it names.
fn fn_values(program: &Program) -> FxHashSet<FnId> {
    program
        .fns
        .iter()
        .filter_map(|(_, def)| def.body.as_ref())
        .flat_map(|body| body.exprs.iter())
        .filter_map(|(_, expr)| match expr.kind {
            ExprKind::FnRef { id, .. } => Some(id),
            _ => None,
        })
        .collect()
}

fn make_instances(program: &mut Program, interner: &Interner) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    // Nothing to make: no generic function, and no interface to build a table
    // of methods for.
    if program.fns.iter().all(|(_, f)| f.generics.is_empty()) && program.impls.is_empty() {
        return diagnostics;
    }
    let start = program.types.len();
    let mut instances: FxHashMap<(FnId, TyList, u32), FnId> = FxHashMap::default();
    let mut work: Vec<(FnId, u32)> = program
        .fns
        .iter()
        .filter(|(_, f)| f.generics.is_empty() && f.body.is_some())
        .map(|(id, _)| (id, 0))
        .collect();
    // A `main` that returns a `Result` hands it to the prelude, which reports
    // the error and answers with the exit code. Nothing calls that function,
    // so nothing else would make its instance.
    if let Some((generic, error)) = entry_report(program, interner) {
        let list = program.types.intern_list(&[error]);
        let instance = make_instance(program, generic, list, 0);
        instances.insert((generic, list, 0), instance);
        work.push((instance, 1));
        program.entry_report = Some(instance);
    }
    let mut reported = FxHashSet::default();
    let mut next = 0;
    while next < work.len() {
        let (id, depth) = work[next];
        next += 1;
        let module = program.fns[id].module;
        let args: Vec<Ty> = match program.fns[id].instance_of {
            Some((_, list)) => program.types.list(list).to_vec(),
            None => Vec::new(),
        };
        // An `@intrinsic` has no body to walk: the compiler writes it where
        // it is called.
        let Some(mut body) = program.fns[id].body.take() else {
            continue;
        };
        // The tables of methods this body needs, with every method concrete.
        let dyn_refs: Vec<(InterfaceId, Ty)> = body
            .exprs
            .iter()
            .filter_map(|(_, expr)| {
                let ExprKind::DynRef { value, interface } = expr.kind else {
                    return None;
                };
                let TyKind::Ref(pointee, _) = program.types.kind(body.exprs[value].ty) else {
                    return None;
                };
                Some((interface, program.types.subst(pointee, &args)))
            })
            .collect();
        // The drop of every type this body ends a value of, made concrete.
        // A drop is called from the code generator rather
        // than from an expression, so nothing else would instantiate it.
        let mut dropped: Vec<Ty> = Vec::new();
        for (_, local) in body.locals.iter() {
            let ty = program.types.subst(local.ty, &args);
            collect_drops(program, ty, &mut dropped);
        }
        for (_, expr) in body.exprs.iter() {
            let ty = program.types.subst(expr.ty, &args);
            collect_drops(program, ty, &mut dropped);
        }
        for ty in dropped {
            // A generator is dropped by its `next`, told to drop what it
            // holds rather than go on, and that `next` is the one of its
            // type.
            if let TyKind::Struct(id, list) = program.types.kind(ty)
                && let Some(generator) = program.structs[id].generator
            {
                if program.generator_next.contains_key(&ty) {
                    continue;
                }
                let next = if list == TyList::EMPTY {
                    generator.next
                } else {
                    let key = (generator.next, list, module);
                    match instances.get(&key) {
                        Some(&instance) => instance,
                        None => {
                            let instance = make_instance(program, generator.next, list, module);
                            instances.insert(key, instance);
                            work.push((instance, depth + 1));
                            instance
                        }
                    }
                };
                program.generator_next.insert(ty, next);
                continue;
            }
            if program.drop_fns.contains_key(&ty) {
                continue;
            }
            let interface = program
                .prelude_items
                .interface(KnownInterface::Destroy)
                .expect("a drop needs its interface");
            let method = program.interfaces[interface].methods[0].id;
            let Some((implementation, method_args)) =
                implementation_of(program, interface, method, &[ty])
            else {
                continue;
            };
            let concrete = if method_args.is_empty() {
                implementation
            } else {
                let list = program.types.intern_list(&method_args);
                let key = (implementation, list, module);
                match instances.get(&key) {
                    Some(&instance) => instance,
                    None => {
                        let instance = make_instance(program, implementation, list, module);
                        instances.insert(key, instance);
                        work.push((instance, depth + 1));
                        instance
                    }
                }
            };
            program.drop_fns.insert(ty, concrete);
        }
        for (interface, ty) in dyn_refs {
            if program
                .vtables
                .iter()
                .any(|table| table.interface == interface && table.ty == ty)
            {
                continue;
            }
            let methods = program.interfaces[interface].methods.clone();
            let mut concrete = Vec::new();
            for method in methods {
                let Some((implementation, args)) =
                    implementation_of(program, interface, method.id, &[ty])
                else {
                    continue;
                };
                if args.is_empty() {
                    concrete.push(implementation);
                    continue;
                }
                let list = program.types.intern_list(&args);
                let key = (implementation, list, module);
                let instance = match instances.get(&key) {
                    Some(&instance) => instance,
                    None => {
                        let instance = make_instance(program, implementation, list, module);
                        instances.insert(key, instance);
                        work.push((instance, depth + 1));
                        instance
                    }
                };
                concrete.push(instance);
            }
            program.vtables.push(VTableDef {
                interface,
                ty,
                methods: concrete,
            });
        }
        // Calls whose `Self` is an array, which the slice's implementation
        // answers: the call, and the element.
        let mut lent_as_slices: Vec<(ExprId, FnId, Ty)> = Vec::new();
        // Calls whose `Self` is a `&T`, which `T`'s implementation answers:
        // the call, the interface's method, and how many references.
        let mut read_through: Vec<(ExprId, FnId, u32)> = Vec::new();
        // Calls of `clone` on a `&T`, which copy the reference.
        let mut copied_references: Vec<ExprId> = Vec::new();
        for (at, expr) in body.exprs.iter_mut() {
            // A lambda inside a generic function is generic in the same
            // way, and its instance comes with the enclosing one: nothing
            // calls it by name, so nothing else would make it.
            if let ExprKind::Closure { id: code, .. } = &mut expr.kind {
                if !program.fns[*code].generics.is_empty() && !args.is_empty() {
                    let concrete = program.types.intern_list(&args);
                    let key = (*code, concrete, module);
                    let instance = match instances.get(&key) {
                        Some(&instance) => instance,
                        None => {
                            let instance = make_instance(program, *code, concrete, module);
                            instances.insert(key, instance);
                            work.push((instance, depth + 1));
                            instance
                        }
                    };
                    *code = instance;
                }
                continue;
            }
            // A call, or a function named as a value.
            let (ExprKind::Call {
                callee, type_args, ..
            }
            | ExprKind::FnRef {
                id: callee,
                type_args,
            }) = &mut expr.kind
            else {
                continue;
            };
            if *type_args == TyList::EMPTY {
                continue;
            }
            let mut concrete = program.types.subst_list(*type_args, &args);
            // A call of an interface's method goes to the implementation for
            // the type `Self` turned out to be. An extension's is the same
            // for every implementer, and is made for `Self` as it is.
            if let Some(interface) = program.fns[*callee].interface
                && !program.interfaces[interface].extensions.contains(callee)
            {
                let mut tys = program.types.list(concrete).to_vec();
                // A `&T` is answered by `T`'s implementation, given what the
                // references point to, and is copied as a view is.
                let (base, depth) = through_references(program, tys[0]);
                if depth > 0 && program.is_clone(interface) {
                    copied_references.push(at);
                    continue;
                }
                tys[0] = base;
                let Some((implementation, rest)) =
                    implementation_of(program, interface, *callee, &tys)
                else {
                    // Plain data with no `clone` of its own is copied.
                    if program.is_clone(interface) && program.clones_by_copy(base) {
                        copied_references.push(at);
                    }
                    continue;
                };
                if depth > 0 {
                    read_through.push((at, *callee, depth));
                }
                if let Some(&self_ty) = tys.first()
                    && let TyKind::Array(elem, _) = program.types.kind(self_ty)
                {
                    lent_as_slices.push((at, *callee, elem));
                }
                *callee = implementation;
                concrete = program.types.intern_list(&rest);
                if concrete == TyList::EMPTY {
                    *type_args = TyList::EMPTY;
                    continue;
                }
            }
            let key = (*callee, concrete, module);
            let instance = match instances.get(&key) {
                Some(&instance) => instance,
                None if depth >= MAX_INSTANCE_DEPTH => {
                    let generic = &program.fns[*callee];
                    if reported.insert(*callee) {
                        let name = interner.resolve(generic.name);
                        let diagnostic = Diagnostic::error(
                            codes::INSTANTIATION_DEPTH,
                            format!("the instances of `{name}` never end"),
                            generic.name_span,
                            "each instance calls a larger one",
                        )
                        .with_note(format!(
                            "instances made from instances more than {MAX_INSTANCE_DEPTH} deep are taken to be an endless series"
                        ));
                        diagnostics.push(diagnostic);
                    }
                    continue;
                }
                None => {
                    let instance = make_instance(program, *callee, concrete, module);
                    instances.insert(key, instance);
                    work.push((instance, depth + 1));
                    instance
                }
            };
            *callee = instance;
            *type_args = TyList::EMPTY;
        }
        for (call, method, elem) in lent_as_slices {
            lend_as_slices(program, &mut body, &args, call, method, elem);
        }
        for (call, method, depth) in read_through {
            read_through_references(program, &mut body, &args, call, method, depth);
        }
        for call in copied_references {
            copy_reference(program, &mut body, &args, call);
        }
        program.fns[id].body = Some(body);
    }
    // Runaway instances leave runaway types behind, which say nothing more.
    if diagnostics.is_empty() {
        complete_types(program, start, interner, &mut diagnostics);
    }
    diagnostics
}

/// The function that implements `method` for the type `Self` stands for, and
/// the type arguments left for it: the method's own.
/// Every type inside `ty` that cleans up after itself, including `ty`.
/// A drop is reached through whatever holds the value, so
/// the parts are walked as the drop glue walks them.
fn collect_drops(program: &Program, ty: Ty, out: &mut Vec<Ty>) {
    fn walk(program: &Program, ty: Ty, out: &mut Vec<Ty>, seen: &mut FxHashSet<Ty>) {
        if !seen.insert(ty) {
            return;
        }
        if program.has_drop(ty) && !out.contains(&ty) {
            out.push(ty);
        }
        match program.types.kind(ty) {
            TyKind::Own(inner) | TyKind::Array(inner, _) | TyKind::Slice(inner) => {
                walk(program, inner, out, seen);
            }
            TyKind::Struct(..) => {
                for field in program.field_tys(ty) {
                    walk(program, field, out, seen);
                }
            }
            TyKind::Enum(..) => {
                for field in program.variant_field_tys(ty).into_iter().flatten() {
                    walk(program, field, out, seen);
                }
            }
            _ => {}
        }
    }
    let mut seen = FxHashSet::default();
    walk(program, ty, out, &mut seen);
}

/// A call of an interface's method whose `Self` is an array, gone to the
/// slice's implementation: each argument the method takes as `&Self` is
/// lent as the slice of all of the array, which is what that
/// implementation takes.
fn lend_as_slices(
    program: &mut Program,
    body: &mut crate::Body,
    args: &[Ty],
    call: ExprId,
    method: FnId,
    elem: Ty,
) {
    let ExprKind::Call {
        args: call_args, ..
    } = &body.exprs[call].kind
    else {
        return;
    };
    let call_args = call_args.clone();
    let slice = program.types.intern(TyKind::Slice(elem));
    let mut lent = call_args.clone();
    for (i, param) in program.fns[method].params.clone().iter().enumerate() {
        // `Self` is the method's first type parameter.
        let TyKind::Ref(target, kind) = program.types.kind(param.ty) else {
            continue;
        };
        if !matches!(program.types.kind(target), TyKind::Param(p) if p.index == 0) {
            continue;
        }
        let Some(&arg) = call_args.get(i) else {
            continue;
        };
        let (arg_ty, span) = (body.exprs[arg].ty, body.exprs[arg].span);
        let arg_ty = program.types.subst(arg_ty, args);
        let array = match program.types.kind(arg_ty) {
            TyKind::Ref(array, _) => array,
            _ => continue,
        };
        // `&place` lends the place itself; any other reference is followed.
        let place = match body.exprs[arg].kind {
            ExprKind::Ref(place) => place,
            _ => body.exprs.alloc(Expr {
                kind: ExprKind::Deref(arg),
                ty: array,
                span,
            }),
        };
        let whole = body.exprs.alloc(Expr {
            kind: ExprKind::SubSlice {
                base: place,
                lo: None,
                hi: None,
            },
            ty: slice,
            span,
        });
        let reference = program.types.intern(TyKind::Ref(slice, kind));
        lent[i] = body.exprs.alloc(Expr {
            kind: ExprKind::Ref(whole),
            ty: reference,
            span,
        });
    }
    if let ExprKind::Call { args, .. } = &mut body.exprs[call].kind {
        *args = lent;
    }
}

/// `ty` without the `&` references around it, and how many there were: a
/// `&&Node` is answered by `Node`'s implementations.
fn through_references(program: &Program, ty: Ty) -> (Ty, u32) {
    let (mut ty, mut depth) = (ty, 0);
    while let Some(inner) = program.referred(ty) {
        ty = inner;
        depth += 1;
    }
    (ty, depth)
}

/// A call of an interface's method whose `Self` is a `&T`, gone to `T`'s
/// implementation: each argument the method takes as `&Self` is read
/// through `depth` references, to the `&T` that implementation takes.
fn read_through_references(
    program: &mut Program,
    body: &mut crate::Body,
    args: &[Ty],
    call: ExprId,
    method: FnId,
    depth: u32,
) {
    let ExprKind::Call {
        args: call_args, ..
    } = &body.exprs[call].kind
    else {
        return;
    };
    let mut read = call_args.clone();
    for (i, param) in program.fns[method].params.clone().iter().enumerate() {
        // `Self` is the method's first type parameter.
        let TyKind::Ref(target, _) = program.types.kind(param.ty) else {
            continue;
        };
        if !matches!(program.types.kind(target), TyKind::Param(p) if p.index == 0) {
            continue;
        }
        let Some(&arg) = read.get(i) else {
            continue;
        };
        let span = body.exprs[arg].span;
        let mut ty = program.types.subst(body.exprs[arg].ty, args);
        let mut e = arg;
        for _ in 0..depth {
            let TyKind::Ref(inner, _) = program.types.kind(ty) else {
                break;
            };
            e = body.exprs.alloc(Expr {
                kind: ExprKind::Deref(e),
                ty: inner,
                span,
            });
            ty = inner;
        }
        read[i] = e;
    }
    if let ExprKind::Call { args, .. } = &mut body.exprs[call].kind {
        *args = read;
    }
}

/// `clone` of plain data: what the receiver refers to, copied — a `&T` as
/// a view is, and a value that owns no memory.
fn copy_reference(program: &mut Program, body: &mut crate::Body, args: &[Ty], call: ExprId) {
    let ExprKind::Call {
        args: call_args, ..
    } = &body.exprs[call].kind
    else {
        return;
    };
    let Some(&receiver) = call_args.first() else {
        return;
    };
    let ty = program.types.subst(body.exprs[call].ty, args);
    body.exprs[call].kind = ExprKind::Deref(receiver);
    body.exprs[call].ty = ty;
}

fn implementation_of(
    program: &Program,
    interface: InterfaceId,
    method: FnId,
    args: &[Ty],
) -> Option<(FnId, Vec<Ty>)> {
    let (&self_ty, rest) = args.split_first()?;
    let index = program.interfaces[interface]
        .methods
        .iter()
        .position(|m| m.id == method)?;
    let (owner, owner_args) = match program.types.kind(self_ty) {
        TyKind::Struct(id, list) => (TypeDef::Struct(id), list),
        TyKind::Enum(id, list) => (TypeDef::Enum(id), list),
        // A built-in type the prelude, or a module's own interface, gave
        // methods to. A slice's element is its argument.
        // An array has its slice's.
        TyKind::Slice(elem) | TyKind::Array(elem, _) => (
            TypeDef::Builtin(BuiltinOwner::Slice),
            program.types.find_list(&[elem])?,
        ),
        kind => (TypeDef::Builtin(BuiltinOwner::of(kind)?), TyList::EMPTY),
    };
    // An interface that takes types is implemented once per set of them,
    // and the call's type arguments say which.
    let takes = program.interfaces[interface].generics.len();
    let wanted = rest.get(..takes).unwrap_or_default().to_vec();
    let owner_list = program.types.list(owner_args).to_vec();
    let implementation = program
        .impls
        .iter()
        .find(|i| {
            i.interface == interface
                && i.ty == owner
                && (takes == 0
                    || crate::args_match_with(&program.types, i.args, &owner_list, &wanted))
        })?
        .methods
        .get(index)
        .copied()?;
    // A method left to its default is the interface's own, which is generic
    // in `Self`; the type's own method is generic in the type's parameters.
    // The interface's own types follow `Self` in the call's arguments. A
    // default body is generic in them too; the type's own method has them
    // written in its type's parameters already, so it takes only the
    // method's own after them — `Chars.next` is no instance of anything.
    let args = match program.fns[implementation].interface {
        Some(_) => std::iter::once(self_ty)
            .chain(rest.iter().copied())
            .collect(),
        None => {
            let own = rest.get(takes..).unwrap_or_default();
            let mut args = program.types.list(owner_args).to_vec();
            args.extend_from_slice(own);
            args
        }
    };
    Some((implementation, args))
}

/// The prelude's `exitCode` or `exitVoid`, and the error type to make it
/// for, when `main` returns a `Result` whose error can be written as text.
/// A `main` that is wrong in some other way is reported by
/// `check_main`, which runs later; here it is simply left alone.
fn entry_report(program: &Program, interner: &Interner) -> Option<(FnId, Ty)> {
    let (_, main) = crate::main_fn(program, interner)?;

    let TyKind::Enum(id, args) = program.types.kind(main.ret) else {
        return None;
    };
    if program.prelude_items.enumeration(KnownEnum::Result) != Some(id) {
        return None;
    }
    let args = program.types.list(args);
    let (value, error) = (args[0], args[1]);
    let text = program.prelude_items.interface(KnownInterface::Text)?;
    if !program.implements(error, text) {
        return None;
    }
    match value {
        crate::Types::I64 => program
            .prelude_items
            .function(KnownFn::ExitCode)
            .map(|f| (f, error)),
        crate::Types::UNIT => program
            .prelude_items
            .function(KnownFn::ExitVoid)
            .map(|f| (f, error)),
        _ => None,
    }
}

/// A copy of `generic` for the type arguments `args`, in `module`.
fn make_instance(program: &mut Program, generic: FnId, args: TyList, module: u32) -> FnId {
    let tys = program.types.list(args).to_vec();
    let mut def = program.fns[generic].clone();
    let types = &mut program.types;
    def.generics = Vec::new();
    def.instance_of = Some((generic, args));
    def.module = module;
    def.is_pub = false;
    for param in &mut def.params {
        param.ty = types.subst(param.ty, &tys);
    }
    def.ret = types.subst(def.ret, &tys);
    if let Some(body) = &mut def.body {
        for (_, local) in body.locals.iter_mut() {
            local.ty = types.subst(local.ty, &tys);
        }
        for (_, expr) in body.exprs.iter_mut() {
            expr.ty = types.subst(expr.ty, &tys);
            // The types an expression holds besides its own: the code a
            // closure is called through, a `dyn` method's type, and the
            // environment a drop function belongs to. Left generic, a
            // closure that answers a type parameter was called with the
            // wrong signature.
            match &mut expr.kind {
                ExprKind::CallClosure { code_ty, .. } => *code_ty = types.subst(*code_ty, &tys),
                ExprKind::DynCall { fn_ty, .. } => *fn_ty = types.subst(*fn_ty, &tys),
                ExprKind::DropRef(ty) => *ty = types.subst(*ty, &tys),
                _ => {}
            }
        }
    }
    program.fns.alloc(def)
}

/// The types of every field of a struct or enum type, with its type
/// arguments substituted, interned.
pub(crate) fn children(program: &mut Program, ty: Ty) -> Vec<Ty> {
    let (declared, args): (Vec<Ty>, TyList) = match program.types.kind(ty) {
        TyKind::Struct(id, args) => (
            program.structs[id].fields.iter().map(|f| f.ty).collect(),
            args,
        ),
        TyKind::Enum(id, args) => (
            program.enums[id]
                .variants
                .iter()
                .flat_map(|v| v.fields.iter().map(|f| f.ty))
                .collect(),
            args,
        ),
        _ => return Vec::new(),
    };
    let args = program.types.list(args).to_vec();
    declared
        .into_iter()
        .map(|t| program.types.subst(t, &args))
        .collect()
}

/// Interns the field types of the instances inside `ty`, and of those
/// inside them. Returns false if that ran into types nested too deeply to be
/// real, whose instances never end.
pub(crate) fn complete_type(program: &mut Program, ty: Ty) -> bool {
    let mut seen = FxHashSet::default();
    let mut work = vec![ty];
    while let Some(ty) = work.pop() {
        if program.types.depth(ty) > MAX_TYPE_DEPTH {
            return false;
        }
        if !seen.insert(ty) {
            continue;
        }
        match program.types.kind(ty) {
            TyKind::Own(t) | TyKind::Array(t, _) | TyKind::Slice(t) | TyKind::Ref(t, _) => {
                work.push(t)
            }
            TyKind::Struct(..) | TyKind::Enum(..) => work.extend(children(program, ty)),
            _ => {}
        }
    }
    true
}

/// Interns the field types of every instance interned since `from`, so that
/// later phases, which share the types read-only, can look them up.
pub(crate) fn complete_types(
    program: &mut Program,
    from: usize,
    interner: &Interner,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut reported = FxHashSet::default();
    let mut i = from;
    while i < program.types.len() {
        let ty = program.types.nth(i);
        i += 1;
        let (TyKind::Struct(_, args) | TyKind::Enum(_, args)) = program.types.kind(ty) else {
            continue;
        };
        if args == TyList::EMPTY {
            continue;
        }
        if program.types.depth(ty) > MAX_TYPE_DEPTH {
            let (name, span) = match program.types.kind(ty) {
                TyKind::Struct(id, _) => (program.structs[id].name, program.structs[id].span),
                TyKind::Enum(id, _) => (program.enums[id].name, program.enums[id].span),
                _ => unreachable!("matched above"),
            };
            if reported.insert(span) {
                let name = interner.resolve(name);
                let diagnostic = Diagnostic::error(
                    codes::INSTANTIATION_DEPTH,
                    format!("the instances of `{name}` never end"),
                    span,
                    "each instance holds a larger one",
                )
                .with_note(format!(
                    "types nested more than {MAX_TYPE_DEPTH} deep are taken to be an endless series"
                ));
                diagnostics.push(diagnostic);
            }
            continue;
        }
        children(program, ty);
    }
}

impl Program {
    /// What `wip test` calls a test: its name, and the module it belongs
    /// to where that is not the program's own, as `board::placesABall`.
    /// It is what the runner prints and what a filter
    /// is matched against.
    pub fn test_name(&self, id: FnId, interner: &Interner) -> String {
        let def = &self.fns[id];
        let name = interner.resolve(def.name);
        match self.modules.get(def.module as usize) {
            Some(module) if !module.is_empty() => format!("{module}::{name}"),
            _ => name.to_string(),
        }
    }

    /// A function's name for messages and listings: its module path, its
    /// name, and an instance's type arguments, as `list::push<i64>`.
    pub fn fn_name(&self, id: FnId, interner: &Interner) -> String {
        let def = &self.fns[id];
        let (module, args) = match def.instance_of {
            Some((generic, args)) => (self.fns[generic].module, args),
            None => (def.module, TyList::EMPTY),
        };
        let name = self.with_args(interner.resolve(def.name), args, interner);
        // A method is named after the type it belongs to.
        let name = match self.owner_of_fn(id) {
            Some(owner) => format!("{}.{name}", self.owner_name(owner, interner)),
            None => name,
        };
        let module = &self.modules[module as usize];
        if module.is_empty() {
            name
        } else {
            format!("{module}::{name}")
        }
    }

    /// The type a function belongs to, for an instance the type its generic
    /// function belongs to.
    pub fn owner_of_fn(&self, id: FnId) -> Option<TypeDef> {
        let def = &self.fns[id];
        match def.instance_of {
            Some((generic, _)) => self.fns[generic].owner,
            None => def.owner,
        }
    }

    /// The type a definition and its arguments name: `Vec<i64>` from
    /// `Vec` and `[i64]`. It is looked up rather than made, since anything
    /// that has a method of that type has the type already.
    pub fn type_of(&self, owner: TypeDef, args: crate::TyList) -> Option<Ty> {
        let kind = match owner {
            TypeDef::Struct(id) => TyKind::Struct(id, args),
            TypeDef::Enum(id) => TyKind::Enum(id, args),
            TypeDef::Builtin(_) => return None,
        };
        self.types.find(kind)
    }

    /// Every method a type declares, whichever kind of type it is.
    pub fn methods_of(&self, owner: TypeDef) -> &[FnId] {
        match owner {
            TypeDef::Struct(id) => &self.structs[id].methods,
            TypeDef::Enum(id) => &self.enums[id].methods,
            TypeDef::Builtin(builtin) => match self.builtins.get(&builtin) {
                Some(built) => &built.methods,
                None => &[],
            },
        }
    }

    /// The name of a struct or an enum, as a symbol. A built-in type has no
    /// declaration to take a name from; [`Program::owner_name`] answers for
    /// every kind.
    pub fn type_name_sym(&self, owner: TypeDef) -> Symbol {
        match owner {
            TypeDef::Struct(id) => self.structs[id].name,
            TypeDef::Enum(id) => self.enums[id].name,
            TypeDef::Builtin(_) => unreachable!("a built-in type has no name of its own"),
        }
    }

    /// The name of the type a method belongs to, for messages and symbols.
    pub fn owner_name(&self, owner: TypeDef, interner: &Interner) -> String {
        match owner {
            TypeDef::Struct(id) => interner.resolve(self.structs[id].name).to_string(),
            TypeDef::Enum(id) => interner.resolve(self.enums[id].name).to_string(),
            TypeDef::Builtin(builtin) => builtin.text(),
        }
    }

    /// Whether code is generated for a function: it has a body, and is not
    /// generic, and is not a constant's, which the compiler
    /// runs instead.
    pub fn is_compiled(&self, id: FnId) -> bool {
        let def = &self.fns[id];
        // A method the compiler writes has no body of a program's, and is
        // compiled all the same.
        (def.body.is_some() || def.generated.is_some())
            && def.generics.is_empty()
            && !def.compile_time
    }
}
