# Cheat sheet

Wip on one page, an example for each part of the language. Each runs, as
every example in this reference does; the pages it links to have the
rules. The words in it are in the [glossary](glossary.md).

## Values and bindings

```wip,run
val answer = 42 // an `i64`: a whole number is one unless something says
var count: u8 = 3 // `var` may be assigned again
count += 1
val ratio = 1.5 // an `f64`
val ok = true
val letter = 'w'
val name = "wip" // a `str`; it becomes a `String` where one is expected
val point = (3, 4) // a tuple
val primes = [2, 3, 5, 7] // an array, `[i64; 4]`
val total: i64 = count // a `u8` widens to a type that holds it
val small = 300 as u8 // `as` may lose something: 44
assert(answer == 42 && count == 4 && primes[1] == 3 && point.1 == 4)
assert(total + count == 8 && small == 44)
assert(ratio > 1.0 && ok && letter != 'x' && name.len() == 3)
assert("\(name) has \(name.len()) letters" == "wip has 3 letters")
```

[Page 1](01-types.md), [page 4](04-expressions.md).

## Functions

```wip,run
fn area(width: i64, height: i64 = 1): i64 = width * height

fn describe(n: i64): str = {
    if n < 0 then return "negative"
    return if n == 0 then "zero" else "positive"
}

fn main() = {
    assert(area(3) == 3 && area(3, height: 4) == 12)
    assert(describe(-2) == "negative")
}
```

A function written `= expression` answers it; one whose body is a block
ends with `return`. [Page 3](03-functions.md)

## Choices

```wip,run
fn classify(c: char): str = when {
    c.isDigit() => "digit"
    c.isLetter() => "letter"
    else => "other"
}

fn main() = {
    val n = 7
    val parity = if n % 2 == 0 then "even" else "odd"
    val size = match n {
        0 => "none"
        1..=9 => "small"
        _ => "large"
    }
    assert(parity == "odd" && size == "small" && classify('7') == "digit")
}
```

`if` with `then` for one expression a branch, braces for blocks; `when`
for the first of several conditions; `match` for the values of one thing.
[Page 4](04-expressions.md)

## Loops

```wip,run
var total = 0
for i in 0..5 {
    total += i // 0, 1, 2, 3, 4
}
for x in [10, 20] {
    total += x
}
var n = 3
while n > 0 {
    n -= 1
}
outer: for i in 0..3 {
    for j in 0..3 {
        if i * j == 2 then break outer
    }
}
assert(total == 40 && n == 0)
```

[Page 4](04-expressions.md#loops)

## Structs and enums

```wip,run
struct Point {
    x: f64
    y: f64

    fn length(): f64 = (self.x * self.x + self.y * self.y).sqrt()
}

enum Shape {
    Circle(radius: f64)
    Rect(width: f64, height: f64)
    Empty
}

fn area(shape: &Shape): f64 = match shape {
    .Circle(radius) => 3.0 * radius * radius
    .Rect(width, height) => width * height
    .Empty => 0.0
}

fn main() = {
    val p = Point(x: 3.0, y: 4.0) // two fields or more are named
    assert(p.length() == 5.0)
    assert(area(.Rect(width: 2.0, height: 3.0)) == 6.0)
    val shape = Shape::Circle(radius: 1.0) // nothing expected: the enum's name
    assert(shape is .Circle(..) && area(&shape) == 3.0)
}
```

[Page 1](01-types.md#structs), [page 5](05-patterns.md)

## Methods

```wip,run
struct Counter {
    count: i64 = 0
}

extend Counter {
    static fn starting(at: i64): Counter = Counter(count: at)

    fn current(): i64 = self.count // reads: `self` is `&Counter`

    var fn bump() = {
        self.count += 1 // writes: `self` is `&var Counter`
    }

    move fn finish(): i64 = self.count // takes the value
}

fn main() = {
    var counter = Counter::starting(5)
    counter.bump()
    assert(counter.current() == 6)
    assert(counter.finish() == 6)
}
```

A field is written by the module that declares its type; `pub` lets others
read it, and `pub var` write it. [Page 3](03-functions.md#methods)

## Ownership

```wip,run
fn shout(text: String): String = "\(text)!"

fn main() = {
    val numbers = [1, 2, 3]
    val copy = numbers // plain data is copied
    assert(numbers[0] == copy[0])

    val text = String::of("hey")
    val loud = shout(text) // a `String` moves: `text` is given away
    assert(loud == "hey!")

    val boxed = own Point(x: 1, y: 2) // a box on the heap
    assert(boxed.x == 1)
}

struct Point {
    x: i64
    y: i64
}
```

A value that owns memory moves where it is used as a value, and ends with
its owner. [Page 6](06-ownership.md)

## References

```wip,run
fn total(values: &[i64]): i64 = {
    var sum = 0
    for value in values {
        sum += value
    }
    return sum
}

fn double(value: &var i64) = {
    value *= 2
}

fn main() = {
    val numbers = [1, 2, 3]
    assert(total(&numbers) == 6) // lent for reading
    var n = 4
    double(&var n) // lent for writing, for the call
    assert(n == 8)
}
```

A `&var` lives for the call it is lent to; a `&` may also be kept in a
view. [Page 7](07-references.md)

## Views, projections, `keeps`

```wip,run
struct Board {
    cells: Vec<i64>

    lend fn at(i: i64): &i64 = lend self.cells[i] // reads or writes
}

view struct Word {
    text: str // a view: it borrows the text
}

struct Words {
    names: Vec<String>

    // What it puts in `into` borrows `self`, and says so.
    fn into(into: &var Vec<Word>) keeps self = {
        for name in self.names {
            into.push(Word(text: name.toStr()))
        }
    }
}

fn main() = {
    var board = Board(cells: [1, 2])
    board.at(0) = 10
    assert(board.at(0) == 10)

    val words = Words(names: ["a", "b"])
    var found: Vec<Word> = Vec()
    words.into(&var found)
    assert(found[1].text == "b")
}
```

[Page 7](07-references.md#projections-lending-a-place-out-of-a-call)

## Errors

```wip,run
fn parse(text: str): Result<i64, str> = {
    val n = text.toInt().mapErr((_) => "not a number")?
    if n < 0 then return .Err("negative")
    return .Ok(n)
}

fn firstWord(text: str): Option<str> = {
    val .Some(word) = text.split(" ").next() else return .None
    return .Some(word)
}

fn main() = {
    assert(parse("12") == .Ok(12) && parse("x") == .Err("not a number"))
    assert(firstWord("a b") == .Some("a"))
    val sure = parse("3").unwrap() // a panic where it holds an error
    assert(sure == 3)
}
```

`?` passes an error up; `panic` and `assert` stop the program, for what
must not happen. [Page 9](09-errors.md)

## Interfaces and generics

```wip,run
interface Shape {
    fn area(): f64
}

@derive(Eq, Text)
struct Square {
    side: f64
}

extend Square: Shape {
    fn area(): f64 = self.side * self.side
}

fn largest<T: Shape>(shapes: &[T]): f64 = {
    var best = 0.0
    for shape in shapes {
        best = best.max(shape.area())
    }
    return best
}

fn areaOf(shape: &dyn Shape): f64 = shape.area()

fn main() = {
    val squares = [Square(side: 2.0), Square(side: 3.0)]
    assert(largest(&squares) == 9.0)
    assert(areaOf(&squares[0]) == 4.0)
    assert(squares[0] != squares[1] && "\(squares[0])" == "Square(side: 2)")
}
```

[Page 8](08-interfaces.md), [page 3](03-functions.md#generic-functions)

## Closures

```wip,run
fn apply(x: i64, f: (x: i64) => i64): i64 = f(x) // code alone
fn applyLent(x: i64, f: &(x: i64) => i64): i64 = f(x) // may capture

fn main() = {
    assert(apply(4, (x) => x * 2) == 8)
    val step = 10
    assert(applyLent(1, (x) => x + step) == 11)
    // An adapter keeps the closure it is lent, as a view of what it read.
    val shifted = [1, 2].copies().map((x) => x + step).toVec()
    assert(shifted == [11, 12])
    // `map` calls its closure one call at a time, so it may write.
    var calls = 0
    val tens = [1, 2].map((x) => {
        calls += 1
        return x * step
    })
    assert(tens == [10, 20])
    // A closure kept in a `val` may write, to its last use.
    val count = () => calls += 1
    count()
    assert(calls == 3)
}
```

[Page 3](03-functions.md#functions-as-values)

## Collections and text

```wip,run
import std::collections::{Map}

fn main() = {
    var list: Vec<i64> = [3, 1, 2]
    list.push(4)
    list.sort()
    assert(list == [1, 2, 3, 4])

    var counts = Map<str, i64>()
    for word in "a b a".split(" ") {
        counts.atOrPut(word, 0) += 1
    }
    assert(counts.at("a") == 2 && counts.get("z").isNone())

    var text = String::of("wip")
    text.push(" lang")
    assert(text.toUppercase() == "WIP LANG" && text.contains("lang"))
    val evens = list.copies().filter((n) => n % 2 == 0).toVec()
    assert(evens == [2, 4])
}
```

[Page 10](10-library.md)

## Modules

```wip,run
import geometry::{Point, origin}

fn main() = {
    var p = origin()
    p.y = 5 // `pub var`: any module writes it
    assert(p.x == 0 && p.y == 5)
}

// file: geometry/geometry.wip
pub struct Point {
    pub x: i64 // `pub`: read anywhere, written here
    pub var y: i64
}

pub fn origin(): Point = Point(x: 0, y: 0)
```

A directory is a module, and `pub` is what it exports. [Page 2](02-modules.md)

## The tools

| Command | Does |
|---|---|
| `wip run main.wip` | builds and runs a program |
| `wip build --release main.wip` | writes an optimised executable |
| `wip test .` | runs the `@test` functions of `*.test.wip` files |
| `wip check main.wip` | checks without building |
| `wip fmt .` | lays the code out |
| `wip doc .` | writes the documentation of what a module exports |
| `wip bindgen header.h` | writes a module that calls a C library |

[Page 12](12-tools.md)
