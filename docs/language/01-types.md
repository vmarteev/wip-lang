# 1. Values and types

Every value has a type, and every type is known where the value is written.
Nothing is converted on its own: a number becomes another kind of number
only where the program says `as`.

## Numbers

The integers are `i8`, `i16`, `i32`, `i64`, `i128` and `isize`, and `u8`,
`u16`, `u32`, `u64`, `u128` and `usize`. The floats are `f32` and `f64`.
None of the names is a keyword.

A literal takes the type expected of it, and `i64` where nothing expects
anything:

```wip,run
val count = 3 // i64, since nothing says otherwise
val small: u8 = 3 // the literal is a u8 here
val ratio: f32 = 1.0 / 60.0
assert(count == 3)
assert(small == 3)
assert(ratio > 0.0)
```

Arithmetic that cannot hold its answer panics. Where wrapping
is what a program wants — a hash, a random number — it says so with `+%`,
`-%` and `*%`:

```wip,run
var state: u64 = 14695981039346656037
state = state *% 1099511628211
assert(state != 0)
```

Where a program asks whether the answer fits, every integer type has
`checkedAdd`, `checkedSub`, `checkedMul`, `checkedDiv`, `checkedRem`,
`checkedNeg` and `checkedShl`, which answer nothing where the operator
would panic — a division by zero too — and `saturatingAdd`,
`saturatingSub` and `saturatingMul`, which answer the least or the
greatest value there is instead:

```wip,run
val count: u8 = 200
assert(count.checkedAdd(100) == .None && count.checkedAdd(50) == .Some(250))
assert(count.saturatingAdd(100) == 255 && (3 as u8).saturatingSub(5) == 0)
assert((7 as i64).checkedDiv(0) == .None)
```

`as` converts between number types, and truncates or rounds as that
conversion does in C:

```wip,run
val n = 300
assert(n as u8 == 44)
assert(3.7 as i64 == 3)
assert(65 as u8 as char == 'A')
```

A float's bits are an unsigned integer of its width, and back:
`x.toBits()` and `f64::fromBits(bits)`, `f32`'s likewise. The float
methods — `sqrt`, `floor`, `round`, `abs`, `min` and the rest — give the
same bits on every machine, and in a `@comptime` constant as when the
program runs:

```wip,run
assert((1.0).toBits() == 0x3FF0000000000000)
assert(f64::fromBits(0x4000000000000000) == 2.0)
assert((-2.5).round() == -3.0 && (2.0).sqrt() == 1.4142135623730951)
```

`x.mulAdd(a, b)` is `x * a + b` rounded once, where the two operations
would round twice: the processor's fused multiply-add.
The compiler never makes one of the other, since the answers differ:

```wip,run
assert((0.1).mulAdd(10.0, -1.0) == 5.551115123125783e-17)
assert(0.1 * 10.0 - 1.0 == 0.0)
```

An integer's bits are counted, turned and reordered by methods every
integer type has, each one instruction of the processor's:

| Method | Answers |
|---|---|
| `x.countOnes()` | how many of its bits are one |
| `x.leadingZeros()`, `x.trailingZeros()` | how many zeros come before its highest one bit, and after its lowest; its width, for 0 |
| `x.rotateLeft(n)`, `x.rotateRight(n)` | its bits turned by `n`, those that leave one end coming in at the other |
| `x.swapBytes()`, `x.reverseBits()` | its bytes, and its bits, in the other order |

An answer is the type of the number, a count too, so that a count is a
shift's amount as it is. A rotation takes its amount modulo the width, as
a shift does, and a signed number's bits are its two's complement:

```wip,run
val set: u64 = 0b1011_0000
assert(
    set.countOnes() == 3 &&
        set.trailingZeros() == 4 &&
        set.leadingZeros() == 56,
)
assert((0x12345678 as u32).swapBytes() == 0x78563412)
assert((0x81 as u8).rotateLeft(1) == 3 && (-1 as i8).countOnes() == 8)
val size: u64 = 1000
assert(1 << (64 - (size - 1).leadingZeros()) == 1024)
```

## `bool` and `char`

`bool` is `true` or `false`, and is what `if` and `while` take. A `char` is
one Unicode scalar value, written in single quotes, and is 32 bits wide:

```wip,run
val letter = 'A'
val emoji = '🙂'
assert(letter.isAsciiLetter())
assert(letter as u32 == 65)
assert(emoji as u32 == 128578)
```

A text's bytes are `u8`s, and `b'x'` is the byte of an ASCII character,
with the escapes a character has. It is a `u8`, and a
character that is not ASCII, being more than one byte, is not one:

```wip,run
val path = "usr/bin"
assert(path[3] == b'/')
assert(path[0] - b'a' + b'A' == b'U')
```

## Text

`str` is a view of bytes that belong to something else: a literal, or part
of a `String`. It knows its length, it is not zero-terminated, and copying
one copies the view rather than the bytes.

`String` owns its bytes and can grow. Text written between `\(` and `)` in a
literal builds one:

```wip,run
val name = "world"
var greeting = String::of("hello, ")
greeting.push(name)
assert(greeting == "hello, world")
assert(greeting.len() == 12)

val built = "\(greeting.len()) bytes"
assert(built == "12 bytes")
```

Text becomes a `String` where one is expected, copied: a literal, as a
number takes the type expected of it, and any other `str` — a parameter, a
piece of a line, what a call answers. That is a field, an argument, what a
function answers, an element pushed, a variable declared or assigned as a
`String`. Where nothing expects a `String`, text is a `str`, which costs
nothing.

```wip,run
struct Dialog {
    title: String
}

fn main() = {
    val dialog = Dialog(" Cannot edit ")
    var names: Vec<String> = Vec()
    names.push("first")
    for word in "second third".split(" ") {
        names.push(word)
    }
    assert(dialog.title == " Cannot edit " && names[0] == "first")
    assert(names[2] == "third")
}
```

The copy owns its bytes, so it outlives the text it was made from. Where
the text is the whole of a `String` the function is done with, the
`String` itself is given, and moved; copying it there is a warning.

Text of several lines is a **text block**: the lines between two `"""`s,
the opening one ending its line and the closing one beginning its own.
The closing `"""`'s indentation is taken off every line,
so the text is indented with the code around it; quotes need no escape,
and `\(…)` works as in a string. The line breaks after the opening and
before the closing are not part of the text, and neither is space at the
end of a line, unless an escape such as `\t` writes it:

```wip,run
val name = "pacman"
val json = """
    {
      "name": "\(name)"
    }
    """
assert(json == "{\n  \"name\": \"pacman\"\n}")
```

A `str` compares, hashes and prints as its bytes, and a `String` compares
with a `str` directly, as above. [Page 10](10-library.md) has what else they
do; `cstring`, which C uses, is on [page 11](11-c.md).

## Arrays and slices

An array holds a fixed number of values, and its length is part of its type:
`[i64; 3]`. A slice, `&[T]`, is a view of some of them, with its length
carried beside the pointer. An array becomes a slice where
one is expected.

```wip,run
fn total(values: &[i64]): i64 = {
    var sum = 0
    for value in values {
        sum += value
    }
    return sum
}

fn main() = {
    val numbers = [3, 1, 4]
    assert(numbers.len() == 3)
    assert(numbers[0] == 3)
    assert(total(&numbers) == 8)
    // Part of an array is a slice too.
    assert(total(&numbers[1..]) == 5)
}
```

An index is an integer of any type, and so are a range's bounds:
what is checked is its value, against the length, so a
`u8` read from one table indexes the next as it is. One too large for an
`i64` is past every length, and is said so with what it was:

```wip,run
val SYMBOLS: [u16; 4] = [257, 258, 259, 260]
val EXTRA: [u8; 3] = [0, 1, 2]

fn main() = {
    val code: u8 = 2
    assert(SYMBOLS[code] == 259 && EXTRA[SYMBOLS[0] - 257] == 0)
    val table = SYMBOLS
    val count: u32 = 2
    assert(table[1..1 + count].len() == 2)
}
```

A slice is a reference, so it is passed to a function rather than kept in a
variable: references are second-class, which is [page 7](07-references.md).

A container whose elements lie one after another — a `Vec`, a `String` as
its bytes, anything that implements `Items` — is lent as them where a call
expects a slice: `total(&values)` is `total(&values.items())`, and `&var`
lends them to be written. A generic `&[T]` finds its `T`
there too:

```wip,run
fn total(values: &[i64]): i64 = {
    var sum = 0
    for value in values {
        sum += value
    }
    return sum
}

fn count<T>(values: &[T]): i64 = values.len()

fn main() = {
    val numbers = Vec::of(own [3, 1, 4])
    assert(total(&numbers) == 8)
    assert(count(String::of("abc")) == 3)
}
```

Arrays and slices compare where their elements do: equal where they are
as long and hold equal elements, and ordered as a dictionary orders words.
An array, a slice and a buffer of one element type
compare with each other, either way round, and a range of elements is an
operand as it stands. They hash where their elements do
too, so a `Vec` of arrays sorts and a `Set` of them can be kept.

```wip,run
val a = [1, 2, 3]
val b = [1, 2, 4]
assert(a != b && a < b)
var pairs = Vec::of(own [[2, 1], [1, 5]])
pairs.sort()
assert(pairs[0] == [1, 5])
val list = Vec::of(own [1, 2, 3, 4])
assert(list.items() == [1, 2, 3, 4] && list.items()[1..3] == [2, 3])
```

A block of values whose size is known only when the program runs is written
`own [value; count]`, and its type is `own<[T]>`:

```wip,run
val count = 4
val zeros = own [0; count]
assert(zeros.len() == 4)
assert(zeros[3] == 0)
```

A list whose length is decided as it is built is written `own [ … ]`
with statements among its elements that `yield`, or `own for`, the list
of what one loop yields; it is an `own<[T]>` too. `yield`
hands a value to the list being built, and `if` and `for` are the
statements they always are, adding what they yield:

```wip,run
fn main() = {
    val loud = true
    val names = own [
        "title",
        if loud { yield "shouted" },
        for i in 0..2 { yield if i == 0 then "first" else "second" },
    ]
    assert(names.len() == 4)
    assert(names[1] == "shouted")
    assert(names[3] == "second")

    val evens = own for i in 0..10 { if i % 2 == 0 { yield i } }
    assert(evens.len() == 5)
}
```

The elements are moved in. Without `own` such a list is refused: an
array's length is part of its type, known where it is written.

`Vec<T>` is the growing one, and is the usual answer; it is on
[page 10](10-library.md). `Vec::of(own [1, 2, 3])` makes a list a vector,
and `v.intoBuffer()` hands a vector's values back as an `own<[T]>`,
neither moving a value.

## Tuples

Two to four values of any types, written `(a, b)`, are a tuple. Its elements
are read by their number, and a tuple pattern takes it apart:

```wip,run
val pair = (2, "two")
assert(pair.0 == 2)
assert(pair.1 == "two")

val (number, name) = pair
assert(number == 2 && name == "two")
```

A tuple is a struct the prelude declares — `Tuple2<A, B>` and its siblings —
so anything true of a struct is true of it.

## Structs

A struct is named fields, and is built as a variant is: its name, with the
fields as a call's arguments, by a call's rules. Where there are two or
more, each is named, a variable of a field's name too: `File(fileNo: fileNo,
name: n)`. One field is given by position or by name. A field is private
to the module unless it says `pub`, and may have a default, which a literal
that leaves it out takes:

```wip,run
struct Ball {
    pub color: i64
    pub var age: i64 = 0
}

fn main() = {
    val fresh = Ball(color: 3)
    assert(fresh.age == 0)

    var older = Ball(color: 3, age: 7)
    older.age += 1
    assert(older.age == 8)

    // `..` takes the rest of the fields from another value.
    val same = Ball(age: 1, ..older)
    assert(same.color == 3 && same.age == 1)
}
```

A field written `var` may be changed through a `var` binding, as `age` is
above; a field without it is set when the value is made and not after.

A default runs where the literal is, each time it leaves the field out,
so it may be code — an empty list, a text — as well as a constant:

```wip,run
struct Queue {
    name: String = String::of("jobs")
    waiting: Vec<i64> = Vec()
}

fn main() = {
    var first = Queue()
    first.waiting.push(1)
    val second = Queue()
    assert(first.waiting.len() == 1 && second.waiting.len() == 0)
}
```

A struct with no fields is its name alone, as a variant with none is: the
declaration ends at its line, and its value is built as any struct's is,
`Csv()`. Such a type is one an interface is implemented for, for its own
sake — a format, a strategy, a marker:

```wip,run
interface Format {
    fn extension(): str
}

pub struct Csv
pub struct Markdown

extend Csv: Format {
    fn extension(): str = "csv"
}

extend Markdown: Format {
    fn extension(): str = "md"
}

fn main() = {
    assert(Csv().extension() == "csv" && Markdown().extension() == "md")
}
```

A `view struct` and an `extern struct` keep their braces: a struct with no
fields borrows nothing, and C has no struct without members.

## Enums

An enum is one of several variants, each of which may carry values. Where
the type is known, a variant is written with a leading dot;
where none is, `.Some` and `.None` are the prelude's `Option`:

```wip,run
enum Shape {
    Point
    Circle(radius: f64)
    Rect(width: f64, height: f64)
}

fn area(shape: &Shape): f64 = match shape {
    .Point => 0.0
    .Circle(radius) => 3.14159 * radius * radius
    .Rect(width, height) => width * height
}

fn main() = {
    assert(area(Shape::Rect(width: 2.0, height: 3.0)) == 6.0)
    assert(area(.Point) == 0.0)

    val shape: Shape = .Circle(radius: 1.0)
    assert(shape is .Circle(..))
}
```

A variant's field may have a default, which the variant takes where it is
built without that field, as a struct's field and a parameter do.
`()` takes every one:

```wip,run
enum Shape {
    Circle(radius: i64 = 1)
    Rect(width: i64 = 80, height: i64)
}

fn width(shape: &Shape): i64 = match shape {
    .Circle(radius) => radius * 2
    .Rect(width, ..) => width
}

fn main() = {
    assert(width(.Circle()) == 2)
    assert(width(.Rect(height: 3)) == 80)
    assert(width(.Rect(width: 5, height: 3)) == 5)
}
```

An enum whose variants carry nothing is numbered in the order they are
written, and answers `count()`, `fromIndex(n)` and `all()`, every variant
as an array:

```wip,run
@derive(Eq)
enum Colour {
    Red
    Green
    Blue
}

fn main() = {
    assert(Colour::count() == 3)
    assert(Colour::fromIndex(1) == .Some(.Green))
    assert(Colour::fromIndex(9).isNone())
    assert(Colour::all() == [Colour::Red, Colour::Green, Colour::Blue])
}
```

Taking an enum apart is [page 5](05-patterns.md).

## Aliases

`type` gives a name to a type, and may take parameters of its own. An alias
is the type it names, not a new one:

```wip,run
type Id = i64
type Pair<T> = (T, T)

fn main() = {
    val id: Id = 7
    assert(id == 7)
    val both: Pair<i64> = (1, 2)
    assert(both.0 + both.1 == 3)
}
```

## Generic types

A struct, an enum or a function may take type parameters, written after its
name. A parameter may be constrained by the interfaces its type must
implement, and by `copy` for a type that is plain data:

```wip,run
struct Box<T> {
    value: T
}

fn largest<T: Ord>(values: &[T]): &T = {
    var best = 0
    var i = 1
    while i < values.len() {
        if values[i] > values[best] then best = i
        i += 1
    }
    lend values[best]
}

fn main() = {
    val boxed = Box("in here")
    assert(boxed.value == "in here")
    val numbers = [3, 9, 4]
    assert(largest(&numbers) == 9)
}
```

A generic function is checked once, and compiled once for each set of type
arguments it is called with. Interfaces and constraints are
[page 8](08-interfaces.md).

A struct's or an enum's last type parameters may have defaults, which a use
that leaves them out gets, and so may an interface's
([page 8](08-interfaces.md)). A default may name the parameters before it,
and where nothing decides a parameter, its default is what it is inferred
to be:

```wip,run
struct Tag<T> {
    number: i64
}

struct Pool<T, K = T> {
    items: Vec<T> = Vec()
}

extend Pool<T, K> {
    var fn add(value: T): Tag<K> keeps value = {
        self.items.push(move value)
        return Tag(self.items.len())
    }
}

struct Names {
    unused: i64
}

fn main() = {
    var numbers: Pool<i64> = Pool() // Pool<i64, i64>
    val first: Tag<i64> = numbers.add(5)
    var words: Pool<String, Names> = Pool()
    val named: Tag<Names> = words.add("one")
    assert(first.number == 1 && named.number == 1)
}
```

A function has no defaults: its type arguments are inferred where it is
called. An `extend` block names every parameter, defaults too.

## `own<T>` and views

`own<T>` is a value on the heap that this value owns, and that ends when it
does. A `view struct` is a struct that may hold a `str` or a
reference, and a `view enum` an enum whose variants may; each is kept only
as long as what it borrows:

```wip
view struct Words {
    text: str
    at: i64
}
```

Both belong to [page 6](06-ownership.md) and [page 7](07-references.md),
which say what may be done with them.

## Nothing

A function that answers nothing answers `void`, which is written `{}` as a
value. It is what a block with no value has, and what `main` may answer:

```wip,run
fn shout(text: str): void = {
    assert(text.len() > 0)
}

fn main() = {
    // A function that answers nothing is called for what it does.
    shout("hi")
    // `{}` is the value itself, where one is asked for.
    val nothing: void = {}
    assert(true)
}
```

`void` has one value, so every `void` is equal to every other, comes before
none, and is written `{}`. What holds one compares where the rest of it
does, and a function that can fail and answers nothing is tested as any
other:

```wip,run
@derive(Eq, Text)
enum Missing {
    Item(index: i64)
}

fn markDone(index: i64): Result<void, Missing> =
    if index < 3 then .Ok({}) else .Err(.Item(index))

fn main() = {
    assert(markDone(2) == .Ok({}))
    assert(markDone(5) == .Err(.Item(5)))
    assert("\(markDone(2))" == ".Ok({})")
}
```
