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

An argument by position goes where its position says, whatever it is
called. So `draw(height, width)`, for `draw(width: i64, height: i64)`, puts
the height in `width`, and nothing on the line says so: it is a warning
(E0361), wherever two arguments or more are the parameters' own names in
each other's places. Where that is meant, the arguments are named, and
the warning goes:

```wip,run
fn draw(width: i64, height: i64): i64 = width * 10 + height

fn turned(width: i64, height: i64): i64 = draw(width: height, height: width)

fn main() = {
    assert(turned(2, 3) == 32)
}
```

A variable named as another parameter, where that parameter is given
something else, is not a swap, and is not reported: `binary(.Mul, rhs,
size)`, for `binary(kind, lhs, rhs)`, puts `rhs` in `lhs` and a size in
`rhs`.

## Implicit parameters

A parameter marked `implicit` is filled, where a call leaves it out, from
the caller's implicit of the same name: one of the caller's own implicit
parameters, or an `implicit val` or `implicit var` in scope. What a
function needs is still said in its signature; what is not written again
is the same name at every call that only passes it on. An implicit
`&var` is lent as an argument is, and a shared one shared, so the borrow
rules see the call as if it were written; it may always be passed by
name:

```wip,run
struct Dice {
    next: i64
}

extend Dice {
    var fn roll(): i64 = {
        self.next = (self.next * 7 + 3) % 10
        return self.next
    }
}

fn twice(implicit dice: &var Dice): i64 = dice.roll() + dice.roll()

// Only passes it on: the calls write nothing for it.
fn round(implicit dice: &var Dice): i64 = twice() + twice()

fn main() = {
    implicit var dice = Dice(next: 1)
    val first = round()
    var other = Dice(next: 1)
    assert(round(dice: &var other) == first, "given by name")
}
```

- **By name and type.** An implicit is found by its name, and must then
  be of the type the parameter takes. Two implicits of one type do not
  collide, and a reader looking for where `dice` comes from looks for
  `dice`.
- **Only implicits fill implicits.** An ordinary variable of the name is
  not used, so that declaring one never changes what an unrelated call
  receives. Where nothing implicit has the name, the call is refused
  (E0455), with the two ways to declare one.
- **Implicit parameters come after the others,** and are a reference, `&`
  or `&var`, or plain data, copied; a value that owns memory would be
  moved out of the caller at every call. Their type names none of the
  function's type parameters, and they have no default (E0454).
- **A lambda sees the implicits where it is written,** as it captures
  anything around it, and declares none of its own. A function with
  implicit parameters is not a value (E0456): it would be called somewhere
  else and filled from there, where a lambda that calls it, `(x) =>
  f(x)`, is filled where it is written.
- **An interface's method says its implicits,** and an implementation
  declares the same ones; a call through the interface, a constraint or a
  `&dyn` fills what the interface says.
- **`implicit` is a word only there,** before a parameter and before
  `val` or `var`. Elsewhere it is a name.

```wip,error=E0455
struct Log {
    lines: i64
}

fn say(text: str, implicit log: &Log): i64 = log.lines + text.len()

fn main() = {
    val log = Log(lines: 0)
    say("not implicit, so not used")
}
```

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

    fn count(): i64 = self.count

    var fn advance() = {
        self.count += self.step
    }

    move fn total(): i64 = self.count
}

fn main() = {
    var counter = Counter::by(3)
    counter.advance()
    counter.advance()
    assert(counter.count() == 6)
    assert(counter.total() == 6)
}
```

A method is called with a dot; a `static fn` is called through the type,
`Counter::by(3)`. A method may be `pub`, which exports it from the module
as any other name is exported.

A field and a method may share a name, as `count` does here: `counter.count`
reads the field, and `counter.count()` calls the method. A field that holds
a function is called as a method is, so it may not share a method's name.

A `var fn` changes the place it is called on, which must be one that can be
written: a `var`, or one behind a `&var`. It may also be called on a value
made where it is called, such as what a call answers. That value is lent
`&var` for the call and ends with its statement, so what the method changed
goes with it, and what it answers is what is wanted:

```wip,run
fn main() = {
    val text = "first\nsecond"
    // An iterator made to be asked for one value needs no name.
    assert(text.lines().next() == .Some("first"))
    assert(Vec::of([1, 2, 3]).pop() == .Some(3))
}
```

## Functions that yield

A function that answers `Iterator<T>` and hands its values over with
`yield` is a generator: calling it runs nothing, and each value is made
as it is asked for. [Page 4](04-expressions.md#generators)
has how it works.

## Functions as values

There are four ways to hand code to a function, and they differ in what the
code may carry with it:

| Written | Is | May capture |
|---|---|---|
| `(x: i64) => i64` | a function value: code alone | nothing |
| `&(x: i64) => i64` | a closure, lent for the call | what is around it, to read |
| `&var (x: i64) => i64` | a closure, lent for the call | what is around it, to read and change |
| `own<(x: i64) => i64>` | a closure that owns what it captured | its own copies, and outlives the call |

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

A type's function is a value too, written through the type, `Type::name`,
as a function of a module is by its name. A `static fn` is the function it
is; a method takes its receiver as its first parameter — `&Self`, `&var
Self` or `Self`, as it declares — so the value is called as the method is
called through its type:

```wip,run
struct Shape {
    side: i64

    fn area(): i64 = self.side * self.side
}

fn main() = {
    val shapes = [Shape(side: 2), Shape(side: 3)]
    assert(shapes.map(Shape::area) == [4, 9])
    val area: (shape: &Shape) => i64 = Shape::area
    assert(area(&shapes[1]) == 9)
    val build: (text: str) => String = String::of
    assert(build("bc") == "bc")
}
```

A generic type's arguments come from the type expected, as a generic
function's do, and where a type has one function of the name for each of
two interfaces, `Meters::from`, the type expected says which. `x.name` is
not a value, since it would hold `x`; and a projection, an `@inline`
function, and one the compiler writes where it is called, as `str::len`,
are not values either. Where one is wanted, a lambda that calls it stands
in for it:

```wip,error=E0337
struct Grid {
    cells: [i64; 4]

    fn total(): i64 = self.cells[0] + self.cells[1]
}

fn main() = {
    val grid = Grid(cells: [1, 2, 3, 4])
    val bound = grid.total
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
`return`s, or else its body's value, and a `?` before that is checked once
it is said ([page 9](09-errors.md)):

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

What calls a closure one call at a time, in order — a slice's `map`,
`filter`, `fold`, `find` and `any`, an `Option`'s `map`, an iterator's
`fold` and `find` — takes `&var (…) => R`, so the closure may change what it
captures, `self` in a `var fn` among them. What shares one closure among
threads, `future::map`, takes `&(…) => R`, and a closure that writes is
refused there. A closure that reads goes where one that writes is taken,
since it writes nothing; not the other way round:

```wip,run
struct Interner {
    names: Vec<String>

    var fn intern(name: str): i64 = {
        self.names.push(name)
        return self.names.len() - 1
    }

    var fn ids(names: &[str]): Vec<i64> = names.map((name) => self.intern(name))
}

fn main() = {
    var interner = Interner(names: [])
    assert(interner.ids(["a", "b"]) == [0, 1])
    val first: Option<str> = .Some("c")
    assert(first.map((name) => interner.intern(name)) == .Some(2))
}
```

A lambda that is a `val`'s whole value is kept there, and may be called
and passed on for as long as the block goes on. What it only reads, it
reads where it is, as a view reads what it borrows, so that must not
change while the closure is still used. What it writes, it holds as a
`&var` is held, from the `val` to the closure's last use: nothing else
reads or changes it until then (E0414), and the closure is `&var (…) =>
R`, written so where its type is said:

```wip,run
fn twice(f: &var (x: i64) => void) = {
    f(1)
    f(2)
}

fn main() = {
    val names = Vec::of(["ann", "bartholomew", "cy"])
    val limit = 3
    val short = (name: &str) => name.len() <= limit
    assert(short(&names[0]) && !short(&names[1]))
    assert(names.items().countWhere(short) == 2)

    var seen = 0
    val note: &var (x: i64) => void = (x) => seen += x
    note(1)
    twice(note)
    assert(seen == 4)
}
```

```wip,error=E0414
fn main() = {
    var seen = 0
    val note = (x: i64) => seen += x
    note(1)
    val early = seen
    note(2)
}
```

It is the one `&var` a variable holds: a `var` holds no closure, since it
could be pointed at another while the first still held what it
captured.

A lambda that captures where a plain function value is expected is refused,
and says which name it captured. An iterator's adapter keeps the closure
it is lent, and is a view of what the closure captured: `words.filter((w)
=> w.startsWith(prefix))` borrows `prefix`, and is used while that stays
as it was, in its statement, its `val`'s block or its loop
([page 10](10-library.md)). A closure that must outlive the block it is
written in — one a struct owns, or another thread runs — is written `own`:
`Future::run(own () => …)`. A named function goes where either kind of
closure is expected, a module's or a type's, since it captures nothing: it
is lent for a call, or owned with nothing to own:

```wip,run
fn double(x: i64): i64 = x * 2

fn main() = {
    val words = "a bc".split(" ").map(String::of).toVec()
    assert(words == ["a", "bc"])
    val doubled = Vec::of([1, 2]).intoIterator().map(double).toVec()
    assert(doubled == [2, 4])
    val kept: own<(x: i64) => i64> = double
    assert(kept(21) == 42)
}
```

A variable that holds a function value is captured as any variable is: a
closure reads it where it is, and one that writes may assign it.

```wip,run
fn each(n: i64, visit: &var (x: i64) => void) = {
    for i in 1..n + 1 {
        visit(i)
    }
}

fn main() = {
    val scale = (x: i64) => x * 2
    var total = 0
    each(3, (x) => total += scale(x))
    assert(total == 12)
}
```

A function value is passed as it is, and `&` of one is refused: it would
have a closure's type, `&(…) => R`, and a closure is an environment and its
code where a function value is code alone. Where a closure is expected, a
lambda that calls it is one:

```wip,error=E0450
fn apply(f: &(x: i64) => i64, x: i64): i64 = f(x)

fn main() = {
    val double = (x: i64) => x * 2
    assert(apply(&double, 4) == 8)
}
```

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

Type arguments are inferred from the arguments, from what the
arguments hold — a `T` from a `ptr<T>` or a `&[T]` — and from what an
argument's type implements: with `I: Iterator<(K, V)>`, an iterator that
hands out `(str, i64)` says `K` and `V`, as `Map::fromPairs` is called with
no type written. A variable's are also inferred from the first place that
names it ([page 1](01-types.md#generic-types)): `var counts = Map()`, then
`counts.put(word, 1)`. A length parameter, `val N`, is inferred from the
length of an array given where `[T; N]` is taken: `total([1, 2, 3])` is
`total<3>`, which a call writes with a constant's name, `total<SIDE>(…)`. Where they cannot be, they are written at the call:
`firstEqual<i64>(…)`. A generic type's static
function is called through the type, and the type's arguments are written
after its name, `Pair<i64>::zero()`; the function's own are still inferred.

```wip,run
struct Pair<T> {
    first: T
    second: T
}

extend Pair<T> {
    static fn of<U: copy>(a: T, b: T, tag: U): (Pair<T>, U) = (
        Pair(first: a, second: b),
        tag,
    )
}

fn main() = {
    val (pair, tag) = Pair<i64>::of(1, 2, 'x')
    assert(pair.first + pair.second == 3 && tag == 'x')
}
```

What the call's value is expected to be says them too, as a `val`'s type
does: `val numbers: Vec<i64> = json::decode(text)?`. The type expected
passes through `?` to what the `Option` or the `Result` holds, and through
a method called on the value to what that method answers, read in the
type the value is — `.unwrap()` answers a `Result<T, E>`'s `T` — where
the call's own arguments do not say. A method's own type arguments are
written after its name, `answer.read<Vec<i64>>()`:

```wip,run
import std::json
import std::json::{DecodeError}

struct Answer {
    text: String
}

extend Answer {
    fn read<T: Decode>(): Result<T, DecodeError> = json::decode(&self.text)
}

fn total(answer: &Answer): Result<i64, DecodeError> = {
    val numbers: Vec<i64> = answer.read()?
    return .Ok(numbers.sum())
}

fn main() = {
    val answer = Answer(text: String::of("[1, 2, 3]"))
    assert(total(&answer) == .Ok(6))
    val numbers: Vec<i64> = answer.read().unwrap()
    assert(numbers.len() == 3)
    val words = answer.read<Vec<String>>()
    assert(words.isErr())
}
```

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

## `become`

`become f(args)` leaves the function and calls `f` in its place: the
function's frame is gone before `f` runs, and what `f` answers is answered
to the function's own caller. It stands where `return` may, followed by a
call, so two functions that call each other at the end use one frame
however deep they go:

```wip,run
fn isEven(n: u64): bool = {
    if n == 0 then return true
    become isOdd(n - 1)
}

fn isOdd(n: u64): bool = {
    if n == 0 then return false
    become isEven(n - 1)
}

fn main() = {
    assert(isEven(1_000_000) && !isOdd(1_000_000))
}
```

The call takes the frame, so it must fit in it:

- **The callee is a function called by its name** — a function, a method or
  a static function, not a closure, a function value, a `dyn` method, a C
  function or an `@inline` one — and `become` is not written in a lambda, a
  projection or a generator.
- **It takes what the function was given and answers what it answers:** its
  parameters, a method's receiver first, have the function's parameter
  types in their order, and its result is the function's.
- **Nothing of the frame is left:** no local still owns memory (E0422), no
  `defer` is waiting (E0422), and no argument refers into the frame — `&` of
  a local, or a closure lent for the call (E0421).
- **A struct, an enum, an array, a `str` or a slice is a parameter passed
  on.** Such a value is passed by the address of its place, and a place of
  the frame is gone before the call reads it (E0461); a parameter the
  function was given is in its caller's frame. An aggregate result is
  fine: it is written where the function's caller asked for its own.

A panic's calls, and a debugger's, show `isOdd` called from where `isEven`
was. `@tailrec` stays for a function's calls to itself, which it makes a
jump to its own top, with no rule on its signature.

```wip,error=E0460
fn add(n: i64, m: i64): i64 = n + m

fn one(n: i64): i64 = {
    become add(n, 1) // takes two, where `one` was given one
}
```
