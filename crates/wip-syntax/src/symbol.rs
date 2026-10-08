use std::fmt;

use lasso::{Key, Rodeo, Spur};

/// An interned string: an identifier or the contents of a string literal.
///
/// Comparing symbols compares integers. Get the text back with
/// [`Interner::resolve`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(Spur);

impl Symbol {
    /// Symbols are numbered from zero, in the order they were interned.
    pub fn index(self) -> usize {
        self.0.into_usize()
    }

    /// The name an interpolated literal gives the `String` it builds.
    /// Every interner holds it, always fourth, so the
    /// parser — which has no interner — can still name it.
    pub fn text() -> Symbol {
        Symbol::well_known(TEXT_INDEX)
    }

    /// `String`, the type an interpolated literal builds.
    pub fn string_type() -> Symbol {
        Symbol::well_known(TEXT_INDEX + 1)
    }

    /// `String::withCapacity`, which an interpolated literal begins with.
    pub fn with_capacity() -> Symbol {
        Symbol::well_known(TEXT_INDEX + 2)
    }

    /// `String::push`, which an interpolated literal's text goes through.
    pub fn push() -> Symbol {
        Symbol::well_known(TEXT_INDEX + 3)
    }

    /// `Text::appendTo`, which an interpolated literal's values go through.
    pub fn append_to() -> Symbol {
        Symbol::well_known(TEXT_INDEX + 4)
    }

    /// `Eq::equals`, which `==` goes through.
    pub fn equals() -> Symbol {
        Symbol::well_known(TEXT_INDEX + 5)
    }

    /// `Ord::compare`, which `<` and its siblings go through.
    pub fn compare() -> Symbol {
        Symbol::well_known(TEXT_INDEX + 6)
    }

    /// `Hash::hashInto`, which a map asks a key for.
    pub fn hash_into() -> Symbol {
        Symbol::well_known(TEXT_INDEX + 7)
    }

    /// `At::at`, which `x[i]` calls.
    pub fn at() -> Symbol {
        Symbol::well_known(NUMBER_INDEX + 3)
    }

    /// `count`, `fromIndex` and its parameter `index`: what an enum whose
    /// variants carry nothing gets.
    pub fn count() -> Symbol {
        Symbol::well_known(NUMBER_INDEX)
    }

    pub fn from_index() -> Symbol {
        Symbol::well_known(NUMBER_INDEX + 1)
    }

    pub fn index_param() -> Symbol {
        Symbol::well_known(NUMBER_INDEX + 2)
    }

    /// `all`: every variant of an enum of names.
    pub fn all() -> Symbol {
        Symbol::well_known(NUMBER_INDEX + 4)
    }

    /// `TupleN`, the prelude's struct that a tuple of `n` elements is.
    /// The parser writes the sugar into it, so every
    /// interner holds the names.
    pub fn tuple(n: usize) -> Symbol {
        assert!((MIN_TUPLE..=MAX_TUPLE).contains(&n), "a tuple's arity");
        Symbol::well_known(TUPLE_INDEX + n - MIN_TUPLE)
    }

    /// `_0`, `_1`, …: the field a tuple's element lies in.
    pub fn tuple_field(i: usize) -> Symbol {
        assert!(i < MAX_TUPLE, "a tuple's field");
        Symbol::well_known(TUPLE_INDEX + (MAX_TUPLE - MIN_TUPLE + 1) + i)
    }

    /// The hidden variable a `for` walks an iterator in, when the iterator
    /// is made where the loop is written.
    pub fn walked() -> Symbol {
        Symbol::well_known(WALKED_INDEX)
    }

    /// What a binder with no field name is called: `.Some((a, b))` names
    /// no field, and the one field a variant has is the one it takes.
    /// No program can write it.
    pub fn positional() -> Symbol {
        Symbol::well_known(POSITIONAL_INDEX)
    }

    /// What a list built as it runs calls the vector it fills.
    /// No program can write it.
    pub fn list() -> Symbol {
        Symbol::well_known(LIST_INDEX)
    }

    /// `Vec`, which a list built as it runs fills.
    pub fn vec_type() -> Symbol {
        Symbol::well_known(LIST_INDEX + 1)
    }

    /// `Vec::intoBuffer`, which hands the list over.
    pub fn into_buffer() -> Symbol {
        Symbol::well_known(LIST_INDEX + 2)
    }

    /// What a generator's struct is called. No program can
    /// write it.
    pub fn generator() -> Symbol {
        Symbol::well_known(GENERATOR_INDEX)
    }

    /// The field of a generator that says where it stopped.
    /// No program can write it.
    pub fn state() -> Symbol {
        Symbol::well_known(GENERATOR_INDEX + 1)
    }

    /// `Iterator::next`, which a generator's code is.
    pub fn next() -> Symbol {
        Symbol::well_known(GENERATOR_INDEX + 2)
    }

    /// `String::of`, which a literal becomes where a `String` is expected.
    pub fn of() -> Symbol {
        Symbol::well_known(OF_INDEX)
    }

    /// The function a field's or a parameter's default that is code becomes.
    /// No program can write it.
    pub fn default_code() -> Symbol {
        Symbol::well_known(DEFAULT_INDEX)
    }

    /// The function a top-level `assert` becomes.
    pub fn assert() -> Symbol {
        Symbol::well_known(ASSERT_INDEX)
    }

    /// What a parameter written `_` is called: it is declared, and no
    /// expression can name it.
    pub fn ignored() -> Symbol {
        Symbol::well_known(IGNORED_INDEX)
    }

    /// `Text::appendFitted`, which a piece of an interpolation that says
    /// its width goes through.
    pub fn append_fitted() -> Symbol {
        Symbol::well_known(FITTED_INDEX)
    }

    /// The options a piece of an interpolation may say: `width`, `fill`
    /// and `align`, in that order.
    pub fn fitting_options() -> [Symbol; 3] {
        [
            Symbol::well_known(FITTED_INDEX + 1),
            Symbol::well_known(FITTED_INDEX + 2),
            Symbol::well_known(FITTED_INDEX + 3),
        ]
    }

    /// `Option::Some`, which a piece's `align:` is given in.
    pub fn some() -> Symbol {
        Symbol::well_known(FITTED_INDEX + 4)
    }

    /// The prelude's `indexOfUnsigned` and `indexOfSigned`, through which
    /// an index of 64 bits or more becomes the `i64` a position is, or
    /// panics with what it was.
    pub fn index_of() -> [Symbol; 2] {
        [
            Symbol::well_known(INDEX_OF_INDEX),
            Symbol::well_known(INDEX_OF_INDEX + 1),
        ]
    }

    /// The options a piece of an interpolation may say of how a number is
    /// written, `precision` and `radix`, and the methods each becomes,
    /// `withPrecision` and `inRadix`.
    pub fn written() -> [Symbol; 4] {
        [
            Symbol::well_known(WRITTEN_INDEX),
            Symbol::well_known(WRITTEN_INDEX + 1),
            Symbol::well_known(WRITTEN_INDEX + 2),
            Symbol::well_known(WRITTEN_INDEX + 3),
        ]
    }

    /// The option of a piece written in a radix that says its digits above
    /// 9 are upper case: `\(byte, radix: 16, upper: true)`.
    pub fn upper() -> Symbol {
        Symbol::well_known(UPPER_INDEX)
    }

    /// `from`, after a function's result type: what the result borrows,
    /// `fn text(i: i64): str from self.ast`. A word only there, so a
    /// method may be named `from`.
    pub fn from_word() -> Symbol {
        Symbol::well_known(FROM_INDEX)
    }

    fn well_known(index: usize) -> Symbol {
        Symbol(Spur::try_from_usize(index).expect("a symbol index"))
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol({})", self.0.into_usize())
    }
}

/// Owns the text of every [`Symbol`].
///
/// Symbols are numbered in the order they are first interned, so the numbering
/// is deterministic for a given input.
pub struct Interner {
    rodeo: Rodeo,
}

impl Default for Interner {
    /// `self` is a keyword, so no file interns it, but a method's receiver is
    /// a parameter of that name. Every interner holds it, so
    /// [`Interner::self_symbol`] means the same symbol in all of them.
    fn default() -> Interner {
        let mut rodeo = Rodeo::default();
        rodeo.get_or_intern(SELF);
        rodeo.get_or_intern(SELF_TYPE);
        rodeo.get_or_intern(LAMBDA);
        rodeo.get_or_intern(TEXT);
        rodeo.get_or_intern(STRING);
        rodeo.get_or_intern(WITH_CAPACITY);
        rodeo.get_or_intern(PUSH);
        rodeo.get_or_intern(APPEND_TO);
        rodeo.get_or_intern(EQUALS);
        rodeo.get_or_intern(COMPARE);
        rodeo.get_or_intern(HASH_INTO);
        for name in NUMBERS {
            rodeo.get_or_intern(name);
        }
        for name in TUPLES {
            rodeo.get_or_intern(name);
        }
        for name in TUPLE_FIELDS {
            rodeo.get_or_intern(name);
        }
        rodeo.get_or_intern(WALKED);
        rodeo.get_or_intern(POSITIONAL);
        rodeo.get_or_intern(LIST);
        rodeo.get_or_intern(VEC);
        rodeo.get_or_intern(INTO_BUFFER);
        rodeo.get_or_intern(GENERATOR);
        rodeo.get_or_intern(STATE);
        rodeo.get_or_intern(NEXT);
        rodeo.get_or_intern(OF);
        rodeo.get_or_intern(DEFAULT);
        rodeo.get_or_intern(ASSERT);
        rodeo.get_or_intern(IGNORED);
        for name in FITTED {
            rodeo.get_or_intern(name);
        }
        for name in INDEX_OF {
            rodeo.get_or_intern(name);
        }
        for name in WRITTEN {
            rodeo.get_or_intern(name);
        }
        rodeo.get_or_intern(UPPER);
        rodeo.get_or_intern(FROM);
        Interner { rodeo }
    }
}

/// The name of a method's receiver.
const SELF: &str = "self";
/// The type a method belongs to, and an interface's type parameter.
const SELF_TYPE: &str = "Self";
/// What a lambda's hidden function is called.
const LAMBDA: &str = "lambda";
/// What an interpolated literal calls the `String` it builds.
/// No program can write it, so nothing of a program's own
/// can be confused with it.
const TEXT: &str = "text#";
/// The names an interpolated literal is rewritten with: `String`,
/// `withCapacity`, `push` and `appendTo`. The parser has no
/// interner of its own, so every interner holds them, in this order, and
/// they mean the same symbol in all of them.
const STRING: &str = "String";
const WITH_CAPACITY: &str = "withCapacity";
const PUSH: &str = "push";
const APPEND_TO: &str = "appendTo";
/// `Eq::equals`, whose body the compiler writes for a type that asks with
/// `@derive(Eq)`.
const EQUALS: &str = "equals";
/// `Ord::compare`, whose body the compiler writes for a type that asks with
/// `@derive(Ord)`.
const COMPARE: &str = "compare";
/// `Hash::hashInto`, whose body the compiler writes for a type that asks
/// with `@derive(Hash)`.
const HASH_INTO: &str = "hashInto";
/// Where the five above are in every interner, after `self`, `Self` and
/// `lambda`.
const TEXT_INDEX: usize = 3;
/// The names of what an enum whose variants carry nothing gets.
/// They are declared by the compiler, not read from a
/// file, so every interner holds them.
const NUMBERS: [&str; 5] = ["count", "fromIndex", "index", "at", "all"];
/// Where those four are, after the names above.
const NUMBER_INDEX: usize = TEXT_INDEX + 8;

/// The prelude's tuples, which `(A, B)` and `(a, b)` are written into.
/// The parser has no interner, so every interner holds
/// them, in this order, as it holds the names above.
const TUPLES: [&str; MAX_TUPLE - MIN_TUPLE + 1] = ["Tuple2", "Tuple3", "Tuple4"];
/// The fields a tuple's elements lie in.
const TUPLE_FIELDS: [&str; MAX_TUPLE] = ["_0", "_1", "_2", "_3"];
/// A tuple has two elements or more, and the prelude writes them out to
/// four.
pub const MIN_TUPLE: usize = 2;
pub const MAX_TUPLE: usize = 4;
/// Where the tuples are in every interner, after the names above.
const TUPLE_INDEX: usize = NUMBER_INDEX + NUMBERS.len();

/// What a `for` over an iterator made where it is written calls it.
/// No program can write it.
const WALKED: &str = "iterator#";
/// Where it is, after the tuples' names.
const WALKED_INDEX: usize = TUPLE_INDEX + TUPLES.len() + TUPLE_FIELDS.len();

/// A binder that names no field, which takes the one field its variant has.
/// No program can write it.
const POSITIONAL: &str = "field#";
/// Where it is, after the name above.
const POSITIONAL_INDEX: usize = WALKED_INDEX + 1;

/// The names a list built as it runs is written with: its vector, `Vec`
/// and `intoBuffer`, after the others.
const LIST: &str = "list#";
const VEC: &str = "Vec";
const INTO_BUFFER: &str = "intoBuffer";
const LIST_INDEX: usize = POSITIONAL_INDEX + 1;

/// The names a generator is written with: its struct, the field that says
/// where it stopped, and `Iterator::next`, which runs it on.
/// No program can write the first two.
const GENERATOR: &str = "generator#";
const STATE: &str = "state#";
const NEXT: &str = "next";
const GENERATOR_INDEX: usize = LIST_INDEX + 3;
/// `String::of`, which a literal becomes where a `String` is expected.
const OF: &str = "of";
const OF_INDEX: usize = GENERATOR_INDEX + 3;
/// What the function a default that is code becomes is called.
/// No program can write it.
const DEFAULT: &str = "default#";
const DEFAULT_INDEX: usize = OF_INDEX + 1;
/// What the function a top-level `assert` becomes is called, which is how
/// a failure while it runs names it.
const ASSERT: &str = "assert";
const ASSERT_INDEX: usize = DEFAULT_INDEX + 1;
/// A parameter written `_`. `_` is a token of its own, never
/// a name, so nothing a program writes reaches it.
const IGNORED: &str = "_";
const IGNORED_INDEX: usize = ASSERT_INDEX + 1;
/// The names a piece of an interpolation with a width is written with:
/// `Text::appendFitted`, its options, and `Some`, which an alignment is
/// given in.
const FITTED: [&str; 5] = ["appendFitted", "width", "fill", "align", "Some"];
const FITTED_INDEX: usize = IGNORED_INDEX + 1;
/// What an index of 64 bits or more is made a position through.
const INDEX_OF: [&str; 2] = ["indexOfUnsigned", "indexOfSigned"];
const INDEX_OF_INDEX: usize = FITTED_INDEX + FITTED.len();
/// A piece of an interpolation written to a precision or in a radix: the
/// two options, and the methods that make the value they write.
const WRITTEN: [&str; 4] = ["precision", "radix", "withPrecision", "inRadix"];
const WRITTEN_INDEX: usize = INDEX_OF_INDEX + INDEX_OF.len();
/// The option that writes a radix's digits above 9 in upper case.
const UPPER: &str = "upper";
const UPPER_INDEX: usize = WRITTEN_INDEX + WRITTEN.len();
/// The word after a result type that says what the result borrows.
const FROM: &str = "from";
const FROM_INDEX: usize = UPPER_INDEX + 1;

impl Interner {
    pub fn new() -> Interner {
        Interner::default()
    }

    /// The symbol for `self`, a method's receiver.
    pub fn self_symbol(&self) -> Symbol {
        Symbol(self.rodeo.get(SELF).expect("every interner holds `self`"))
    }

    /// The name a lambda's hidden function goes by.
    pub fn lambda_symbol(&self) -> Symbol {
        Symbol(
            self.rodeo
                .get(LAMBDA)
                .expect("every interner holds `lambda`"),
        )
    }

    /// The symbol for `Self`, the type a method belongs to.
    pub fn self_type_symbol(&self) -> Symbol {
        Symbol(
            self.rodeo
                .get(SELF_TYPE)
                .expect("every interner holds `Self`"),
        )
    }

    pub fn intern(&mut self, text: &str) -> Symbol {
        Symbol(self.rodeo.get_or_intern(text))
    }

    pub fn resolve(&self, sym: Symbol) -> &str {
        self.rodeo.resolve(&sym.0)
    }

    /// Whether a name is one the compiler gave, as an interpolation's
    /// `text#` and a loop's `iterator#`, which no program can write.
    pub fn is_hidden(&self, sym: Symbol) -> bool {
        self.resolve(sym).contains('#')
    }

    /// Interns every string of `other`, in its order, and returns what each
    /// of its symbols is here: `map[sym.index()]`. Files lexed with separate
    /// interners and absorbed in file order are numbered as if they had been
    /// lexed with this one.
    pub fn absorb(&mut self, other: &Interner) -> Vec<Symbol> {
        other
            .rodeo
            .strings()
            .map(|text| self.intern(text))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_symbols_are_where_the_parser_expects() {
        let interner = Interner::new();
        assert_eq!(interner.resolve(Symbol::text()), "text#");
        assert_eq!(interner.resolve(Symbol::string_type()), "String");
        assert_eq!(interner.resolve(Symbol::with_capacity()), "withCapacity");
        assert_eq!(interner.resolve(Symbol::push()), "push");
        assert_eq!(interner.resolve(Symbol::append_to()), "appendTo");
        assert_eq!(interner.resolve(Symbol::into_buffer()), "intoBuffer");
        assert_eq!(interner.resolve(Symbol::generator()), "generator#");
        assert_eq!(interner.resolve(Symbol::default_code()), "default#");
        assert_eq!(interner.resolve(Symbol::state()), "state#");
        assert_eq!(interner.resolve(Symbol::next()), "next");
        assert_eq!(interner.resolve(Symbol::of()), "of");
        assert_eq!(interner.resolve(Symbol::ignored()), "_");
        assert_eq!(interner.resolve(Symbol::append_fitted()), "appendFitted");
        let [width, fill, align] = Symbol::fitting_options();
        assert_eq!(interner.resolve(width), "width");
        assert_eq!(interner.resolve(fill), "fill");
        assert_eq!(interner.resolve(align), "align");
        assert_eq!(interner.resolve(Symbol::some()), "Some");
        let written = Symbol::written().map(|sym| interner.resolve(sym).to_string());
        assert_eq!(written, ["precision", "radix", "withPrecision", "inRadix"]);
        assert_eq!(interner.resolve(Symbol::upper()), "upper");
        assert_eq!(interner.resolve(Symbol::from_word()), "from");
        let [unsigned, signed] = Symbol::index_of();
        assert_eq!(interner.resolve(unsigned), "indexOfUnsigned");
        assert_eq!(interner.resolve(signed), "indexOfSigned");
    }
}
