# 1. Values and types

Every value has a type, and every type is known where the value is written.
Nothing that may change a value is converted on its own: an integer is
taken as a wider integer type that holds every value of it, and any other
conversion of a number is written `as`.

## Numbers

The integers are `i8`, `i16`, `i32`, `i64`, `i128` and `isize`, and `u8`,
`u16`, `u32`, `u64`, `u128` and `usize`. The floats are `f32` and `f64`.
None of the names is a keyword.

A literal takes the type expected of it — the other operand's beside an
operator, the other branch's in an `if` ([page 4](04-expressions.md)) —
and `i64` where nothing expects anything:

```wip,run
val count = 3 // i64, since nothing says otherwise
val small: u8 = 3 // the literal is a u8 here
val ratio: f32 = 1.0 / 60.0
assert(count == 3)
assert(small == 3)
assert(ratio > 0.0)
```

Beside an operator, on either side, so does what takes its type as a
literal does: a literal in parentheses, an `if`, a `match` or a block of
literals. `(if c then 1.0 else -1.0) * x` is of `x`'s type, as `x * (if c
then 1.0 else -1.0)` is.

A variable whose value is such a literal is one until the first place
after it that names it, and takes the type the literal would take there:
beside an operand, an argument, a field, a `return`, a place lent, or a
value assigned to it. So naming a number does not change what it is.
Where that first place says nothing of a type — a method called on it, a
shift's amount, a lambda, an integer that widens to the other operand's —
it is `i64` or `f64`, and a later use that wants another is an error that
says where the first use was:

```wip,run
fn half(x: f32): f32 = x / 2.0

fn main() = {
    val width: f32 = 4.0
    var sum = 0.0
    for x in [width, 1.0] {
        sum += x // an f32: what is added to it says so
    }
    val side = if sum > 0.0 then 1.0 else -1.0
    val reach = side * width // an f32 too
    val size = 3.0
    assert(half(size) == 1.5 && reach == 4.0 && sum == 5.0)
}
```

An integer constant is as the literal it stands for: where an integer of
another type is expected, or beside one in an operator, it is that type,
if its value fits, and an error if it does not. A constant is a top-level
`val`, or an expression of constants and literals with the arithmetic,
bitwise and shift operators — not one `@comptime` code works out, and not
an `as`, which says its type already. Its value cannot change, so nothing
can be lost, and it says nothing a reader needs; where the program
computes a value, `as` still says where it changes type. A constant
where nothing is expected keeps its own type:

```wip,run
val GRAPHICS: i32 = 1
val TRANSFER: i32 = 4

fn main() = {
    val usage: u32 = GRAPHICS | TRANSFER
    val queues: u32 = 5
    assert(usage == 5 && queues & GRAPHICS != 0)
    val plain = GRAPHICS // an i32
    assert(plain as i64 == 1)
}
```

An integer the program computes is taken as a wider integer type where
that type holds every value of it, with no `as`: an unsigned type as a
wider unsigned or signed one, a signed type as a wider signed one. Where
a type is expected — an argument, a field, an element, an assignment, a
`return` — it widens to it; beside a wider operand of an operator it
widens, and the operation is the wider type's; and the branches of an
`if`, the arms of a `match` and the elements of an array are of the type
that holds them all. Nothing widens from a signed type to an unsigned
one, between two types of one width (`i64` and `u64`), from an integer to
a float, or to a `usize` or an `isize` from more than 32 bits, the width
an address may have; each of these is `as`, which marks a conversion that
may lose something.

```wip,run
fn area(width: i64, height: i64): i64 = width * height

fn main() = {
    val side: u8 = 200
    val count: u32 = 70000
    assert(area(side, 3) == 600)
    val total: i64 = count
    assert(total + side == 70200 && side < total)
    val sizes = [side, 1000 as u16] // a [u16; 2]
    assert(sizes[0] == 200)
}
```

An operation is done in its operands' type, and what is expected of its
answer does not reach into it: `val total: i64 = count * 2` of a `u8`
multiplies as `u8`s, and widens the product. Where an operator's answer is
widened so, it is a warning (E0366), whose fix widens an operand, so the
work is done in the wider type; a cast of the answer says the narrow work
is meant.

```wip,run
val count: u8 = 200
val total: i64 = (count as i64) * 2
assert(total == 400)
```

```wip,error=E0301
val n: i64 = -1
val size: u64 = n
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

`as` converts between number types. An integer keeps its low bits where
the type is narrower and its bits where only the sign differs, a float
rounds to the nearest the type holds, and a float made an integer drops
its fraction and stops at the type's least or greatest value, `NaN` at 0:

```wip,run
val n = 300
assert(n as u8 == 44)
assert(-1 as u64 == 18446744073709551615)
assert(3.7 as i64 == 3)
assert(1e20 as i64 == 9223372036854775807)
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
    val numbers: Vec<i64> = [3, 1, 4]
    assert(total(&numbers) == 8)
    assert(count(String::of("abc")) == 3)
}
```

Arrays and slices compare where their elements do: equal where they are
as long and hold equal elements, and ordered as a dictionary orders words.
An array, a slice, a buffer and a `Vec` of one element type
compare with each other, either way round, as their elements, and a range
of elements is an operand as it stands; nothing is built to compare with.
A `Vec<String>` compares with a list of text, each `String` with the text
beside it. They hash where their elements do
too, so a `Vec` of arrays sorts and a `Set` of them can be kept.

```wip,run
val a = [1, 2, 3]
val b = [1, 2, 4]
assert(a != b && a < b)
var pairs: Vec<[i64; 2]> = [[2, 1], [1, 5]]
pairs.sort()
assert(pairs[0] == [1, 5])
val list: Vec<i64> = [1, 2, 3, 4]
assert(list == [1, 2, 3, 4] && list.items()[1..3] == [2, 3])
val names: Vec<String> = ["ann", "bob"]
assert(names == ["ann", "bob"])
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

The elements are moved in. Where nothing expects a `Vec` or an
`own<[T]>`, such a list without `own` is refused: an array's length is
part of its type, known where it is written.

`Vec<T>` is the growing one, and is the usual answer; it is on
[page 10](10-library.md). A list literal takes the type expected of it,
as text does: where a `Vec<T>` is expected — a field, an argument, what a
function answers, a variable declared as one, the branches of an `if` —
it is a vector, its values moved in, and where an `own<[T]>` is expected
it is built on the heap. Each element is checked against the element
type, so text in a `Vec<String>` is a `String`; `[]` is an empty vector,
and a repeat and a list that yields are vectors too. Only the prelude's
`Vec` is made so; any other collection is made by a call. Where nothing
expects one, a list is an array, on the stack, and `Vec::of([1, 2, 3])`
makes it a vector, since `of` expects a buffer. `v.intoBuffer()` hands a
vector's values back as an `own<[T]>`, neither moving a value.

```wip,run
struct Call {
    args: Vec<i64>
    names: Vec<String>
}

fn sum(values: Vec<i64>): i64 = {
    var total = 0
    for value in values {
        total += value
    }
    return total
}

fn main() = {
    val call = Call(args: [1, 2], names: ["a", "b"])
    assert(call.args == [1, 2] && call.names[1] == "b")
    assert(sum([1, 2, 3]) == 6)
    var grown: Vec<i64> = []
    grown.append([4, 5])
    val rows = [10, 20]
    val built: Vec<i64> = [1, for row in rows { yield row }]
    assert(grown == [4, 5] && built == [1, 10, 20])
    val counted = Vec::of([1, 2, 3])
    assert(counted.len() == 3)
}
```

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
so anything true of a struct is true of it, but for one: a pattern takes a
tuple apart by position, and a struct by its fields' names
([page 5](05-patterns.md)).

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

A field may be named by a keyword — `type`, `when`, `in` — as a format or
a C header names it, since where a field's name stands nothing else can:
declared, after `.`, and before the `:` of a literal's value or a
pattern's. A keyword names nothing else, so a pattern binds such a field
under a name of its own:

```wip,run
enum Kind {
    Key
    Mouse
}

struct Event {
    type: Kind
    when: i64
}

fn main() = {
    val event = Event(type: .Mouse, when: 120)
    assert(event.type is .Mouse && event.when == 120)
    val (when: at, ..) = event
    assert(at == 120)
}
```

A field is changed through a `var` binding or a `&var`, as `age` is
above, and by the module that declares the type: there, every field is
read and written. `var` on a field says who else may write it. `pub var`
lets any module write it, where `pub` alone lets them only read it
([page 2](02-modules.md#what-pub-means)); `var` on a private field is
refused (E0124), since nothing outside the module reaches it.

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

`..base` takes each field the literal does not name from `base`, as the
field written out would, `field: base.field`, before any default: plain
data is copied, and a field that owns memory is moved out of `base`,
whose fields the literal named stay its own. `base` may also be a value
that is no place — a call, `..defaults()`, or a copy, `..m.clone()` —
which the literal holds while it takes what it needs, and drops after:

```wip,run
struct Job {
    done: bool
    name: String
    steps: Vec<i64>
}

fn main() = {
    var job = Job(
        done: false,
        name: String::of("parse"),
        steps: Vec::of([1, 2]),
    )
    job.done = true
    // `name` and `steps` are moved out of `job`; `done` was named.
    val retried = Job(done: false, ..job)
    assert(!retried.done && retried.name == "parse" && retried.steps.len() == 2)
    assert(job.done)

    val kept = Job(done: true, name: String::of("link"), steps: Vec())
    val again = Job(done: false, ..Job(steps: Vec::of([3]), ..kept))
    assert(again.steps.len() == 1 && again.name == "link")
}
```

Behind a reference, a field that owns memory cannot be moved out, as
`name: base.name` cannot: the literal is built from a copy, `..base.clone()`,
or from a value the function owns. And building a value writes every
field, so `..` takes only the fields the literal's module may write.

```wip,error=E0405
struct Job {
    done: bool
    name: String
}

fn retried(job: &Job): Job = Job(done: false, ..job)

fn main() = {}
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

A struct is laid out as C lays it out: its fields in the order written,
each at a multiple of its own alignment, the struct as aligned as its most
aligned field and its size a multiple of that. Two annotations say another
layout, for what reads the bytes — a GPU, a file, a protocol — and change
nothing a program computes from the fields. `@align(n)` on a field puts it
at a multiple of `n`, and on a struct gives the struct alignment `n`; `n`
is a power of two up to 4096, and wherever the value lives — a local, a
field, an element of a `Vec`, an `own` — it lies at a multiple of it.
`@packed` puts each field right after the one before, with no padding, and
gives the struct alignment 1:

```wip,run
import std::c

// What a shader reads: a matrix, a direction and an intensity, and a colour
// at 80, where its rules put a three-float vector at a multiple of 16.
struct Uniforms {
    model: [f32; 16]
    @align(16) lightDir: [f32; 3]
    intensity: f32
    @align(16) color: [f32; 4]
}

@packed
struct ChunkHeader {
    kind: u8
    length: u32
}

fn main() = {
    assert(c::offsetOf<Uniforms>("color") == 80 && c::sizeOf<Uniforms>() == 96)

    var header = ChunkHeader(kind: 1, length: 5)
    header.length += 1
    assert(c::sizeOf<ChunkHeader>() == 5 && header.length == 6)
}
```

A field of a packed struct may lie at any address, and a reference is
always aligned, so it is not lent to be written: `&var header.length` is
refused, and the field is copied out, changed and written back. A shared
`&header.length` lends a copy. A packed struct holds plain data — numbers,
`bool`, `char`, pointers and structs of them — since what owns memory or
borrows it is reached by reference. `@packed` and `@align` are not written
on one struct.

```wip,error=E0453
@packed
struct ChunkHeader {
    kind: u8
    length: u32
}

fn grow(length: &var u32) = {
    length += 1
}

fn main() = {
    var header = ChunkHeader(kind: 1, length: 5)
    grow(&var header.length)
}
```

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

A variant is built from another value the same way, `.Rect(width: 5,
..shape)`, where `shape` is known to be that variant: in the arm of a
`match` on it whose pattern is the variant, or in the block of an `is`
test on it. Plain data is copied; where a field that owns memory is
taken, the whole of `shape` is given up, as a `move fn`'s `self` is
here:

```wip,run
enum Thing {
    Lamp(name: String, turn: i64)
    Door(name: String, turn: i64, open: bool)
    Rock
}

extend Thing {
    move fn turned(degrees: i64): Thing = match self {
        .Lamp(turn, ..) => .Lamp(turn: (turn + degrees) % 360, ..self)
        .Door(turn, ..) => .Door(turn: (turn + degrees) % 360, ..self)
        _ => self
    }
}

fn main() = {
    val door = Thing::Door(name: String::of("gate"), turn: 300, open: false)
    assert(door.turned(90) is .Door(name: "gate", turn: 30, open: false))
}
```

Elsewhere the variant is not known, and `..` is refused (E0130):

```wip,error=E0130
enum Thing {
    Lamp(name: String, turn: i64)
    Rock
}

fn turned(thing: Thing): Thing = .Lamp(turn: 90, ..thing)

fn main() = {}
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
        self.items.push(value)
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

A parameter may be a length instead of a type: `val N`, an `i64` known
where the program compiles. It stands wherever an array's length may,
`[T; N]`, and in the code it is a value, the number each use gives. A use
gives it a number, a constant's name or a length parameter of its own, or
leaves it to be inferred from an array, as a type parameter is:

```wip,run
val SIDE = 3

struct Ring<T, val N> {
    items: [T; N]
    at: i64 = 0
}

extend Ring<T: copy, N> {
    fn capacity(): i64 = N
    fn last(): T = self.items[N - 1]
}

fn filled<T: copy, val N>(value: T): [T; N] = [value; N]

fn total<val N>(numbers: [i64; N]): i64 = {
    var sum = 0
    for n in numbers {
        sum += n
    }
    return sum
}

fn main() = {
    val small: Ring<f32, 2> = Ring(items: [0.5, 1.5])
    val square: Ring<u8, SIDE> = Ring(items: filled(7))
    assert(small.capacity() == 2 && small.last() == 1.5)
    assert(square.capacity() == 3 && square.last() == 7)
    assert(total([1, 2, 3, 4]) == 10)
}
```

A length is the number it is given, and a generic is checked once, for
every length: so a type does no arithmetic on one, `[T; N + 1]`, which
would have to be shown equal to another type without its number. The code
does, since there `N` is a value. A number is a length's argument in a
type; among an expression's type arguments a length is given by a
constant's name, since `f(a < 3, b > (c))` compares. An interface takes
types alone. An array has methods of its own in the prelude, `extend [T;
N]`, which may answer an array of the same length, and a module may
implement its own interface for every array so:

```wip,error=E0369
fn widened<val N>(numbers: [i64; N]): [i64; N + 1] = [0; N + 1]
```

An empty container, or a `.None`, says nothing of what it will hold. A
`val` or a `var` whose value leaves type arguments open takes them from
the first place after it that names it: a method called on it, whose
arguments say them as a generic function's arguments say its own; the type
expected there, of an argument, a field or a `return`; or a value assigned
to it. A parameter that place leaves open takes its default. The type is
then what it would be had it been written, and the language server shows
it. A number literal's variable waits the same way, for its type
([Numbers](#numbers)):

```wip,run
import std::collections::{Map}

struct Hit {
    at: f64
}

fn main() = {
    var small = Vec()
    var large = Vec()
    for n in 0..5 {
        if n < 2 then small.push(n) else large.push(n * 10) // Vec<i64>
    }
    var counts = Map()
    counts.put("one", 1) // Map<str, i64>
    var nearest = .None
    nearest = .Some(Hit(at: 2.5)) // Option<Hit>
    assert(small.len() == 2 && large.len() == 3 && counts.len() == 1)
    assert(nearest.isSome())
}
```

The first place must say them. A test, a walk, or a method whose arguments
say nothing of them — `len()`, `isEmpty()` — is an error there, and the type
is written where the variable is declared, which is also where a reader
looks for it when the next line does not say it:

```wip,error=E0315
fn main() = {
    var sizes = Vec()
    if sizes.isEmpty() { // nothing says what it holds
        sizes.push(1)
    }
}
```

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
