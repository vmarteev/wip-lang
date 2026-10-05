//! Name resolution, the typed IR and the type checker.
//!
//! [`lower`] resolves names and checks types in one pass over the syntax tree
//! and produces a [`Program`]: every expression has a type, every name points
//! at its definition, and the operations the surface syntax leaves implicit —
//! auto-deref and the `&own<T>` to `&T` coercion — are explicit nodes.

mod hir;
mod known;
mod lower;
mod mono;
mod ty;

pub use hir::*;
pub use known::{KnownEnum, KnownFn, KnownInterface, KnownStruct, PreludeItems};

/// The module whose `pub` items every file sees without an import.
pub const PRELUDE: &str = "std::prelude";
pub use lower::{Lowered, ModuleAst, lower, lower_file, lower_with_threads};
pub use mono::instantiate;
pub use ty::{FloatTy, IntTy, RefKind, Ty, TyKind, TyList, TyParam, Types};
use wip_syntax::{Diagnostic, Interner, Span, codes};

/// The program's `main`, which the monomorphizer looks for as well.
pub(crate) fn main_fn<'p>(program: &'p Program, interner: &Interner) -> Option<(FnId, &'p FnDef)> {
    program
        .fns
        .iter()
        .find(|(_, f)| !f.is_extern && f.module == 0 && interner.resolve(f.name) == "main")
}

/// The entry point, and what it hands back.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub main: FnId,
    /// The error type, when `main` returns a `Result`. The exit code then
    /// comes from the prelude rather than from `main` itself.
    pub error: Option<Ty>,
    /// Whether the `Result`'s value is an `i64` rather than nothing.
    pub has_code: bool,
}

/// Finds the entry point: `fn main()` or `fn main(args: &[cstring])`,
/// returning `i64` (the exit code), nothing, or a `Result` of either.
/// Whether an implementation's type arguments, read in the type's own
/// parameters, are the ones a constraint asks for: `extend Vec<T>:
/// Items<T>` answers `Items<Card>` for a `Vec<Card>`. An argument that is a
/// parameter is looked up; one that is written out must match exactly.
pub fn args_match(types: &ty::Types, declared: TyList, owner: &[Ty], wanted: TyList) -> bool {
    args_match_with(types, declared, owner, types.list(wanted))
}

/// The same, where the wanted arguments are already a slice.
pub fn args_match_with(types: &ty::Types, declared: TyList, owner: &[Ty], wanted: &[Ty]) -> bool {
    let declared = types.list(declared);
    declared.len() == wanted.len()
        && declared
            .iter()
            .zip(wanted)
            .all(|(&d, &w)| match types.kind(d) {
                ty::TyKind::Param(p) => owner.get(p.index as usize) == Some(&w),
                // An argument written in the type's own parameters rather
                // than being one — `extend Zipped<I, T, J, V>:
                // Iterator<(T, V)>` — is read with them before it is
                // compared.
                _ if types.is_generic(d) => types.try_subst_find(d, owner) == Some(w),
                _ => d == w,
            })
}

pub fn check_main(program: &Program, interner: &Interner) -> Result<Entry, Box<Diagnostic>> {
    let main = main_fn(program, interner);
    let Some((id, main)) = main else {
        let diagnostic = Diagnostic::error(
            codes::INVALID_MAIN,
            "no `main` function",
            Span::at(0),
            "the program starts at `main`",
        )
        .with_help("add `fn main(): i64 = 0`");
        return Err(Box::new(diagnostic));
    };
    // `main` takes nothing, or the program's arguments as C passes them.
    let is_args = |ty: Ty| {
        matches!(program.types.kind(ty), TyKind::Ref(inner, RefKind::Shared)
            if program.types.kind(inner) == TyKind::Slice(Types::CSTRING))
    };
    if let Some(first) = main.generics.first() {
        let diagnostic = Diagnostic::error(
            codes::CANNOT_BE_GENERIC,
            "`main` cannot be generic",
            first.span,
            "a type parameter",
        )
        .with_note("the program starts at `main`, and nothing gives it type arguments");
        return Err(Box::new(diagnostic));
    }
    let valid = match main.params.as_slice() {
        [] => true,
        [args] => is_args(args.ty),
        _ => false,
    };
    if !valid {
        let first = main.params.first().expect("`main` has parameters").span;
        let last = main.params.last().expect("`main` has parameters").span;
        let diagnostic = Diagnostic::error(
            codes::INVALID_MAIN,
            "`main` takes no parameters, or the program's arguments",
            first.to(last),
            "not `args: &[cstring]`",
        )
        .with_help("write `fn main(args: &[cstring])`; `args[0]` is the program's name, as in C")
        .with_note("the arguments are the C strings the operating system passes");
        return Err(Box::new(diagnostic));
    }
    // A `Result` of nothing or of an exit code, whose error is reported and
    // whose exit code is then 1.
    let result = match program.types.kind(main.ret) {
        TyKind::Enum(enum_id, args)
            if program.prelude_items.enumeration(KnownEnum::Result) == Some(enum_id) =>
        {
            let args = program.types.list(args);
            Some((args[0], args[1]))
        }
        _ => None,
    };
    if let Some((value, error)) = result {
        let span = main.ret_span.unwrap_or(main.span);
        if value != Types::I64 && value != Types::UNIT {
            let diagnostic = Diagnostic::error(
                codes::INVALID_MAIN,
                "`main` must return a `Result` of `i64` or of nothing",
                span,
                format!("its value is `{}`", program.ty_name(value, interner)),
            )
            .with_note("the value `main` returns is the process exit code");
            return Err(Box::new(diagnostic));
        }
        let Some(text) = program.prelude_items.interface(KnownInterface::Text) else {
            unreachable!("the prelude declares `Text`")
        };
        if !program.implements(error, text) {
            let diagnostic = Diagnostic::error(
                codes::INVALID_MAIN,
                format!(
                    "`main`'s error type, `{}`, cannot be written as text",
                    program.ty_name(error, interner)
                ),
                span,
                "does not implement `Text`",
            )
            .with_help(format!(
                "write `extend {}: Text`, whose `appendTo` says what it reports",
                program.ty_name(error, interner)
            ))
            .with_note("a `main` that fails prints its error on standard error and exits 1");
            return Err(Box::new(diagnostic));
        }
        return Ok(Entry {
            main: id,
            error: Some(error),
            has_code: value == Types::I64,
        });
    }
    if main.ret != Types::I64 && main.ret != Types::UNIT {
        let diagnostic = Diagnostic::error(
            codes::INVALID_MAIN,
            "`main` must return `i64`, nothing, or a `Result` of either",
            main.ret_span.unwrap_or(main.span),
            format!("returns `{}`", program.ty_name(main.ret, interner)),
        )
        .with_note(
            "the value `main` returns is the process exit code, and a `Result`'s error is reported",
        );
        return Err(Box::new(diagnostic));
    }
    Ok(Entry {
        main: id,
        error: None,
        has_code: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wip_syntax::Interner;

    /// A type parameter, the `index`th of its item's.
    fn param(types: &mut Types, interner: &mut Interner, index: u32, name: &str) -> Ty {
        types.intern(TyKind::Param(TyParam {
            index,
            name: interner.intern(name),
            copy: false,
        }))
    }

    /// `extend Vec<T>: Items<T>` answers `Items<Card>` for a `Vec<Card>`,
    /// and not `Items<i64>`.
    #[test]
    fn a_parameter_is_read_as_the_types_argument() {
        let mut types = Types::new();
        let mut interner = Interner::new();
        let t = param(&mut types, &mut interner, 0, "T");
        let declared = types.intern_list(&[t]);
        let card = Types::F64;
        assert!(args_match_with(&types, declared, &[card], &[card]));
        assert!(!args_match_with(&types, declared, &[card], &[Types::I64]));
    }

    /// An argument written out is compared as it is.
    #[test]
    fn an_argument_written_out_must_be_the_one_asked_for() {
        let mut types = Types::new();
        let declared = types.intern_list(&[Types::I64]);
        assert!(args_match_with(&types, declared, &[], &[Types::I64]));
        assert!(!args_match_with(&types, declared, &[], &[Types::F64]));
        assert!(
            !args_match_with(&types, declared, &[], &[]),
            "one argument, not none"
        );
    }

    /// An argument made of the type's parameters — `Iterator<(T, V)>` of
    /// `Zipped<I, T, J, V>` — is read with them before it is compared. It
    /// was once compared as written, which matched only a bare parameter
    /// and made `zip(…).count()` say a zipped iterator was not one.
    #[test]
    fn an_argument_made_of_parameters_is_read_with_them() {
        let mut types = Types::new();
        let mut interner = Interner::new();
        let params: Vec<Ty> = ["I", "T", "J", "V"]
            .iter()
            .enumerate()
            .map(|(i, name)| param(&mut types, &mut interner, i as u32, name))
            .collect();
        let of_t = types.intern(TyKind::Slice(params[1]));
        let declared = types.intern_list(&[of_t]);
        let owner = [Types::UNIT, Types::I64, Types::UNIT, Types::F64];
        let of_i64 = types.intern(TyKind::Slice(Types::I64));
        let of_f64 = types.intern(TyKind::Slice(Types::F64));
        assert!(args_match_with(&types, declared, &owner, &[of_i64]));
        assert!(!args_match_with(&types, declared, &owner, &[of_f64]));
    }
}
