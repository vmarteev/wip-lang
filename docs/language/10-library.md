# 10. The standard library

The library is in two halves: the **prelude**, which every file sees without
an import, and the modules under `std`, which are imported. The prelude's
names are reserved, so `Option` means the same type in every program.
The prelude and the collections are on this page, and the rest of `std`
on three more:

- [10.1. Threads](10-library-threads.md): `std::future`, and work that
  borrows — `both`, `each`, `map` — and `std::sync`.
- [10.2. The system](10-library-system.md): arguments, logs, time,
  random numbers, other programs, files and directories, watching them,
  files embedded, and what the target is.
- [10.3. Connections and formats](10-library-formats.md): `std::net`,
  HTTP and TLS, compression, JSON, bytes, base64, hex and URLs, HTML and
  XML, regular expressions, secrets and digests.

## `Option` and `Result`

```wip,run
fn main() = {
    val found: Option<i64> = 7
    assert(found.isSome())
    assert(found.unwrapOr(0) == 7)
    assert(found.map((n) => n * 2) == 14)
    assert(found.fold(() => 0, (n) => n * 2) == 14)

    val missing: Option<i64> = .None
    assert(missing.unwrapOr(0) == 0)
    assert(missing.or(1) == 1)
    // Made only where it is needed.
    assert(missing.orElse(() => 2) == 2)
    assert(missing.unwrapOrElse(() => 3) == 3)
    assert(missing.toResult("nothing there").isErr())

    val answer: Result<i64, str> = 2
    assert(answer.map((n) => n + 1) == 3)
    assert(answer.toOption() == 2)

    val failed: Result<i64, str> = .Err("no")
    assert(failed.mapErr((e) => e.len()) == .Err(2))
    assert(failed.unwrapOr(9) == 9)
    assert(failed.unwrapOrElse((e) => e.len()) == 2)
    assert(failed.orElse((e) => e.len()) == 2)
}
```

`unwrapOr` and `or` are given their alternative as a value, which is
evaluated whether or not it is used; `unwrapOrElse` and `orElse` are given a
function that makes it, called only where it is needed — on a `Result`,
with the error. Both also carry `flatMap`, `fold`, `filter`
(on `Option`), `any`, `all` and `forEach`. `fold` takes a function for each
case, and calls only the one that applies: `found.fold(() => 0, (n) => n *
2)`. Each is given its function for the call, which may change what it
captures. `unwrap` panics where there is nothing to answer, at the line
that called it, so it is for what cannot happen.

`asRef()` lends an option's value where it lies, as an `Option<&T>`, which
`map` makes into what it looks at: an `Option<String>` read as an
`Option<str>` is `host.asRef().map((name) => name.toStr())`, and borrows
the option until it is no longer used.

An `Option` may be filled where it is empty and written where it is not:

```wip,run
fn main() = {
    var slot: Option<i64> = .None
    slot.getOrInsert(7) += 1
    assert(slot == 8)
}
```

## `Vec<T>`

A growing buffer, written in Wip over one primitive the compiler knows.
`Vec()` is an empty one; a list where a `Vec` is expected is one, its
values moved in, and `Vec::of([ … ])` makes one where nothing is expected,
taking the values a list was built with; `Vec::filled(count, value)`
holds clones of a value. A `Vec` compares with a list as its elements:

```wip,run
fn main() = {
    var numbers = Vec()
    numbers.push(1)
    numbers.push(2)
    assert(numbers.len() == 2 && numbers[0] == 1)
    assert(numbers.pop() == 2)

    var made = Vec::of([3, 1, 2])
    made.sort()
    assert(made == [1, 2, 3])
    assert(made[1..] == [2, 3])
    made.insert(0, 0)
    assert(made.remove(0) == 0)
    assert(made.swapRemove(0) == 1)
    made.retain((n) => n > 2)
    assert(made == [3])
    var listed: Vec<i64> = [4, 5]
    listed = []
    assert(listed.isEmpty())

    val filled: Vec<i64> = Vec::filled(3, 7)
    assert(filled.len() == 3 && filled[2] == 7)
    val names = Vec::filled(2, String::of("x"))
    assert(names[1] == "x")
    assert(Vec<i64>::withCapacity(10).isEmpty())
}
```

`at(i)` is the lending pair behind `v[i]`, and `get(i)` the element or
nothing where `v[i]` would panic; `items()` lends the whole as a slice, and
`v[a..b]` is a range of that slice, as an array's is. A container whose
`x[i]` is a position and which lends its elements as one slice — a
`Sequence` and an `Items` — is ranged so: a `Set` by the order its keys
went in, and a type of a program's own. `truncate`, `clear` and `reserve`
are there, and a `Vec` ends what it holds when it ends.

`last()` answers the last value, or nothing; `atLast()` lends it where it
lies, to read or to write, as the top of a stack is, and an empty vector
panics. `takeAll()` hands over everything a vector holds and leaves it
empty, as `Option`'s `take()` leaves nothing — what a field's values are
taken out for while `self` changes — and a `String`, a `Map`, a `Set` and
a `Deque` take theirs the same way:

```wip,run
fn main() = {
    var frames = Vec::of([(1, 10), (2, 20)])
    frames.atLast().1 += 5
    assert(frames[1] == (2, 25) && frames.atLast().0 == 2)
    val all = frames.takeAll()
    assert(all.len() == 2 && frames.isEmpty())
}
```

Values that own memory are moved many at once: `splitOff(at)` answers the
values from `at` on, in a vector of their own, and keeps those before it,
and `append(values)` moves every value of another vector to the end, in
order, leaving nothing behind it. Nothing is copied or cloned, and a
`splitOff` past the end panics:

```wip,run
fn main() = {
    var words: Vec<String> = ["a", "b", "c"]
    var rest = words.splitOff(1)
    assert(words.len() == 1 && rest[0] == "b")
    rest.append(["d"])
    words.append(rest)
    assert(words.len() == 4 && words[3] == "d")
}
```

Elements that are plain data are copied many at once, in one copy of the
memory: `pushAll(items)` puts them at the end of a `Vec`,
and a slice's `copyFrom(source)` makes each element the one at its index
in `source`, which must be as long — a range is lent as the receiver,
where it is written:

```wip,run
fn main() = {
    var window = own [0 as u8; 6]
    val block: [u8; 3] = [7, 8, 9]
    window[2..5].copyFrom(&block)
    assert(window[1] == 0 && window[2] == 7 && window[4] == 9)
    var list = Vec()
    list.pushAll(&block)
    list.pushAll(&window[2..4])
    assert(list.len() == 5 && list[4] == 8)
}
```

`copyWithin(from:, to:, count:)` copies elements from one part of a slice
or a `Vec` to another, in one copy too. Where the two
parts overlap, what is read is what was there before the copy began, as
C's `memmove` and Rust's `copy_within` copy:

```wip,run
fn main() = {
    var line = own [1, 2, 3, 4, 5]
    line.copyWithin(from: 0, to: 1, count: 4)
    assert(line[1] == 1 && line[4] == 4)
}
```

A slice and a `Vec` make new values from their elements, as Scala's
collections do, and at once, where an iterator's adapters wait to be
walked. The function each is given is lent for the call, one call at a
time, so it may change what it captures, `&var (…) => R`; a closure that
reads goes there too:

```wip,run
fn main() = {
    val numbers = [3, 1, 4, 1, 5, 9]
    assert(numbers.map((n) => n * 2) == [6, 2, 8, 2, 10, 18])
    assert(numbers.filter((n) => n > 3) == [4, 5, 9])
    assert(numbers.fold(0, (sum, n) => sum + n) == 23 && numbers.sum() == 23)
    assert(numbers.find((n) => n > 3) == .Some(&4))
    assert(numbers.first() == .Some(&3) && numbers.max() == 9)
    assert(numbers.take(2).len() == 2 && numbers.skip(4).len() == 2)
    assert(numbers.join(" ") == "3 1 4 1 5 9")
    val words = ["pear", "fig", "plum"]
    assert(words.minBy((w) => w.len()) == .Some(&"fig"))
}
```

`map`, `flatMap`, `fold`, `indexWhere` and `lastIndexWhere`, `take` and
`skip` work on any elements. An array has these methods too, and its `map`
answers an array of its length, allocating nothing: `[1, 2, 3].map((n) =>
n * 10)` is an `[i64; 3]`; a part of it, `xs[0..2]`, maps to a vector, as a
slice does. A slice's methods answer by what they do: what picks an element
where it lies lends it — `first`, `last`, `find`, `minBy` and `maxBy`, the
element whose key is least or greatest, the first of equals, and `take` and
`skip`, a part of the slice — for any elements; what works out a value
answers it — `min` and `max` a clone, `sum` and `product` of plain data,
`count`, `countWhere`. `find` answers what it finds, and a position is
`indexOf`, on a slice, text, a `Map` and a `Set` alike. `sum` and `product`
add up and multiply elements that implement `Add` and `Zero`, or `Multiply`
and `One`, as every number does: nothing sums to zero and multiplies to
one. `filter`, `partition`, `min` and `max` clone the elements they answer,
as `toVec` does, so they are there for any element that clones: plain data
is copied, and a value that owns memory cloned; a vector that would clone
none keeps what it wants in place with `retain`. `join` writes each element
as text, and `joinTo(&var out, separator)` writes them onto a `String`
there is; every sequence has `joinToBy`, which writes each element as a
closure says — a `Deque`, a map's values, a slice of pairs.

A list is made from what is walked without a loop: an
iterator's `toVec()` collects what it hands out, and a slice's clones its
elements — a part of a vector, a set's keys, an array:

```wip,run
fn main() = {
    val parts = "usr/local/bin".split("/").toVec()
    assert(parts.len() == 3 && parts[1] == "local")
    val names = parts.map((part) => String::of(part))
    val rest = names.skip(1).toVec()
    assert(rest.len() == 2 && rest[0] == "local")
}
```

### `InlineVec<T, N>`

`InlineVec<T, N>` holds at most `N` values where it is — in a local, a
field, an element — and never allocates: a draw's constants, a buffer of a
few kilobytes before it is sent, the return addresses of a trace. It has a
`Vec`'s methods but those that would grow it: `push` panics where it is
full, and `tryPush` answers `false`; `capacity()` is `N`, and `isFull()`
says whether it is reached. It lends its values as a slice, so what is
written for `&[T]` works on it, and `for x in v` walks it. It holds what
owns memory as well as plain data, and ends what it holds where it ends;
where its element is plain data, so is it, and it is copied with what
holds it. `InlineVec::of(items)` and `pushAll` clone what they are given:

```wip,run
struct Draw {
    constants: InlineVec<u8, 16>
    count: i64
}

fn main() = {
    val draw = Draw(constants: InlineVec::of([1, 2, 3]), count: 3)
    val again = draw // plain data: copied
    assert(again.constants.len() == 3 && draw.constants[2] == 3)
    var names: InlineVec<String, 2> = InlineVec()
    names.push("one")
    assert(names.tryPush("two"))
    assert(!names.tryPush("three") && names.isFull())
    assert(names.items() == ["one", "two"])
}
```

What is read is read into one where it lies, for plain data:
`readInto(most, blank, read)` gives each of up to `most` slots past what
it holds the value `blank`, lends them to `read`, and holds as many as
`read` answers it filled, or nothing more where it fails:

```wip,run
fn main() = {
    var buffer: InlineVec<u8, 16> = InlineVec()
    val got: Result<i64, str> = buffer.readInto(16, 0, (room) => {
        room[0] = b'o'
        room[1] = b'k'
        return 2
    })
    assert(got == 2 && str::fromBytes(&buffer.items()) == "ok")
}
```

## `String` and `str`

```wip,run
fn main() = {
    var built = String()
    built.push("hello")
    built.pushChar(',')
    built.push(" world")
    built.pushInt(42)
    assert(built.len() == 14)
    assert(built.startsWith("hello"))

    val text = "  one,two  "
    assert(text.trim() == "one,two")
    assert(text.trim().split(",").count() == 2)
    assert(text.contains("two"))
    assert(text.indexOf("two") == 6)
    assert("42".toInt() == 42)
    assert("4.5".toFloat() == 4.5)
    assert("\(0.1 + 0.2)" == "0.30000000000000004")
    assert("\(1e20)" == "1e+20")
    assert("añ".chars().count() == 2)
}
```

Text is searched from either end — `indexOf` and `lastIndexOf`, the
position, as a slice's and a map's are — and compared in
any case without making anything, `equalsAnyCase` and `startsWithAnyCase`;
`toAsciiUpper()` and `toAsciiLower()` change the case of its ASCII letters
alone, as a protocol or a format that is ASCII asks; a `String` takes its
last character back with `popChar`, as a backspace does:

```wip,run
fn main() = {
    val path = "a/b/c.tar.gz"
    assert(path.indexOf(".") == 5 && path.lastIndexOf(".") == 9)
    assert("Content-Length".equalsAnyCase("content-length"))
    assert("Content-Type: text/plain".startsWithAnyCase("content-type"))
    assert("Grüße".toAsciiUpper() == "GRüßE")
    var typed = String::of("añ")
    assert(typed.popChar() == 'ñ' && typed == "a")
}
```

`text[i]` is a byte, a `u8`, and panics past the end; `get(i)` is that
byte or nothing, by value as `text[i]` is, so it compares with a byte:
`line.get(at) == b'/'` is false at the end, where nothing is. Its
opposite is not: `line.get(at) != b' '` is true at the end too, since
nothing is not a space, so a walk to a byte tests what it found:

```wip,run
fn main() = {
    val line = "  \tword!"
    var at = 0
    while line.get(at) is .Some(b' ' | b'\t') do
        at += 1

    assert(at == 3 && line.get(at) == b'w')
    while line.get(at) is .Some(byte) && byte != b'!' do
        at += 1

    assert(line.get(at) == b'!' && line.get(at + 1).isNone())
}
```

`String::of(text)` copies a `str` into a new `String`, which text where a
`String` is expected does without the call; `toStr()` lends the
bytes back as a `str`; `toCstring()` makes the zero-terminated kind C wants
([page 11](11-c.md)).

A `String` is lent as a `str` wherever its text is read for a call: an
argument where a `str` or a `&str` is expected, the receiver of a method
`str` has, a slice of it, and a side of a comparison with text. Nowhere
else: a `str` kept in a variable or a field borrows the `String`, and is
written `toStr()`, which says where the bytes are looked at:

```wip,run
val KNOWN = ["open", "edit"]

fn width(text: str): i64 = text.len()

fn main() = {
    val name = String::of("edit")
    assert(width(name) == 4)
    assert(KNOWN.contains(name))
    assert(name.startsWith("ed") && name == "edit")
    val kept: str = name.toStr()
    assert(kept.len() == 4)
}
```

The other way round, text is lent as a `String` where a `&String` is taken:
a `String` made over the text's own bytes, which allocates nothing and
copies nothing, and which nothing can grow or free behind a `&`. So a map
keyed by `String` is looked up by a literal or a `str`, without a copy of
the key for every lookup. Where a `String` is taken by value, to be kept,
the text is copied into one; where a `&var String` is taken, it is
neither, since what the callee wrote would be lost.

```wip,run
import std::collections::{Map}

fn main() = {
    var counts: Map<String, i64> = Map()
    for word in "the cat and the hat".split(" ") do
        counts.atOrPut(word, 0) += 1

    assert(counts.at("the") == 2 && !counts.contains("dog"))
}
```

`stripPrefix` and `stripSuffix` answer the text with a prefix or a suffix
taken off, and `splitOnce` the text either side of its first separator —
or nothing, where it is not there — and `rsplitOnce` of its last.
`trimMatches`, `trimStartMatches` and `trimEndMatches` take a pattern off
as many times as it is there; `rsplit` answers the pieces from the last,
`splitn(count, separator)` at most so many, the last the rest, and
`splitWhitespace()` the words between runs of white space. `toU64`,
`toI128` and `toU128` read the numbers `toInt` is too narrow for, with the
same `radix:`:

```wip,run
fn main() = {
    val line = "#define WIDTH 32"
    assert(line.stripPrefix("#define ") == "WIDTH 32")
    assert(line.stripPrefix("#undef ").isNone())
    assert("shapes.h".stripSuffix(".h") == "shapes")
    val .Some((name, value)) = "WIDTH 32".splitOnce(" ") else return
    assert(name == "WIDTH" && value == "32")
    assert("==title==".trimMatches("=") == "title")
    assert("k=v=w".splitn(2, "=").toVec() == ["k", "v=w"])
    assert(" a  b ".splitWhitespace().count() == 2)
    assert("18446744073709551615".toU64().isOk())
}
```

`replace(pattern, by:)` makes a new `String` with each occurrence of the
pattern replaced, left to right and none overlapping another, or the first
so many where `count: .Some(n)` says; an empty pattern panics, since
nothing says where it would be. `repeat(times)` makes the text that many
times over:

```wip,run
fn main() = {
    assert("a-b-c".replace("-", by: "+") == "a+b+c")
    assert("aaa".replace("aa", by: "b") == "ba")
    assert("a-b-c".replace("-", by: "+", count: 1) == "a+b-c")
    assert("ab".repeat(3) == "ababab" && "ab".repeat(0) == "")
}
```

A float is written as C's `%g` writes it, with as many digits as it
takes to read back as the same value, and six at the least, a `NaN` as
`nan` whatever its sign. `toFloat()` answers the nearest `f64` to the
text, the one whose significand is even where the text is halfway between
two, `TooLarge` past the largest, and zero below half the least. The
library works out both itself, from the float's bits and the text's
digits, so each is the same on every system, and in a `@comptime`
constant as when the program runs.

```wip,run
@comptime
val WRITTEN: i64 = "\(0.1 + 0.2)".len()

/// 2^53 + 1, halfway between 2^53 and the float above it.
@comptime
val HALFWAY: f64 = "9007199254740993".toFloat().unwrapOr(0.0)

fn main() = {
    assert(WRITTEN == 19 && "\(0.1 + 0.2)" == "0.30000000000000004")
    assert("\(1e20)" == "1e+20" && "\(5e-324)" == "5e-324" && "\(-0.0)" == "-0")
    assert(HALFWAY == 9007199254740992.0 && "1e-400".toFloat() == 0.0)
}
```

A text's length is in bytes, and its characters are counted by `chars()`;
what it takes on a terminal is a third number. `width()` answers it, for a
`char` or a `str`: 0 for a combining mark, a format character or a
control, 2 for a wide character — most of Chinese, Japanese and Korean,
and emoji — and 1 for the rest. A cluster of several
characters drawn as one, as a flag is, counts as its characters do.

```wip,run
fn main() = {
    assert("héllo".width() == 5)
    assert("中文".width() == 4)
    assert("中文".len() == 6)
    assert("e\u{301}".width() == 1)
    assert('😀'.width() == 2)
}
```

A character's case is Unicode's, one character to one: `toLowercase()`
and `toUppercase()`, for a `char` or a `str`, and `isUppercase()` and
`isLowercase()` for a `char`. A character whose other case is several —
`ß` in upper case is `SS` — keeps itself. `folded()` is for comparing
without case: two texts that differ only in case fold alike, as `K`, `k`
and the Kelvin sign do, and `Σ`, `σ` and `ς`.

```wip,run
fn main() = {
    assert("Привет".toLowercase() == "привет")
    assert('ß'.toUppercase() == 'ß')
    assert("ΣΟΦΟΣ".folded() == "σοφος".folded())
    assert("Straße".folded() != "STRASSE".folded(), "one character to one")
    assert('Ж'.isUppercase() && !'ж'.isUppercase())
}
```

What a character is, is Unicode's too: `isLetter()` for a letter of any
script, `isNumeric()` for a number of any script — a digit, `Ⅻ`, `½` —
and `isAlphanumeric()` for either. `isDigit()` is `0` to `9` alone, which
is what a program reading a number asks, and `isAsciiLetter()` `a` to `z`
and `A` to `Z`; `isAsciiAlphanumeric()`, `isAsciiUppercase()`,
`isAsciiLowercase()` and `isHexDigit()` are what a lexer asks of ASCII.
`digitValue(radix)` is what a character is worth as a digit, `0` to `9`
and then `a` to `z`, or nothing.

```wip,run
fn main() = {
    assert('é'.isLetter() && '中'.isLetter() && !'_'.isLetter())
    assert('٣'.isNumeric() && !'٣'.isDigit())
    assert("année2".chars().all((c) => c.isAlphanumeric()))
    assert('f'.digitValue(16) == 15 && 'g'.digitValue(16) == .None)
}
```

A byte is asked the same of what it is as an ASCII character, for a
program that reads text a byte at a time: `isDigit()`, `isHexDigit()`,
`isAsciiLetter()`, `isAsciiAlphanumeric()`, `isAsciiUppercase()`,
`isAsciiLowercase()`, and C's `isspace`, `ispunct` and `isprint` as
`isAsciiWhitespace()`, `isAsciiPunctuation()` and `isAsciiPrintable()`;
`digitValue(radix)`, `toAsciiUpper()` and `toAsciiLower()` as a
character's. A byte past 127 is none of them: it is part of a character
UTF-8 writes in several.

```wip,run
fn main() = {
    val line = "x = 0x1F;"
    assert(line[0].isAsciiLetter() && line[1].isAsciiWhitespace())
    assert(line[2].isAsciiPunctuation() && line[6].isHexDigit())
    assert(line[7].digitValue(16) == 15)
    assert(!"é"[0].isAsciiPrintable())
}
```

### Reading text from the front: `std::text::Scanner`

A parser reads its text through a `Scanner`: the text and `at`, the byte
offset of what is read next. `peek()` is the byte there, or nothing at the
end, and `peekAt(n)` one further on; `next()` takes it; `take(byte)` and
`takeText(text)` take what comes next only where it is that, and say
whether they did, and `takeTextAnyCase(text)` where it is that in any case,
as `startsWithAnyCase` compares. `skipWhile(test)` goes past the bytes a
test holds for, and `takeWhile(test)` answers them; `since(start)` is the
text from a position to here, the token just read, and `rest()` what is
left. `peekChar()` and `nextChar()` read a character, where a format lets
one be anything, and `peekCharAt(n)`, `takeChar(c)`, `skipWhileChar(test)`
and `takeWhileChar(test)` are the byte operations' twins in characters, for
a name or a text that may hold any. `peekCharAt(n)` counts characters, and
reads the ones before it to find it. The byte operations read a syntax of
ASCII faster, since they decode nothing. What these answer borrows the
text, not the scanner, so a token is kept while reading goes on, and `at`
set back to where it has been reads again from there.

```wip,run
import std::text::{Scanner}

fn main() = {
    var s = Scanner::of("añob → x")
    val name = s.takeWhileChar((c) => c.isLetter())
    s.skipWhileChar((c) => c == ' ')
    assert(name == "añob" && s.peekCharAt(2) == 'x' && s.takeChar('→'))
}
```

`position()` is the line and the column of what is read next, each from 1,
the column in characters, and `positionAt(at)` those of any byte: they are
worked out from the start of the text when an error asks for them, and
reading counts nothing.

```wip,run
import std::text::{Position, Scanner}

/// The name and the number of a line `name = 640`, or where it is wrong.
fn setting(line: str): Result<(str, i64), Position> = {
    var s = Scanner::of(line)
    val name = s.takeWhile((b) => b.isAsciiLetter())
    s.skipWhile((b) => b == b' ')
    if name.isEmpty() || !s.take(b'=') then
        return .Err(s.position())

    s.skipWhile((b) => b == b' ')
    val start = s.at
    s.skipWhile((b) => b.isDigit())
    val .Ok(value) = s.since(start).toInt() else {
        return .Err(s.positionAt(start))
    }
    return (name, value)
}

fn main() = {
    assert(setting("width = 640") == .Ok(("width", 640)))
    val .Err(place) = setting("width : 640") else return
    assert(place.line == 1 && place.column == 7)
}
```

## Walking: `Items`, `Iterator` and the adapters

A container that can be walked as a slice implements `Items<T>`, which is
what `for x in container` uses. Values that come one at a time implement
`Iterator<T>`, whose adapters and consumers are in the prelude:

```wip,run
fn main() = {
    val text = "the quick brown fox"

    // Adapters answer another iterator, and keep the closure they are lent:
    // a view of what it captured.
    val long = text.split(" ").filter((w) => w.len() > 3).toVec()
    assert(long == ["quick", "brown"])

    var pairs = 0
    for (i, word) in text.split(" ").enumerate() do
        pairs += i + word.len()

    assert(pairs == 22)

    // Consumers walk it to the end, and are lent a closure that may write.
    assert(text.split(" ").count() == 4)
    assert(text.split(" ").any((w) => w == "fox"))
    assert(text.split(" ").all((w) => w.len() >= 3))
    assert(text.split(" ").find((w) => w.startsWith("b")) == "brown")

    // `peekable()` looks at the next element before it is taken.
    var letters = "ab".chars().peekable()
    assert(letters.peek() == .Some(&'a') && letters.next() == 'a')

    // A slice walks through `copies()`, which copies each element out,
    // through `forwards()` and `backwards()`, which lend each where it lies,
    // and through `indexed()`, which lends each with its position.
    val numbers = [1, 2, 3, 4]
    assert(numbers.copies().skip(1).take(2).toVec() == [2, 3])
    assert(numbers.countWhere((n) => n > 2) == 2)
    val names: Vec<String> = ["a", "b"]
    var seen = String()
    for (index, name) in names.indexed() do
        seen.push("\(index)\(name)")

    for name in names.backwards() do
        seen.push(name)

    assert(seen == "0a1bba")
}
```

`copies()` hands out copies, so its elements are plain data, which is what
`sum` and arithmetic on them want; `forwards()` and `backwards()` lend
them, `&T`, so they walk any list, from the first element or from the last;
`indexed()` lends them from the first, each in a pair with its position
from 0, `(i64, &T)`. It is the walk `forwards().enumerate()` makes, which
is warned of on a list, with the fix that writes `indexed()` (E0482); an
`enumerate()` after `backwards()` or `copies()` counts what they hand out,
and stays. Over values that own memory, `copies()` is refused, and names
`forwards()`. Three walks, then, by what they do: they lend, they copy, or
— `for x in move c` and `intoIterator()` — they take.

The adapters are `map`, `filter`, `filterMap` — what a closure makes of an
element, where it makes something — `flatMap`, `flatten` where the elements
are iterators, `enumerate`, `take`, `takeWhile`, `skip`, `chain`, `zip` and
`peekable`. The consumers are `count`, `toVec`, `find`, `findMap`,
`position`, `nth`, `last`, `any`, `all`, `fold`, `forEach`, `minBy` and
`maxBy`. `nth` passes over the elements before the one it answers, and the
iterator goes on after it.

An adapter's closure is lent, and reads: the adapter is a view of what it
captured, as of what it walks, so a chain may read a `str`, a reference or
`self` around it, and is used while those stay as they were — used up in
its statement, kept in a `val` for its block, or walked by a `for` for the
loop — and does not leave the function that holds them. A closure that
names nothing around it borrows nothing, so its chain leaves as far as
what it walks may. A lazy walk that leaves its function reading what is
around it is a generator, which borrows only what it is given
([page 4](04-expressions.md)):

```wip,run
import std::iter::{Forwards, Mapped}

fn double(x: &i64): i64 = x * 2

fn doubled(values: &[i64]): Mapped<Forwards<i64>, i64> from values = {
    return values.forwards().map(double)
}

fn main() = {
    val prefix = "b"
    val words = "alpha beta bold".split(" ")
    var picked = words.filter((w) => w.startsWith(prefix))
    assert(picked.next() == "beta" && picked.count() == 1)
    assert(doubled([1, 2]).sum() == 6)
}
```

```wip,error=E0436
fn main() = {
    var step = 1
    val values = [1, 2]
    var shifted = values.forwards().map((x) => x + step)
    step = 2
    val first = shifted.next()
}
```

A collection is made from what an iterator hands out by its `collect`,
as a `Map` is from pairs ([`std::collections`](#stdcollections)):

```wip,run
import std::collections::{Map}

fn main() = {
    val numbers = "1 x 22 y".split(" ")
        .filterMap((w) => w.toInt().toOption())
        .toVec()
    assert(numbers == [1, 22])
    val letters = "ab cd".split(" ").flatMap((w) => w.chars()).toVec()
    assert(letters.len() == 4 && letters[2] == 'c')
    val words = "ab cd".split(" ").map((w) => w.chars()).flatten()
    assert(words.count() == 4)
    assert("12a3".chars().takeWhile((c) => c.isDigit()).count() == 2)
    assert("a b".split(" ").chain("c".split(" ")).last() == "c")
    assert("a b c".split(" ").position((w) => w == "c") == 2)
    var total = 0
    "1 2 3".split(" ").forEach((w) => { total += w.toInt().unwrap() })
    assert(total == 6)
    val years = [30, 40]
    val ages = Map::collect("ann bo".split(" ").zip(years.copies()))
    assert(ages.at("bo") == 40)
}
```

A slice also answers about itself directly: `indexOf` and `lastIndexOf`,
`contains`, `minIndex`, `maxIndex`, `countWhere`, `isSorted`, and sorts in
place: `sort`, `sortBy` an order given, `sortByKey` a key made of each.
They are stable — equals stay in the order they came, so sorting by one
key and then another orders by the second and, among equals, the first —
and take a buffer of half the elements; `sortUnstable` and
`sortUnstableBy` take none, and equals may change places. A `Vec`'s
`dedup()` keeps the first of each run of equal neighbours, and
`dedupBy(key)` of neighbours whose keys are equal:

```wip,run
struct Person {
    name: String
    age: i64
}

fn main() = {
    var people: Vec<Person> = [
        Person(name: "cy", age: 30),
        Person(name: "ann", age: 40),
        Person(name: "bo", age: 30),
    ]
    people.sortByKey((p) => p.name.toString())
    people.sortByKey((p) => p.age)
    assert(people[0].name == "bo" && people[1].name == "cy")
    var ages = people.forwards().map((p) => p.age).toVec()
    ages.dedup()
    assert(ages == [30, 40])
}
```

A sorted one is searched by halves: `binarySearch(value)` answers
`.Ok(i)` where the value is, or `.Err(i)` where it would go, and
`binarySearchBy` takes a closure that says how an element stands to the one
sought:

```wip,run
fn main() = {
    val numbers = [1, 3, 5, 7]
    assert(numbers.binarySearch(5) == 2)
    assert(numbers.binarySearch(4) == .Err(2))
    val sought = 7
    assert(numbers.binarySearchBy((n: &i64) => n.compare(&sought)) == 3)
}
```

`count()` is an iterator's, and counts every element; a slice's length is
`len()`. Where its element adds up, an iterator has `sum`, where it
multiplies `product`, and where it is ordered `min` and `max`, the first
of equals, as a slice does ([page 8](08-interfaces.md)):

```wip,run
fn main() = {
    val text = "wide 字\nab"
    assert(text.chars().map((c) => c.width()).sum() == 9)
    assert(text.lines().map((line) => line.len()).max() == 8)
    assert("pear fig plum".split(" ").min() == "fig")
    val values = [3, 9, 4]
    assert(values.copies().product() == 108)
    assert(values.backwards().max() == .Some(&9))
}
```

An iterator of a program's own is most easily a generator: a loop that
yields where a value stands, or a function that answers `Iterator<T>`
([page 4](04-expressions.md#generators)).

## `std::collections`

`Map<K, V>` and `Set<K>` keep what they hold in the order it went in, and
are walked as slices:

```wip,run
import std::collections::{Map, Set}

fn main() = {
    var counts = Map<str, i64>()
    counts.put("a", 1)
    counts.atOrPut("b", 0) += 2
    assert(counts.len() == 2)
    assert(counts.at("a") == 1)
    assert(counts.contains("b"))
    assert(counts.indexOf("a") == 0)
    assert(counts.get("a") == .Some(&1) && counts.get("z").isNone())
    assert(counts.keyAt(1) == "b")
    counts.at("a") += 10
    assert(counts.remove("a") == 11)
    assert(counts == [("b", 2)])

    var seen = Set<i64>()
    assert(seen.add(1))
    assert(!seen.add(1))
    assert(seen.contains(1))
    assert(seen.len() == 1 && seen[0] == 1)
    assert(seen.remove(1))
}
```

`get` answers the value by reference, or nothing, and borrows the map and
not the key; `at` lends it to read or write, and panics where there is
none. `remove` keeps the order and costs the entries after it;
`swapRemove` moves the last into the gap and costs nothing; `values()`
lends a map's values to walk or to write. `atOrPutWith(key, make)` is
`atOrPut` whose value is made only where there is none. Two maps are equal
with the same keys and equal values under them, and two sets with the same
keys, whatever order they went in.

`SortedMap<K, V>` and `SortedSet<K>` answer what `Map` and `Set` answer,
by the same names, for a key that is `Ord` rather than `Hash`, and keep
what they hold in the order of the keys, whatever order it went in: a
walk of one starts at its least key, and two are equal entry by entry.
What only an order answers is theirs: `first()` and `last()`, the entries
of the least and the greatest key; `positionOf(key)`, where the first key
not less than `key` is, and `len()` where every key is; and `within(from,
to)`, the positions of the keys from `from` up to `to`, which a walk
takes and `keyAt` and `values()` read. Their entries lie in one vector,
found by halves, so a lookup is quick and a new key moves those after
it; a list collected into one is sorted once:

```wip,run
import std::collections::{SortedMap, SortedSet}

fn main() = {
    var ages: SortedMap<str, i64> = [("cy", 3), ("ada", 1), ("bob", 2)]
    ages.put("dee", 4)
    assert(ages.keyAt(0) == "ada" && ages.at("bob") == 2)
    assert(ages.first() is .Some(least) && least.key == "ada")
    assert(ages.last() is .Some(most) && most.value == 4)
    assert(ages.positionOf("b") == 1)
    val between: Vec<str> = [
        for at in ages.within("b", "d") do
            yield ages.keyAt(at)
    ]
    assert(between == ["bob", "cy"])

    var seen: SortedSet<i64> = [3, 1, 2]
    assert(!seen.add(2) && seen.items() == [1, 2, 3])
}
```

Each collection that holds what an iterator hands out is made from one by a
static function of one name, `collect`, which takes any iterator of its
elements: `Vec`, `Set`, `SortedSet`, `Deque`, `String` of `char`, and
`Map`, `SortedMap` and `PriorityQueue` of pairs. They are the method of the
prelude's `Collect<T>`, which a type of a program's own implements to be
collected too. Where the type is expected, the call is `.collect(items)`,
and a list written there is that collection, made through it. A `Set` keeps
the first of equals and a `Map` the later value of a key, as `add` and
`put` do. `iter.toVec()` is `Vec::collect(iter)` at the end of a chain.

```wip,run
import std::collections::{Deque, Map, Set}

struct Total {
    sum: i64
}

extend Total: Collect<i64> {
    static fn collect<I: Iterator<i64>>(items: I): Total = {
        var total = Total(sum: 0)
        for x in move items do
            total.sum += x

        return total
    }
}

fn main() = {
    val words = ["b", "a", "b"]
    val seen: Set<str> = .collect(words.copies())
    assert(seen.len() == 2 && seen[0] == "b")
    val lengths = Map::collect(words.copies().map((w) => (w, w.len())))
    assert(lengths.len() == 2)
    val queue: Deque<i64> = [1, 2, 3]
    assert(queue[2] == 3)
    val text: String = .collect("abc".chars().filter((c) => c != 'b'))
    assert(text == "ac")
    val total: Total = [1, 2, 3]
    assert(total.sum == 6)
}
```

`Deque<T>` is pushed and taken at either end at the same small cost, where
a `Vec` is quick at its back alone: a queue, a stack, or a list pushed at
its front. It is read and written by position, walked from front to back,
and lends its ends, `first()` and `last()`, as a slice does:

```wip,run
import std::collections::{Deque}

fn main() = {
    var waiting: Deque<i64> = Deque()
    waiting.pushBack(2)
    waiting.pushBack(3)
    waiting.pushFront(1)
    assert(waiting.len() == 3 && waiting[0] == 1)
    assert(waiting.popFront() == 1)
    assert(waiting.popBack() == 3)
    assert(waiting == Deque::of([2]) && waiting.first() == .Some(&2))
}
```

`Arena<T>` holds values that link to each other — a tree's parents, a list
linked both ways, a graph — where no one owner could hold the links. `add`
answers a `Handle<T>`, a slot and a generation: plain data, kept in a
field, a `Vec` or a map, and not taken where a handle of another type is.
`arena[handle]` lends the value to read or to write, and panics where the
value was removed, even once another has taken its slot; `get` and
`contains` ask instead:

```wip,run
import std::collections::{Arena, Handle}

struct Node {
    value: i64
    parent: Option<Handle<Node>>
}

fn depth(tree: &Arena<Node>, node: Handle<Node>): i64 = {
    var count = 0
    var at = tree[node].parent
    while at is .Some(up) {
        count += 1
        at = tree[up].parent
    }
    return count
}

fn main() = {
    var tree: Arena<Node> = Arena()
    val root = tree.add(Node(value: 1, parent: .None))
    val leaf = tree.add(Node(value: 2, parent: root))
    tree[leaf].value += 10
    assert(depth(&tree, leaf) == 1)
    assert(tree.remove(leaf).isSome())
    assert(tree.get(leaf).isNone() && !tree.contains(leaf))
    val other = tree.add(Node(value: 3, parent: .None))
    assert(
        other != leaf && !tree.contains(leaf),
        "the slot is reused, the handle is not",
    )
}
```

The values lie packed, as a map's do: `for entry in arena` visits each
`ArenaEntry`'s `handle` and `value`, and `values()` lends them to walk and
write. `remove` moves the last value into the gap, so it costs the same
however many there are, and the order is the order values went in only
until the first `remove`. `pair(a, b, body)` lends two values to write at
once. `retain(keep)` keeps the values a closure answers true for, which it
may write as it goes, and removes the rest: a collector's sweep. `clear()`
makes every handle stale, and `clone()` copies the values under the same
handles. A handle is not tied to its arena: one from another arena of the
same type may name a value there.

`Arena<T>` is `Arena<T, K = T>`, and its handles are `Handle<K>`. Where the
code that keeps handles cannot name the values' type — a game's rules,
which know a ball by its handle, in a module below the one that says what a
ball is — the second type is one that code can name: `Arena<Ball, Balls>`
gives `Handle<Balls>`, with `Balls` declared beside the rules.

`PriorityQueue<T, P>` takes the value of the smallest priority first, of
any `P` that is `Ord`, a float among them: what finding a path, a queue of
timers or a simulation's next event asks. `push(value, priority)` puts
one in; `pop()` takes the first with its priority, `Option<(T, P)>`;
`first()` and `firstPriority()` lend them without taking; values of equal
priority come out in the order they were pushed, so a queue gives the same
order on every run. It is a binary heap, each push and pop a logarithm of
how many it holds, and it is not walked: what is wanted in order is
popped.

```wip,run
import std::collections::{PriorityQueue}

fn main() = {
    var due: PriorityQueue<str, f64> = PriorityQueue()
    due.push("bell", 2.5)
    due.push("door", 0.5)
    due.push("light", 2.5)
    assert(due.first() == .Some(&"door") && due.firstPriority() == .Some(&0.5))
    assert(due.pop() == .Some(("door", 0.5)))
    assert(due.pop() == .Some(("bell", 2.5))) // pushed before "light"
    assert(due.pop() == .Some(("light", 2.5)) && due.isEmpty())
}
```

## Values several owners hold: `std::shared`

`Shared<T>` is a value on the heap that several owners hold and read.
`share()` makes another owner, and is the one place a count goes up;
`value()` lends the value to read; the last owner to let go drops it.
`edit()` writes it where `T` clones: in place where nothing else holds it,
and otherwise a copy first, which the editing owner holds alone. A
`Weak<T>` links to the value without keeping it, as a child to its parent:
`upgrade()` answers an owner while one remains. The counts are atomic, so
owners may be on different threads.

```wip,run
import std::shared::{Shared, Weak}

fn main() = {
    val first = Shared::of(Vec::of([1, 2, 3]))
    var second = first.share()
    assert(first.owners() == 2 && second.value().len() == 3)
    second.edit().push(4) // a copy: `first` is shared
    assert(first.value().len() == 3 && second.value().len() == 4)

    val link: Weak<Vec<i64>> = first.downgrade()
    assert(link.upgrade().isSome())
    destroy(first)
    assert(link.upgrade().isNone())
}
```

## Vectors, matrices and rotations: `std::math`

`std::math` has the geometry every game and every program that draws
writes for itself. `Vec2`, `Vec3` and `Vec4` are vectors of `f32`, `x`,
`y`, `z` and `w`, added, subtracted and negated, scaled by an `f32` on
either side, and multiplied axis by axis; `dot`, `cross`, `length`,
`normalized`, `lerp`, `reflect` and the rest are methods. `Quat` is a
rotation, `Mat3` and `Mat4` matrices, and `Aabb`, `Ray`, `Plane` and
`Sphere` the shapes they are tested against, a ray's `hit` answering how
far along it a shape is first met.

The conventions are OpenGL's, glam's and GLM's: right-handed, `y` up,
angles in radians, a matrix stored a column at a time as a shader takes
it, and multiplying a vector on its right, so that `a * b` does `b` first.
A projection names the depth range it gives, since a wrong one draws
nothing and says nothing: `perspective` and `orthographic` give OpenGL's
`-1` to `1`, and `perspectiveZeroToOne` and `orthographicZeroToOne`
Vulkan's, Metal's and Direct3D's `0` to `1`. Equality is exact;
`approxEquals(other, tolerance)` is the comparison a test wants. A zero
vector's `normalized()` is NaN, as IEEE's division answers, and
`normalizedOr(fallback)` answers the fallback instead.

Each type is laid out as C lays out its fields, so a value crosses to a C
library as it is. `Vec2` and `Vec3` are aligned as their `f32`s are, a
`Vec3` twelve bytes, as a vertex's position; `Vec4`, `Quat` and `Mat4` are
aligned to 16, as a GPU's `vec4` and `mat4` and a SIMD register.

`Noise` is Ken Perlin's improved noise, values that change smoothly from
point to point, which terrain and clouds are made of: `Noise::perlin()`
over his own permutation, or `Noise::seeded(seed)`, the same world for the
same seed; `at2(x, y)` and `at3(x, y, z)` from -1 to 1, and `fractal2` and
`fractal3`, the sum of octaves each finer and fainter than the last. The
helpers a game writes are methods of `f64` and `f32` in the prelude:
`lerp(to, t)`, `smoothstep(edge0, edge1)` and `fract()` as GLSL has them,
`sign()`, `toRadians()` and `toDegrees()`; `PI_F32` and `TAU_F32` are `PI`
and `TAU` for code in `f32`:

```wip,run
import std::math::{Noise, PI_F32}

fn main() = {
    val world = Noise::seeded(2026)
    val ground = world.fractal2(12.5, 3.25, 5)
    assert(ground >= -1.0 && ground <= 1.0)
    assert(Noise::perlin().at3(3.14, 42.0, 7.0) == 0.13691995878400012)

    val fade: f32 = 0.25
    assert(fade.smoothstep(0.0, 1.0) == 0.15625)
    assert((90.0 as f32).toRadians() == PI_F32 / 2.0)
    assert(2.75.fract() == 0.75 && (2.0 as f32).lerp(10.0, 0.25) == 4.0)
}
```

`Vec2d`, `Vec3d`, `Vec4d` and `Mat4d` are the same in `f64`, for a world
larger than an `f32` keeps exact — a millimetre to about eight kilometres
from the origin. A renderer takes positions `relativeTo` the camera, in
`f32`, where they are exact again:

```wip,run
import std::math::{Aabb, Mat4, Mat4d, Quat, Ray, Vec3, Vec3d, Vec4}

fn main() = {
    val eye = Vec3::of(0.0, 2.0, 6.0)
    val view = Mat4::lookAt(eye, target: Vec3::zero(), up: Vec3::unitY())
    val aspect: f32 = 16.0 / 9.0
    val projection = Mat4::perspectiveZeroToOne(1.0, aspect, 0.1, 100.0)
    val spin = Quat::axisAngle(Vec3::unitY(), 0.5)
    val moved = Mat4::translation(Vec3::unitX())
    val model = moved * Mat4::rotation(spin)

    // The origin, seen by the camera: straight ahead, down `-z`.
    val ahead = Vec3::of(0.0, 0.0, -eye.length())
    assert(view.transformPoint(Vec3::zero()).approxEquals(ahead, 0.0001))
    val clip = projection * view * model * Vec4::unitW()
    val depth = clip.z / clip.w
    assert(depth > 0.0 && depth < 1.0, "between the near and far planes")

    var position = Vec3::zero()
    val velocity = Vec3::of(2.0, 0.0, -1.0)
    position += velocity * 0.5
    assert(position == Vec3::of(1.0, 0.0, -0.5))
    val turned = spin * Vec3::unitX()
    assert((turned.length() - 1.0).abs() < 0.0001, "it keeps lengths")

    val crate = Aabb::of(Vec3::splat(-1.0), Vec3::splat(1.0))
    val ray = Ray::of(Vec3::of(0.0, 0.0, 5.0), -Vec3::unitZ())
    assert(crate.hit(ray) == 4.0)

    val ship = Vec3d::of(4.2e9, 0.0, 0.0)
    val camera = Vec3d::of(4.2e9 - 3.5, 0.0, 0.0)
    assert(ship.relativeTo(camera) == Vec3::of(3.5, 0.0, 0.0))
    val placed = Mat4d::translation(ship).relativeTo(camera)
    assert(placed.transformPoint(Vec3::zero()) == Vec3::of(3.5, 0.0, 0.0))
}
```

## Every module

Each of `std`'s modules, and the page that describes it:

| Module | What is in it | Page |
|---|---|---|
| `std::args` | `Arguments`, `Argument` and `ArgumentError`: the program's arguments, read as options | [10.2](10-library-system.md) |
| `std::io` | `print`, `println`, `printInt`, `eprintln`, `flush`, `stdin`, `Reader`, `env`, `environment`, `setEnv`, `unsetEnv`, `isTerminal`, `IoError`; `Read` and `Write`, what bytes are read from and written to, `copy`, and `stdout`, `stderr` and their `Printer`; and `EventQueue` with its `Interest` and `Event` | [10.2](10-library-system.md), [10.3](10-library-formats.md) |
| `std::fs` | whole files, written, read and replaced; reading a directory, and walking the tree under one; what the filesystem says about a path, its owner among it; a path's mode and owner set; directories for a while | [10.2](10-library-system.md) |
| `std::process` | `Command`, `Child` and `Exit`: other programs started and waited for; the program's own `currentDir`, `setCurrentDir`, `executable`, `besideProgram` and `id`; `Signals` and `Signal`: Ctrl-C and its like, caught; `fork`, `newGroup` and `sendToGroup`; `userId`, `becomeUser` and `changeRoot`; `limit` and `setLimit` | [10.2](10-library-system.md) |
| `std::users` | `User` and `Group`: the system's users and groups, found by name or by number, a Unix's | [10.2](10-library-system.md) |
| `std::path` | a path joined, and taken apart: its name, parent, extension and stem; cleaned, made relative, its extension replaced; matched by a glob | [10.2](10-library-system.md) |
| `std::mem` | `replace`, a place's value taken out with another left in, and `swap`, two places' values exchanged; `using` and `onHeap`, where a call's allocations come from: an `Arena`, a `Custom` one of a program's own `Allocator`, or the heap | [6](06-ownership.md#where-memory-comes-from-memusing) |
| `std::digest` | `sha256(bytes)`, `sha512(bytes)` and `sha1(bytes)`, digests of bytes in hand, and `Sha256`, `Sha512` and `Sha1`, written in pieces and then finished; `hmacSha256`, `hmacSha512` and `hmacSha1`, a digest under a key; `crc32(bytes)` and `adler32(bytes)`, the checksums gzip, zip, PNG and zlib end their data with, and `Crc32` and `Adler32`, written in pieces and read after any | [10.3](10-library-formats.md) |
| `std::c` | a value lent to C as a callback's `void *`, and had back in the callback; `Pinned`, a value C holds until it lets go; a slice's address for C to read; C's `sizeof` and `_Alignof` | [11](11-c.md) |
| `std::embed` | a file's bytes or text, read when the program is compiled, as a constant | [10.2](10-library-system.md) |
| `std::watch` | `Watcher`: which files and directories changed, as the system tells | [10.2](10-library-system.md) |
| `std::sync` | `Atomic<T>` and `Mutex<T>`: what several threads may change at once, with waits that give up and a lock only tried; `Lazy<T>`, a value made once, by the first thread that asks; `ring`, its `Filler` and `Drainer`: values handed between two threads without waiting | [10.1](10-library-threads.md) |
| `std::future` | `Future`, work run on a thread of its own and waited for; `both`, `each` and `map`, work that borrows, across threads; `Pool`, threads started once for them; `channel`, its `Sender` and `Receiver` | [10.1](10-library-threads.md) |
| `std::shared` | `Shared<T>` and `Weak<T>`: a value several owners hold and read | above |
| `std::random` | `Random`: numbers seeded or from the system, not for cryptography | [10.2](10-library-system.md) |
| `std::markup` | `Html` and `Xml`: templates an interpolated literal builds, each value escaped as its place in the page needs, and a hole refused where it cannot be; `push`, `joined` and `trusted` | [10.3](10-library-formats.md) |
| `std::secret` | `bytes` and `fill`, from the system's generator for secrets; `equal`, two secrets compared in a time that does not say where they differ | [10.3](10-library-formats.md) |
| `std::net` | `Listener`, `Stream` and `Address`: TCP and Unix-domain connections | [10.3](10-library-formats.md) |
| `std::tls` | `TlsStream` and `TlsError`: a TLS connection, the server's certificate checked by the system | [10.3](10-library-formats.md) |
| `std::http` | `get`, `Client`, `Request`, `Response` and `Incoming`: HTTP/1.1 requests, a body read whole or as it comes, undone where it was sent gzipped, sent from bytes, a file or any `io::Read` | [10.3](10-library-formats.md) |
| `std::compress` | `gzip` and `gunzip`, `zlib` and `unzlib`, `deflate` and `inflate`, and `Inflater` and `Deflater` for bytes in pieces: deflate, through the system's zlib, a Unix's alone | [10.3](10-library-formats.md) |
| `std::math` | `PI`, `TAU` and `E`, and the vectors, matrices, rotations and shapes above; the functions are methods of `f64` and `f32`, `x.sin()`, written in Wip so that a `@comptime` constant computes the program's bits | above |
| `std::text` | `Scanner` and `Position`, above; `ParseError`, and the `Split`, `Lines` and `Chars` types the prelude's methods answer | above |
| `std::iter` | `Walk`, `Forwards`, `Backwards`, `Taking` (what a `Vec` gives up to `for x in move v`), and the adapter types `Mapped`, `Filtered`, `FilterMapped`, `FlatMapped`, `Chained`, `Enumerated`, `Taken`, `TakenWhile`, `Skipped`, `Zipped`, `Peekable` | above |
| `std::collections` | `Map`, `Set`, `MapEntry`, `SortedMap`, `SortedSet`, `Deque`, `PriorityQueue`, and `Arena` with its `Handle`, `ArenaEntry` and `ArenaValues` | above |
| `std::log` | `Log`, `Level` and `Record`: what a program says as it runs, kept from a least level, to standard error, the system's log or a closure of the program's | [10.2](10-library-system.md) |
| `std::time` | `Duration`, a length of time; `Instant`, a moment on a clock that only goes forward; `sleep`; `FixedStep`, a game's updates at a steady rate; `now`, the date it is; `Utc`, a moment as a calendar has it, and `Local`, as it has it where the program runs | [10.2](10-library-system.md) |
| `std::bytes` | numbers read and written at a place in bytes, `u32le` and `putU32le`; `Reader` and `Writer`, the same in turn; and a binary format for `Encode` and `Decode` | [10.3](10-library-formats.md) |
| `std::json` | `Json`, `parse`, `encode` and `decode`, and `JsonError`: JSON read into a tree or into a type that derives `Decode`, and written from one | [10.3](10-library-formats.md) |
| `std::regex` | `Regex`, `Found`, `escape` and `RegexError`: regular expressions, compiled once and matched, searched and replaced | [10.3](10-library-formats.md) |
| `std::base64` | `encode` and `decode`, in the standard alphabet or the one for URLs, padded or not, and `Base64Error` | [10.3](10-library-formats.md) |
| `std::hex` | `encode` and `decode`, two digits a byte, and `HexError` | [10.3](10-library-formats.md) |
| `std::url` | `Url`, a URL's text and its parts, read with `parse` or built by a literal, each hole encoded as its part needs; `encode` and `decode`, percent-encoding for any part of a URL, and `UrlError` | [10.3](10-library-formats.md) |
| `std::libc` | the C library as std calls it: `printf`, `fwrite`, `stdout`, `read`, `open` and its flags, `getentropy`, `errno()` — C's, promising nothing C does not | [11](11-c.md) |

```wip,run
import std::digest
import std::io
import std::math
import std::mem

fn main() = {
    io::println("a line on standard output")
    assert(math::TAU > 6.28)
    assert((2.0).sqrt() > 1.41)
    var state = String::of("reading")
    assert(mem::replace(&var state, "idle") == "reading" && state == "idle")
    val text = "abc"
    assert(digest::sha256(&text)[0] == 0xBA)
    assert(digest::crc32(&text) == 0x352441C2)
    assert(digest::adler32(&text) == 0x024D0127)
}
```
