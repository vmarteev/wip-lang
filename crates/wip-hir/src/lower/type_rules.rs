//! Rules about where types may appear, and the built-in type names.

use super::*;

/// C's `long` is 64 bits where Unix's LP64 says so, and 32 bits on Windows,
/// which is why bindings must not write `i64` for it.
const C_LONG: Ty = if cfg!(windows) {
    Types::I32
} else {
    Types::I64
};
const C_ULONG: Ty = if cfg!(windows) {
    Types::U32
} else {
    Types::U64
};

/// C's `char` is signed or unsigned by target, not by the standard: Apple
/// and Windows make it signed everywhere, and ARM's own ABI makes it
/// unsigned, which is what Linux on ARM follows.
const C_CHAR: Ty = if cfg!(all(
    any(target_arch = "aarch64", target_arch = "arm"),
    not(any(target_os = "macos", target_os = "ios", windows))
)) {
    Types::U8
} else {
    Types::I8
};

pub(super) const BUILTIN_TYPES: [(&str, Ty); 37] = [
    // What does not return: a `panic`, a `return`, a loop that never ends.
    ("never", Types::NEVER),
    ("i8", Types::I8),
    ("i16", Types::I16),
    ("i32", Types::I32),
    ("i64", Types::I64),
    ("i128", Types::I128),
    ("isize", Types::ISIZE),
    ("u8", Types::U8),
    ("u16", Types::U16),
    ("u32", Types::U32),
    ("u64", Types::U64),
    ("u128", Types::U128),
    ("usize", Types::USIZE),
    ("f32", Types::F32),
    ("f64", Types::F64),
    ("bool", Types::BOOL),
    ("str", Types::STR),
    // A Unicode scalar value.
    ("char", Types::CHAR),
    ("cstring", Types::CSTRING),
    ("void", Types::UNIT),
    // C's own names, fixed per target, so that bindings never write `i64`
    // for `long`. They are built in rather than declared in
    // `std`, which has no way to differ per target. Only 64-bit targets are
    // supported, so `size_t` is `u64` — Wip's `usize` stays its own, and
    // does not cross into C.
    ("c_char", C_CHAR),
    ("c_schar", Types::I8),
    ("c_uchar", Types::U8),
    ("c_short", Types::I16),
    ("c_ushort", Types::U16),
    ("c_int", Types::I32),
    ("c_uint", Types::U32),
    ("c_long", C_LONG),
    ("c_ulong", C_ULONG),
    ("c_longlong", Types::I64),
    ("c_ulonglong", Types::U64),
    ("c_float", Types::F32),
    ("c_double", Types::F64),
    ("size_t", Types::U64),
    ("ssize_t", Types::I64),
    ("intptr_t", Types::I64),
    ("uintptr_t", Types::U64),
];

impl<'a> Lowerer<'a> {
    pub(super) fn has_error(&self, ty: Ty) -> bool {
        self.program.types.has_error(ty)
    }

    pub(super) fn contains_ref(&self, ty: Ty) -> bool {
        self.program.types.contains_ref(ty)
    }

    /// Whether `ty` has a slice anywhere but directly behind a reference.
    pub(super) fn has_bare_slice(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Slice(_) => true,
            TyKind::Ref(inner, _) => match self.kind(inner) {
                TyKind::Slice(elem) => self.has_bare_slice(elem),
                _ => self.has_bare_slice(inner),
            },
            // A buffer owns its elements.
            TyKind::Own(inner) => match self.kind(inner) {
                TyKind::Slice(elem) => self.has_bare_slice(elem),
                _ => self.has_bare_slice(inner),
            },
            TyKind::Array(t, _) => self.has_bare_slice(t),
            _ => false,
        }
    }

    /// Reports `never` where a value would have to exist.
    /// Returns whether it did.
    pub(super) fn no_never(&mut self, ty: Ty, type_id: ast::TypeId, what: &str) -> bool {
        if ty != Types::NEVER {
            return false;
        }
        let span = self.ast.types[type_id].span;
        let diagnostic = Diagnostic::error(
            codes::NEVER_VALUE,
            format!("{what} cannot have type `never`"),
            span,
            "no value has this type",
        )
        .with_note(
            "`never` is what does not return: a `panic`, a `return`, a loop that never ends",
        );
        self.report(diagnostic);
        true
    }

    /// Whether a `dyn` type is anywhere but directly behind `&` or `&var`.
    pub(super) fn has_bare_dyn(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Dyn(..) => true,
            TyKind::Ref(inner, _) if matches!(self.kind(inner), TyKind::Dyn(..)) => false,
            TyKind::Ref(inner, _) | TyKind::Own(inner) | TyKind::Array(inner, _) => {
                self.has_bare_dyn(inner)
            }
            TyKind::Slice(elem) => self.has_bare_dyn(elem),
            _ => false,
        }
    }

    /// Reports a `dyn` type anywhere but behind `&`.
    /// Returns whether it did.
    pub(super) fn bare_dyn(&mut self, ty: Ty, type_id: ast::TypeId) -> bool {
        if !self.has_bare_dyn(ty) {
            return false;
        }
        let span = self.ast.types[type_id].span;
        let diagnostic = Diagnostic::error(
            codes::UNSIZED_DYN,
            "a `dyn` type must be borrowed",
            span,
            "a value of unknown type has no size of its own",
        )
        .with_note("a `dyn` value lives somewhere else, and is known by a pointer and a table of methods, so it exists only behind `&` or `&var`")
        .with_fix("borrow it", [Edit::insert(span.lo, "&")]);
        self.report(diagnostic);
        true
    }

    /// Reports a slice type that is not behind `&`, with a
    /// fix that adds the `&` when `fixable`. Returns whether it did.
    pub(super) fn bare_slice(&mut self, ty: Ty, type_id: ast::TypeId, fixable: bool) -> bool {
        if !self.has_bare_slice(ty) {
            return false;
        }
        let span = self.ast.types[type_id].span;
        let mut diagnostic = Diagnostic::error(
            codes::UNSIZED_SLICE,
            "a slice type must be borrowed",
            span,
            "a slice has no size of its own",
        )
        .with_note("a slice's elements live somewhere else, so a slice exists only behind `&`, `&var` or `own`");
        if fixable && matches!(self.kind(ty), TyKind::Slice(_)) {
            diagnostic = diagnostic.with_fix("borrow it", [Edit::insert(span.lo, "&")]);
        }
        self.report(diagnostic);
        true
    }

    /// Reports a type that borrows — a `str`, a view, or what holds one —
    /// where it would be stored: a field that is not a view's, or a
    /// variant's payload that is not a view enum's. It borrows bytes that
    /// live somewhere else, so it keeps to where the checker can see it.
    /// Returns whether it did, with a help of the caller's where it has a
    /// better one.
    pub(super) fn stored_view_at(
        &mut self,
        ty: Ty,
        span: Span,
        what: &str,
        help: Option<String>,
    ) -> bool {
        // A `&` reference is a view, and is refused where one is.
        if !self.program.holds_view(ty) {
            return false;
        }
        let (message, label) = if ty == Types::STR {
            (
                format!("{what} cannot hold a `str`"),
                "a `str` borrows bytes that live somewhere else".to_string(),
            )
        } else {
            (
                format!("{what} cannot hold {}", self.ty_name(ty)),
                format!("{} borrows, as a `str` does", self.ty_name(ty)),
            )
        };
        let help = help.unwrap_or_else(|| {
            "`String` owns its bytes, and hands out a `str` with `toStr()`".to_string()
        });
        let diagnostic = Diagnostic::error(codes::STORED_STR, message, span, label)
            .with_note("a `str`, a `&` reference, a `view struct` and what holds one borrow what lives somewhere else, so they stand in a parameter, a local, a result and a view's field, and not where they would outlive it")
            .with_help(help);
        self.report(diagnostic);
        true
    }

    /// Whether a `&var` stands inside `ty`: anywhere but a function type's
    /// parameters, which may be one.
    pub(super) fn holds_var_ref(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            TyKind::Ref(inner, kind) => kind == crate::RefKind::Var || self.holds_var_ref(inner),
            TyKind::Own(inner)
            | TyKind::Array(inner, _)
            | TyKind::Slice(inner)
            | TyKind::Ptr(inner)
            | TyKind::Slots(inner) => self.holds_var_ref(inner),
            TyKind::Struct(_, list) | TyKind::Enum(_, list) => {
                let args = self.program.types.list(list).to_vec();
                args.into_iter().any(|arg| self.holds_var_ref(arg))
            }
            TyKind::Fn(_, ret) => self.holds_var_ref(ret),
            _ => false,
        }
    }

    /// Reports a `&var` inside a type, where `what` would keep it: only a
    /// parameter's whole type may be one. Returns whether it
    /// did.
    pub(super) fn no_var_ref(&mut self, ty: Ty, type_id: ast::TypeId, what: &str) -> bool {
        if !self.holds_var_ref(ty) {
            return false;
        }
        let span = self.ast.types[type_id].span;
        let diagnostic = Diagnostic::error(
            codes::REF_OUTSIDE_PARAMETER,
            format!("{what} cannot hold a `&var` reference"),
            span,
            "a `&var` reference",
        )
        .with_note("a `&` reference is kept where a view is, but a `&var` is a parameter's alone: a view may be copied, and two copies of a `&var` would each write one place")
        .with_help("hold `&`, and take `&var` as a parameter where the change is made");
        self.report(diagnostic);
        true
    }

    /// Reports a reference where none may stand: a constant, what a C
    /// pointer points at, a function value's whole result. Returns whether
    /// it did, so that the caller can use the error type instead and avoid
    /// follow-on errors.
    pub(super) fn no_ref(
        &mut self,
        ty: Ty,
        type_id: ast::TypeId,
        message: &str,
        help: Option<&str>,
    ) -> bool {
        if !self.contains_ref(ty) {
            return false;
        }
        let span = self.ast.types[type_id].span;
        let mut diagnostic = Diagnostic::error(
            codes::REF_OUTSIDE_PARAMETER,
            message,
            span,
            "a reference type",
        )
        .with_note("a `&` reference stands where a view does — a parameter, a local, a view's field, a type argument — and a `&var` only as a parameter's whole type");
        if let Some(help) = help {
            diagnostic = diagnostic.with_help(help);
        }
        self.report(diagnostic);
        true
    }

    /// Types for which no further errors should be reported.
    pub(super) fn is_poisoned(&self, ty: Ty) -> bool {
        ty == Types::NEVER || self.has_error(ty)
    }

    pub(super) fn builtin(&self, sym: Symbol) -> Option<Ty> {
        let text = self.text(sym);
        BUILTIN_TYPES
            .iter()
            .find(|(name, _)| *name == text)
            .map(|&(_, ty)| ty)
    }
}
