# 5. Patterns

A pattern says what shape a value has and gives names to its parts. The same
patterns stand in every place one can: a `match` arm, an `is` test, a `val`
or a `var`, with or without `else`, and a `for` binding.

## What a pattern can be

| Pattern | Matches |
|---|---|
| `_` | anything, binding nothing |
| `name` | anything, and binds it |
| `7`, `'x'`, `b'x'`, `"text"`, `true` | a value the scrutinee must equal |
| `OP_ADD`, a constant's name | the value it stands for |
| `'a'..='z'`, `0..10`, `..0`, `10..` | an integer or a character between two ends |
| `.Some(n)`, `Shape::Round(radius)` | that variant, binding what it carries |
| `.Round(..)` | that variant, whatever it carries |
| `.Some(_)` | a variant of one field, whatever it holds: `..`, for the one field |
| `Point(x, y)`, `Point(x: across, ..)` | a struct, by field name, renamed or the rest passed over |
| `(a, b)` | a tuple |
| `[a, b]`, `[first, ..rest]`, `[.., last]` | an array or a slice, by its elements from either end |
| `a \| b` | either, each binding the same names |
| `.Some(.None)`, `(.Int(a), .Float(b))` | a pattern inside a pattern, to any depth |

```wip,run
enum Shape {
    Round(radius: i64)
    Square(side: i64)
    Empty
}

fn describe(shape: &Shape): str = match shape {
    .Round(radius) if radius > 10 => "a big circle"
    .Round(..) => "a circle"
    .Square(..) | .Empty => "not a circle"
}

fn main() = {
    assert(describe(Shape::Round(radius: 20)) == "a big circle")
    assert(describe(Shape::Round(radius: 1)) == "a circle")
    assert(describe(.Empty) == "not a circle")
}
```

A variant's fields are named where it declares them, so a pattern may name
the field it takes: `.Round(radius: r)`. A variant with one field needs no
name, and a pattern that names none of a variant's several fields is told to
say which it means.

Alternatives may bind, where each binds the same names, each name of one
type: whichever matched gives the arm its fields. A field of another name
is renamed to join them. Where an arm's alternatives do not fit on its
line, `wip fmt` puts as many on each line as fit, each line after the first
beginning with `|`:

```wip,run
enum View {
    Text(size: i64, text: String)
    Label(size: i64, text: String)
    Column(size: i64)
    Row(size: i64)
    Over
}

fn sizeOf(view: &View): i64 = match view {
    .Text(size, ..) | .Label(size, ..) | .Column(size) | .Row(size) => size
    .Over => 1
}

fn textOf(view: View): String = match move view {
    .Text(text, ..) | .Label(text, ..) => move text
    _ => String()
}

fn main() = {
    assert(sizeOf(View::Row(size: 3)) == 3 && sizeOf(.Over) == 1)
    assert(textOf(View::Label(size: 1, text: "taken")) == "taken")
    val round = Shape::Round(radius: 4)
    assert(round is .Round(radius: n) | .Square(side: n) && n == 4)
}

enum Shape {
    Round(radius: i64)
    Square(side: i64)
    Empty
}
```

A binding is what it would be in an arm of its own: on a place it aliases
the field that matched, and under `match move` it takes it, the rest of the
value dropped. An `is` test and a `val … else` bind so too, and `|` binds
inside a pattern as at its top: `.Some(.Named(name) | .Tag(name))`.

Text matches a `str`, and a `String` by the text it holds, as `==` compares
them. So a `String` is matched against the words a program takes, and a
value that keeps what it names in a `String` of its own is tested by its
fields. Text has no end of values, so a `match` on it ends with `_`:

```wip,run
enum Problem {
    Unknown(word: String)
    Empty
}

fn run(word: String): i64 = match word {
    "add" => 1
    "remove" => 2
    _ => 0
}

fn main() = {
    assert(run(String::of("remove")) == 2)
    val problem = Problem::Unknown(String::of("frob"))
    assert(problem is .Unknown("frob"))
}
```

## Ranges

A range matches the integers or characters between its ends:
`lo..=hi` takes both, and `lo..hi` leaves `hi` out, as `..` does in a
`for`. Either end may be left off: `..0` is every value below zero, and
`10..` every value from ten. The ends are literals or constants of the type
being matched, and a range with nothing in it, `'9'..='0'` or `5..5`, is
refused:

```wip,run
val LIMIT: i64 = 100

fn class(c: char): str = match c {
    '0'..='9' => "digit"
    'a'..='f' | 'A'..='F' => "hex letter"
    _ => "other"
}

fn size(n: i64): str = match n {
    ..0 => "negative"
    0 => "zero"
    1..LIMIT => "some"
    LIMIT.. => "many"
}

fn main() = {
    assert(class('7') == "digit")
    assert(class('B') == "hex letter")
    assert(class('z') == "other")
    assert(size(-3) == "negative")
    assert(size(99) == "some")
    assert(size(100) == "many")
}
```

## Constants

A name that is a constant — this module's, one it imports, or one the
prelude exports — is the value it stands for, as it is at a range's end,
and not a new binding. It may be an integer, a character,
a `bool` or a `str`, of the type being matched. A local of the same name
hides the constant everywhere else, so a pattern that names it is refused:

```wip,run
val OP_CONSTANT: u8 = 0
val OP_ADD: u8 = 1

fn name(op: u8): str = match op {
    OP_CONSTANT => "constant"
    OP_ADD => "add"
    _ => "unknown"
}

fn main() = {
    assert(name(1) == "add")
    assert(name(9) == "unknown")
}
```

## Arrays and slices

`[a, b]` matches exactly two elements, and `..` stands for any number
between those a pattern names: `[first, ..]` is one or more, `[.., last]`
the same from the end, `[]` none. `..rest` binds the
elements between as a slice of where they lie. An element binds as a
field does: a binding of a place refers to its element, and one of a
value made where it is matched takes it.

```wip,run
fn sum(xs: &[i64]): i64 = match xs {
    [] => 0
    [first, ..rest] => first + sum(rest)
}

fn command(words: &[str]): str = match words {
    ["wip", "run", ..] => "runs"
    ["wip", ..] => "another of wip's"
    _ => "not wip"
}

fn main() = {
    assert(sum([1, 2, 3, 4]) == 10)
    assert(command(["wip", "run", "x"]) == "runs")

    // An array has its one length, so this cannot fail.
    val [a, b] = [3, 4]
    assert(a + b == 7)

    // A `Vec` is matched through the slice it lends.
    val words = Vec::of(own ["a", "b", "c"])
    val [.., last] = words.items() else {
        return
    }
    assert(last == "c")
}
```

A `match` on a slice covers every length: the lengths its arms name, and
those longer than any of them. A `..rest` borrows, so the value must be a
place — a variable, or what a reference refers to — rather than one made
where it is matched. After `..`, a constant's name is a range, as a
constant's name is its value anywhere in a pattern: `[..LIMIT]` is one
element below `LIMIT`.

```wip,error=E0311
fn first(xs: &[i64]): i64 = match xs {
    [] => 0
    [only] => only
}
```

## Guards

`if` after a pattern is a guard: the arm matches where the pattern matches
*and* the guard is true. An arm with a guard covers nothing
on its own, since it may not match — a `match` whose only arm for a variant
is guarded is still missing that variant:

```wip,run
fn size(n: i64): str = match n {
    0 | 1 | 2 => "small"
    n if n < 0 => "negative"
    n if n < 100 => "middling"
    _ => "large"
}

fn main() = {
    assert(size(1) == "small")
    assert(size(-5) == "negative")
    assert(size(50) == "middling")
    assert(size(1000) == "large")
}
```

## Every value is covered

A `match` must cover every value its scrutinee can have. Where one is left
out, the compiler names it — `.Some(.None)`, `(.Green, .Green)` — rather
than saying only that something is missing. An arm that matches nothing the
arms before it leave is reported as unreachable:

```wip,error=E0311
enum Colour {
    Red
    Green
}

fn name(colour: Colour): str = match colour {
    .Red => "red"
}
```

Numbers and characters are covered by interval: the arms of `size` above
cover every `i64`, and `0..=127` with `128..=255` every `u8`, with no `_`.
What is left out is named as a range, "`10..` is not covered", and an arm
inside the ranges above it, `'a'..='f'` after `'a'..='z'`, is unreachable:

```wip,error=E0311
fn sign(n: i64): str = match n {
    ..0 => "negative"
    0..=9 => "small"
}
```

The check is one question asked twice: is this pattern useful against the
ones before it. It is Maranget's algorithm, which is what `rustc` and OCaml
use.

## `val`, and `val … else`

A `val` takes a value apart where the pattern always matches — a struct, a
tuple, a name:

```wip,run
struct Point {
    x: i64
    y: i64
}

fn main() = {
    val here = Point(x: 1, y: 2)
    val Point(x, y) = here
    assert(x + y == 3)

    val Point(x: across, ..) = here
    assert(across == 1)

    val (first, second) = (10, 20)
    assert(first + second == 30)
}
```

Where the pattern can fail, the binding says where to go instead, and that
block must leave — `return`, `break`, `continue`, or a panic:

```wip,run
fn firstBig(values: &[i64]): i64 = {
    val .Some(at) = firstIndexOver(&values, 10) else {
        return -1
    }
    return values[at]
}

fn firstIndexOver(values: &[i64], least: i64): Option<i64> = {
    var i = 0
    while i < values.len() {
        if values[i] > least then return .Some(i)
        i += 1
    }
    return .None
}

fn main() = {
    val numbers = [3, 99, 4]
    assert(firstBig(&numbers) == 99)
    val small = [1, 2]
    assert(firstBig(&small) == -1)
}
```

## `var`

`var` takes a value apart into variables of their own: each name holds its
part, as `var x = …` holds its value, and can be written without writing
anything else. A value made where it is bound gives its
parts up, a place copies plain data out and keeps its own, and a part that
owns memory is taken from a place with `move`, as `var x = move place`
takes it. `else` works as it does for `val`:

```wip,run
struct Point {
    x: i64
    y: i64
}

fn parse(text: str): Option<i64> = if text == "7" then .Some(7) else .None

fn main() = {
    var (count, label) = (0, String::of("seen"))
    count += 1
    label.push(" once")
    assert(count == 1 && label == "seen once")

    val here = Point(x: 1, y: 2)
    var Point(x, y) = here
    x += y
    assert(x == 3 && here.x == 1)

    var .Some(n) = parse("7") else {
        return
    }
    n *= 2
    assert(n == 14)
}
```

Every name of the pattern is a variable; where only some are written, the
others are variables that never change.

```wip,error=E0404
fn main() = {
    val pair = (1, String::of("a"))
    var (n, text) = pair
    n += text.len()
}
```

## What a binding is

A binding from a pattern is the place it matched, not a copy: writing
through it writes into the value that was matched.
A `var`'s are the exception, above: they are variables of their own.
That is what `match` and `for` bindings are for:

```wip,run
enum Counter {
    Counting(n: i64)
    Stopped
}

fn main() = {
    var counter: Counter = .Counting(1)
    match counter {
        .Counting(n) => n += 1
        .Stopped => {}
    }
    assert(counter is .Counting(2))
}
```

A binding of plain data that is only read — never assigned through,
borrowed as `&var` or lent — is a copy instead, so the place it came from
may change while the binding is in scope. A binding that is
written through holds its place until its arm or block ends:

```wip,run
struct Game {
    fruit: Option<i64> = .Some(500)
    score: i64 = 0
}

fn main() = {
    var game = Game()
    if game.fruit is .Some(points) {
        game.fruit = .None
        game.score += points
    }
    assert(game.score == 500 && game.fruit.isNone())
}
```

A `val … else` goes further: a binding whose type is plain data takes a
copy, so the place it came from is free for the rest of the block.
A payload that owns memory is still bound as the place it
is, and writing through a copied binding is refused, since it would change
the copy:

```wip,run
struct Board {
    selected: Option<i64>
}

fn main() = {
    var board = Board(selected: .Some(7))
    val .Some(id) = board.selected else {
        assert(false, "something is selected")
        return
    }
    // The place is free again: `id` is a copy of the number.
    board.selected = .None
    assert(id == 7)
    assert(board.selected.isNone())
}
```

### Two things matched together

A tuple written as a `match`'s scrutinee is not built: each element is
matched where it is, as a `match` on it alone would match it. So two
references are matched through what they refer to, a local is matched in
place, and nothing is copied or given up but what `move` takes:

```wip,run
enum Shape {
    Circle(radius: i64)
    Named(name: String)
}

fn same(a: &Shape, b: &Shape): bool = match (a, b) {
    (.Circle(x), .Circle(y)) => x == y
    (.Named(x), .Named(y)) => x == y
    _ => false
}

fn main() = {
    val one = Shape::Named(String::of("one"))
    assert(same(&one, &one))
    assert(!same(&one, Shape::Circle(radius: 1)))
}
```

The arms are tuple patterns, `(p, q)`, or `_`; a name for the whole —
`pair => …` — has no tuple to bind, and is refused. A tuple held in a
variable is a value, and is matched as one.

### Through a reference

A pattern that tests a value — a variant, a number, a range, text, a
slice, a tuple or a struct taken apart — tests what a reference refers to
wherever it meets one, at any depth, as a `match` on the reference itself
would. A lookup answers an `Option<&T>`, and is matched as the value it
lends:

```wip,run
enum Token {
    Word(text: String)
    Space
}

fn width(token: Option<&Token>): i64 = match token {
    .Some(.Word(text)) => text.len()
    .Some(.Space) => 1
    .None => 0
}

fn main() = {
    val tokens: Vec<Token> = Vec::of(own [
        Token::Word(text: "hello"),
        Token::Space,
    ])
    assert(width(tokens.first()) == 5 && width(tokens.last()) == 1)
    assert("x7".chars().peekable().peek() is .Some('a'..='z'))
}
```

The `match` covers every case of what the reference refers to, so it
needs no `_`. A name at the reference binds the reference itself:
`.Some(token)` gives `token: &Token`. A name below it aliases what it
names there, to read: it cannot be written, `match move` takes nothing
from behind a reference, and it borrows what the reference borrows, so it
is not used after that changes.

## Patterns that must always match

A `val` without `else` and a `for` binding take every value apart, so their
patterns may not test:

```wip,error=E0123
fn main() = {
    val found: Option<i64> = .Some(1)
    val .Some(n) = found
    assert(n == 1)
}
```
