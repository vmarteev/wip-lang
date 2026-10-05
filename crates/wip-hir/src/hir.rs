//! The typed IR.
//!
//! Compared with the syntax tree: names are resolved to [`LocalId`]s,
//! [`FnId`]s and field or variant indices; every expression has a [`Ty`];
//! parentheses are gone; `else if` is an `else` block whose value is the
//! inner `if`; a block's value is separated from its statements, and a value
//! that is discarded is a statement; and auto-deref and the `&own<T>` to `&T`
//! coercion appear as explicit [`ExprKind::Deref`] and [`ExprKind::Ref`]
//! nodes. Spans are kept on every node for the diagnostics of later phases.

use la_arena::{Arena, Idx};
use rustc_hash::FxHashSet;
use wip_syntax::{Interner, Span, Symbol};

use crate::KnownInterface;
use crate::ty::{Ty, TyKind, TyList, Types};

pub type InterfaceId = Idx<InterfaceDef>;
pub type StructId = Idx<StructDef>;
pub type EnumId = Idx<EnumDef>;
pub type FnId = Idx<FnDef>;
/// A top-level `val`.
pub type ConstId = Idx<ConstDef>;
pub type GlobalId = Idx<GlobalDef>;
/// A C type declared `type name` in an extern block.
pub type OpaqueId = Idx<OpaqueDef>;
pub type LocalId = Idx<Local>;
pub type ExprId = Idx<Expr>;
pub type StmtId = Idx<Stmt>;

#[derive(Debug, Default, Clone)]
pub struct Program {
    pub types: Types,
    /// The interfaces of every module.
    pub interfaces: Arena<InterfaceDef>,
    /// What implements what: the interface, the type, and the function of
    /// each of the interface's methods, in its order.
    pub impls: Vec<ImplDef>,
    /// The tables of methods a program needs: one per interface and type
    /// used behind `&dyn`, with the function of each method.
    pub vtables: Vec<VTableDef>,
    /// Each top-level `assert`, as the function the compiler runs to check
    /// it.
    pub asserts: Vec<FnId>,
    /// Every module's path, `a::b::c`, with the root module first and
    /// empty.
    pub modules: Vec<String>,
    pub structs: Arena<StructDef>,
    pub enums: Arena<EnumDef>,
    /// Wip functions and extern declarations, in source order.
    pub fns: Arena<FnDef>,
    /// Whether moves leave poison and drops check for it:
    /// a debug build does, a release build does not. The
    /// build says so before the program is compiled.
    pub check_moves: bool,
    /// The C libraries the program links, from `@link("name")` on an extern
    /// block, in the order they were read and without repeats.
    pub libraries: Vec<String>,
    /// The frameworks an Apple target links, from `@framework("Cocoa")`,
    /// in the order they were read and without repeats.
    pub frameworks: Vec<String>,
    /// The directories the program's C looks for headers in, from
    /// `@include("src")`: the module that named each, as an index into
    /// `modules`, and the path relative to the module's own directory.
    pub c_includes: Vec<(usize, String)>,
    /// The C files a module names with `@source("src/rcore.c")`, which
    /// live below its own directory and are compiled with the program:
    /// the module, as an index into `modules`, the path relative to it,
    /// and the language it said they are, where it said.
    pub c_sources: Vec<(usize, String, Option<CLanguage>)>,
    /// The header a module puts before every C file of its own, from
    /// `@prefix("build.h")`: how a vendored library is configured, since
    /// its settings must reach each of its translation units.
    pub c_prefix: Vec<(usize, String)>,
    /// What each module's C is compiled with defined, `NAME` or
    /// `NAME=value`, by the module that said so.
    pub c_defines: Vec<(usize, String)>,
    /// Top-level `val`s: constants, whose values are known where they are
    /// written and put in place of every use.
    pub consts: Arena<ConstDef>,
    /// C types declared `type name`, whose contents Wip does not know.
    pub opaques: Arena<OpaqueDef>,
    /// The variables C owns, declared `val` or `var` in an extern block.
    pub globals: Arena<GlobalDef>,
    /// The prelude's items the compiler itself uses — the interface `==`
    /// goes through, the enum `main` may return — by what each is for.
    pub prelude_items: crate::PreludeItems,
    /// The environment of a closure made from a named function: a struct
    /// with nothing in it, shared by all of them.
    pub fn_closure_env: Option<StructId>,
    /// The writing halves of `lend fn`s, whose bodies are the reading
    /// halves' checked again with `self` a `&var Self`.
    pub lent_halves: rustc_hash::FxHashSet<FnId>,
    /// The methods `@derive` wrote as code: checked as the
    /// program's own, but placed at the annotation, so an editor passes
    /// over them.
    pub derived: rustc_hash::FxHashSet<FnId>,
    /// The instance of one of those two that this program's entry point
    /// calls, made by the monomorphizer.
    pub entry_report: Option<FnId>,
    /// The methods the prelude gave each built-in type.
    pub builtins: rustc_hash::FxHashMap<BuiltinOwner, BuiltinImpl>,
    /// For every type that cleans up after itself, the function to call
    /// where a value of it ends: concrete, so a generic type's instance has
    /// its own.
    pub drop_fns: rustc_hash::FxHashMap<Ty, FnId>,
    /// Names that leave no expression of their own, and what each named,
    /// for an editor: a type written in a signature, a
    /// field or an annotation, and a constant, whose value takes its place.
    pub names: Vec<(Span, Named)>,
    /// For every generator type a program ends a value of or walks, the
    /// instance of its `next`, which its drop calls too.
    pub generator_next: rustc_hash::FxHashMap<Ty, FnId>,
    /// The frame of each generator type: the types of the locals of its
    /// `next`, which the generator keeps between calls, after its declared
    /// fields. Known once the monomorphizer has made the instances, and
    /// read by [`Program::field_tys`].
    pub frames: rustc_hash::FxHashMap<Ty, Vec<Ty>>,
}

/// What a name written in the program named.
#[derive(Debug, Clone, Copy)]
pub enum Named {
    Type(Ty),
    Const(ConstId),
    /// The type an `extend` block gives methods to, or a static function
    /// or a variant is named through: `Ball` in `Ball::new()`.
    Owner(TypeDef),
    /// A variant, in an expression or a pattern.
    Variant(EnumId, u32),
}

impl Program {
    /// The type as it is written in source, for diagnostics.
    pub fn ty_name(&self, ty: Ty, interner: &Interner) -> String {
        self.name_of(ty, interner, false)
    }

    /// Type arguments as a symbol spells them: as a message would, but for
    /// a generator, which is named by the number of its struct, since
    /// every one would read `Iterator<T>`.
    pub fn symbol_args(&self, args: TyList, interner: &Interner) -> String {
        self.args_of("", args, interner, true)
    }

    /// A struct or enum with its type arguments, as a message writes it:
    /// without those at the end that are their defaults, as the program
    /// would write it. A symbol spells every one.
    fn written_args(
        &self,
        name: &str,
        generics: &[GenericParamDef],
        args: TyList,
        interner: &Interner,
        symbol: bool,
    ) -> String {
        let all = self.types.list(args);
        let mut shown = all.len();
        if !symbol {
            while shown > 0 {
                let Some(ParamDefault::Ty(default)) = generics.get(shown - 1).map(|p| p.default)
                else {
                    break;
                };
                if self.types.try_subst_find(default, &all[..shown - 1]) != Some(all[shown - 1]) {
                    break;
                }
                shown -= 1;
            }
        }
        if shown == all.len() {
            return self.args_of(name, args, interner, symbol);
        }
        if shown == 0 {
            return name.to_string();
        }
        let args: Vec<String> = all[..shown]
            .iter()
            .map(|&t| self.name_of(t, interner, symbol))
            .collect();
        format!("{name}<{}>", args.join(", "))
    }

    fn name_of(&self, ty: Ty, interner: &Interner, symbol: bool) -> String {
        match self.types.kind(ty) {
            TyKind::Error => "{unknown}".to_string(),
            TyKind::Never => "never".to_string(),
            TyKind::Ptr(inner) => format!("ptr<{}>", self.name_of(inner, interner, symbol)),
            TyKind::Slots(inner) => format!("Slots<{}>", self.name_of(inner, interner, symbol)),
            TyKind::Opaque(id) => interner.resolve(self.opaques[id].name).to_string(),
            TyKind::Unit => "void".to_string(),
            TyKind::Int(t) => t.name().to_string(),
            TyKind::Float(crate::FloatTy::F32) => "f32".to_string(),
            TyKind::Float(crate::FloatTy::F64) => "f64".to_string(),
            TyKind::Bool => "bool".to_string(),
            TyKind::Char => "char".to_string(),
            TyKind::Str => "str".to_string(),
            TyKind::Cstring => "cstring".to_string(),
            TyKind::Dyn(id, args) => {
                let name = format!("dyn {}", interner.resolve(self.interfaces[id].name));
                self.args_of(&name, args, interner, symbol)
            }
            // A tuple is the prelude's struct, and prints as the tuple it
            // was written as.
            TyKind::Struct(id, args) if self.structs[id].is_tuple => {
                let args: Vec<String> = self
                    .types
                    .list(args)
                    .iter()
                    .map(|&t| self.name_of(t, interner, symbol))
                    .collect();
                format!("({})", args.join(", "))
            }
            // A generator has no name a program could write, so it is
            // called by what it is.
            TyKind::Struct(id, args) if symbol && self.structs[id].generator.is_some() => self
                .args_of(
                    &format!("generator{}", u32::from(id.into_raw())),
                    args,
                    interner,
                    symbol,
                ),
            TyKind::Struct(id, args) if self.structs[id].generator.is_some() => {
                let elem = self.structs[id].generator.expect("checked").elem;
                let elem = self.types.subst_find(elem, self.types.list(args));
                format!("Iterator<{}>", self.name_of(elem, interner, symbol))
            }
            TyKind::Struct(id, args) => {
                let def = &self.structs[id];
                let name = self.declared_name(def.name, def.module, interner);
                self.written_args(&name, &def.generics, args, interner, symbol)
            }
            TyKind::Enum(id, args) => {
                let def = &self.enums[id];
                let name = self.declared_name(def.name, def.module, interner);
                self.written_args(&name, &def.generics, args, interner, symbol)
            }
            TyKind::Param(param) => interner.resolve(param.name).to_string(),
            TyKind::Fn(params, ret) => {
                let params: Vec<String> = self
                    .types
                    .list(params)
                    .iter()
                    .map(|&t| self.name_of(t, interner, symbol))
                    .collect();
                // Names are not part of the type, so messages leave them out.
                format!(
                    "({}) => {}",
                    params.join(", "),
                    self.name_of(ret, interner, symbol)
                )
            }
            TyKind::Own(inner) => format!("own<{}>", self.name_of(inner, interner, symbol)),
            TyKind::Ref(inner, crate::RefKind::Shared) => {
                format!("&{}", self.name_of(inner, interner, symbol))
            }
            TyKind::Ref(inner, crate::RefKind::Var) => {
                format!("&var {}", self.name_of(inner, interner, symbol))
            }
            TyKind::Array(elem, len) => {
                format!("[{}; {len}]", self.name_of(elem, interner, symbol))
            }
            TyKind::Slice(elem) => format!("[{}]", self.name_of(elem, interner, symbol)),
        }
    }

    /// A type's name as a message says it: with its module's path where
    /// the program has another type of that name, so that a program's own
    /// `Map` and `std::collections::Map` read apart. The
    /// root module's is written bare, as the program writes it.
    pub fn declared_name(&self, name: Symbol, module: u32, interner: &Interner) -> String {
        let bare = interner.resolve(name);
        let path = &self.modules[module as usize];
        if path.is_empty() {
            return bare.to_string();
        }
        let structs = self
            .structs
            .iter()
            .filter(|(_, s)| !s.is_env && !s.is_tuple)
            .map(|(_, s)| (s.name, s.module));
        let enums = self.enums.iter().map(|(_, e)| (e.name, e.module));
        let shared = structs
            .chain(enums)
            .any(|(other, owner)| other == name && owner != module);
        match shared {
            true => format!("{path}::{bare}"),
            false => bare.to_string(),
        }
    }

    /// `name<A, B>`, or `name` without type arguments.
    pub fn with_args(&self, name: &str, args: TyList, interner: &Interner) -> String {
        self.args_of(name, args, interner, false)
    }

    fn args_of(&self, name: &str, args: TyList, interner: &Interner, symbol: bool) -> String {
        let args = self.types.list(args);
        if args.is_empty() {
            return name.to_string();
        }
        let args: Vec<String> = args
            .iter()
            .map(|&t| self.name_of(t, interner, symbol))
            .collect();
        format!("{name}<{}>", args.join(", "))
    }

    /// The type of an array's or a slice's elements.
    pub fn element_ty(&self, ty: Ty) -> Ty {
        match self.types.kind(ty) {
            TyKind::Array(elem, _) | TyKind::Slice(elem) => elem,
            _ => unreachable!("elements of a type that is not an array or a slice"),
        }
    }

    /// The type of field `index` of a struct type, with the struct's type
    /// arguments in place of its parameters.
    pub fn field_ty(&self, ty: Ty, index: u32) -> Ty {
        let TyKind::Struct(id, args) = self.types.kind(ty) else {
            unreachable!("fields of a type that is not a struct")
        };
        let fields = &self.structs[id].fields;
        // A generator's frame follows its declared fields.
        if index as usize >= fields.len() {
            return self.frames[&ty][index as usize - fields.len()];
        }
        let field = fields[index as usize].ty;
        self.types.subst_find(field, self.types.list(args))
    }

    /// The types of a struct type's fields, in order: for a generator,
    /// its frame too, once it is known.
    pub fn field_tys(&self, ty: Ty) -> Vec<Ty> {
        let TyKind::Struct(id, _) = self.types.kind(ty) else {
            unreachable!("fields of a type that is not a struct")
        };
        let mut tys: Vec<Ty> = (0..self.structs[id].fields.len() as u32)
            .map(|i| self.field_ty(ty, i))
            .collect();
        if let Some(frame) = self.frames.get(&ty) {
            tys.extend(frame.iter().copied());
        }
        tys
    }

    /// The type of field `field` of variant `variant` of an enum type.
    pub fn variant_field_ty(&self, ty: Ty, variant: u32, field: u32) -> Ty {
        let TyKind::Enum(id, args) = self.types.kind(ty) else {
            unreachable!("variants of a type that is not an enum")
        };
        let field = self.enums[id].variants[variant as usize].fields[field as usize].ty;
        self.types.subst_find(field, self.types.list(args))
    }

    /// The types of the fields of each variant of an enum type.
    pub fn variant_field_tys(&self, ty: Ty) -> Vec<Vec<Ty>> {
        let TyKind::Enum(id, _) = self.types.kind(ty) else {
            unreachable!("variants of a type that is not an enum")
        };
        let variants = &self.enums[id].variants;
        (0..variants.len() as u32)
            .map(|v| {
                (0..variants[v as usize].fields.len() as u32)
                    .map(|f| self.variant_field_ty(ty, v, f))
                    .collect()
            })
            .collect()
    }

    /// Whether a type cleans up after itself: it implements `Destroy`,
    /// and the compiler calls its method where a value of
    /// it ends.
    pub fn has_drop(&self, ty: Ty) -> bool {
        // A generator left half-run drops what it holds at the `yield` it
        // stopped at, which its bits cannot say.
        if let TyKind::Struct(id, _) = self.types.kind(ty)
            && self.structs[id].generator.is_some()
        {
            return true;
        }
        let Some(interface) = self.prelude_items.interface(KnownInterface::Destroy) else {
            return false;
        };
        let Some(def) = self.type_def(ty) else {
            return false;
        };
        self.impls
            .iter()
            .any(|i| i.interface == interface && i.ty == def)
    }

    /// Whether `ty` implements `interface`. The checker has its own, which
    /// also knows what a type parameter promises; this one
    /// answers for a concrete type, which is what the entry point and the
    /// monomorphizer ask about.
    pub fn implements(&self, ty: Ty, interface: InterfaceId) -> bool {
        if self.is_clone(interface) && self.clones_by_copy(ty) {
            return true;
        }
        if let Some(inner) = self.referred(ty) {
            return self.answered_through_references(interface)
                && self.implements(inner, interface);
        }
        let owner = match self.type_def(ty) {
            Some(def) => def,
            None => match BuiltinOwner::of(self.types.kind(ty)) {
                Some(builtin) => TypeDef::Builtin(builtin),
                None => return false,
            },
        };
        self.impls
            .iter()
            .any(|i| i.interface == interface && i.ty == owner)
    }

    /// What a `&` reference to a value refers to, where `ty` is one: not a
    /// lent closure, which is its code and what it captured.
    pub fn referred(&self, ty: Ty) -> Option<Ty> {
        match self.types.kind(ty) {
            TyKind::Ref(inner, crate::RefKind::Shared)
                if !matches!(self.types.kind(inner), TyKind::Fn(..)) =>
            {
                Some(inner)
            }
            _ => None,
        }
    }

    /// Whether `ty` is plain data, whose clone is a copy of it: it owns no
    /// memory, and holds no `&var` reference or lent closure, of which a copy
    /// would make a second. A type whose fields were never made is not known to
    /// be, and is taken not to be.
    pub fn clones_by_copy(&self, ty: Ty) -> bool {
        fn plain(program: &Program, ty: Ty, seen: &mut FxHashSet<Ty>) -> bool {
            if program.has_drop(ty) || program.is_intrinsic_type(ty) {
                return false;
            }
            let types = &program.types;
            let each = |fields: Vec<Ty>, args, seen: &mut FxHashSet<Ty>| {
                fields.into_iter().all(|field| {
                    types
                        .try_subst_find(field, types.list(args))
                        .is_some_and(|field| plain(program, field, seen))
                })
            };
            match types.kind(ty) {
                TyKind::Own(_) | TyKind::Slots(_) | TyKind::Error => false,
                TyKind::Param(param) => param.copy,
                TyKind::Ref(_, crate::RefKind::Var) => false,
                TyKind::Ref(inner, _) => !matches!(types.kind(inner), TyKind::Fn(..)),
                TyKind::Struct(id, args) => {
                    let fields = program.structs[id].fields.iter().map(|f| f.ty).collect();
                    !seen.insert(ty) || each(fields, args, seen)
                }
                TyKind::Enum(id, args) => {
                    let fields = program.enums[id]
                        .variants
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|f| f.ty))
                        .collect();
                    !seen.insert(ty) || each(fields, args, seen)
                }
                TyKind::Array(elem, len) => len == 0 || plain(program, elem, seen),
                _ => true,
            }
        }
        plain(self, ty, &mut FxHashSet::default())
    }

    /// Whether `ty` is a struct the standard library declares `@intrinsic`.
    pub fn is_intrinsic_type(&self, ty: Ty) -> bool {
        matches!(self.types.kind(ty), TyKind::Struct(id, _) if self.structs[id].is_intrinsic)
    }

    /// Whether `ty` is an `Option` of a function, laid out as the C function
    /// pointer with `.None` as null.
    pub fn nullable_function(&self, ty: Ty) -> bool {
        let TyKind::Enum(id, args) = self.types.kind(ty) else {
            return false;
        };
        self.prelude_items.enumeration(crate::KnownEnum::Option) == Some(id)
            && matches!(self.types.kind(self.types.list(args)[0]), TyKind::Fn(..))
    }

    /// Whether `interface` is the prelude's `Clone`, which plain data
    /// implements by being copied.
    pub fn is_clone(&self, interface: InterfaceId) -> bool {
        self.prelude_items.interface(KnownInterface::Clone) == Some(interface)
    }

    /// Whether a `&T` implements `interface` where `T` does, answered by
    /// `T`'s implementation: every method reads `Self` through a `&` alone,
    /// as `Eq`, `Ord`, `Hash` and `Text` do, so each is given what the
    /// references point to. One that takes `Self` by value,
    /// changes it or answers one is not about the value alone.
    pub fn answered_through_references(&self, interface: InterfaceId) -> bool {
        // A parameter of the interface whose default is `Self` stands for
        // it, as `Add<Rhs = Self, Out = Self>`'s do: in a
        // method's types, `Self` is the first parameter and the
        // interface's own follow it.
        let generics = &self.interfaces[interface].generics;
        let stands_for_self = |index: u32| {
            index == 0
                || generics.get(index as usize - 1).is_some_and(|param| {
                    matches!(param.default, ParamDefault::Ty(default)
                        if self.types.any(default, &|kind| matches!(kind, TyKind::Param(p) if p.index == 0)))
                })
        };
        let is_self = |kind: TyKind| matches!(kind, TyKind::Param(p) if stands_for_self(p.index));
        let mentions_self = |ty: Ty| self.types.any(ty, &is_self);
        self.interfaces[interface].methods.iter().all(|method| {
            let def = &self.fns[method.id];
            let params_read = def
                .params
                .iter()
                .all(|param| match self.types.kind(param.ty) {
                    TyKind::Ref(inner, crate::RefKind::Shared)
                        if is_self(self.types.kind(inner)) =>
                    {
                        true
                    }
                    _ => !mentions_self(param.ty),
                });
            params_read && !mentions_self(def.ret)
        })
    }

    /// The function that drops a value of `ty`, where its type implements
    /// `Destroy`.
    pub fn drop_method(&self, ty: Ty) -> Option<FnId> {
        let interface = self.prelude_items.interface(KnownInterface::Destroy)?;
        let def = self.type_def(ty)?;
        self.impls
            .iter()
            .find(|i| i.interface == interface && i.ty == def)
            .and_then(|i| i.methods.first().copied())
    }

    /// The declaration a type belongs to, for looking up what implements
    /// what.
    fn type_def(&self, ty: Ty) -> Option<TypeDef> {
        match self.types.kind(ty) {
            TyKind::Struct(id, _) => Some(TypeDef::Struct(id)),
            TyKind::Enum(id, _) => Some(TypeDef::Enum(id)),
            _ => None,
        }
    }

    /// Whether a value of `ty` borrows: a `str`, a `view struct`, a reference,
    /// or anything that holds one — an array of them, or a generic type given
    /// one. Such a value is kept to parameters, locals and results, and to the
    /// fields of a view.
    pub fn holds_view(&self, ty: Ty) -> bool {
        match self.types.kind(ty) {
            TyKind::Str | TyKind::Ref(..) => true,
            TyKind::Struct(id, args) => {
                self.structs[id].is_view
                    || self.types.list(args).iter().any(|&a| self.holds_view(a))
            }
            TyKind::Enum(id, args) => {
                self.enums[id].is_view || self.types.list(args).iter().any(|&a| self.holds_view(a))
            }
            TyKind::Array(inner, _) | TyKind::Own(inner) | TyKind::Slice(inner) => {
                self.holds_view(inner)
            }
            _ => false,
        }
    }

    /// Whether dropping a value of type `ty` does anything: it owns heap
    /// memory — `own` anywhere inside it, through structs, arrays and enum
    /// payloads — or it, or something inside it, cleans up after itself.
    /// A type parameter may, unless it is `copy`.
    /// A value of this type is moved, not copied; so is
    /// one of a type the compiler knows, which holds no memory but must not
    /// be copied.
    pub fn owns_memory(&self, ty: Ty) -> bool {
        fn check(program: &Program, ty: Ty, seen: &mut FxHashSet<Ty>) -> bool {
            if program.has_drop(ty) || program.is_intrinsic_type(ty) {
                return true;
            }
            match program.types.kind(ty) {
                // A block of slots frees itself.
                TyKind::Own(_) | TyKind::Slots(_) => true,
                TyKind::Param(param) => !param.copy,
                TyKind::Struct(..) => {
                    seen.insert(ty)
                        && program
                            .field_tys(ty)
                            .into_iter()
                            .any(|f| check(program, f, seen))
                }
                TyKind::Enum(..) => {
                    seen.insert(ty)
                        && program
                            .variant_field_tys(ty)
                            .into_iter()
                            .flatten()
                            .any(|f| check(program, f, seen))
                }
                TyKind::Array(elem, len) => len > 0 && check(program, elem, seen),
                _ => false,
            }
        }
        check(self, ty, &mut FxHashSet::default())
    }
}

/// A type parameter of a generic item.
#[derive(Debug, Clone)]
pub struct GenericParamDef {
    /// The name the parameter was declared with, which its type carries.
    pub name: Symbol,
    /// The name it is written under here: an `extend` block may rename the
    /// type's parameters, and they are the same parameters.
    pub written: Symbol,
    /// Declared `T: copy`.
    pub copy: bool,
    /// The interfaces it must implement, with the types
    /// each takes where it takes any.
    pub interfaces: Vec<Constraint>,
    /// What it is where a use of the type leaves it out.
    pub default: ParamDefault,
    pub span: Span,
}

/// What a struct's or an enum's type parameter is where a use leaves it
/// out: `struct Arena<T, K = T>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParamDefault {
    /// Nothing: a use writes it, or it is inferred.
    #[default]
    None,
    /// Written, and not read yet: defaults are read after every type is
    /// named, and before the types' bodies.
    Pending,
    /// The type, which may name the parameters before it.
    Ty(Ty),
}

/// One constraint on a type parameter: an interface, and the types it takes
/// here — `T: Eq` has none, `C: Items<T>` has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constraint {
    pub interface: InterfaceId,
    pub args: TyList,
}

/// An interface: methods a type may implement. Its methods
/// are checked with `Self` as its one type parameter, so an implementation
/// substitutes the type for it.
#[derive(Debug, Clone)]
pub struct InterfaceDef {
    pub name: Symbol,
    /// `interface From<T>`: the types it is about, which come after `Self`
    /// in its methods' type parameters.
    pub generics: Vec<GenericParamDef>,
    /// One function per method, in the order they are written.
    pub methods: Vec<InterfaceMethodDef>,
    /// `@oneOf(toString, appendTo)`: groups of methods with defaults, of
    /// each of which an implementation writes one at least.
    pub one_of: Vec<OneOf>,
    /// The module that declares it, and whether it is exported.
    pub module: u32,
    pub is_pub: bool,
    pub span: Span,
}

/// One `@oneOf` of an interface: the methods it names, each where it was
/// named, and the annotation.
#[derive(Debug, Clone)]
pub struct OneOf {
    pub names: Vec<(Symbol, Span)>,
    pub span: Span,
}

/// A method of an interface.
#[derive(Debug, Clone, Copy)]
pub struct InterfaceMethodDef {
    pub id: FnId,
    /// Whether it has a default body, which an implementation may leave out.
    pub has_default: bool,
}

/// `extend Type: Interface`: which function implements each of the
/// interface's methods, in its order.
#[derive(Debug, Clone)]
pub struct ImplDef {
    pub interface: InterfaceId,
    /// What the interface's own type parameters are here: the arguments of
    /// `extend ConfigError: From<io::IoError>`.
    pub args: crate::TyList,
    pub ty: TypeDef,
    pub methods: Vec<FnId>,
    /// What the type's own parameters must satisfy for this implementation
    /// to hold: `extend Vec<T: Eq>: Eq` holds where `T` implements `Eq`,
    /// and holds always where nothing is written. One per
    /// parameter of the type, in its order.
    pub conditions: Vec<GenericParamDef>,
    /// The module that writes it.
    pub module: u32,
    /// The name of the type, as written.
    pub span: Span,
}

/// A type's table of methods for an interface, as code generation needs it:
/// every method concrete, in the interface's order.
#[derive(Debug, Clone)]
pub struct VTableDef {
    pub interface: InterfaceId,
    pub ty: Ty,
    pub methods: Vec<FnId>,
}

/// A type of the program: what a name means in the types scope, and which
/// type a method belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeDef {
    Struct(StructId),
    Enum(EnumId),
    /// A built-in type, which no module declares: the prelude may give it
    /// methods.
    Builtin(BuiltinOwner),
}

/// A built-in type an `extend` block may name, keyed by its shape rather
/// than by a type, since `extend [T]` is generic in its element.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BuiltinOwner {
    Int(crate::IntTy),
    Float(crate::FloatTy),
    Bool,
    /// `char`.
    Char,
    Str,
    Cstring,
    Slice,
    /// `Slots<T>`, the prelude's uninitialized storage.
    Slots,
}

impl BuiltinOwner {
    /// The built-in type a type belongs to, where it is one.
    pub fn of(kind: TyKind) -> Option<BuiltinOwner> {
        match kind {
            TyKind::Int(t) => Some(BuiltinOwner::Int(t)),
            TyKind::Float(t) => Some(BuiltinOwner::Float(t)),
            TyKind::Bool => Some(BuiltinOwner::Bool),
            TyKind::Char => Some(BuiltinOwner::Char),
            TyKind::Str => Some(BuiltinOwner::Str),
            TyKind::Cstring => Some(BuiltinOwner::Cstring),
            TyKind::Slice(_) => Some(BuiltinOwner::Slice),
            TyKind::Slots(_) => Some(BuiltinOwner::Slots),
            _ => None,
        }
    }

    /// The type as it is written, for messages and for symbols.
    pub fn text(self) -> String {
        match self {
            BuiltinOwner::Int(t) => t.name().to_string(),
            BuiltinOwner::Float(crate::FloatTy::F32) => "f32".to_string(),
            BuiltinOwner::Float(_) => "f64".to_string(),
            BuiltinOwner::Bool => "bool".to_string(),
            BuiltinOwner::Char => "char".to_string(),
            BuiltinOwner::Str => "str".to_string(),
            BuiltinOwner::Cstring => "cstring".to_string(),
            BuiltinOwner::Slice => "slice".to_string(),
            BuiltinOwner::Slots => "Slots".to_string(),
        }
    }
}

/// What the prelude declared for one built-in type.
#[derive(Debug, Clone, Default)]
pub struct BuiltinImpl {
    /// The element parameter of `extend [T]`; empty for the rest.
    pub generics: Vec<GenericParamDef>,
    pub methods: Vec<FnId>,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: Symbol,
    /// `extern struct`: C's layout, and fields C can write.
    pub is_extern: bool,
    /// `extern union`: every field at offset 0, over the same bytes.
    pub is_union: bool,
    /// `@opaque`: C knows where the fields are and Wip does not, so a value
    /// of this type is only ever behind a `ptr<T>`, and each field is read
    /// and written through C.
    pub is_opaque: bool,
    /// `@intrinsic`, in the standard library: a type the compiler knows,
    /// reached only through its intrinsic methods, and moved rather than
    /// copied though it holds no memory — `std::sync::Atomic`, a copy of
    /// which would be a read that is not atomic.
    pub is_intrinsic: bool,
    /// `@header("config.h")` on the declaration, which the C that reaches
    /// its fields includes.
    pub header: Option<Symbol>,
    /// For an `@opaque` struct, the pair of functions that read and write
    /// each field, in the fields' order.
    pub accessors: Vec<(FnId, FnId)>,
    /// Its type parameters, which its fields' types refer to.
    pub generics: Vec<GenericParamDef>,
    pub fields: Vec<FieldDef>,
    /// Its methods and static functions, in source order.
    pub methods: Vec<FnId>,
    /// What a closure captured: a hidden struct of references, made for one
    /// lambda.
    pub is_env: bool,
    /// `view struct`: may hold a `str` or a `&` reference, and is kept
    /// where a `str` is.
    pub is_view: bool,
    /// A tuple: the prelude's `TupleN`, which `(A, B)` is written into and
    /// which prints as it was written.
    pub is_tuple: bool,
    /// A generator: the state of a loop or a function that yields, made by
    /// the compiler and not named by a program.
    pub generator: Option<GeneratorDef>,
    /// The module that declares it, and whether it is exported.
    pub module: u32,
    pub is_pub: bool,
    /// The name in the declaration.
    pub span: Span,
}

/// What makes a struct a generator: the method that runs
/// it on to its next `yield`, and the type of what it yields. Its fields
/// are where it is, first, and then what it was given; what its `next`
/// keeps between calls comes after them, in [`Program::frames`].
#[derive(Debug, Clone, Copy)]
pub struct GeneratorDef {
    pub next: FnId,
    pub elem: Ty,
    /// The function that answers it, where it is a function that yields
    /// rather than a loop: its parameters are the generator's fields.
    pub of: Option<FnId>,
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    /// Whether another module may read it. A field is
    /// the declaring module's unless it says `pub`.
    pub is_pub: bool,
    /// Whether another module may write it: `pub var`.
    /// A `pub` field is read out there and written here.
    pub is_var: bool,
    pub name: Symbol,
    pub ty: Ty,
    pub span: Span,
    /// What a literal that leaves the field out puts there.
    pub default: Option<Box<DefaultValue>>,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: Symbol,
    pub generics: Vec<GenericParamDef>,
    pub variants: Vec<VariantDef>,
    /// Its methods and static functions, in source order.
    pub methods: Vec<FnId>,
    /// The module that declares it, and whether it is exported.
    pub module: u32,
    pub is_pub: bool,
    /// `view enum`: its variants may borrow.
    pub is_view: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VariantDef {
    pub name: Symbol,
    pub fields: Vec<FieldDef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: Symbol,
    pub name_span: Span,
    /// For a method, how it takes its receiver and the type it belongs to;
    /// the receiver is its first parameter, except for a `static fn`.
    pub receiver: Option<Receiver>,
    pub owner: Option<TypeDef>,
    /// For a method of an interface, that interface; its `Self` is the
    /// method's first type parameter.
    pub interface: Option<InterfaceId>,
    /// Its type parameters. A generic function's body is checked once, and
    /// never compiled: only its instances are.
    pub generics: Vec<GenericParamDef>,
    /// For an instance, the generic function and the type arguments it was
    /// made with.
    pub instance_of: Option<(FnId, TyList)>,
    pub params: Vec<ParamDef>,
    pub ret: Ty,
    pub ret_span: Option<Span>,
    pub is_extern: bool,
    /// `@export("C")`: C calls it by a plain symbol, its own name.
    pub exports_c: bool,
    /// `@header("<math.h>")` on the block it was declared in: it may be a
    /// macro or a `static inline`, with no symbol to call, so every call
    /// goes through C that includes the header.
    pub header: Option<Symbol>,
    /// `@symbol("sqlite3_open")`: what C calls it, when the declaration
    /// gives it a name of Wip's own.
    pub symbol: Option<Symbol>,
    /// What this reads, or writes when it takes the value as its last
    /// argument: an accessor the compiler declared, not a function C has.
    pub accesses: Option<Access>,
    /// `...`: a C function that takes more arguments than it declares.
    /// Each call to one is made through C of the
    /// compiler's writing, with the types that call passes.
    pub is_variadic: bool,
    /// For a declaration the compiler made from a call to a variadic one:
    /// which declaration it was made from. Its own
    /// parameters are the types that call passes.
    pub variadic_of: Option<FnId>,
    /// The hidden function of a lambda, which has no name of its own.
    pub is_lambda: bool,
    /// The `next` of this generator: its body is the loop or the function
    /// that yields, run on from where it last stopped.
    pub generator: Option<StructId>,
    /// `@tailrec`: every call it makes to itself is a tail call, and each is
    /// compiled as a jump back to the top.
    pub is_tailrec: bool,
    /// `@inline`: every call to it is spliced into its caller, or the
    /// program does not compile.
    pub is_inline: bool,
    /// `@test`: a test `wip test` runs. It takes nothing,
    /// answers nothing, and is compiled only by `wip test`, since it lives
    /// in a `.test.wip` file.
    pub is_test: bool,
    /// A body the compiler writes rather than a program.
    pub generated: Option<Generated>,
    /// `@intrinsic`: what the compiler writes in place of its body.
    pub intrinsic: Option<Intrinsic>,
    /// `None` for extern functions, and until the body has been checked.
    pub body: Option<Body>,
    /// A projection lends a place that belongs to one of its reference
    /// parameters, or to a constant table.
    /// Once the body has been checked, this says which; a projection's
    /// return type is a reference.
    pub projects: Option<Lent>,
    /// Whether the compiler runs it once the program is checked, and does
    /// not compile it: the initializer of a `@comptime` constant,
    /// or a top-level `assert`.
    pub compile_time: bool,
    /// The module that declares it, and whether it is exported.
    pub module: u32,
    pub is_pub: bool,
    /// The signature.
    pub span: Span,
}

/// A body the compiler writes. Each is a method of
/// `Slots<T>`, the one type that holds memory holding nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intrinsic {
    /// `Slots::alloc(count)`: a block of that many slots.
    SlotsAlloc,
    /// `slots.moveIn(index, value)`: the value goes in the slot, and nothing
    /// is dropped, since the slot held nothing.
    SlotsMoveIn,
    /// `slots.moveOut(index)`: the value comes out, and the slot holds
    /// nothing again.
    SlotsMoveOut,
    /// `assumed(&option)` and `assumedVar(&var option)`: what an `Option`
    /// holds, lent where it lies and not checked — for the standard
    /// library, where something it keeps says the option is not empty.
    OptionAssumed,
    /// `slots.slot(index)`: the slot, lent where it lies and not checked:
    /// the prelude has checked the index against what it knows holds a
    /// value, which the slots do not know.
    SlotsSlot,
    /// `slots.takeBuffer(length)`: the block, as a buffer of its first
    /// `length` slots, which hold values; the slots are left holding no
    /// block at all.
    SlotsTakeBuffer,
    /// `Slots::takeFrom(&var buffer)`: a buffer's block as slots, all of
    /// which hold values; the buffer is left empty.
    SlotsTakeFrom,
    /// `str::fromBytes(bytes)`: the bytes, seen as text. A `str` and a
    /// `&[u8]` are the same pair — a pointer and a length — so this copies
    /// it and says nothing about what the bytes mean.
    StrFromBytes,
    /// `cstring::fromBytes(bytes)`: the pointer alone, which the caller
    /// promises ends with a NUL.
    CstringFromBytes,
    /// `c::pointerTo(values)`: the address of a slice's first element, for
    /// C to read.
    SlicePointer,
    /// `x.toBits()` and `f64::fromBits(bits)`: a float's bits as the
    /// unsigned integer of its width, and back.
    FloatToBits,
    FloatFromBits,
    /// `x.sqrt()`, `x.floor()`, `x.ceil()`, `x.trunc()`: what the machine
    /// does exactly, one instruction each.
    FloatSqrt,
    FloatFloor,
    FloatCeil,
    FloatTrunc,
    /// `x.mulAdd(a, b)`: `x * a + b`, rounded once.
    FloatMulAdd,
    /// `x.countOnes()`, `x.leadingZeros()`, `x.trailingZeros()`,
    /// `x.swapBytes()`, `x.reverseBits()`: an integer's bits, one
    /// instruction each.
    IntCountOnes,
    IntLeadingZeros,
    IntTrailingZeros,
    IntSwapBytes,
    IntReverseBits,
    /// `x.rotateLeft(n)` and `x.rotateRight(n)`: its bits turned, those
    /// that leave one end coming in at the other.
    IntRotateLeft,
    IntRotateRight,
    /// `text.items()`: the bytes a `str` is, seen as a slice — the same
    /// pair the other way about, which is what lets `for byte in text`
    /// walk a `str`.
    StrBytes,
    /// `values.swap(i, j)`: the two elements change places. It is not a
    /// move out of either, since it leaves no hole, which is why it works
    /// for elements that own memory.
    SliceSwap,
    /// `char::fromScalar(code)`: the number, as the character it already
    /// is. The prelude calls it only where it has checked the number is a
    /// Unicode scalar value.
    CharFromScalar,
    /// `values.len()`, `text.len()`: the length a slice or a `str` carries
    /// beside its pointer. A direct call is read by the
    /// checker; this is the body the method has for everything else.
    Len,
    /// `intoAddress(own value)`: the address of the box, as a `ptr<u8>`
    /// C can hold; the box is no longer the `own`'s to free.
    OwnIntoAddress,
    /// `fromAddress(address)`: the box at an address `intoAddress` gave,
    /// owned again.
    OwnFromAddress,
    /// `unbox(boxed)`: the value an `own` holds, taken out, and the box
    /// freed with nothing in it dropped.
    OwnUnbox,
    /// `addressOf(&var value)`: the reference's address, as the `void *`
    /// C holds for a callback.
    ReferenceAddress,
    /// `referenceAt(data)`: the `&var` at an address `addressOf` gave, for
    /// a callback C made.
    AddressReference,
    /// `sizeOf<T>()` and `alignOf<T>()`: what C's `sizeof` and `_Alignof`
    /// answer for `T`, which the layout decides.
    SizeOf,
    AlignOf,
    /// `atomicAdd(at, amount)`: the integer at an address — a C pointer, or
    /// a `&` to an `Atomic`'s value — added to as one step that no other
    /// thread sees half of; answers what it held before.
    AtomicAdd,
    /// `atomicLoad(at)`: the value there, read as one step.
    AtomicLoad,
    /// `atomicStore(at, value)`: the value there, written as one step.
    AtomicStore,
    /// `boxAddress(value)`: the address of the box an `own` points to,
    /// which the `own` keeps: what C is given of a `c::Pinned`.
    BoxAddress,
    /// `sliceAt(first, count)`: the slice of `count` elements from an
    /// address, lent for writing: how a slice is lent in two parts that do
    /// not overlap, for the standard library alone.
    SliceAt,
    /// `closureCode(work)` and `closureCaptures(work)`: a closure lent for a
    /// call, as the address of its code and of what it captured, which a
    /// thread the call waits for is given.
    ClosureCode,
    ClosureCaptures,
    /// `atomicSubtract(at, amount)`, as `atomicAdd`.
    AtomicSubtract,
    /// `atomicSwap(at, value)`: written, answering what was there.
    AtomicSwap,
    /// `atomicCompareSwap(at, expected, value)`: written only where it held
    /// `expected`; answers what it held, so that it was written where that
    /// is `expected`.
    AtomicCompareSwap,
    /// `offsetBytes(pointer, bytes)`: the address that many bytes on from
    /// a C pointer, which C writes `p + n`.
    OffsetBytes,
    /// `frameTables()`: the program's tables of functions and lines, one
    /// for each module, listed in the prelude's object.
    FrameTables,
    /// `embed::bytes(path)` and `embed::text(path)`: a file's contents,
    /// read when the program is compiled, as a constant of the program.
    /// The checker answers them, and no call is made.
    EmbedBytes,
    EmbedText,
    /// `runtimeWords()`: the address of a block of zeroed words the
    /// program has once, which the allocator keeps its counters in.
    RuntimeWords,
}

/// A C type whose contents Wip does not know: `type sqlite3` in an extern
/// block. Only `ptr<sqlite3>` is ever written.
#[derive(Debug, Clone)]
pub struct OpaqueDef {
    pub name: Symbol,
    pub module: u32,
    /// Exported from its module, so that a module of bindings can name it
    /// in what it exports.
    pub is_pub: bool,
    pub span: Span,
}

/// What a C file of a module is compiled as, where `@source` says so
/// rather than its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CLanguage {
    C,
    ObjectiveC,
}

/// What a projection lends from, which is what a call to it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lent {
    /// The reference parameter of that index: the call holds the argument.
    Param(u32),
    /// Constant tables alone, which live as long as the program and which
    /// nothing writes: the call holds nothing.
    Tables,
}

/// A method whose body the compiler writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generated {
    /// `count()`: how many variants an enum that carries nothing has.
    VariantCount,
    /// `fromIndex(n)`: the variant with that number, or nothing.
    FromIndex,
    /// `all()`: every variant, in order, as an array.
    AllVariants,
}

/// What an accessor the compiler declared reaches: a variable C owns,
/// or a field of a struct C lays out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Access {
    Global(GlobalId),
    Field(StructId, u32),
}

/// A variable C owns: `extern "C" { var optind: c_int }`. Wip never reads or
/// writes it directly. Both go through C of the compiler's writing, since the
/// name may be a macro rather than a symbol — `errno` is one — and C is what
/// knows.
#[derive(Debug, Clone)]
pub struct GlobalDef {
    pub name: Symbol,
    /// `@symbol("__stderrp")`: what C calls it.
    pub symbol: Option<Symbol>,
    pub ty: Ty,
    /// `var`: Wip may write it. A `val` is read-only.
    pub is_mut: bool,
    /// `@header` on the block it was declared in, which the C that reads it
    /// includes.
    pub header: Option<Symbol>,
    /// The function a use of it becomes, and the one an assignment becomes.
    pub getter: FnId,
    pub setter: Option<FnId>,
    pub module: u32,
    pub is_pub: bool,
    pub span: Span,
}

/// A top-level `val`: a constant. Its value is worked out
/// where it is written, so nothing runs before `main` and a use is the value
/// itself.
#[derive(Debug, Clone)]
pub struct ConstDef {
    pub name: Symbol,
    pub ty: Ty,
    /// `None` until it has been worked out, and where working it out failed.
    pub value: Option<ConstValue>,
    /// Where folding cannot work it out, the function that computes it,
    /// which the compiler runs once the program is checked; its value is
    /// `None` until then.
    pub code: Option<FnId>,
    pub module: u32,
    pub is_pub: bool,
    pub span: Span,
}

/// What a constant is: the value a use of it stands for.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Int(u128),
    Float(f64),
    Bool(bool),
    /// A `str` or a `cstring` literal; which one the constant's type says.
    Str(Symbol),
    /// An array of constants.
    Array(Vec<ConstValue>),
    /// A struct or a tuple of constants, its fields in declaration order.
    Struct(Vec<ConstValue>),
    /// A variant of an enum, and the constants it holds.
    Variant {
        variant: u32,
        fields: Vec<ConstValue>,
    },
    /// A file's bytes, embedded: a `&[u8]`, or a `str`
    /// whose bytes are UTF-8, pointing at them where the program keeps
    /// them.
    Bytes(std::sync::Arc<[u8]>),
}

impl ConstValue {
    /// Whether the program keeps it once and reads it where it lies: an
    /// array, a struct or tuple, or a variant that holds something. A
    /// number, a `bool`, a string or a bare variant is written where it is
    /// used.
    /// Whether it holds a file's bytes, which are read
    /// where the program keeps them, never made again where it is used.
    pub fn holds_bytes(&self) -> bool {
        match self {
            ConstValue::Bytes(_) => true,
            ConstValue::Array(parts) | ConstValue::Struct(parts) => {
                parts.iter().any(ConstValue::holds_bytes)
            }
            ConstValue::Variant { fields, .. } => fields.iter().any(ConstValue::holds_bytes),
            ConstValue::Int(_)
            | ConstValue::Float(_)
            | ConstValue::Bool(_)
            | ConstValue::Str(_) => false,
        }
    }

    pub fn is_table(&self) -> bool {
        match self {
            ConstValue::Array(_) | ConstValue::Struct(_) | ConstValue::Bytes(_) => true,
            ConstValue::Variant { fields, .. } => !fields.is_empty(),
            ConstValue::Int(_)
            | ConstValue::Float(_)
            | ConstValue::Bool(_)
            | ConstValue::Str(_) => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: Symbol,
    pub name_span: Span,
    pub ty: Ty,
    pub span: Span,
    /// The value a call that gives the parameter nothing passes.
    pub default: Option<Box<DefaultValue>>,
}

/// A field's or a parameter's default.
#[derive(Debug, Clone)]
pub enum DefaultValue {
    /// A constant, copied into each literal or call that uses it.
    Constant { exprs: Arena<Expr>, root: ExprId },
    /// Code: the body of a function of its own, generic in the type
    /// parameters of the type or function it belongs to, which each literal
    /// or call that uses it calls with its own type arguments.
    Code(FnId),
}

#[derive(Debug, Default, Clone)]
pub struct Body {
    pub locals: Arena<Local>,
    /// For each binding that aliases part of a place, the expression it was
    /// matched from, so that a projection can tell what it lends.
    pub aliases: rustc_hash::FxHashMap<LocalId, ExprId>,
    /// Each binding that is an alias for part of a place, whose type is a
    /// reference to that part. A binding whose type is a reference and that is
    /// not here holds a reference of its own, a payload's `&Node`.
    pub alias_bindings: rustc_hash::FxHashSet<LocalId>,
    /// The `match`es that are a step of a walk one place at a time: the
    /// binding refers into the walked container, which the step holds, as a
    /// `for` over a slice does.
    pub sequence_walks: rustc_hash::FxHashSet<ExprId>,
    /// The bindings that took a copy of plain data: a `val … else`'s, so
    /// that a write through one says where it would go,
    /// and a `match`'s or an `is`'s that nothing writes through.
    pub copied_bindings: rustc_hash::FxHashSet<LocalId>,
    /// One local per parameter, in order.
    pub params: Vec<LocalId>,
    pub exprs: Arena<Expr>,
    pub stmts: Arena<Stmt>,
    /// The expression after `=`, whose value the function returns. Set once
    /// the body has been checked.
    pub value: Option<ExprId>,
    /// The calls of a `@tailrec` function to itself, which are compiled as a
    /// jump back to the top rather than a call.
    pub tail_calls: Vec<ExprId>,
}

impl Body {
    /// The function's body expression.
    pub fn value(&self) -> ExprId {
        self.value.expect("a checked body has a value")
    }

    /// Renumbers every type in the body. Only locals and expressions hold
    /// types, and calls and function values hold lists of them: nothing else
    /// inside an [`ExprKind`] or a [`Pattern`] is one, and a type added there
    /// would have to be renumbered here too.
    pub(crate) fn map_types(
        &mut self,
        map: impl Fn(Ty) -> Ty,
        map_list: impl Fn(TyList) -> TyList,
    ) {
        for (_, local) in self.locals.iter_mut() {
            local.ty = map(local.ty);
        }
        for (_, expr) in self.exprs.iter_mut() {
            expr.ty = map(expr.ty);
            match &mut expr.kind {
                ExprKind::Call { type_args, .. } | ExprKind::FnRef { type_args, .. } => {
                    *type_args = map_list(*type_args);
                }
                // The code's type as it is called.
                ExprKind::CallClosure { code_ty, .. } => *code_ty = map(*code_ty),
                // The environment whose drop function this is.
                ExprKind::DropRef(ty) => *ty = map(*ty),
                // The method's type as it is called.
                ExprKind::DynCall { fn_ty, .. } => *fn_ty = map(*fn_ty),
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Local {
    pub name: Symbol,
    pub ty: Ty,
    pub kind: LocalKind,
    /// The name where it is declared.
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    Param,
    Let {
        keyword: Span,
    },
    Var,
    /// A name bound by a `match` pattern.
    Binding,
}

impl Local {
    pub fn mutable(&self) -> bool {
        self.kind == LocalKind::Var
    }
}

#[derive(Debug, Default, Clone)]
pub struct Block {
    pub stmts: Vec<StmtId>,
    /// The block's value: its last expression, unless that is discarded (a
    /// block whose value is not needed keeps it as a statement).
    pub value: Option<ExprId>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Let {
        local: LocalId,
        init: ExprId,
    },
    /// `val pattern = value else { … }`: the bindings are
    /// in scope for the rest of the block; the `else` block leaves.
    Guard {
        scrutinee: ExprId,
        pattern: Pattern,
        /// `else { … }`; a pattern that cannot fail has none.
        else_block: Option<Block>,
    },
    Defer(ExprId),
    Return(Option<ExprId>),
    While {
        cond: ExprId,
        body: Block,
    },
    /// `for` over the elements of an array, a slice or a buffer.
    /// The pattern binds each element in turn, as a
    /// `match` binding refers to a field: a name, `_`, or a struct taken
    /// apart.
    ForElements {
        binding: Pattern,
        elements: ExprId,
        body: Block,
    },
    /// `for` over the integers from `lo` up to `hi`, not including it, or
    /// including it where it is `inclusive`.
    ForRange {
        binding: Option<LocalId>,
        lo: ExprId,
        hi: ExprId,
        inclusive: bool,
        body: Block,
    },
    /// How many loops out the jump goes: 0 is the innermost, and a name
    /// written on it says which.
    Break {
        depth: u32,
    },
    Continue {
        depth: u32,
    },
    Expr(ExprId),
    /// `yield value` in a generator: hands `.Some(value)`, which this
    /// holds, to whoever asked, and waits to be asked again.
    Yield(ExprId),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    /// The value's bits, truncated to the width of its type; a negative value
    /// is in two's complement.
    Int(u128),
    Float(f64),
    Bool(bool),
    Str(Symbol),
    /// A `&T` to a constant table, which the program keeps once, in
    /// read-only data: a use of the table is this, dereferenced.
    Table(ConstId),
    /// `null`: a C pointer that points at nothing.
    Null,
    /// A value of its type with every byte zero: what a field of an
    /// `extern struct` that a literal leaves out is, as C's designated
    /// initializer leaves it.
    Zeroed,
    /// `p.isNull`: whether a C pointer points at nothing.
    IsNull(ExprId),
    /// Two strings compared by their bytes: negative, zero or positive, as
    /// C's `memcmp` is.
    StrCmp {
        lhs: ExprId,
        rhs: ExprId,
    },
    /// `s.toStr()`: C's NUL-terminated bytes, seen as a `str` — the pointer
    /// C gave, and the length found by walking to the NUL.
    CstrToStr(ExprId),
    Local(LocalId),
    /// `yield place` in a projection: lends the place and returns its
    /// address to the caller.
    /// `lend place`.
    Lend(ExprId),
    /// A function named as a value: its address. A generic
    /// function has its type arguments, until an instance replaces it.
    FnRef {
        id: FnId,
        type_args: TyList,
    },
    /// A call through a value of a function type.
    CallValue {
        callee: ExprId,
        args: Vec<ExprId>,
    },
    /// A call. `args` are in the order of the parameters, and are evaluated
    /// in `order` ([`evaluation_order`]).
    Call {
        callee: FnId,
        args: Vec<ExprId>,
        /// The type arguments of a call to a generic function; empty once
        /// the call is to an instance.
        type_args: TyList,
        order: Vec<u32>,
    },
    Unary {
        op: UnaryOp,
        operand: ExprId,
    },
    Binary {
        op: BinaryOp,
        lhs: ExprId,
        rhs: ExprId,
        /// `+%`, `-%` and `*%`: the answer wraps rather than the program
        /// panicking.
        wrapping: bool,
    },
    /// A numeric conversion to the expression's type.
    Cast(ExprId),
    /// `colour as i64`: which variant an enum that carries nothing holds,
    /// as the expression's integer type.
    VariantIndex(ExprId),
    /// The length of an array or `str`, as an `i64`.
    Len(ExprId),
    Block(Block),
    /// Without an `else`, the type is `()`.
    If {
        cond: ExprId,
        then_block: Block,
        else_block: Option<Block>,
    },
    /// `place = value`, or `place op= value` with `op`,
    /// checked as the operator is, or wrapping where `wrapping` says.
    /// `place` satisfies [`Body::is_place`].
    Assign {
        place: ExprId,
        op: Option<BinaryOp>,
        wrapping: bool,
        value: ExprId,
    },
    /// A field of a struct-typed expression (never a reference or `own`).
    Field {
        base: ExprId,
        index: u32,
    },
    /// An element of an array or slice.
    Index {
        base: ExprId,
        index: ExprId,
    },
    /// The elements `lo..hi` of an array or slice, as a slice; only ever
    /// borrowed. An array coerced to a slice is this with
    /// no bounds.
    SubSlice {
        base: ExprId,
        lo: Option<ExprId>,
        hi: Option<ExprId>,
    },
    /// The value behind a `&T` or `own<T>`.
    Deref(ExprId),
    /// `&expr`: a reference to a place, or to a temporary.
    Ref(ExprId),
    Move(ExprId),
    /// `own expr`: a heap allocation.
    Own(ExprId),
    /// Field values in declaration order, evaluated in `order`.
    /// `Event { key: … }` or `Event {}`: a C union, whose bytes are zero
    /// and then the one field it names, since every field is the same
    /// bytes.
    Union {
        id: StructId,
        field: Option<(u32, ExprId)>,
    },
    Struct {
        id: StructId,
        fields: Vec<ExprId>,
        order: Vec<u32>,
    },
    /// Field values in declaration order, evaluated in `order`.
    Variant {
        id: EnumId,
        variant: u32,
        args: Vec<ExprId>,
        order: Vec<u32>,
    },
    Array(Vec<ExprId>),
    ArrayRepeat {
        elem: ExprId,
        count: u64,
    },
    /// `own [elem; count]` with a count known only at run time: `count`
    /// copies of `elem` in a new allocation, as an `own<[T]>`.
    OwnRepeat {
        elem: ExprId,
        count: ExprId,
    },
    /// An `own<[T; N]>` as an `own<[T]>`: the length moves from the type into
    /// the value.
    Unsize(ExprId),
    /// A `&T` as a `&dyn Interface`: the reference, with the table of `T`'s
    /// methods for that interface beside it.
    DynRef {
        value: ExprId,
        interface: InterfaceId,
    },
    /// A closure: what it captured, and the address of its code.
    /// `env` builds the captures — in the creator's frame
    /// for a closure lent for one call, on the heap for an owned one, whose
    /// environment holds its own drop function first.
    Closure {
        id: FnId,
        env: ExprId,
    },
    /// The address of the drop function of `own<ty>`, which an owned
    /// closure's environment carries so that a closure value can be dropped
    /// without knowing which lambda made it.
    DropRef(Ty),
    /// An owned closure lent for one call: the same pair, seen as
    /// `&(…) => R`.
    LendClosure(ExprId),
    /// A call through a closure: the code takes what was captured first.
    /// `code_ty` is the function type as it is called, with the captures'
    /// pointer before the arguments.
    CallClosure {
        callee: ExprId,
        args: Vec<ExprId>,
        code_ty: Ty,
    },
    /// A call through a `&dyn Interface`: the method at `index` of the
    /// interface, found in the table the reference carries. `args[0]` is the
    /// receiver.
    DynCall {
        interface: InterfaceId,
        index: u32,
        /// The method's type as it is called: the receiver is one pointer,
        /// whatever type it points to.
        fn_ty: Ty,
        args: Vec<ExprId>,
        /// The order the arguments are evaluated in.
        order: Vec<u32>,
    },
    /// `panic(message)`: the message and the place go to standard error, and
    /// the program ends. Its type is `never`. A message built
    /// when the program runs is `note`, a `str`, and what is written in the
    /// program follows it.
    Panic {
        note: Option<ExprId>,
        message: Option<Symbol>,
    },
    /// `value is pattern`: a `bool`. Its bindings, if any,
    /// are bound where the test succeeds.
    Is {
        scrutinee: ExprId,
        pattern: Pattern,
    },
    /// The scrutinee has already been dereferenced to the enum (or other
    /// value) the patterns test.
    Match {
        scrutinee: ExprId,
        arms: Vec<Arm>,
    },
    /// A tuple written as a `match`'s scrutinee, which is not built: each
    /// element is matched where it is, as a `match` on it alone would
    /// match it. Its type is the tuple's; nothing but the
    /// `match` reads it, and the arms' patterns are the tuple's.
    Places(Vec<ExprId>),
    Error,
}

/// The indices of `len` operands, in the order they are evaluated: the
/// order they were written. `order` lists them, and is empty
/// when that is the order they are stored in.
pub fn evaluation_order(order: &[u32], len: usize) -> impl Iterator<Item = usize> + '_ {
    (0..len).map(move |i| order.get(i).map_or(i, |&j| j as usize))
}

/// The `order` of operands evaluated in the sequence `written`, a list of
/// their indices: empty when that is already `0, 1, …`.
pub fn order_of(written: Vec<u32>) -> Vec<u32> {
    if written.iter().enumerate().all(|(i, &j)| i as u32 == j) {
        Vec::new()
    } else {
        written
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

pub use wip_syntax::ast::{BinaryOp, Receiver};

impl ExprKind {
    /// Every expression directly inside this one, in the order they are
    /// evaluated, but for what a block or a `match` arm holds: those are
    /// [`ExprKind::blocks`] and [`ExprKind::arms`], since an expression that
    /// holds statements is not the same as one that holds expressions.
    ///
    /// A walk written against this keeps working when a kind is added: the
    /// structure of an expression is described here, once.
    pub fn children(&self) -> Vec<ExprId> {
        let one = |e: ExprId| vec![e];
        match self {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Table(_)
            | ExprKind::Null
            | ExprKind::Zeroed
            | ExprKind::Local(_)
            | ExprKind::FnRef { .. }
            | ExprKind::DropRef(_)
            | ExprKind::Error => Vec::new(),
            ExprKind::Panic { note, .. } => note.iter().copied().collect(),
            ExprKind::Lend(e)
            | ExprKind::Unary { operand: e, .. }
            | ExprKind::Cast(e)
            | ExprKind::VariantIndex(e)
            | ExprKind::Len(e)
            | ExprKind::Field { base: e, .. }
            | ExprKind::Deref(e)
            | ExprKind::Ref(e)
            | ExprKind::Move(e)
            | ExprKind::Own(e)
            | ExprKind::ArrayRepeat { elem: e, .. }
            | ExprKind::Unsize(e)
            | ExprKind::IsNull(e)
            | ExprKind::CstrToStr(e)
            | ExprKind::DynRef { value: e, .. }
            | ExprKind::Closure { env: e, .. }
            | ExprKind::LendClosure(e)
            | ExprKind::Is { scrutinee: e, .. }
            | ExprKind::Match { scrutinee: e, .. }
            | ExprKind::If { cond: e, .. } => one(*e),
            ExprKind::Binary { lhs, rhs, .. } | ExprKind::StrCmp { lhs, rhs } => {
                vec![*lhs, *rhs]
            }
            ExprKind::Assign { place, value, .. } => vec![*place, *value],
            ExprKind::Index { base, index } => vec![*base, *index],
            ExprKind::OwnRepeat { elem, count } => vec![*elem, *count],
            ExprKind::SubSlice { base, lo, hi } => {
                [Some(*base), *lo, *hi].into_iter().flatten().collect()
            }
            ExprKind::Union { field, .. } => field.iter().map(|&(_, e)| e).collect(),
            ExprKind::Struct { fields: parts, .. }
            | ExprKind::Variant { args: parts, .. }
            | ExprKind::Array(parts)
            | ExprKind::Call { args: parts, .. }
            | ExprKind::Places(parts)
            | ExprKind::DynCall { args: parts, .. } => parts.clone(),
            ExprKind::CallValue { callee, args } | ExprKind::CallClosure { callee, args, .. } => {
                let mut parts = vec![*callee];
                parts.extend(args.iter().copied());
                parts
            }
            ExprKind::Block(_) => Vec::new(),
        }
    }

    /// The blocks this expression holds: its own, and the branches of an
    /// `if`. A block holds statements, so what is inside one runs whether or
    /// not the block produces a value.
    pub fn blocks(&self) -> Vec<&Block> {
        match self {
            ExprKind::Block(block) => vec![block],
            ExprKind::If {
                then_block,
                else_block,
                ..
            } => [Some(then_block), else_block.as_ref()]
                .into_iter()
                .flatten()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The arms of a `match`, whose bodies run in place of one another.
    pub fn arms(&self) -> &[Arm] {
        match self {
            ExprKind::Match { arms, .. } => arms,
            _ => &[],
        }
    }
}

impl StmtKind {
    /// The expressions directly inside this statement, in the order they are
    /// evaluated, but for what a block of it holds: those are
    /// [`StmtKind::blocks`], as for an expression.
    pub fn children(&self) -> Vec<ExprId> {
        match self {
            StmtKind::Let { init: e, .. }
            | StmtKind::Guard { scrutinee: e, .. }
            | StmtKind::Defer(e)
            | StmtKind::While { cond: e, .. }
            | StmtKind::ForElements { elements: e, .. }
            | StmtKind::Expr(e)
            | StmtKind::Yield(e) => vec![*e],
            StmtKind::ForRange { lo, hi, .. } => vec![*lo, *hi],
            StmtKind::Return(value) => value.iter().copied().collect(),
            StmtKind::Break { .. } | StmtKind::Continue { .. } => Vec::new(),
        }
    }

    /// The blocks this statement holds: a loop's body, a guard's `else`.
    pub fn blocks(&self) -> Vec<&Block> {
        match self {
            StmtKind::Guard {
                else_block: Some(b),
                ..
            }
            | StmtKind::While { body: b, .. }
            | StmtKind::ForElements { body: b, .. }
            | StmtKind::ForRange { body: b, .. } => vec![b],
            _ => Vec::new(),
        }
    }

    /// The locals this statement declares, if it declares any.
    pub fn declares(&self) -> Vec<LocalId> {
        match self {
            StmtKind::Let { local, .. } => vec![*local],
            StmtKind::ForRange { binding, .. } => binding.iter().copied().collect(),
            // A `for` binds a name, or a struct's fields.
            StmtKind::ForElements { binding, .. } => {
                let mut locals = Vec::new();
                binding.locals(&mut locals);
                locals
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub pattern: Pattern,
    /// `if …` after the pattern: the arm matches only where it is true.
    pub guard: Option<ExprId>,
    pub body: ExprId,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Binding(LocalId),
    /// `binders[i]` takes field `i`.
    Variant {
        variant: u32,
        binders: Vec<Binder>,
    },
    /// `Point(x, y)`: a struct taken apart where it is bound.
    /// It matches every value of its type, so it tests
    /// nothing and only binds.
    Fields(Vec<Binder>),
    /// A number the value must equal, in the bits of the scrutinee's own
    /// type.
    Int(u128),
    Bool(bool),
    /// Text the value must equal, by its bytes.
    Str(Symbol),
    /// The values from `lo` to `hi`, both included, in the bits of the
    /// scrutinee's type; an end left off is the type's least or greatest
    /// value, and is not tested.
    Range {
        lo: Option<u128>,
        hi: Option<u128>,
    },
    /// `.Round(..) | .Square(..)`: any one of them.
    Any(Vec<Pattern>),
    /// `[first, ..rest]`: the elements of an array or a slice, `prefix`
    /// from its start and `suffix` from its end. With a
    /// `rest`, it matches as many elements as these name or more, and the
    /// rest binds those between, as a slice; without one, exactly as many.
    Slice {
        prefix: Vec<Binder>,
        rest: Option<SliceRest>,
        suffix: Vec<Binder>,
    },
    Error,
}

/// The `..` of a slice pattern.
#[derive(Debug, Clone, Copy)]
pub enum SliceRest {
    /// `..`: the elements between are passed over.
    Ignored,
    /// `..rest`: they are bound, as a reference to a slice of where they
    /// lie.
    Bind(LocalId),
}

/// What a pattern does with one field.
#[derive(Debug, Clone)]
pub enum Binder {
    /// The field is not named, or is named `_`.
    Ignored,
    /// The field, bound under a name.
    Bind(LocalId),
    /// The field, taken apart in turn.
    Nested(Pattern),
}

impl Binder {
    /// The locals this binder declares, at any depth.
    pub fn locals(&self, out: &mut Vec<LocalId>) {
        match self {
            Binder::Ignored => {}
            Binder::Bind(local) => out.push(*local),
            Binder::Nested(pattern) => pattern.locals(out),
        }
    }
}

impl Pattern {
    /// What an arm's pattern on a tuple matched in place asks of each of
    /// its `n` elements: the tuple pattern's, or `_` for
    /// each where the arm takes them all.
    pub fn elements(&self, n: usize) -> Vec<Pattern> {
        match self {
            Pattern::Fields(binders) => binders
                .iter()
                .map(|binder| match binder {
                    Binder::Ignored => Pattern::Wildcard,
                    Binder::Bind(local) => Pattern::Binding(*local),
                    Binder::Nested(pattern) => pattern.clone(),
                })
                .collect(),
            Pattern::Error => vec![Pattern::Error; n],
            _ => vec![Pattern::Wildcard; n],
        }
    }

    /// The locals this pattern declares, at any depth.
    pub fn locals(&self, out: &mut Vec<LocalId>) {
        match self {
            Pattern::Wildcard
            | Pattern::Error
            | Pattern::Int(_)
            | Pattern::Range { .. }
            | Pattern::Bool(_)
            | Pattern::Str(_) => {}
            Pattern::Binding(local) => out.push(*local),
            Pattern::Variant { binders, .. } | Pattern::Fields(binders) => {
                binders.iter().for_each(|b| b.locals(out))
            }
            Pattern::Slice {
                prefix,
                rest,
                suffix,
            } => {
                prefix.iter().for_each(|b| b.locals(out));
                if let Some(SliceRest::Bind(local)) = rest {
                    out.push(*local);
                }
                suffix.iter().for_each(|b| b.locals(out));
            }
            // Any one of several.
            Pattern::Any(alternatives) => alternatives.iter().for_each(|p| p.locals(out)),
        }
    }
}

impl Body {
    /// Whether `id` denotes a storage location: a local, a field or element
    /// of one, or the value behind a reference or `own`.
    pub fn is_place(&self, id: ExprId) -> bool {
        match self.exprs[id].kind {
            ExprKind::Local(_) | ExprKind::Deref(_) => true,
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.is_place(base),
            _ => false,
        }
    }
}
