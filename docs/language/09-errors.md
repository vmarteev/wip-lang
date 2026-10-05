# 9. Failure

Wip has two ways for something to go wrong, and they are for different
things: a `Result` is a failure the caller is expected to deal with, and a
panic is a mistake in the program. There are no exceptions.

## `Result` and `Option`

`Result<T, E>` is `.Ok(value)` or `.Err(error)`; `Option<T>` is
`.Some(value)` or `.None`. Both are ordinary enums in the prelude, taken
apart the way any enum is ([page 5](05-patterns.md)):

```wip,run
// `@derive(Eq)` so that a `Result` holding one can be compared; an error
// that is only matched on needs no equality.
@derive(Eq)
enum ParseTrouble {
    Empty
    NotANumber
}

fn double(text: str): Result<i64, ParseTrouble> = {
    if text.isEmpty() then return .Err(.Empty)
    val .Ok(number) = text.toInt() else {
        return .Err(.NotANumber)
    }
    return .Ok(number * 2)
}

fn main() = {
    assert(double("21") == .Ok(42))
    assert(double("") == .Err(.Empty))
    assert(double("x").isErr())
}
```

Both carry the methods a program reaches for — `isSome`, `isNone`, `isOk`,
`isErr`, `unwrap`, `map`, `mapErr`, `unwrapOr`, `getOrInsert` — which are on
[page 10](10-library.md).

Where nothing says what is expected, `.Some` and `.None` are `Option`'s,
and `.Some(x)` says the rest; `.Ok` and `.Err`, which each
say only half of a `Result`, still need its type:

```wip,run
fn main() = {
    val wanted = 3
    val found = if wanted > 2 then .Some(wanted) else .None
    assert(found == .Some(3))
}
```

## `?`

`value?` answers the value where it is `.Ok` or `.Some`, and leaves the
function with the failure where it is not. It stands in a function that
answers the matching kind:

```wip,run
@derive(Eq)
enum Trouble {
    NotANumber
}

fn number(text: str): Result<i64, Trouble> = match text.toInt() {
    .Ok(n) => .Ok(n)
    .Err(..) => .Err(.NotANumber)
}

fn sum(a: str, b: str): Result<i64, Trouble> = {
    val first = number(a)?
    val second = number(b)?
    return .Ok(first + second)
}

fn main() = {
    assert(sum("1", "2") == .Ok(3))
    assert(sum("1", "x") == .Err(.NotANumber))
}
```

Where the error types differ, `?` converts through `From`, which the program
writes: nothing is converted that was not written down:

```wip,run
import std::text::{ParseError}

@derive(Eq)
enum ConfigError {
    NotANumber(text: String)
    Empty
}

extend ConfigError: From<ParseError> {
    static fn from(
        error: ParseError,
    ): ConfigError = .NotANumber(text: "\(error)")
}

fn port(text: str): Result<i64, ConfigError> = {
    if text.isEmpty() then return .Err(.Empty)
    // `toInt` fails with a `ParseError`, which becomes a `ConfigError`
    // on its way out.
    val number = text.toInt()?
    return .Ok(number)
}

fn main() = {
    assert(port("8080") == .Ok(8080))
    assert(port("").isErr())
    assert(port("http").isErr())
}
```

## `main` may answer a `Result`

A program's `main` may answer nothing, an `i64` exit code, or a
`Result` — and then the error is printed, by itself, through `Text`, and the
exit code is 1:

```wip,ignore
fn main(): Result<void, io::IoError> = {
    val text = fs::readFile("config")?
    io::println(text)
    return .Ok()
}
```

## `panic`

`panic("message")` prints the message, where it was written and the calls
that led there, and ends the program with status 101. The
message is text: a string written at the call, or one built when it
panics, `panic("no ghost at \(tile)")`. Nothing is freed on
the way out.

```wip,ignore
fn checkAge(age: i64): i64 =
    if age < 0 then panic("an age cannot be negative") else age
```

A panic is for what must not happen: an index outside a container, a
division by zero, `unwrap` on nothing, arithmetic that overflows. Those all
panic with a message of their own, naming the place.

After the place come the calls, innermost first, each with its function
and line, in a debug build and a release build alike:

```
panicked: index 7 is out of bounds for length 3
  at main.wip:1:41
  in crash (main.wip:1)
  in middle (main.wip:5)
  in main (main.wip:10)
```

The runtime's own frames and code the compiler writes on its own, such as
the drop of a value, are left out. Where the calls went through C — a
callback `qsort` or a library's event loop calls — the frames of C are one
line, `in C`, and the calls go on to the Wip that called C.

## `assert`

`assert(condition)` says what must hold. Where it does not, the program
panics with the condition's own text, and a note, if the call gives one,
stands before it. The note is text, and is built only when
the condition does not hold, so `\(…)` in it costs nothing while it does:

```wip,run
fn place(count: i64): i64 = {
    assert(count > 0, "a placement places something")
    assert(count < 100, "\(count) is more than the board holds")
    return count
}

fn main() = {
    assert(place(3) == 3)
}
```

An assert is compiled into every build — there is no flag that takes it out
— so what costs too much to check every time does not belong in one.
Failures read like this:

```
panicked: a placement places something: count > 0
  at board.wip:12:5
```

`assert` is also how a test says what it expects; tests are
[page 12](12-tools.md).

An assert written at the top level, beside the declarations, is checked
while the program is compiled, for the target it is compiled for, and a
condition that does not hold is a compile error saying what it would say at
run time. It is for what the program relies on and can know before it runs:
a layout C is told of, a table's length against an enum's count, a table
that must be sorted. Its condition may call functions, as a `@comptime`
constant's may ([page 2](02-modules.md)); `@target` says which target it is
for:

```wip,run
import std::c

struct Vertex {
    x: f32
    y: f32
    color: u32
}

val KEYS: [i64; 4] = [1, 3, 5, 9]

assert(c::sizeOf<Vertex>() == 12, "a vertex is what the shader reads")
assert(isSorted(&KEYS), "KEYS is searched by binarySearch")

fn isSorted(values: &[i64]): bool = {
    for i in 1..values.len() {
        if values[i - 1] > values[i] then return false
    }
    return true
}

fn main() = {
    assert(KEYS.binarySearch(5) == .Ok(2))
}
```

An assert inside a function is checked when it runs, in every build, even
where its condition could be worked out ahead.

## `never`

A function that does not answer at all — one that always panics, or always
leaves — has the result type `never`. A value of type `never` fits wherever
a value is expected, since there is none to fit:

```wip,ignore
fn giveUp(): never = panic("gave up")

val n: i64 = if ready then 1 else giveUp()
```
