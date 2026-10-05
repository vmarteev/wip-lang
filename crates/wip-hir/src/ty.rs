use indexmap::IndexSet;
use wip_syntax::Symbol;

use crate::hir::{EnumId, StructId};

/// An interned type. Two `Ty`s are equal exactly when the types are equal.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Ty(u32);

/// An interned list of types: the type arguments of a generic struct, enum
/// or function.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TyList(u32);

impl TyList {
    /// No type arguments: every type that is not generic.
    pub const EMPTY: TyList = TyList(0);
}

/// A type parameter of the generic item it is written in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TyParam {
    /// Its position in the item's list of type parameters.
    pub index: u32,
    pub name: Symbol,
    /// Declared `T: copy`: it owns no memory.
    pub copy: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TyKind {
    /// The type of an expression that already produced an error. It is
    /// compatible with everything, so one mistake does not cause more errors.
    Error,
    /// The type of an expression that never produces a value, such as a
    /// `match` whose arms all return.
    Never,
    /// `void`: what a function without a return type returns.
    Unit,
    Int(IntTy),
    Float(FloatTy),
    Bool,
    /// A Unicode scalar value: a number from 0 to 10FFFF that is not a
    /// surrogate, in 32 bits.
    Char,
    /// A pointer and a length.
    Str,
    /// A pointer to NUL-terminated bytes, for C.
    Cstring,
    /// A struct, with its type arguments if it is generic.
    Struct(StructId, TyList),
    Enum(EnumId, TyList),
    /// A type parameter, inside the generic item that declares it.
    Param(TyParam),
    /// `(a: A, b: B) => R`: the address of a function that takes the types in
    /// the list and returns the second type, `void` if nothing.
    /// Parameter names are not part of the type.
    Fn(TyList, Ty),
    Own(Ty),
    Ref(Ty, RefKind),
    Array(Ty, u64),
    /// `[T]`: elements of a number known only at run time. Only behind a
    /// reference.
    Slice(Ty),
    /// `dyn Interface`: a value of some type that implements it, known only
    /// behind a reference, as a pointer and a table of methods.
    Dyn(crate::InterfaceId, TyList),
    /// `ptr<T>`: a C pointer. It may be null, it is copied
    /// like an integer, it is never dropped, and the move checker leaves it
    /// alone — which is why it can be held in a variable or a field, where a
    /// reference cannot.
    Ptr(Ty),
    /// A C type whose contents Wip does not know, declared `type name` in an
    /// extern block. No value of one exists: it is only ever
    /// used as `ptr<name>`.
    Opaque(crate::OpaqueId),
    /// `Slots<T>`: a block of slots, none of them initialized — the one
    /// value in Wip whose contents may be nothing at all. Only the prelude
    /// may name it, and `Vec<T>` is what it is for.
    Slots(Ty),
}

/// The integer types.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}

impl IntTy {
    pub fn signed(self) -> bool {
        use IntTy::*;
        matches!(self, I8 | I16 | I32 | I64 | I128 | Isize)
    }

    /// The width in bits. `isize` and `usize` are 64 bits wide: the
    /// prototype targets 64-bit platforms only.
    pub fn bits(self) -> u32 {
        use IntTy::*;
        match self {
            I8 | U8 => 8,
            I16 | U16 => 16,
            I32 | U32 => 32,
            I64 | U64 | Isize | Usize => 64,
            I128 | U128 => 128,
        }
    }

    pub fn name(self) -> &'static str {
        use IntTy::*;
        match self {
            I8 => "i8",
            I16 => "i16",
            I32 => "i32",
            I64 => "i64",
            I128 => "i128",
            Isize => "isize",
            U8 => "u8",
            U16 => "u16",
            U32 => "u32",
            U64 => "u64",
            U128 => "u128",
            Usize => "usize",
        }
    }

    /// The bits of the type's width set.
    pub fn mask(self) -> u128 {
        match self.bits() {
            128 => u128::MAX,
            bits => (1 << bits) - 1,
        }
    }

    /// The largest value.
    pub fn max(self) -> u128 {
        if self.signed() {
            self.mask() >> 1
        } else {
            self.mask()
        }
    }

    /// The magnitude of the smallest value: 0 for an unsigned type.
    pub fn min_magnitude(self) -> u128 {
        if self.signed() {
            1 << (self.bits() - 1)
        } else {
            0
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FloatTy {
    F32,
    F64,
}

/// What a reference allows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RefKind {
    /// `&T`: reads.
    Shared,
    /// `&var T`: reads and writes, and excludes every other reference to the
    /// same place.
    Var,
}

/// What a type contains, anywhere inside it.
#[derive(Debug, Clone, Copy, Default)]
struct Facts {
    /// A type parameter.
    generic: bool,
    /// The error type.
    error: bool,
    reference: bool,
    /// How deeply its parts nest: 1 for a type without parts.
    depth: u32,
}

/// The type interner. Interning order is deterministic, so `Ty` values are
/// stable for a given program.
#[derive(Debug, Clone)]
pub struct Types {
    set: IndexSet<TyKind>,
    /// What each type contains, recorded when it is interned, so that no
    /// question about a type's parts walks them: a type can share parts many
    /// times over, as `Pair<Pair<T, T>, Pair<T, T>>` does.
    facts: Vec<Facts>,
    lists: IndexSet<Box<[Ty]>>,
    /// Types and lists, in the order they were interned, so that another
    /// interner can absorb them in the same order.
    log: Vec<Interned>,
}

#[derive(Debug, Clone, Copy)]
enum Interned {
    Ty,
    List,
}

impl Types {
    pub const ERROR: Ty = Ty(0);
    pub const NEVER: Ty = Ty(1);
    pub const UNIT: Ty = Ty(2);
    pub const I64: Ty = Ty(3);
    pub const F64: Ty = Ty(4);
    pub const BOOL: Ty = Ty(5);
    pub const STR: Ty = Ty(6);
    pub const I32: Ty = Ty(7);
    pub const I8: Ty = Ty(8);
    pub const I16: Ty = Ty(9);
    pub const I128: Ty = Ty(10);
    pub const ISIZE: Ty = Ty(11);
    pub const U8: Ty = Ty(12);
    pub const U16: Ty = Ty(13);
    pub const U32: Ty = Ty(14);
    pub const U64: Ty = Ty(15);
    pub const U128: Ty = Ty(16);
    pub const USIZE: Ty = Ty(17);
    pub const F32: Ty = Ty(18);
    pub const CSTRING: Ty = Ty(19);
    /// A Unicode scalar value.
    pub const CHAR: Ty = Ty(20);
    /// An address, where MIR holds one whose pointee it does not name: a
    /// `&dyn`'s receiver, a closure's captures, a table of methods.
    pub const PTR_U8: Ty = Ty(21);

    pub fn new() -> Types {
        let mut types = Types {
            set: IndexSet::new(),
            facts: Vec::new(),
            lists: IndexSet::new(),
            log: Vec::new(),
        };
        types.lists.insert(Box::new([]));
        let builtins = [
            (TyKind::Error, Types::ERROR),
            (TyKind::Never, Types::NEVER),
            (TyKind::Unit, Types::UNIT),
            (TyKind::Int(IntTy::I64), Types::I64),
            (TyKind::Float(FloatTy::F64), Types::F64),
            (TyKind::Bool, Types::BOOL),
            (TyKind::Str, Types::STR),
            (TyKind::Int(IntTy::I32), Types::I32),
            (TyKind::Int(IntTy::I8), Types::I8),
            (TyKind::Int(IntTy::I16), Types::I16),
            (TyKind::Int(IntTy::I128), Types::I128),
            (TyKind::Int(IntTy::Isize), Types::ISIZE),
            (TyKind::Int(IntTy::U8), Types::U8),
            (TyKind::Int(IntTy::U16), Types::U16),
            (TyKind::Int(IntTy::U32), Types::U32),
            (TyKind::Int(IntTy::U64), Types::U64),
            (TyKind::Int(IntTy::U128), Types::U128),
            (TyKind::Int(IntTy::Usize), Types::USIZE),
            (TyKind::Float(FloatTy::F32), Types::F32),
            (TyKind::Cstring, Types::CSTRING),
            (TyKind::Char, Types::CHAR),
            (TyKind::Ptr(Types::U8), Types::PTR_U8),
        ];
        for (kind, expected) in builtins {
            assert_eq!(types.intern(kind), expected);
        }
        types
    }

    pub fn intern(&mut self, kind: TyKind) -> Ty {
        let (index, added) = self.set.insert_full(kind);
        if added {
            let parts: Vec<Ty> = match kind {
                TyKind::Own(t)
                | TyKind::Ref(t, _)
                | TyKind::Array(t, _)
                | TyKind::Slice(t)
                | TyKind::Ptr(t)
                | TyKind::Slots(t) => {
                    vec![t]
                }
                TyKind::Struct(_, args) | TyKind::Enum(_, args) => self.list(args).to_vec(),
                TyKind::Fn(params, ret) => {
                    let mut parts = self.list(params).to_vec();
                    parts.push(ret);
                    parts
                }
                _ => Vec::new(),
            };
            let mut facts = Facts {
                generic: matches!(kind, TyKind::Param(_)),
                error: kind == TyKind::Error,
                reference: matches!(kind, TyKind::Ref(..)),
                depth: 1,
            };
            for part in parts {
                let part = self.facts[part.0 as usize];
                facts.generic |= part.generic;
                facts.error |= part.error;
                // A function's parameters may be references, but its address
                // holds none.
                facts.reference |= part.reference && !matches!(kind, TyKind::Fn(..));
                facts.depth = facts.depth.max(part.depth + 1);
            }
            self.facts.push(facts);
            self.log.push(Interned::Ty);
            // A buffer's elements are dropped where they lie by a function
            // that takes a pointer to one, `own<T>`, so that type is there
            // when code is generated, after which nothing is interned.
            if let TyKind::Own(inner) = kind
                && let TyKind::Slice(elem) = self.kind(inner)
            {
                self.intern(TyKind::Own(elem));
            }
        }
        Ty(index as u32)
    }

    pub fn intern_list(&mut self, tys: &[Ty]) -> TyList {
        match self.lists.get_index_of(tys) {
            Some(index) => TyList(index as u32),
            None => {
                self.log.push(Interned::List);
                TyList(self.lists.insert_full(tys.into()).0 as u32)
            }
        }
    }

    pub fn list(&self, list: TyList) -> &[Ty] {
        &self.lists[list.0 as usize]
    }

    /// Whether `ty`, or any type inside it, has a kind that `test` accepts:
    /// through `own`, references, arrays, slices, and type arguments.
    pub fn any(&self, ty: Ty, test: &impl Fn(TyKind) -> bool) -> bool {
        let kind = self.kind(ty);
        test(kind)
            || match kind {
                TyKind::Own(t) | TyKind::Ref(t, _) | TyKind::Array(t, _) | TyKind::Slice(t) => {
                    self.any(t, test)
                }
                TyKind::Struct(_, list) | TyKind::Enum(_, list) => {
                    self.list(list).iter().any(|&t| self.any(t, test))
                }
                TyKind::Fn(params, ret) => {
                    self.list(params).iter().any(|&t| self.any(t, test)) || self.any(ret, test)
                }
                _ => false,
            }
    }

    /// Whether `ty` contains a type parameter, so that it stands for
    /// different types in different instances.
    pub fn is_generic(&self, ty: Ty) -> bool {
        self.facts[ty.0 as usize].generic
    }

    /// Whether `ty` contains the error type.
    pub fn has_error(&self, ty: Ty) -> bool {
        self.facts[ty.0 as usize].error
    }

    /// Whether `ty` contains a reference.
    pub fn contains_ref(&self, ty: Ty) -> bool {
        self.facts[ty.0 as usize].reference
    }

    /// How deeply the parts of `ty` nest: 1 for a type without parts.
    pub fn depth(&self, ty: Ty) -> u32 {
        self.facts[ty.0 as usize].depth
    }

    /// `ty` with each type parameter replaced by its argument in `args`.
    pub fn subst(&mut self, ty: Ty, args: &[Ty]) -> Ty {
        if !self.is_generic(ty) {
            return ty;
        }
        let kind = match self.kind(ty) {
            TyKind::Param(p) => return args[p.index as usize],
            TyKind::Own(t) => TyKind::Own(self.subst(t, args)),
            TyKind::Ref(t, k) => TyKind::Ref(self.subst(t, args), k),
            TyKind::Array(t, n) => TyKind::Array(self.subst(t, args), n),
            TyKind::Slice(t) => TyKind::Slice(self.subst(t, args)),
            TyKind::Ptr(t) => TyKind::Ptr(self.subst(t, args)),
            TyKind::Slots(t) => TyKind::Slots(self.subst(t, args)),
            TyKind::Struct(id, list) => TyKind::Struct(id, self.subst_list(list, args)),
            TyKind::Enum(id, list) => TyKind::Enum(id, self.subst_list(list, args)),
            TyKind::Fn(params, ret) => {
                TyKind::Fn(self.subst_list(params, args), self.subst(ret, args))
            }
            _ => unreachable!("only types with parts can contain a type parameter"),
        };
        self.intern(kind)
    }

    /// `ty` with the type parameter named `name` replaced by `with`, and
    /// every other left as it is: a constraint's arguments about the
    /// parameter it constrains, where that parameter's argument is known
    /// and the item's others are not.
    pub fn replace_param(&mut self, ty: Ty, name: Symbol, with: Ty) -> Ty {
        if !self.is_generic(ty) {
            return ty;
        }
        let kind = match self.kind(ty) {
            TyKind::Param(p) if p.name == name => return with,
            TyKind::Param(_) => return ty,
            TyKind::Own(t) => TyKind::Own(self.replace_param(t, name, with)),
            TyKind::Ref(t, k) => TyKind::Ref(self.replace_param(t, name, with), k),
            TyKind::Array(t, n) => TyKind::Array(self.replace_param(t, name, with), n),
            TyKind::Slice(t) => TyKind::Slice(self.replace_param(t, name, with)),
            TyKind::Ptr(t) => TyKind::Ptr(self.replace_param(t, name, with)),
            TyKind::Slots(t) => TyKind::Slots(self.replace_param(t, name, with)),
            TyKind::Struct(id, list) => {
                let tys: Vec<Ty> = self.list(list).to_vec();
                let tys: Vec<Ty> = tys
                    .into_iter()
                    .map(|t| self.replace_param(t, name, with))
                    .collect();
                TyKind::Struct(id, self.intern_list(&tys))
            }
            TyKind::Enum(id, list) => {
                let tys: Vec<Ty> = self.list(list).to_vec();
                let tys: Vec<Ty> = tys
                    .into_iter()
                    .map(|t| self.replace_param(t, name, with))
                    .collect();
                TyKind::Enum(id, self.intern_list(&tys))
            }
            TyKind::Fn(params, ret) => {
                let tys: Vec<Ty> = self.list(params).to_vec();
                let tys: Vec<Ty> = tys
                    .into_iter()
                    .map(|t| self.replace_param(t, name, with))
                    .collect();
                let params = self.intern_list(&tys);
                TyKind::Fn(params, self.replace_param(ret, name, with))
            }
            _ => unreachable!("only types with parts can contain a type parameter"),
        };
        self.intern(kind)
    }

    pub fn subst_list(&mut self, list: TyList, args: &[Ty]) -> TyList {
        let tys: Vec<Ty> = self.list(list).to_vec();
        let tys: Vec<Ty> = tys.into_iter().map(|t| self.subst(t, args)).collect();
        self.intern_list(&tys)
    }

    /// Like [`Types::subst`], for phases that share the types read-only: the
    /// type must have been interned already. Every field type of every
    /// interned instance is.
    pub fn subst_find(&self, ty: Ty, args: &[Ty]) -> Ty {
        self.try_subst_find(ty, args)
            .expect("the substituted type was interned while checking")
    }

    /// [`Types::subst_find`], answering nothing where the substituted type
    /// was never interned.
    pub fn try_subst_find(&self, ty: Ty, args: &[Ty]) -> Option<Ty> {
        if !self.is_generic(ty) {
            return Some(ty);
        }
        let kind = match self.kind(ty) {
            // A parameter the arguments do not reach is not a type this
            // can find: the answer is nothing rather than a panic.
            TyKind::Param(p) => return args.get(p.index as usize).copied(),
            TyKind::Own(t) => TyKind::Own(self.try_subst_find(t, args)?),
            TyKind::Ref(t, k) => TyKind::Ref(self.try_subst_find(t, args)?, k),
            TyKind::Array(t, n) => TyKind::Array(self.try_subst_find(t, args)?, n),
            TyKind::Slice(t) => TyKind::Slice(self.try_subst_find(t, args)?),
            TyKind::Ptr(t) => TyKind::Ptr(self.try_subst_find(t, args)?),
            TyKind::Slots(t) => TyKind::Slots(self.try_subst_find(t, args)?),
            TyKind::Struct(id, list) => TyKind::Struct(id, self.subst_list_find(list, args)?),
            TyKind::Enum(id, list) => TyKind::Enum(id, self.subst_list_find(list, args)?),
            TyKind::Fn(params, ret) => TyKind::Fn(
                self.subst_list_find(params, args)?,
                self.try_subst_find(ret, args)?,
            ),
            _ => unreachable!("only types with parts can contain a type parameter"),
        };
        self.find(kind)
    }

    fn subst_list_find(&self, list: TyList, args: &[Ty]) -> Option<TyList> {
        let tys = self
            .list(list)
            .iter()
            .map(|&t| self.try_subst_find(t, args))
            .collect::<Option<Vec<Ty>>>()?;
        self.lists.get_index_of(&*tys).map(|i| TyList(i as u32))
    }

    /// The type of `kind`, if it has been interned. Once type checking is
    /// done the program is shared read-only, so later phases look types up
    /// rather than interning new ones.
    pub fn find(&self, kind: TyKind) -> Option<Ty> {
        self.set.get_index_of(&kind).map(|i| Ty(i as u32))
    }

    /// The list of `tys`, if it has been interned, for the same reason as
    /// [`Types::find`].
    pub fn find_list(&self, tys: &[Ty]) -> Option<TyList> {
        self.lists.get_index_of(tys).map(|i| TyList(i as u32))
    }

    /// How many types have been interned.
    pub(crate) fn len(&self) -> usize {
        self.set.len()
    }

    /// The type interned `index`th.
    pub(crate) fn nth(&self, index: usize) -> Ty {
        Ty(index as u32)
    }

    /// Interns the types `other` added after its first `start`, which the
    /// two interners share, and returns where each of `other`'s types is
    /// here. A type is interned after the types inside it, so each of those
    /// is already in the map when it is needed.
    pub(crate) fn absorb(&mut self, other: &Types, mark: Mark) -> TyMap {
        let mut map = TyMap {
            start: mark.types,
            added: Vec::new(),
            list_start: mark.lists,
            lists: Vec::new(),
        };
        let (mut next_ty, mut next_list) = (mark.types, mark.lists);
        for entry in &other.log[mark.log..] {
            match entry {
                Interned::Ty => {
                    let kind = match other.set[next_ty] {
                        TyKind::Own(t) => TyKind::Own(map.get(t)),
                        TyKind::Ref(t, k) => TyKind::Ref(map.get(t), k),
                        TyKind::Array(t, n) => TyKind::Array(map.get(t), n),
                        TyKind::Slice(t) => TyKind::Slice(map.get(t)),
                        TyKind::Ptr(t) => TyKind::Ptr(map.get(t)),
                        TyKind::Slots(t) => TyKind::Slots(map.get(t)),
                        TyKind::Struct(id, list) => TyKind::Struct(id, map.get_list(list)),
                        TyKind::Enum(id, list) => TyKind::Enum(id, map.get_list(list)),
                        TyKind::Fn(params, ret) => TyKind::Fn(map.get_list(params), map.get(ret)),
                        kind @ (TyKind::Error
                        | TyKind::Never
                        | TyKind::Unit
                        | TyKind::Int(_)
                        | TyKind::Float(_)
                        | TyKind::Bool
                        | TyKind::Char
                        | TyKind::Str
                        | TyKind::Cstring
                        | TyKind::Dyn(..)
                        | TyKind::Opaque(_)
                        | TyKind::Param(_)) => kind,
                    };
                    let ty = self.intern(kind);
                    map.added.push(ty);
                    next_ty += 1;
                }
                Interned::List => {
                    let tys: Vec<Ty> = other.lists[next_list].iter().map(|&t| map.get(t)).collect();
                    let list = self.intern_list(&tys);
                    map.lists.push(Some(list));
                    next_list += 1;
                }
            }
        }
        map
    }

    /// Where interning has got to, for [`Types::absorb`].
    pub(crate) fn mark(&self) -> Mark {
        Mark {
            types: self.set.len(),
            lists: self.lists.len(),
            log: self.log.len(),
        }
    }

    pub fn kind(&self, ty: Ty) -> TyKind {
        self.set[ty.0 as usize]
    }

    /// An integer or float type: the types arithmetic and `as` work on.
    pub fn is_numeric(&self, ty: Ty) -> bool {
        matches!(self.kind(ty), TyKind::Int(_) | TyKind::Float(_))
    }

    pub fn is_integer(&self, ty: Ty) -> bool {
        matches!(self.kind(ty), TyKind::Int(_))
    }

    /// Where a value of an integer or character type stands in the type's
    /// order, from its bits: a signed number's with the sign bit turned
    /// over, so that the least value comes first, and anything else's as
    /// they are. What a range pattern and the check of what a `match`
    /// covers compare.
    pub fn order_key(&self, ty: Ty, bits: u128) -> Option<u128> {
        match self.kind(ty) {
            TyKind::Int(t) if t.signed() => Some((bits & t.mask()) ^ (1 << (t.bits() - 1))),
            TyKind::Int(t) => Some(bits & t.mask()),
            TyKind::Char => Some(bits),
            _ => None,
        }
    }

    /// The bits of the value at `key` in the type's order: what
    /// [`Types::order_key`] undoes.
    pub fn from_order_key(&self, ty: Ty, key: u128) -> u128 {
        match self.kind(ty) {
            TyKind::Int(t) if t.signed() => key ^ (1 << (t.bits() - 1)),
            _ => key,
        }
    }

    /// The least and the greatest key of a type's values: an integer's
    /// every value, and a `char`'s from U+0000 to U+10FFFF, of which the
    /// surrogates are not values.
    pub fn order_range(&self, ty: Ty) -> Option<(u128, u128)> {
        match self.kind(ty) {
            TyKind::Int(t) => Some((0, t.mask())),
            TyKind::Char => Some((0, 0x10_FFFF)),
            _ => None,
        }
    }

    pub fn is_float(&self, ty: Ty) -> bool {
        matches!(self.kind(ty), TyKind::Float(_))
    }

    pub fn int_ty(&self, ty: Ty) -> Option<IntTy> {
        match self.kind(ty) {
            TyKind::Int(t) => Some(t),
            _ => None,
        }
    }
}

/// How many types and lists an interner had, where a copy of it started.
#[derive(Clone, Copy)]
pub(crate) struct Mark {
    types: usize,
    lists: usize,
    log: usize,
}

/// Where the types of one interner are in another ([`Types::absorb`]).
pub(crate) struct TyMap {
    /// Types below this are the same in both.
    start: usize,
    added: Vec<Ty>,
    /// Lists below this are the same in both.
    list_start: usize,
    lists: Vec<Option<TyList>>,
}

impl TyMap {
    pub(crate) fn get(&self, ty: Ty) -> Ty {
        match (ty.0 as usize).checked_sub(self.start) {
            Some(i) => self.added[i],
            None => ty,
        }
    }

    pub(crate) fn get_list(&self, list: TyList) -> TyList {
        match (list.0 as usize).checked_sub(self.list_start) {
            Some(i) => self.lists[i].expect("every list was absorbed"),
            None => list,
        }
    }

    /// Whether every type and list keeps its number, so nothing needs
    /// renumbering.
    pub(crate) fn is_identity(&self) -> bool {
        self.added
            .iter()
            .enumerate()
            .all(|(i, ty)| ty.0 as usize == self.start + i)
            && self
                .lists
                .iter()
                .enumerate()
                .all(|(i, list)| list.is_some_and(|l| l.0 as usize == self.list_start + i))
    }
}

impl Default for Types {
    fn default() -> Types {
        Types::new()
    }
}
