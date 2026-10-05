//! Every diagnostic code, in one place. A code names one rule, and says
//! so here in a line; the note a diagnostic carries says why the rule is
//! there.
//!
//! Only this module makes a `Code`, so two crates cannot take one number,
//! and a code that is retired is listed in [`RETIRED`] and never taken
//! again.

/// A stable diagnostic code such as `E0002`. Its field is this module's,
/// so a code is made here and nowhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Code(&'static str);

impl Code {
    /// The code as it is printed: `E0002`.
    pub fn as_str(self) -> &'static str {
        self.0
    }

    /// What the code means, in a line.
    pub fn meaning(self) -> &'static str {
        ALL.iter()
            .find(|(code, _)| *code == self)
            .map_or("", |(_, meaning)| meaning)
    }

    /// The code printed as `number`, if there is one.
    pub fn named(number: &str) -> Option<Code> {
        ALL.iter()
            .map(|(code, _)| *code)
            .find(|code| code.0 == number)
    }
}

/// Declares each code, with the line that says what it is, and the table
/// of all of them.
macro_rules! codes {
    ($($(#[doc = $doc:literal])* $name:ident = $code:literal: $meaning:literal;)*) => {
        $(
            #[doc = $meaning]
            #[doc = ""]
            $(#[doc = $doc])*
            pub const $name: Code = Code($code);
        )*

        /// Every code, and the line that says what it is, in the order of
        /// their numbers.
        pub const ALL: &[(Code, &str)] = &[$(($name, $meaning),)*];
    };
}

/// Codes once given and since retired, which are never given again.
pub const RETIRED: &[&str] = &[
    "E0006", "E0102", "E0103", "E0107", "E0110", "E0116", "E0327", "E0502", "E0503",
];

codes! {
    // The lexer's.
    UNEXPECTED_CHAR = "E0001": "A character that is not part of the language.";
    UNTERMINATED_STRING = "E0002": "A string literal with no closing quote.";
    UNKNOWN_ESCAPE = "E0003": "An escape sequence Wip does not have, or a `\\u{…}` that is not a Unicode scalar value.";
    FLOAT_MISSING_FRACTION = "E0004": "A decimal point with no digit after it.";
    LEADING_ZERO = "E0005": "A number literal that starts with a zero.";
    NUMBER_SUFFIX = "E0007": "A suffix on a number literal that names no number type.";
    SINGLE_QUOTES = "E0008": "A character literal that is not closed, or holds more than one character.";
    TILDE = "E0009": "`~`, which Wip writes `!`.";
    DIGIT_SEPARATOR = "E0010": "A `_` in a number that is not a single separator between digits.";
    NUMBER_BASE = "E0011": "`0x`, `0o` or `0b` without the digits its base allows.";
    BYTE_LITERAL = "E0012": "A byte literal that is not one ASCII character.";
    /// Text on the line of its opening `"""`, a line indented less than its
    /// closing one, a `"""` inside it, or a backslash ending a line.
    TEXT_BLOCK = "E0013": "A text block whose opening, closing or lines are not as a text block has them.";

    // The parser's.
    EXPECTED = "E0101": "Something other than what the grammar allows here.";
    MATCH_ARM_SEPARATOR = "E0104": "Two `match` arms on one line with no `,` between them.";
    UNSUPPORTED_ABI = "E0105": "An `extern` block with an ABI other than `\"C\"`.";
    FUNCTION_BODY = "E0106": "A function with no body, or a body not introduced by `=`.";
    UNCLOSED_BRACE = "E0108": "A `{` that is never closed.";
    DOUBLE_BORROW = "E0109": "`&&` where a reference to a reference was meant.";
    INTEGER_TOO_LARGE = "E0111": "An integer literal too large for any integer type.";
    SEPARATOR = "E0112": "Two statements or members on one line with nothing between them.";
    LEADING_OPERATOR = "E0113": "A line that starts with a binary operator.";
    LET = "E0114": "`let`, which Wip writes `val`.";
    ARROW_RETURN_TYPE = "E0115": "A return type after `->`, which Wip writes after `:`.";
    BINDER_AFTER_REST = "E0117": "A binding after `..` in a pattern.";
    UNDERSCORE_BINDER = "E0118": "`_` used as a name to bind.";
    IMPORT_LIST = "E0119": "An import list that names nothing, or is renamed as a whole.";
    TYPE_ARGS_COLONS = "E0120": "`::<…>` before type arguments, which are written `<…>`.";
    FN_TYPE_SYNTAX = "E0121": "`fn(A, B): R`, where a function type is written `(a: A, b: B) => R`.";
    POSITIONAL_AFTER_NAMED = "E0122": "A positional argument after a named one.";
    GUARD_SYNTAX = "E0123": "A guard, `val pattern = value else { … }`, written without `else`.";
    /// `pub` on a field, or `var`, `move` or `static` without `fn`.
    MEMBER_SYNTAX = "E0124": "A struct's field or an enum's variant written wrongly.";
    ANNOTATION_SYNTAX = "E0125": "An annotation with no declaration after it, or on one that cannot carry it.";
    /// It reads as a method call on the line above. A warning, since it means
    /// what it says.
    VARIANT_UNDER_EXPRESSION = "E0126": "A variant at the start of a line, under a statement a `.` could have continued.";
    IMPL_KEYWORD = "E0127": "`impl`, which Wip writes `extend`.";
    YIELD_KEYWORD = "E0128": "`yield` where a value is expected, or where a projection's `lend` was meant.";
    TUPLE_ARITY = "E0129": "A tuple of too few or too many elements.";
    REST_NOT_LAST = "E0130": "`..value` written twice, not last, or in a call that builds no struct.";
    FLOAT_PATTERN = "E0131": "A float literal as a pattern.";
    ASSERT_ARGUMENTS = "E0132": "An `assert` given something other than a condition and a note.";
    BRACE_LITERAL = "E0133": "A struct built with braces, which is written `Row(…)`.";
    INTERPOLATION_OPTION = "E0134": "A piece of an interpolation with an option other than `width`, `fill` and `align`, one given twice, or no width.";

    // Names, modules and packages: the checker's.
    DUPLICATE_DEFINITION = "E0201": "A name defined, or imported, twice.";
    UNKNOWN_TYPE = "E0202": "A type name that names no type.";
    UNKNOWN_NAME = "E0203": "A name that names nothing where it is looked up.";
    UNKNOWN_VARIANT = "E0204": "A variant the enum does not have, or a variant of what is not an enum.";
    VARIANT_WITH_DOT = "E0205": "A variant written `Enum.Variant`, where Wip writes `Enum::Variant`.";
    BUILTIN_REDEFINED = "E0206": "A built-in type's name declared again, or given to a type parameter.";
    BINDING_SHADOWS_VARIANT = "E0207": "A pattern's name that binds, where the variant of that name was probably meant.";
    NOT_A_VALUE = "E0208": "A type, or a projection, used as a value.";
    UNKNOWN_MODULE = "E0209": "A path whose first segment names no module.";
    PRIVATE_ITEM = "E0210": "An item of another module that is not `pub`, or a field it may not write.";
    TYPE_ARGUMENT_COUNT = "E0211": "The wrong number of type arguments, or type arguments where none are taken.";
    UNKNOWN_CONSTRAINT = "E0212": "A constraint on a type parameter that names nothing a parameter can be required to be.";
    CANNOT_BE_GENERIC = "E0213": "A generic item that cannot be generic: an extern function, or `main`.";
    PACKAGE = "E0214": "A `package.wip` that says something wrong, or a dependency that is not what it says.";
    INTERNAL_MODULE = "E0215": "A module under `internal/` imported from another package.";
    TYPE_PARAMETER_DEFAULT = "E0216": "A default type argument on what is not a struct or an enum, before a parameter with none, or naming a parameter after it.";

    // Types: the checker's.
    MISMATCHED_TYPES = "E0301": "A value of one type where another is expected.";
    ARGUMENT_COUNT = "E0302": "A call with too few or too many arguments.";
    INVALID_OPERANDS = "E0303": "An operator applied to what it does not take.";
    INVALID_ASSIGNMENT = "E0304": "An assignment to what cannot be assigned.";
    MISSING_RETURN = "E0305": "A function that can reach its end without the value it answers.";
    NOT_CALLABLE = "E0306": "A call of what is not a function.";
    NO_SUCH_FIELD = "E0307": "A field the type does not have.";
    STRUCT_LITERAL_FIELDS = "E0308": "A struct literal whose fields do not fit the struct.";
    INVALID_INDEX = "E0309": "An index into what cannot be indexed, or a constant index out of bounds.";
    INTEGER_OUT_OF_RANGE = "E0310": "An integer literal out of range for its type.";
    NON_EXHAUSTIVE_MATCH = "E0311": "A `match` that leaves a value uncovered.";
    UNREACHABLE_ARM = "E0312": "A `match` arm no value can reach.";
    INVALID_PATTERN = "E0313": "A pattern that cannot match the type it is tested against.";
    NOT_C_COMPATIBLE = "E0314": "A type or a call C cannot take as it is written.";
    CANNOT_INFER = "E0315": "A type that cannot be worked out from what is written.";
    RECURSIVE_TYPE = "E0316": "A type that holds itself directly, and so has no size.";
    MOVE_NOT_A_PLACE = "E0317": "`move` of what is not a variable, a field or an element.";
    INVALID_MAIN = "E0318": "No `main`, or a `main` whose parameters or result a program cannot start with.";
    INVALID_CAST = "E0319": "An `as` between types it does not convert.";
    DISCARDED_VALUE = "E0320": "A warning: a value computed with no side effect and never used.";
    PATTERN_FIELDS = "E0321": "A pattern that names a field twice, or leaves fields out without `..`.";
    LOOP_JUMP_OUTSIDE = "E0322": "`break` or `continue` outside a loop.";
    NOT_ITERABLE = "E0323": "`for` over what cannot be walked.";
    RUNTIME_LENGTH = "E0324": "A length known only at run time, outside `own`.";
    NOT_COPY = "E0325": "A type argument that owns memory, where `copy` is required.";
    INSTANTIATION_DEPTH = "E0326": "Instances of generic items that would never end.";
    UNKNOWN_ARGUMENT_NAME = "E0328": "A named argument for a parameter or a field that does not exist.";
    ARGUMENT_TWICE = "E0329": "A parameter or a field given two values.";
    INVALID_DEFAULT = "E0330": "A default where there can be none, or one that cannot be a default.";
    UNNAMED_FIELDS = "E0331": "A variant or a struct with two or more fields, built without naming them.";
    NAMED_ARGUMENT_TO_VALUE = "E0332": "A named argument in a call through a function value.";
    IS_BINDING_OUTSIDE_CONDITION = "E0333": "An `is` test that binds a name nothing after it can use.";
    GUARD_ELSE_FALLS_THROUGH = "E0334": "A guard's `else` block that can reach its end.";
    SHADOWED_TYPE_PARAMETER = "E0335": "A method's type parameter with the name of one of its type's.";
    SELF_OUTSIDE_METHOD = "E0336": "`self` outside a method, or in a `static fn`.";
    NOT_A_METHOD = "E0337": "A method the type does not have, or a method or a type's function named without a call.";
    /// A type of another module, or one that does not exist.
    IMPL_TARGET = "E0338": "An `extend` block for a type it may not extend, or that names the type's parameters wrongly.";
    /// In a constraint or an `extend` block.
    UNKNOWN_INTERFACE = "E0339": "An interface name that names no interface.";
    DUPLICATE_IMPL = "E0340": "A type that implements one interface twice.";
    IMPL_METHODS = "E0341": "An implementation that leaves out, adds or changes one of the interface's methods.";
    UNSATISFIED_CONSTRAINT = "E0342": "A type argument that does not implement what its parameter requires.";
    NOT_DISPATCHABLE = "E0343": "`&dyn` of an interface with a method that cannot be called through it.";
    UNSIZED_DYN = "E0344": "A `dyn` type that is not behind `&`.";
    PRELUDE_NAME = "E0345": "A name the prelude declares, declared or imported again.";
    PANIC_ARGUMENT = "E0346": "`panic` called with something other than one message in text.";
    NEVER_VALUE = "E0347": "`never` where a value would have to exist.";
    CANNOT_PROPAGATE = "E0348": "`?` on what is not an `Option` or a `Result`, or where the function cannot carry it.";
    LAMBDA_PARAMS = "E0349": "A lambda whose parameters do not match what is expected, or whose types are not known.";
    /// That is a closure.
    LAMBDA_CAPTURE = "E0350": "A lambda that captures a name, where a plain function is expected.";
    YIELD_OUTSIDE_LIST = "E0351": "`yield` with no list being built and no generator around it.";
    /// An `if` without `else` that yields nothing.
    ELEMENT_ADDS_NOTHING = "E0352": "An element of a list that adds nothing to it.";
    /// An array literal, or a bare loop.
    LIST_NEEDS_OWN = "E0353": "A list whose length its `yield`s decide, where no `own` builds it.";
    NO_VALUE = "E0354": "A variable given an `if` without `else`, which has no value.";
    ARGUMENT_NAMED_WITH_EQUALS = "E0355": "`f(name = value)`, where a named argument, `name: value`, was meant.";
    /// A loop that stands for a value with no `yield` in it, or a function that
    /// answers `Iterator<T>` and yields nothing.
    GENERATOR_YIELDS_NOTHING = "E0356": "A generator that yields nothing.";
    /// Such a loop has no function of its own to leave.
    RETURN_IN_GENERATOR = "E0357": "`return` in a loop that is a generator.";
    /// The generator would hold it between the calls that ask for its values.
    GENERATOR_HOLDS_VAR = "E0358": "A function that yields and takes a `&var`.";
    /// Directly or through another: it would be endlessly large.
    GENERATOR_HOLDS_ITSELF = "E0359": "A generator that holds itself.";
    SLICE_PATTERN_REST = "E0360": "A slice pattern with more than one `..`.";

    // Moves, references and borrows: the checker's and the analysis's.
    USE_OF_MOVED_VALUE = "E0401": "A value used after it was moved.";
    USE_OF_POSSIBLY_MOVED_VALUE = "E0402": "A value used after it may have been moved.";
    MOVED_IN_PREVIOUS_ITERATION = "E0403": "A value moved in one iteration of a loop and used in the next.";
    CANNOT_COPY_OWN = "E0404": "A value that owns memory, used where it would be copied.";
    CANNOT_MOVE_OUT_OF_BORROW = "E0405": "A move out of what is only borrowed.";
    CANNOT_MOVE_OUT_OF_ARRAY = "E0406": "A move of an array's element that owns memory.";
    CANNOT_REPEAT_OWN = "E0407": "`[x; n]` of a value that owns memory.";
    ASSIGN_TO_PART_OF_MOVED = "E0408": "An assignment to part of a value that was moved.";
    CANNOT_MOVE_OUT_OF_TEMPORARY = "E0409": "A move out of a temporary.";
    REF_OUTSIDE_PARAMETER = "E0410": "A `&var` inside a type, or a reference where none may stand.";
    REF_OUTSIDE_ARGUMENT = "E0411": "`&var` outside a call's argument, or `&` where no reference is expected.";
    CANNOT_BORROW_VAR = "E0412": "`&var` of what cannot be written.";
    CONFLICTING_REFERENCES = "E0413": "A `&var` argument whose place another argument of the call refers to too.";
    CHANGED_WHILE_BORROWED = "E0414": "A place changed while a binding, or an earlier argument, refers to it.";
    VAR_PARAMETER_LEFT_EMPTY = "E0415": "A `&var` parameter left without a value when the function returns.";
    UNSIZED_SLICE = "E0416": "A slice not behind a reference, or kept in a variable.";
    SLICE_NOT_BORROWED = "E0417": "A range of elements that is not borrowed.";
    PROJECTION_BODY = "E0418": "A projection whose body does not keep the rules of a projection.";
    LEND_OUTSIDE_PROJECTION = "E0419": "`lend` outside a projection.";
    ANNOTATION = "E0420": "An annotation that is unknown, misplaced, given the wrong arguments or written twice.";
    NOT_A_TAIL_CALL = "E0421": "A `@tailrec` function whose call to itself cannot be a jump.";
    TAIL_CALL_LEAVES_WORK = "E0422": "A `@tailrec` function's call to itself with something of the frame still to clean up.";
    NOT_A_CONSTANT = "E0423": "A top-level `val` whose value is not known where it is written.";
    NULL_WITHOUT_TYPE = "E0424": "`null` where no pointer type is expected.";
    /// Rather than behind `ptr`.
    OPAQUE_VALUE = "E0425": "A C type whose contents Wip does not know, used as a value.";
    DROP_RULES = "E0426": "`destroy` called by a program, or a field moved out of a type that has one.";
    /// As a field, a payload or an element.
    STORED_STR = "E0427": "A `str`, a `&` reference or a view kept where it would outlive what it borrows.";
    VARIADIC = "E0428": "`...` on a declaration that is not C's.";
    NAMED_VARIADIC_ARGUMENT = "E0429": "A named argument to a C function that takes more than it declares.";
    /// One that calls itself, one taken as a value, or a C declaration.
    CANNOT_INLINE = "E0430": "An `@inline` function that cannot be spliced into its callers.";
    NOT_COMPARABLE = "E0431": "A field that cannot be compared, in a type that asks for `@derive(Eq)`.";
    /// Such a value is already lent where it is used.
    REF_NOT_A_PLACE = "E0432": "`&` of a value that has no place of its own.";
    /// A method of a conformance is as visible as its interface.
    REDUNDANT_PUB = "E0433": "`pub` where visibility is not the declaration's to give.";
    GUARD_REFUTABILITY = "E0434": "A `val` pattern with an `else` that cannot run, or without one where it can fail.";
    ALIAS = "E0435": "A type alias that takes constraints, or one that names itself.";
    STR_OUTLIVED_ITS_BYTES = "E0436": "A `str`, a `&` reference or a view used after what it borrows changed.";
    STR_OUTLIVES_ITS_ROOT = "E0437": "A `str`, a `&` reference or a view that would outlive what it borrows.";
    ASSERT_NOTE = "E0438": "An assert whose note is not text.";
    /// Or used through what borrowed it before one.
    BORROWED_ACROSS_YIELD = "E0439": "A generator's own local borrowed across a `yield`.";
    NOT_CLONEABLE = "E0440": "A field that cannot be cloned, in a type that asks for `@derive(Clone)`.";
    CONSTANT_PANICKED = "E0441": "A constant whose code panicked while the program was compiled.";
    /// A call into C, a thread, the time.
    NOT_RUN_AT_COMPILE_TIME = "E0442": "A constant whose code does what cannot be done while the program is compiled.";
    EVALUATION_TOO_LONG = "E0443": "A constant whose code ran past the steps or the memory it may take.";
    COMPTIME_MARK = "E0444": "A constant that runs code without `@comptime`, or is marked `@comptime` and runs nothing.";
    ASSERTION_FAILED = "E0445": "A top-level `assert` whose condition does not hold.";
    /// Outside a constant, with a path that is not a literal, not there, or
    /// outside the package, or text that is not UTF-8.
    EMBED = "E0446": "A file embedded where it cannot be, or one that cannot be read as asked.";
    VAR_CONTENTS_KEPT = "E0447": "What a `&var` parameter reaches, kept in another parameter past the call.";

    // `defer`: the checker's.
    RETURN_IN_DEFER = "E0501": "`return` inside a deferred expression.";
    JUMP_IN_DEFER = "E0504": "`break` or `continue` leaving a deferred expression.";

    // The code generator's.
    /// None is left; the code stays reserved.
    UNSUPPORTED = "E0901": "A construct the code generator does not handle.";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code names one rule: no number is taken twice, none is one that
    /// was retired, and the table is in the order of the numbers, as a
    /// reader looks one up. `E0436` was once taken twice, by two crates
    /// that could not see each other's codes, which is why they are here.
    #[test]
    fn every_number_names_one_rule() {
        let numbers: Vec<&str> = ALL.iter().map(|(code, _)| code.as_str()).collect();
        for pair in numbers.windows(2) {
            assert!(pair[0] < pair[1], "{} before {}", pair[0], pair[1]);
        }
        for number in &numbers {
            let digits = number.strip_prefix('E').unwrap_or("");
            assert!(
                digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit()),
                "{number} is not E and four digits"
            );
            assert!(!RETIRED.contains(number), "{number} was retired");
        }
    }

    /// Each says what it is, in a sentence.
    #[test]
    fn every_code_says_what_it_is() {
        for (code, meaning) in ALL {
            assert!(meaning.ends_with('.'), "{}: {meaning}", code.as_str());
            assert_eq!(code.meaning(), *meaning);
            assert_eq!(Code::named(code.as_str()), Some(*code));
        }
        assert_eq!(Code::named("E0110"), None, "a retired code is no code");
    }
}
