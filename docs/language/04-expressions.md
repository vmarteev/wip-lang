# 4. Expressions and statements

## Statements, and where they end

A line break ends a statement, so semicolons are not written. A statement is
a binding, an assignment, or an expression evaluated for what it does.

A line that starts with `.` begins a variant, so a long
chain of calls goes on with the `.` at the end of the line, or inside
parentheses, where a line break ends nothing. A chain
written with the `.` starting each line is warned about, with the fix
that moves it:

```wip,run
val words = ["wip", "is", "short", "and", "plain"]
val lengths = words.walk().
    filter(own (word) => word.len() > 2).
    map(own (word) => word.len()).
    toVec()
assert(lengths == Vec::of(own [3, 5, 3, 5]))
```

`val` binds a name to a value; `var` binds one that may be assigned again:

```wip,run
val fixed = 3
var running = 0
running += fixed
running = running * 2
assert(running == 6)
```

A `val` may not be assigned to, and the compiler says so rather than letting
it pass. The type may be written where the value does not say enough:
`val count: u8 = 3`.

## Everything answers something

A block is an expression: its value is its last expression, unless that ends
with a statement, in which case it answers nothing. `if` and `match` are
expressions too, so there is no ternary operator and no separate statement
form:

```wip,run
val n = 7
val name = if n % 2 == 0 then "even" else "odd"
assert(name == "odd")

val size = match n {
    0 => "none"
    1 => "one"
    _ => "many"
}
assert(size == "many")

val computed = {
    val doubled = n * 2
    doubled + 1
}
assert(computed == 15)
```

An `if` is written one of two ways. **`if condition then
a else b`**, where each branch is one expression — a value, a call, an
assignment, or `return`, `break` or `continue`; and **`if condition { … }
else { … }`**, where a branch is a block of statements. `then` marks where
the condition ends, which is all the braces would; `else` takes the rest of
the expression, as a lambda's body does, so `if c then a else b + 1` is
`else (b + 1)`. A chain of `else if` is one `if` with several branches, and
is written one way throughout. A long one breaks before each `else`, a
line that ends in `then` going on to the next:

```wip,run
fn sign(x: i64): i64 = if x < 0 then -1 else if x == 0 then 0 else 1

fn describe(count: i64): String =
    if count == 0 then "nothing"
    else if count == 1 then "one item"
    else "\(count) items"

fn firstOf(xs: &[i64]): Option<i64> = {
    if xs.isEmpty() then return .None
    return .Some(xs[0])
}

fn main() = {
    assert(sign(-3) == -1 && sign(0) == 0)
    assert(describe(2) == "2 items")
    assert(firstOf([4, 5]) == .Some(4))

    val n = 7
    val total = if n > 3 {
        val half = n / 2
        half * 3
    } else {
        0
    }
    assert(total == 9)
}
```

`if c then { … }` is refused, with the fix that drops `then`; so is a
`then` form whose `else` is a block, a braced `if` whose `else` is an
expression without braces, and a chain that changes form (E0101). A
reader never asks whether a `{` after `then` or `else` begins a block or a
value. `wip fmt` writes the `then` form where every branch is one
expression, and braces where any is more, or where a condition is too long
to stay on one line with its `then`.

An `if` without `else` answers nothing, so it stands where a statement
stands: `if done then return`, whose branch's value is discarded.

An arm's value may also be `return`, `break` or `continue`: that case ends
the function or the loop, and the arm's type is `never`, which any other
arm's takes:

```wip,run
fn doubled(text: str): Result<i64, str> = {
    val n = match text.toInt() {
        .Ok(n) => n
        .Err(..) => return .Err("not a number")
    }
    return .Ok(n * 2)
}

fn main() = {
    assert(doubled("21") == .Ok(42))
    assert(doubled("x").isErr())
}
```

## Operators

| Operators | On |
|---|---|
| `+` `-` `*` `/` `%` | numbers, and types that implement `Add` and its siblings |
| `+%` `-%` `*%` | integers, wrapping instead of panicking |
| `==` `!=` | numbers, `bool`, and types that implement `Eq` |
| `<` `<=` `>` `>=` | numbers, and types that implement `Ord` |
| `&&` `\|\|` `!` | `bool`; `&&` and `\|\|` stop as soon as the answer is known |
| `&` `\|` `^` `<<` `>>` | integers |
| `as` | between number types, and between `char` and a number |

Arithmetic panics where the answer will not fit; `/` and `%` by zero panic.
Between numbers, both sides must be the same type: Wip
converts nothing on its own.

`a op= b` works out the place `a` once and is checked as `op` is, so
`+=` past what the type holds panics too; `+%=`, `-%=` and `*%=` wrap, as
`+%` does:

```wip,run
var hash: u32 = 2166136261
for byte in "wip".items() {
    hash ^= byte as u32
    hash *%= 16777619
}
assert(hash == 0x25F7F447)
```

A shift's amount may be an integer of any type, and the answer is of the
type shifted. It takes the amount modulo the width of what
it shifts, so `x >> 8` of a `u8` is `x` itself, and
`x << 64` of a `u64` is `x`. C makes a `u8` an `int` before it shifts it,
where the same `x >> 8` is `0`: a port of C shifts in a type wide enough
for the amount.

```wip,run
val byte: u8 = 0xF0
assert(byte >> 8 == 0xF0 && byte >> 4 == 0x0F)
assert(((byte as u32) >> 8) as u8 == 0)
val buffer: u64 = 1
val count: u32 = 12
assert(buffer << count == 4096)
```

```wip,error=E0303
val mixed = 1 + 2.5
```

A type of a program's own says what it takes on the other side, and what
it answers, by its implementation of the operator's interface: `a op b`
asks the type of `a` for the implementation whose other side is the type
of `b`. A number beside it is what that implementation takes, and a number
on the left is the built-in type's implementation, written in the module of
the type on the right:

```wip,run
struct Vec2 {
    x: f32
    y: f32
}

extend Vec2: Add {
    fn add(other: &Vec2): Vec2 = Vec2(x: self.x + other.x, y: self.y + other.y)
}

extend Vec2: Multiply<f32> {
    fn multiply(other: &f32): Vec2 = Vec2(x: self.x * other, y: self.y * other)
}

extend f32: Multiply<Vec2, Vec2> {
    fn multiply(other: &Vec2): Vec2 = other * self
}

fn main() = {
    val step = Vec2(x: 1.0, y: 2.0)
    var at = step * 0.5 + 2.0 * step
    at *= 2.0
    assert(at.x == 5.0 && at.y == 10.0)
}
```

`a op= b` is `a = a op b`, so it needs the implementation that answers the
type of `a`.

Comparison of a type of a program's own goes through `Eq` and `Ord`, which
`@derive` writes ([page 8](08-interfaces.md)):

```wip,run
@derive(Eq, Ord)
struct Version {
    major: i64
    minor: i64
}

fn main() = {
    val old = Version(major: 1, minor: 2)
    val new = Version(major: 1, minor: 9)
    assert(old != new)
    assert(old < new)
}
```

## Text built where it is written

A literal may hold expressions between `\(` and `)`. What they answer is
appended through `Text`, and the whole is a `String`:

```wip,run
val name = "world"
val count = 2
val greeting = "hello \(name), \(count * 2) times"
assert(greeting == "hello world, 4 times")
```

A piece may say how wide it is, after a comma: `width:` is the fewest
characters it takes, made up with `fill:` — a space unless said — and
`align:` is `.Left`, `.Right` or `.Center`. Unsaid, a number goes right and
anything else left, and a number filled with `0` keeps its sign in front.
A text already as wide is not cut:

```wip,run
val name = "OP_ADD"
val offset = 7
assert("\(name, width: 10)|" == "OP_ADD    |")
assert("\(offset, width: 4)|\(offset, width: 4, fill: '0')" == "   7|0007")
assert("\(-5, width: 4, fill: '0')" == "-005")
assert("[\("mid", width: 7, align: .Center)]" == "[  mid  ]")
```

A type of a program's own is written by implementing `Text`, with one of
two methods. `toString` is the text as a `String` of its
own, which is simplest where a value has one thing to say; `appendTo`
puts the text at the end of a `String` it is lent, which is cheaper where
a value writes a lot or writes parts that are `Text` themselves, as a list
does. Each is written with the other, so a type writes one and has both,
and `x.toString()` is there for every `Text` type, numbers too:

```wip,run
enum Failure {
    NotFound(path: String)
    Denied
}

extend Failure: Text {
    fn toString(): String = match self {
        .NotFound(path) => "no such file: \(path)"
        .Denied => "permission denied"
    }
}

fn main() = {
    val failure = Failure::NotFound(path: String::of("a.txt"))
    assert("error: \(failure)" == "error: no such file: a.txt")
    assert(Failure::Denied.toString() == "permission denied")
    assert(42.toString() == "42")
}
```

## `is`

`value is pattern` answers whether the value matches, and the names the
pattern binds are in scope where the answer is true: the rest of its `&&`
chain, wherever the chain is, and the first branch of an `if` or the body
of a `while` whose condition it is:

```wip,run
val found: Option<i64> = .Some(7)
val doubled = if found is .Some(n) && n > 3 then n * 2 else 0
assert(doubled == 14)
val small = found is .Some(n) && n < 10
assert(small && found is .Some(m) && m == 7)
```

`value !is pattern` is `!(value is pattern)`, written as one operator, as
`!=` is. It binds nothing: a name in its pattern would be
bound only where the value is not the pattern, and is refused (E0333).

```wip,run
val waiting: Option<i64> = .None
assert(waiting !is .Some(_) && waiting !is .Some(1) | .Some(2))
```

A chain is its own, inside `||` or `!`: in `(p && q) || r`, what `p` binds
is seen by `q` and nowhere else. A test that is the last of its chain,
outside a condition, binds a name nothing can use, and is refused (E0333):
`(..)` tests without binding. What a test on a place binds refers into it
until the chain or the block ends, and the place cannot be written before
then.

The pattern is any a `match` arm takes that can fail: a variant, a value,
a range, alternatives, a struct with a field that tests something.
One that matches every value — `_`, a name, a struct
taken apart — would always be true, and is refused:

```wip,run
fn isHex(c: char): bool = c is '0'..='9' | 'a'..='f' | 'A'..='F'

fn main() = {
    assert(isHex('b'))
    assert(!isHex('g'))
    val sign = '-'
    assert(sign is '-' | '+')
}
```

## Loops

`while` takes a condition. `for` walks anything that can be walked — an
array, a slice, a `Vec`, a range, a map, an iterator:

```wip,run
var total = 0
for n in 0..5 {
    total += n
}
assert(total == 10)

val numbers = [3, 1, 4]
var largest = 0
for n in numbers {
    if n > largest then largest = n
}
assert(largest == 4)

var countdown = 3
var steps = 0
while countdown > 0 {
    countdown -= 1
    steps += 1
}
assert(steps == 3)
```

A range `lo..hi` leaves `hi` out, and `lo..=hi` takes it too, in a loop and
in a slice alike. A range through the largest value its
type holds stops there, rather than step past it:

```wip,run
var sum = 0
for i in 1..=4 {
    sum += i
}
assert(sum == 10)

var bytes = 0
for b in 0 as u8..=255 {
    bytes += 1
}
assert(bytes == 256)
```

`break` leaves a loop and `continue` starts its next turn. A loop may be
named, and then both may say which loop they mean:

```wip,run
var found = 0
rows: for r in 0..5 {
    for c in 0..5 {
        if r * c == 6 {
            found = r * 10 + c
            break rows
        }
    }
}
assert(found == 23)
```

A `for` binding is the element itself, so writing through it writes into
what is being walked:

```wip,run
var numbers = Vec::of(own [1, 2, 3])
for n in numbers {
    n *= 10
}
assert(numbers == Vec::of(own [10, 20, 30]))
```

Part of an array or a slice is walked as the whole is: a range of its
elements is lent to the loop for as long as it runs, and written through
where the whole could be.

```wip,run
val numbers = [1, 2, 3, 4, 5]
var middle = 0
for n in numbers[1..4] {
    middle += n
}
assert(middle == 9)

var cells = [0, 0, 0, 0]
for cell in cells[1..3] {
    cell = 7
}
assert(cells[0] == 0 && cells[1] == 7 && cells[2] == 7 && cells[3] == 0)
```

A loop can take the elements instead, and keep them: `for x in move c`
walks `c.intoIterator()`, and each element is the loop's own. A `Vec`, a
`Deque`, a `Set` and a `Map` give theirs up, in their order; what a loop
that stops early leaves is dropped with it. A temporary is the loop's
already, so a list made where the loop is written is taken without
`move`:

```wip,run
var words: Vec<String> = Vec()
words.push("one")
words.push("two")
var kept: Vec<String> = Vec()
for word in move words {
    kept.push(move word)
}
assert(kept.len() == 2 && kept[0] == "one")
```

## Generators

A loop that yields where a value stands is a generator: an `Iterator`
whose loop runs as its values are asked for, and allocates nothing.
`yield` hands a value over and waits to be asked again.
A function that answers `Iterator<T>` is one too, and nothing of its body
runs until the first value is asked for:

```wip,run
fn words(text: str): Iterator<str> = {
    var start = 0
    for i in 0..text.len() {
        if text[i] == 32 {
            yield text[start..i]
            start = i + 1
        }
    }
    yield text[start..]
}

fn evens(xs: &[i64]): Iterator<i64> = for x in xs { if x % 2 == 0 { yield x } }

fn main() = {
    val xs = [1, 2, 3, 4, 5, 6]
    var squares = for x in xs { yield x * x }
    assert(squares.next() == .Some(1))
    assert(squares.take(2).toVec() == Vec::of(own [4, 9]))

    assert(evens(&xs).count() == 3)
    assert(words("a bb ccc").toVec() == Vec::of(own ["a", "bb", "ccc"]))
}
```

`own for`, which collects what a loop yields at once, is this generator
run to its end ([page 1](01-types.md)).

A loop that is a generator borrows what it names, as a view does, so what
it names must not change while it is used; a function's borrows what it
was given by reference. `return` ends a function's, and `break` a loop's.
What it yields are values, not places: what lends its elements where they
lie is `Items` ([page 10](10-library.md)).

A generator keeps its locals inside itself between values, and may be
moved while it waits, so nothing may point into them across a `yield`:
walking one of its own arrays, or a `str` of one of its own `String`s, is
refused. What lies on the heap stays where it is, so walking a `Vec` it
owns is not; nor is what it walks from outside, by reference. One that walks
itself would hold itself, so the inner one goes on the heap:

```wip,run
enum Tree {
    Leaf(value: i64)
    Node(children: own<[Tree]>)
}

fn leaves(tree: &Tree): Iterator<i64> = {
    match tree {
        .Leaf(value) => { yield value }
        .Node(children) => {
            for child in children {
                for value in own leaves(&child) {
                    yield value
                }
            }
        }
    }
}

fn main() = {
    val tree = Tree::Node(children: own [
        .Leaf(1),
        .Node(children: own [.Leaf(2), .Leaf(3)]),
    ])
    assert(leaves(&tree).toVec() == Vec::of(own [1, 2, 3]))
}
```

## Indexing

`x[i]` reads the element at a position, and writes it where the container
allows: arrays, slices, `Vec`, and any type that implements `Sequence`.
An index outside the container panics.

```wip,run
fn total(values: &[i64]): i64 = {
    var sum = 0
    for value in values {
        sum += value
    }
    return sum
}

fn main() = {
    var numbers = Vec::of(own [1, 2, 3])
    assert(numbers[0] == 1)
    numbers[0] = 10
    assert(numbers[0] == 10)

    // Part of an array is a slice, from one position up to another.
    val part = [1, 2, 3, 4]
    assert(total(&part[1..3]) == 5)
}
```

A lookup whose key is not a position — a map's — is `at`, and is
[page 10](10-library.md).
