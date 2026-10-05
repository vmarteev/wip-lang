# 3. Functions

## Declaring one

A function is `fn name(parameters): Result = body`. The body is one
expression, which is usually a block, and `return` gives the answer, at
the block's end or before it. A block's last expression is its value as
well ([page 4](04-expressions.md)), so `return` may be left off the last
line; the examples here write it.

```wip,run
fn double(n: i64): i64 = n * 2

fn describe(n: i64): str = {
    if n < 0 then return "negative"
    return if n == 0 then "zero" else "positive"
}

fn main() = {
    assert(double(21) == 42)
    assert(describe(-1) == "negative")
    assert(describe(0) == "zero")
}
```

A function that answers nothing leaves the result out, and a function that
never answers at all says `never` ([page 9](09-errors.md)).

## Arguments

Arguments are given by position, by name, or left to a default. A
parameter that has one may be left out, and its default runs at each call
that leaves it out — a constant, or code: `Vec()`. An argument given
by name may stand in any order after the positional ones:

```wip,run
fn line(length: i64, character: str = "-", indent: i64 = 0): i64 =
    length + character.len() + indent

fn main() = {
    assert(line(3) == 4)
    assert(line(3, "=") == 4)
    assert(line(3, indent: 2) == 6)
    assert(line(length: 3, indent: 2) == 6)
}
```

A name at the call is checked against the parameter's, so a call that names
the wrong one is refused rather than passed along.

## Methods

Methods are declared in the type's own body or in an `extend` block for it;
both may stand anywhere in the module that declares the type. The word
before `fn` says what the method does with `self`:

| Written | `self` is | For |
|---|---|---|
| `fn` | `&Self` | reading |
| `var fn` | `&var Self` | changing what it holds |
| `move fn` | `Self` | taking the value apart |
| `static fn` | nothing | making one, or anything that needs no value |
| `lend fn` | both, in turn | lending a place inside it ([page 7](07-references.md)) |

```wip,run
struct Counter {
    count: i64
    step: i64
}

extend Counter {
    static fn by(step: i64): Counter = Counter(count: 0, step: step)

    fn value(): i64 = self.count

    var fn advance() = {
        self.count += self.step
    }

    move fn total(): i64 = self.count
}

fn main() = {
    var counter = Counter::by(3)
    counter.advance()
    counter.advance()
    assert(counter.value() == 6)
    assert(counter.total() == 6)
}
```

A method is called with a dot; a `static fn` is called through the type,
`Counter::by(3)`. A method may be `pub`, which exports it from the module
as any other name is exported.

## Functions that yield

A function that answers `Iterator<T>` and hands its values over with
`yield` is a generator: calling it runs nothing, and each value is made
as it is asked for. [Page 4](04-expressions.md#generators)
has how it works.

## Functions as values

There are three ways to hand code to a function, and they differ in what the
code may carry with it:

| Written | Is | May capture |
|---|---|---|
| `(x: i64) => i64` | a function value: code alone | nothing |
| `&(x: i64) => i64` | a closure, lent for the call | what is around it |
| `own<(x: i64) => i64>` | a closure that owns what it captured | and outlives the call |

A lambda is written `(x) => body`, and its parameter types come from what is
expected of it:

```wip,run
fn applied(value: i64, f: (x: i64) => i64): i64 = f(value)
fn appliedTo(value: i64, f: &(x: i64) => i64): i64 = f(value)

fn twice(x: i64): i64 = x * 2

fn main() = {
    // A function's name is a value; so is a lambda that captures nothing.
    assert(applied(21, twice) == 42)
    assert(applied(21, (x) => x + 1) == 22)

    val step = 10
    // A lambda that uses `step` is a closure, and the parameter says so.
    assert(appliedTo(1, (x) => x + step) == 11)
}
```

A method is not a value, and neither is a type's own function: `x.name` and
`Type::name` are only called. Where a function is wanted, a lambda that calls
it stands in for it:

```wip,error=E0337
fn applied(value: i64, f: (x: i64) => i64): i64 = f(value)

struct Meters {
    value: i64

    static fn doubled(value: i64): i64 = value * 2
}

fn main() = {
    assert(applied(21, (x) => Meters::doubled(x)) == 42)
    assert(applied(21, Meters::doubled) == 42)
}
```

A parameter that is not used is written `_`, in a lambda, a function or a
function type, as many times as there are such parameters; it is passed,
and nothing can name it:

```wip,run
fn first(a: i64, _: i64): i64 = a

fn main() = {
    val failed: Result<i64, str> = .Err("no")
    assert(failed.unwrapOrElse((_) => 7) == 7)
    assert(first(1, 2) == 1)
}
```

What a lambda answers is what is expected of it, where something is; where
nothing is — the `R` of a generic function — it is what the lambda first
`return`s, or else its body's value:

```wip,run
fn apply<R>(f: &() => R): R = f()

fn main() = {
    val size = apply(() => {
        val sides = 4
        return sides * 2
    })
    assert(size == 8)
}
```

A closure written in a closure captures what the one around it captured,
and one written in a method captures `self` as any other name.

A closure lent as `&var (…) => R` holds a `&var` to what it writes, and a
`&` to what it only reads. What it captures is checked as the call's
arguments are, so two closures lent to one call may both read a variable,
and only one may write it:

```wip,run
fn both(first: &var () => i64, second: &var () => i64): i64 = first() + second()

fn main() = {
    val step = 10
    var ran = 0
    var done = 0
    val total = both(
        () => {
            ran += 1
            return step
        },
        () => {
            done += 1
            return step * 2
        },
    )
    assert(total == 30 && ran == 1 && done == 1)
}
```

A lambda that is a `val`'s whole value is kept there, and may be called
and passed on for as long as the block goes on. It reads
what it captures where it is, as a view reads what it borrows, so what it
captured must not change while it is still used, and it does not write it:

```wip,run
fn main() = {
    val names = Vec::of(own ["ann", "bartholomew", "cy"])
    val limit = 3
    val short = (name: &str) => name.len() <= limit
    assert(short(&names[0]) && !short(&names[1]))
    assert(names.items().countWhere(short) == 2)
}
```

A lambda that captures where a plain function value is expected is refused,
and says which name it captured. A closure that must outlive the call it is
given to — one an iterator adapter keeps, or a struct holds — is written
`own` at the call: `values.map(own (x) => x + step)`
([page 10](10-library.md)).

## Generic functions

A function may take type parameters, constrained by the interfaces its
arguments must implement. It is checked once, on
its own terms, and compiled once for each set of type arguments:

```wip,run
fn firstEqual<T: Eq>(values: &[T], wanted: &T): Option<i64> = {
    var i = 0
    while i < values.len() {
        if values[i] == wanted then return .Some(i)
        i += 1
    }
    return .None
}

fn main() = {
    val numbers = [3, 9, 4]
    assert(firstEqual(&numbers, 9) == .Some(1))
    val words = ["a", "b"]
    assert(firstEqual(&words, "c").isNone())
}
```

Type arguments are inferred from the arguments; where they cannot be, they
are written at the call: `firstEqual<i64>(…)`.

## `@tailrec` and `@inline`

Two annotations say how a function is compiled:

- **`@tailrec`** — every call it makes to itself is a tail call, and each is
  compiled as a jump back to the top. A call that is not in tail position is
  an error, so a function that says it recurses in constant space does.
- **`@inline`** — every call to it is spliced into its caller, or the
  program does not compile.

```wip,run
@tailrec
fn sum(upto: i64, total: i64 = 0): i64 =
    if upto == 0 then total else sum(upto - 1, total + upto)

@inline
fn twice(n: i64): i64 = n * 2

fn main() = {
    assert(sum(100000) == 5000050000)
    assert(twice(21) == 42)
}
```
