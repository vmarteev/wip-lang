//! What a call's result may borrow, read from the callee's signature.
//!
//! An argument is among a result's roots only where its type can hold what
//! the result refers to: a `&T` in the result refers to `T`, and a `str` to
//! a text's bytes, `u8`. A reference argument whose pointee holds that as
//! its own — through fields, variants, elements, `own` and owned buffers —
//! lends its place; an argument that reaches it through a borrow lends what
//! it borrows. A type parameter holds itself, and what the results of its
//! interfaces' methods hold: code generic over `K: Hash + Eq` cannot make a
//! `&V` out of a `K`. The signature is read as written, so every call of a
//! function lends the same, and its body can be held to it.

use rustc_hash::FxHashSet;
use wip_hir::{GenericParamDef, TyList};

use super::*;

/// What one parameter lends to what a call answers.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct Lends {
    /// The place a reference argument refers to.
    pub(super) place: bool,
    /// What the argument borrows: a `str`'s bytes, a view's roots.
    pub(super) borrows: bool,
}

impl Lends {
    /// Both: what an argument lends whose parameter is not known, a
    /// function value's, or one past a variadic function's parameters.
    pub(super) const ALL: Lends = Lends {
        place: true,
        borrows: true,
    };
}

/// What a value refers to: the types of the places its references and
/// `str`s point into, and the type parameters it holds values of, which may
/// borrow in turn.
#[derive(Default)]
struct Refers {
    places: FxHashSet<Ty>,
    values: FxHashSet<Ty>,
    /// It may point anywhere: a closure it holds lends what it captured.
    any: bool,
}

impl Refers {
    fn is_empty(&self) -> bool {
        self.places.is_empty() && self.values.is_empty() && !self.any
    }
}

/// What a value can hold: the types reached from its own as its own, and
/// those reached through a borrow.
#[derive(Default)]
struct Holds {
    own: FxHashSet<Ty>,
    borrowed: FxHashSet<Ty>,
    /// It reaches something whose contents are not known where the
    /// function is written: a `dyn`, a closure.
    any: bool,
}

/// The type parameters a signature is written with, whose interfaces say
/// what each may hold.
struct Signature<'p> {
    program: &'p Program,
    generics: &'p [GenericParamDef],
}

impl Signature<'_> {
    /// What `ty` refers to, added to `refers`. `env` maps the type
    /// parameters of an interface's method to the signature's, where the
    /// type is read from one; a parameter it does not map is the method's
    /// own, which the caller supplies.
    fn refers(
        &self,
        ty: Ty,
        env: Option<&[Ty]>,
        refers: &mut Refers,
        seen: &mut FxHashSet<(Ty, Option<Vec<Ty>>)>,
    ) {
        let types = &self.program.types;
        if !seen.insert((ty, env.map(<[Ty]>::to_vec))) {
            return;
        }
        match types.kind(ty) {
            TyKind::Str => {
                refers.places.insert(Types::U8);
            }
            TyKind::Ref(inner, _) => {
                let pointee = match types.kind(inner) {
                    // A lent closure refers to what it captured, and a `dyn`
                    // to a value of any type.
                    TyKind::Fn(..) | TyKind::Dyn(..) => {
                        refers.any = true;
                        return;
                    }
                    // A slice is a run of elements, wherever they are held:
                    // in an array, a `Vec`'s buffer, one on its own.
                    TyKind::Slice(elem) => elem,
                    _ => inner,
                };
                match env {
                    // A type read from an interface's method means the same
                    // in the signature only where it has no parameters, or
                    // is one that maps.
                    Some(env) if types.is_generic(pointee) => match types.kind(pointee) {
                        TyKind::Param(param) => {
                            if let Some(&mapped) = env.get(param.index as usize) {
                                refers.places.insert(mapped);
                            }
                        }
                        _ => refers.any = true,
                    },
                    _ => {
                        refers.places.insert(pointee);
                    }
                }
                self.refers(pointee, env, refers, seen);
            }
            TyKind::Struct(..) => {
                for field in self.program.field_tys(ty) {
                    self.refers(field, env, refers, seen);
                }
            }
            TyKind::Enum(..) => {
                for field in self.program.variant_field_tys(ty).into_iter().flatten() {
                    self.refers(field, env, refers, seen);
                }
            }
            TyKind::Array(elem, _)
            | TyKind::Slice(elem)
            | TyKind::Own(elem)
            | TyKind::Slots(elem) => self.refers(elem, env, refers, seen),
            TyKind::Param(param) => match env {
                Some(env) => {
                    if let Some(&mapped) = env.get(param.index as usize) {
                        self.refers(mapped, None, refers, seen);
                    }
                }
                None => {
                    refers.values.insert(ty);
                }
            },
            _ => {}
        }
    }

    /// What `ty` can hold, added to `holds`: as its own, or through a
    /// borrow where `borrowed`. `env` maps the type parameters of an
    /// interface's method to the signature's, where the type is read from
    /// one; a parameter it does not map is the method's own, which the
    /// caller supplies.
    fn holds(
        &self,
        ty: Ty,
        borrowed: bool,
        env: Option<&[Ty]>,
        holds: &mut Holds,
        seen: &mut FxHashSet<(Ty, bool, Option<Vec<Ty>>)>,
    ) {
        let types = &self.program.types;
        if !seen.insert((ty, borrowed, env.map(<[Ty]>::to_vec))) {
            return;
        }
        if let TyKind::Param(param) = types.kind(ty) {
            match env {
                Some(env) => {
                    if let Some(&mapped) = env.get(param.index as usize) {
                        self.holds(mapped, borrowed, None, holds, seen);
                    }
                }
                None => {
                    if borrowed {
                        holds.borrowed.insert(ty);
                    } else {
                        holds.own.insert(ty);
                    }
                    self.param_holds(ty, param.index as usize, borrowed, holds, seen);
                }
            }
            return;
        }
        // A type read from an interface's method is recorded where it means
        // the same in the signature: where it has no type parameters.
        if env.is_none() || !types.is_generic(ty) {
            if borrowed {
                holds.borrowed.insert(ty);
            } else {
                holds.own.insert(ty);
            }
        }
        match types.kind(ty) {
            // A `str`'s bytes are borrowed, and so is what a reference
            // refers to.
            TyKind::Str => self.holds(Types::U8, true, None, holds, seen),
            // A C string's bytes are C's, and no owner of theirs is known,
            // so what `toStr()` reads of one is taken to lie where the C
            // string does: what is read from `main`'s `args: &[cstring]`
            // borrows `args`, which a caller keeps while it is used.
            TyKind::Cstring => self.holds(Types::U8, borrowed, None, holds, seen),
            TyKind::Ref(inner, _) => self.holds(inner, true, env, holds, seen),
            TyKind::Struct(..) => {
                for field in self.program.field_tys(ty) {
                    self.holds(field, borrowed, env, holds, seen);
                }
            }
            TyKind::Enum(..) => {
                for field in self.program.variant_field_tys(ty).into_iter().flatten() {
                    self.holds(field, borrowed, env, holds, seen);
                }
            }
            TyKind::Array(elem, _)
            | TyKind::Slice(elem)
            | TyKind::Own(elem)
            | TyKind::Slots(elem) => self.holds(elem, borrowed, env, holds, seen),
            // What C points at is C's to keep.
            TyKind::Ptr(elem) => self.holds(elem, true, env, holds, seen),
            // A closure may hold anything it captured, and a `dyn` is a
            // value of any type.
            TyKind::Fn(..) | TyKind::Dyn(..) => holds.any = true,
            _ => {}
        }
    }

    /// What type parameter `index` holds besides itself: what the results
    /// of its interfaces' methods hold and refer to, with `Self` read as
    /// the parameter.
    fn param_holds(
        &self,
        param: Ty,
        index: usize,
        borrowed: bool,
        holds: &mut Holds,
        seen: &mut FxHashSet<(Ty, bool, Option<Vec<Ty>>)>,
    ) {
        let Some(def) = self.generics.get(index) else {
            // A parameter the signature does not declare: nothing is known
            // of it.
            holds.any = true;
            return;
        };
        for constraint in &def.interfaces {
            let env = self.method_env(param, constraint.args);
            let interface = &self.program.interfaces[constraint.interface];
            for method in &interface.methods {
                let ret = self.program.fns[method.id].ret;
                self.holds(ret, borrowed, Some(&env), holds, seen);
                // What the answer refers to may lie in the parameter's own
                // place, or where it borrows: `name(): str` may answer the
                // bytes of a `String` inside it.
                let mut refers = Refers::default();
                self.refers(ret, Some(&env), &mut refers, &mut FxHashSet::default());
                holds.any |= refers.any;
                for ty in refers.places.into_iter().chain(refers.values) {
                    holds.own.insert(ty);
                    holds.borrowed.insert(ty);
                }
            }
        }
    }

    /// How an interface's method names the types a constraint gives it:
    /// `Self` first, then the interface's own parameters.
    fn method_env(&self, param: Ty, args: TyList) -> Vec<Ty> {
        let mut env = vec![param];
        env.extend_from_slice(self.program.types.list(args));
        env
    }

    /// What a parameter of type `param` lends to a value that refers to
    /// `refers`.
    fn lends(&self, refers: &Refers, param: Ty) -> Lends {
        if refers.is_empty() {
            return Lends::default();
        }
        let types = &self.program.types;
        let (is_ref, content) = match types.kind(param) {
            TyKind::Ref(inner, _) => (true, inner),
            _ => (false, param),
        };
        let mut holds = Holds::default();
        self.holds(content, false, None, &mut holds, &mut FxHashSet::default());
        if holds.any {
            return Lends {
                place: is_ref,
                borrows: true,
            };
        }
        let meets = |targets: &FxHashSet<Ty>, held: &FxHashSet<Ty>| {
            targets.iter().any(|target| held.contains(target))
        };
        let place = is_ref && (refers.any || meets(&refers.places, &holds.own));
        let borrows = (refers.any && !holds.borrowed.is_empty())
            || meets(&refers.places, &holds.borrowed)
            || meets(&refers.values, &holds.own)
            || meets(&refers.values, &holds.borrowed);
        Lends { place, borrows }
    }
}

/// What each of `params` lends to a value of type `to`, in a signature
/// written with `generics`.
pub(super) fn lends_to(
    program: &Program,
    generics: &[GenericParamDef],
    params: &[Ty],
    to: Ty,
) -> Vec<Lends> {
    let signature = Signature { program, generics };
    let mut refers = Refers::default();
    signature.refers(to, None, &mut refers, &mut FxHashSet::default());
    signature.through_functions(params, &mut refers);
    params
        .iter()
        .map(|&param| signature.lends(&refers, param))
        .collect()
}

impl Signature<'_> {
    /// What a function the call is given may make of what it is passed:
    /// where its result may refer to what the call's
    /// result refers to, so may what its parameters refer to, and the call's
    /// arguments of those types lend to the call's result. `map(f)` on an
    /// `Option<&String>` makes `f`'s `str` out of the `&String` it is
    /// passed, so the result borrows what the option borrows.
    fn through_functions(&self, params: &[Ty], refers: &mut Refers) {
        let mut functions = Vec::new();
        for &param in params {
            self.functions_in(param, &mut functions, &mut FxHashSet::default());
        }
        loop {
            let mut grew = false;
            for &(fn_params, ret) in &functions {
                let mut made = Refers::default();
                self.refers(ret, None, &mut made, &mut FxHashSet::default());
                let reaches = made.any
                    || made.places.iter().any(|t| refers.places.contains(t))
                    || made.values.iter().any(|t| refers.values.contains(t))
                    || (refers.any && !made.is_empty());
                if !reaches {
                    continue;
                }
                for &input in self.program.types.list(fn_params) {
                    let mut passed = Refers::default();
                    self.refers(input, None, &mut passed, &mut FxHashSet::default());
                    // What the function is passed by value may be borrowed
                    // as it is: a type parameter's value is itself.
                    self.values_in(input, &mut passed.values);
                    for ty in passed.places {
                        grew |= refers.places.insert(ty);
                    }
                    for ty in passed.values {
                        grew |= refers.values.insert(ty);
                    }
                    if passed.any && !refers.any {
                        refers.any = true;
                        grew = true;
                    }
                }
            }
            if !grew {
                return;
            }
        }
    }

    /// The function types `ty` holds or is: a function value, a closure lent
    /// or owned, as the parameters and the result each is written with.
    fn functions_in(&self, ty: Ty, out: &mut Vec<(TyList, Ty)>, seen: &mut FxHashSet<Ty>) {
        if !seen.insert(ty) {
            return;
        }
        match self.program.types.kind(ty) {
            TyKind::Fn(params, ret) => out.push((params, ret)),
            TyKind::Ref(inner, _) | TyKind::Own(inner) => self.functions_in(inner, out, seen),
            _ => {}
        }
    }

    /// The type parameters `ty` holds values of directly, as itself.
    fn values_in(&self, ty: Ty, values: &mut FxHashSet<Ty>) {
        match self.program.types.kind(ty) {
            TyKind::Param(_) => {
                values.insert(ty);
            }
            TyKind::Ref(inner, _) => self.values_in(inner, values),
            _ => {}
        }
    }
}

/// What each parameter of `def` lends to its result.
pub(super) fn signature_lends(program: &Program, def: &FnDef) -> Vec<Lends> {
    let params: Vec<Ty> = def.params.iter().map(|p| p.ty).collect();
    lends_to(program, &def.generics, &params, def.ret)
}
