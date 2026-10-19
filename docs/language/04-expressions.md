# 4. Expressions and statements

## Statements, and where they end

A line break ends a statement, so semicolons are not written. A statement is
a binding, an assignment, or an expression evaluated for what it does.

A line that starts with `.` continues the expression above it, as it
would on one line, so a long chain of calls goes on with a `.` at the
start of each line, or inside parentheses, where a line break ends
nothing. A line that ends with `.` is refused (E0113), with the fix that
moves the `.` to the start of the next: one way to wrap a chain, the one
the eye finds. Where nothing is finished above — first in a
block, after `=>` or `=`, or after a loop — a `.` begins a variant, and
among a `match`'s arms a line that reads as a pattern before `=>` is the
next arm. A variant meant under an expression, as a block's value under a
statement, is written with its type, `Option::None`: written `.None`
there, it is a member of what the line above answers, and refused with
the fix that writes the type (E0126). The formatter writes a chain that
does not fit one call a line, each line starting with its `.`:

```wip,run
val words = ["wip", "is", "short", "and", "plain"]
val lengths = words.copies()
    .filter((word) => word.len() > 2)
    .map((word) => word.len())
    .toVec()
assert(lengths == [3, 5, 3, 5])
```

```wip,error=E0113
val words = ["wip", "is"]
val count = words.copies().
    count()
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

Each branch of an `if` is written one of two ways. **`if condition then
a else b`**, where a branch is one expression — a value, a call, an
assignment, a `match`, or `return`, `break` or `continue`; and **`if
condition { … } else { … }`**, where a branch is a block of statements.
The branches of one `if` may differ, as a `when`'s arms may: `if
xs.isEmpty() then return .None else { … }` answers early beside a body,
and `if cached { … } else 0` puts a short value beside a block. `then`
marks where the condition ends, which is all the braces would, and a block
takes no `then`; `else` takes the rest of the expression, as a lambda's
body does, so `if c then a else b + 1` is `else (b + 1)`. A chain of
`else if` is one `if` with several branches. A long one breaks before each
`else`, a line that ends in `then` going on to the next:

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

fn doubledFirst(xs: &[i64]): Option<i64> = {
    if xs.isEmpty() then return .None else {
        val n = xs[0]
        return .Some(n * 2)
    }
}

fn main() = {
    assert(sign(-3) == -1 && sign(0) == 0)
    assert(describe(2) == "2 items")
    assert(firstOf([4, 5]) == .Some(4) && doubledFirst([4, 5]) == .Some(8))

    val n = 7
    val total = if n > 3 {
        val half = n / 2
        half * 3
    } else 0
    assert(total == 9)
}
```

An `if`'s branches are one type: what is expected of the `if`, or, where
nothing is, the type of the first branch that says one. A branch that
takes the type expected of it — a number, `.None`, a constant — takes the
other's, whichever comes first, as a literal takes the other operand's;
so do a `match`'s arms and an array's elements:

```wip,run
fn main() = {
    val width: i64 = 3
    val height: i64 = 2
    val aspect = if height == 0 then 1.0 else width as f32 / height as f32
    val small: u8 = 7
    val bound = if width > 5 then 0 else small
    val pair = [1.0, aspect]
    val f: f32 = pair[0] + aspect
    val b: u8 = bound
    assert(f == 2.5 && b == 7)
}
```

An array's tuples are taken a position at a time: each position is the
type of the first part there that says one, so a table of tuples types
its literals by the parts beside them, whatever its rows' order. An `if`'s
and a `match`'s tuples are not, since a branch's parts are checked in the
branch's own scope; there, the type is written:

```wip,run
fn main() = {
    val reach: f32 = 12.0
    var count = 0
    for (x, z) in [(0.0, -reach), (-reach, 0.0)] {
        val across: f32 = x + z
        if across == -reach then count += 1
    }
    assert(count == 2)
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

A choice among several conditions is a `when`: each arm is a condition, a
`bool`, and `=>` its body, and the first arm whose condition holds runs —
those after it are not tested. `else =>` is the last arm, where none
held; a `when` that answers a value needs one, as an `if` does. An `is`
in a condition binds for its arm alone. Arms of one expression and of a
block mix, as a `match`'s do, and an arm may be `return`, `break` or
`continue`:

```wip,run
fn kind(c: char): str = when {
    c.isAsciiLetter() || c == '_' => "a name"
    c.isDigit() => "a number"
    c == '"' => {
        val opened = "text"
        opened
    }
    else => "a sign"
}

fn main() = {
    assert(
        kind('x') == "a name" && kind('7') == "a number" && kind('"') == "text",
    )
    val maybe: Option<i64> = .Some(12)
    val size = when {
        maybe is .Some(n) && n > 10 => n
        maybe is .Some(n) => n * 10
        else => 0
    }
    assert(size == 12)
}
```

Two branches are an `if`. A choice among several conditions is a `when`.
A choice among the values of one thing is a `match` on it, and `when x {
… }` is refused with the fix that writes `match x { … }` (E0101): a `when`
takes no value.

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
Between numbers, both sides are of one type, or one is an integer whose
type the other's holds, which widens to it: `byte + total` of a `u8` and
an `i64` adds `i64`s ([page 1](01-types.md)). Two integer types neither
of which holds the other — `i64` and `u64` — and an integer beside a
float are refused, and `as` converts one.

```wip,run
val byte: u8 = 200
val total: i64 = 1000
assert(byte + total == 1200 && byte < total)
```

```wip,error=E0303
val signed: i64 = -1
val unsigned: u64 = 1
val mixed = signed + unsigned
```

`a op= b` works out the place `a` once and is checked as `op` is, so
`+=` past what the type holds panics too; `+%=`, `-%=` and `*%=` wrap, as
`+%` does. A narrower integer `b` widens to `a`'s type:

```wip,run
var hash: u32 = 2166136261
for byte in "wip".items() {
    hash ^= byte
    hash *%= 16777619
}
assert(hash == 0x25F7F447)
```

### The order of an assignment

`place = value` works out what the place's expression computes first — an
index, or what a projection is given — then the value, and finds the place
last, where it is written. So the value may change what the place is in,
and the write goes where the place is then: a list filled in with what is
made next is written as it reads, though making it moves the list.

```wip,run
struct Node {
    value: i64
    next: i64 = -1
}

fn main() = {
    var nodes = Vec()
    nodes.push(Node(value: 1))
    nodes[0].next = {
        nodes.push(Node(value: 2))
        nodes.len() - 1
    }
    assert(nodes[0].next == 1 && nodes[1].value == 2)
}
```

The index was read before the value, so a value that changes it changes
nothing already read; a check that the index is in bounds is made where
the place is found, after the value:

```wip,run
var xs = [0, 0, 0]
var i = 0
xs[i] = {
    i = 2
    7
}
assert(xs[0] == 7 && xs[2] == 0)
```

`a op= b` reads `a` before it works out `b`, so it finds its place first,
and `b` may not change what `a` is in:

```wip,error=E0414
var n = 1
n += {
    n = 10
    1
}
```

A shift's amount may be an integer of any type, and the answer is of the
type shifted. It takes the amount modulo the width of what
it shifts, so a shift of a `u8` by 8 leaves it as it was, and one of a
`u64` by 64 too. C makes a `u8` an `int` before it shifts it,
where the same `x >> 8` is `0`: a port of C shifts in a type wide enough
for the amount. A constant amount at least the width is refused (E0367),
as it is never what was meant, with the fix that widens the value first.

```wip,run
val byte: u8 = 0xF0
val eight = 8
assert(byte >> eight == 0xF0 && byte >> 4 == 0x0F)
assert(((byte as u32) >> 8) as u8 == 0)
val buffer: u64 = 1
val count: u32 = 12
assert(buffer << count == 4096)
```

```wip,error=E0367
val byte: u8 = 0xF0
val gone = byte >> 8
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

A number may say how it is written. `precision:` is how many digits a float
has after its point: its exact value rounded once to the nearest, a tie to
the even digit, as C's `%.3f` writes it, and no point for `0`. `radix:`
writes an integer in a base from 2 to 36, in lower case, after a `-` where
it is negative, as `toInt(radix:)` reads it back, and with `upper: true` its
digits above 9 in upper case. Either goes with a width, and `withPrecision`
and `inRadix` make the same value for a program to keep:

```wip,run
val seconds = 0.0567
assert("\(seconds, precision: 3) s" == "0.057 s")
assert("\(2.5, precision: 0) \(3.5, precision: 0)" == "2 4")
val byte: u8 = 15
assert("\(byte, radix: 16, width: 2, fill: '0')" == "0f")
assert("\(-255, radix: 16)" == "-ff")
assert("\(byte, radix: 16, upper: true)" == "F")
assert(1.5.withPrecision(2).toString() == "1.50")
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
    val failure = Failure::NotFound(String::of("a.txt"))
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

A range is also a value of its own, a `Range<T>` of the integers it counts,
and an iterator, so every walk an iterator has is a range's: `(0..4).any(…)`
asks whether any of the first four counts, without a loop and a flag. It
binds looser than every operator but assignment, so `0..n + 1` counts to
`n + 1`, and a method of a range is called on it in parentheses. Its
`start` is the next it gives, and `end` and `inclusive` say where it stops:

```wip,run
val pressed = [false, true, false, false]
assert((0..4).any((i) => pressed[i]))
assert((1..=10).sum() == 55)
assert((1..=4).map((n) => n * n).toVec() == [1, 4, 9, 16])

val n = 3
val counted = 0..n + 1
assert(counted.count() == 4 && counted.end == 4 && !counted.inclusive)
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
var numbers = Vec::of([1, 2, 3])
for n in numbers {
    n *= 10
}
assert(numbers == [10, 20, 30])
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
    kept.push(word)
}
assert(kept.len() == 2 && kept[0] == "one")
```

What a loop's head makes lives until the loop ends, whatever the loop
walks: a list a call answered, and an iterator over it, or over what a
method of it lends. An iterator kept in a variable has no loop to live
for, and borrowing a temporary there is refused (E0437):

```wip,run
val PRIMES: [i64; 4] = [2, 3, 5, 7]

fn names(): Vec<String> = ["ada", "bob"]

fn main() = {
    var sum = 0
    for (i, p) in PRIMES.forwards().enumerate() {
        sum += i * p
    }
    var letters = 0
    for (i, name) in names().items().forwards().enumerate() {
        letters += i + name.len()
    }
    assert(sum == 34 && letters == 7)
}
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
    assert(squares.take(2).toVec() == [4, 9])

    assert(evens(&xs).count() == 3)
    assert(words("a bb ccc").toVec() == ["a", "bb", "ccc"])
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
    val tree = Tree::Node(children: [
        .Leaf(1),
        .Node(children: [.Leaf(2), .Leaf(3)]),
    ])
    assert(leaves(&tree).toVec() == [1, 2, 3])
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
    var numbers = Vec::of([1, 2, 3])
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
