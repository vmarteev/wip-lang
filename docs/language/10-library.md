# 10. The standard library

The library is in two halves: the **prelude**, which every file sees without
an import, and the modules under `std`, which are imported. The prelude's
names are reserved, so `Option` means the same type in every program.

## `Option` and `Result`

```wip,run
fn main() = {
    val found: Option<i64> = .Some(7)
    assert(found.isSome())
    assert(found.unwrapOr(0) == 7)
    assert(found.map((n) => n * 2) == .Some(14))
    assert(found.fold(() => 0, (n) => n * 2) == 14)

    val missing: Option<i64> = .None
    assert(missing.unwrapOr(0) == 0)
    assert(missing.or(.Some(1)) == .Some(1))
    // Made only where it is needed.
    assert(missing.orElse(() => .Some(2)) == .Some(2))
    assert(missing.unwrapOrElse(() => 3) == 3)
    assert(missing.toResult("nothing there").isErr())

    val answer: Result<i64, str> = .Ok(2)
    assert(answer.map((n) => n + 1) == .Ok(3))
    assert(answer.toOption() == .Some(2))

    val failed: Result<i64, str> = .Err("no")
    assert(failed.mapErr((e) => e.len()) == .Err(2))
    assert(failed.unwrapOr(9) == 9)
    assert(failed.unwrapOrElse((e) => e.len()) == 2)
    assert(failed.orElse((e) => .Ok(e.len())) == .Ok(2))
}
```

`unwrapOr` and `or` are given their alternative as a value, which is
evaluated whether or not it is used; `unwrapOrElse` and `orElse` are given a
function that makes it, called only where it is needed — on a `Result`,
with the error. Both also carry `flatMap`, `fold`, `filter`
(on `Option`), `any`, `all` and `forEach`. `fold` takes a function for each
case, and calls only the one that applies: `found.fold(() => 0, (n) => n *
2)`. `unwrap` panics where there is nothing to answer, so it is for what
cannot happen.

`asRef()` lends an option's value where it lies, as an `Option<&T>`, which
`map` makes into what it looks at: an `Option<String>` read as an
`Option<str>` is `host.asRef().map((name) => name.toStr())`, and borrows
the option until it is no longer used.

An `Option` may be filled where it is empty and written where it is not:

```wip,run
fn main() = {
    var slot: Option<i64> = .None
    slot.getOrInsert(7) += 1
    assert(slot == .Some(8))
}
```

## `Vec<T>`

A growing buffer, written in Wip over one primitive the compiler knows.
`Vec()` is an empty one, `Vec::of(own [ … ])` takes the
values a list was built with, and `Vec::filled(count, value)` holds
clones of a value:

```wip,run
fn main() = {
    var numbers: Vec<i64> = Vec()
    numbers.push(1)
    numbers.push(2)
    assert(numbers.len() == 2 && numbers[0] == 1)
    assert(numbers.pop() == .Some(2))

    var made = Vec::of(own [3, 1, 2])
    made.sort()
    assert(made == Vec::of(own [1, 2, 3]))
    made.insert(0, 0)
    assert(made.remove(0) == 0)
    assert(made.swapRemove(0) == 1)
    made.retain((n) => n > 2)
    assert(made == Vec::of(own [3]))

    val filled: Vec<i64> = Vec::filled(3, 7)
    assert(filled.len() == 3 && filled[2] == 7)
    val names = Vec::filled(2, String::of("x"))
    assert(names[1] == "x")
    assert(Vec<i64>::withCapacity(10).isEmpty())
}
```

`at(i)` is the lending pair behind `v[i]`, `items()` lends the whole as a
slice, `truncate`, `clear` and `reserve` are there, and a `Vec` ends what it
holds when it ends.

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
    var list: Vec<u8> = Vec()
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
walked. The function each is given is lent for the call:

```wip,run
fn main() = {
    val numbers = [3, 1, 4, 1, 5, 9]
    assert(numbers.map((n) => n * 2) == Vec::of(own [6, 2, 8, 2, 10, 18]))
    assert(numbers.filter((n) => n > 3) == Vec::of(own [4, 5, 9]))
    assert(numbers.fold(0, (sum, n) => sum + n) == 23)
    assert(numbers.find((n) => n > 3) == .Some(&4))
    assert(numbers.first() == .Some(&3) && numbers.max() == .Some(9))
    assert(numbers.take(2).len() == 2 && numbers.skip(4).len() == 2)
    assert(numbers.join(" ") == "3 1 4 1 5 9")
}
```

`map`, `flatMap`, `fold`, `indexWhere` and `lastIndexWhere`, `take` and
`skip` work on any elements, and `take` and `skip` lend a part of the slice
rather than copy it. `find`, `first` and `last` answer the element by
reference, for any elements. `filter`, `partition`, `min` and `max` copy the
elements they answer, so they are there for plain data; a vector of values
that own memory keeps what it wants with `retain`. `join` writes each
element as text, and `joinTo(&var out, separator)` writes them onto a
`String` there is; every sequence has `joinToBy`, which writes each element
as a closure says — a `Deque`, a map's values, a slice of pairs.

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
    assert(text.find("two") == .Some(6))
    assert("42".toInt() == .Ok(42))
    assert("4.5".toFloat() == .Ok(4.5))
    assert("\(0.1 + 0.2)" == "0.30000000000000004")
    assert("\(1e20)" == "1e+20")
    assert("añ".chars().count() == 2)
}
```

Text is searched from either end — `find` and `findLast` — and compared in
any case without making anything, `equalsAnyCase`; a `String` takes its
last character back with `popChar`, as a backspace does:

```wip,run
fn main() = {
    val path = "a/b/c.tar.gz"
    assert(path.find(".") == .Some(5) && path.findLast(".") == .Some(9))
    assert("Content-Length".equalsAnyCase("content-length"))
    var typed = String::of("añ")
    assert(typed.popChar() == .Some('ñ') && typed == "a")
}
```

`String::of(text)` copies a `str` into a new `String`; `toStr()` lends the
bytes back as a `str`; `toCstring()` makes the zero-terminated kind C wants
([page 11](11-c.md)).

A `String` is lent as a `str` wherever its text is read for a call: an
argument where a `str` or a `&str` is expected, the receiver of a method
`str` has, a slice of it, and a side of a comparison with text. Nowhere else
— a `str` kept in a variable or a field would outlive the bytes — so there
it is written `toStr()`:

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

`stripPrefix` and `stripSuffix` answer the text with a prefix or a suffix
taken off, and `splitOnce` the text either side of its first separator —
or nothing, where it is not there:

```wip,run
fn main() = {
    val line = "#define WIDTH 32"
    assert(line.stripPrefix("#define ") == .Some("WIDTH 32"))
    assert(line.stripPrefix("#undef ").isNone())
    assert("shapes.h".stripSuffix(".h") == .Some("shapes"))
    val .Some((name, value)) = "WIDTH 32".splitOnce(" ") else {
        return
    }
    assert(name == "WIDTH" && value == "32")
}
```

A float is written as C's `%g` writes it, with as many digits as it
takes to read back as the same value, and six at the least; `toFloat()`
answers the nearest `f64` to the text.

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

## Walking: `Items`, `Iterator` and the adapters

A container that can be walked as a slice implements `Items<T>`, which is
what `for x in container` uses. Values that come
one at a time implement `Iterator<T>`, whose adapters and consumers are in
the prelude:

```wip,run
fn main() = {
    val text = "the quick brown fox"

    // Adapters answer another iterator, and keep an `own` closure.
    val long = text.split(" ").filter(own (w) => w.len() > 3).toVec()
    assert(long == Vec::of(own ["quick", "brown"]))

    var pairs = 0
    for (i, word) in text.split(" ").enumerate() {
        pairs += i + word.len()
    }
    assert(pairs == 22)

    // Consumers walk it to the end, and are lent a plain closure.
    assert(text.split(" ").count() == 4)
    assert(text.split(" ").any((w) => w == "fox"))
    assert(text.split(" ").all((w) => w.len() >= 3))
    assert(text.split(" ").find((w) => w.startsWith("b")) == .Some("brown"))

    // `peekable()` looks at the next element before it is taken.
    var letters = "ab".chars().peekable()
    assert(letters.peek() == .Some(&'a') && letters.next() == .Some('a'))

    // A slice walks through `walk()`, and from the end through
    // `backwards()`, which lends each element where it lies.
    val numbers = [1, 2, 3, 4]
    assert(numbers.walk().skip(1).take(2).toVec() == Vec::of(own [2, 3]))
    assert(numbers.countWhere((n) => n > 2) == 2)
    val names = Vec::of(own [String::of("a"), String::of("b")])
    var seen = String()
    for name in names.backwards() {
        seen.push(name)
    }
    assert(seen == "ba")
}
```

`walk()` hands out copies, so its elements are plain data; `backwards()`
lends them, `&T`, so it walks any list, from the last element to the first.

A slice also answers about itself directly: `indexOf` and `lastIndexOf`,
`contains`, `minIndex`, `maxIndex`, `countWhere`, `isSorted`, `sort` and
`sortBy`. A sorted one is searched by halves: `binarySearch(value)` answers
`.Ok(i)` where the value is, or `.Err(i)` where it would go, and
`binarySearchBy` takes a closure that says how an element stands to the one
sought:

```wip,run
fn main() = {
    val numbers = [1, 3, 5, 7]
    assert(numbers.binarySearch(5) == .Ok(2))
    assert(numbers.binarySearch(4) == .Err(2))
    val sought = 7
    assert(numbers.binarySearchBy((n: &i64) => n.compare(&sought)) == .Ok(3))
}
```

`count()` is an iterator's, and counts every element; a slice's length is
`len()`.

An iterator of a program's own is most easily a generator: a loop that
yields where a value stands, or a function that answers `Iterator<T>`
([page 4](04-expressions.md#generators)).

## Work on another thread: `std::future`

`Future::run` runs an owned closure on a thread of its own, and hands
back a `Future<T>` of what it answers: `get()` waits for it, `poll()`
takes it if it is there without waiting, and `isDone()` says whether the
work has finished. A channel moves values between threads; its receiver
is an `Iterator`, which a loop reads until every sender is gone:

```wip,run
import std::future::{Future, channel}

fn main() = {
    val data = Vec::of(own [1, 2, 3, 4])
    val sum = Future::run(own () => {
        var total = 0
        for x in data {
            total += x
        }
        total
    })
    assert(sum.get() == 10)

    val (tx, rx) = channel<String>()
    val producer = Future::run(own () => {
        for i in 0..3 {
            tx.send("line \(i)")
        }
    })
    var lines = 0
    for line in move rx {
        lines += 1
    }
    assert(lines == 3)
}
```

What the closure captured is moved into it, and is the thread's alone: an
owned closure cannot hold a reference or a view, so nothing two threads
could both reach is ever shared, and nothing needs a lock. A future that is
dropped waits for its thread, so work never outlives what started it.

## Work that borrows: `together`, `each`, `map` and `std::sync`

Work may also borrow what is around it, where the call that runs it waits
for it. `future::together(first, second)` runs `first` on
a thread of its own and `second` on this one, and answers both results
once both are done; `future::each(items, work)` and `future::map(items,
work)` split a slice among the processors. The closures are lent for the
call, so what they capture is checked as the call's arguments are: what
one writes, no other touches, and several may read one place.

Two types change through a `&`, safely from any number of threads:
`std::sync::Atomic<T>` — `load`, `store`, `swap`, `add`, `subtract` and
`replaceIf`, each one step no thread sees half of — for `bool` and the
integers, and `Mutex<T>`, whose `lock` lends what is inside as a `&var`
for its call alone, and whose `lockWhen` waits until it is ready:

```wip,run
import std::future
import std::sync::{Atomic, Mutex}

fn sum(values: &[i64]): i64 = {
    var total = 0
    for value in values {
        total += value
    }
    return total
}

fn main() = {
    val numbers = [1, 2, 3, 4, 5, 6, 7, 8]
    val (low, high) = future::together(
        () => sum(&numbers[0..4]),
        () => sum(&numbers[4..]),
    )
    assert(low + high == 36)

    val counted = Atomic::of(0 as i64)
    var cells = [1, 1, 1, 1]
    future::each(&var cells, (cell) => {
        cell *= 2
        counted.add(1)
    })
    assert(counted.load() == 4 && cells[0] == 2)

    val names: Mutex<Vec<String>> = Mutex::of(Vec())
    future::together(
        () => names.lock((all) => all.push("ann")),
        () => names.lock((all) => all.push("bob")),
    )
    assert(names.lock((all) => all.len()) == 2)
}
```

A slice lends two parts of itself at once, which do not overlap, with
`split(at, (first, second) => …)`: what `each` gives each thread.

## A program's arguments: `std::args`

`main(args: &[cstring])` is given the program's arguments, and
`Arguments::of(&args)` reads them one at a time, from the one after the
program's name: each is a short option, a long one, or a positional
argument, and the program says what each means in a `match`, asking for an
option's value where it takes one:

```wip,run
import std::args::{ArgumentError, Arguments}

// What the options say; it holds names from the arguments, so it is a view.
view struct Options {
    count: i64 = 10
    verbose: bool = false
    files: Vec<str> = Vec()
}

fn options(args: &[cstring]): Result<Options, ArgumentError> = {
    var options = Options()
    var arguments = Arguments::of(&args)
    while arguments.next()? is .Some(argument) {
        match argument {
            .Short('n') | .Long("lines") => options.count = arguments.int()?
            .Short('v') | .Long("verbose") => options.verbose = true
            .Positional(file) => options.files.push(file)
            _ => return .Err(argument.unexpected())
        }
    }
    return .Ok(move options)
}

fn main() = {
    // What `main` would be given for `lines -vn 5 a.txt --lines=3`.
    val given: [cstring; 5] = ["lines", "-vn", "5", "a.txt", "--lines=3"]
    val .Ok(read) = options(&given) else {
        return
    }
    assert(read.count == 3 && read.verbose && read.files.len() == 1)
    val wrong: [cstring; 2] = ["lines", "-x"]
    assert(options(&wrong) is .Err(.UnknownOption(..)))
}
```

It reads them as `getopt_long` does: `-abc` is `-a`, `-b` and `-c`;
`-ofile` and `-o file` are one option with its value, as are
`--output=file` and `--output file`; `--` ends the options, and `-` alone
is positional; options and positional arguments come in any order. A
value is the next argument whatever it looks like, so `-o -` and `-n -5`
are values.

`value()` answers an option's value, and `int()` and `float()` read it as
a number; `optionalValue()` takes only a value written onto its option,
`--color=always`; `rest()` answers every argument not read yet, as it is,
for a program that hands them on; `program()` is the name it was run by.
An `ArgumentError` says what is wrong — "`-n` needs a value", "`--verbose`
takes no value, and was given `yes`" — and `main` that answers one prints
it after `error: ` and exits with 1. A subcommand is a positional argument,
after which the same `Arguments` reads its own options. The usage message
is the program's own, written as text and printed where it matches `-h`.

## Other programs: `std::process`

A `Command` names a program and its arguments, which no shell reads: each
is one argument, whatever it holds. `run()` gives it this program's
terminal and waits for it, `output()` keeps what it printed, `start()`
keeps it running as a `Child`, which is waited for where it is dropped,
and `launch()` lets it go:

```wip,run
import std::process::{Command}

fn main() = {
    val .Ok(said) = Command::of("printf").args([
        "%s",
        "a;$(rm x) b",
    ]).output() else {
        assert(false, "printf is there")
        return
    }
    assert(said.stdout == "a;$(rm x) b")
    assert(said.exit.success())

    assert(Command::of("false").run() == .Ok(.Code(1)))
    assert(Command::of("no-such-program").run() is .Err(.NotFound))
}
```

A program that ends with a code other than 0 has answered, not failed:
`exit.success()` asks. Failing to start is an `IoError`.

`input(text)` gives it text to read on its standard input, which
`output()` writes while it reads what the program prints and `run()`
writes before it waits. A program that stops reading ends the writing,
and the rest is dropped:

```wip,run
import std::process::{Command}

fn main() = {
    val .Ok(sorted) = Command::of("sort").input("b\na\n").output() else {
        assert(false, "sort is there")
        return
    }
    assert(sorted.stdout == "a\nb\n")
}
```

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
    assert(counts.find("a") == .Some(0))
    assert(counts.get("a") == .Some(&1) && counts.get("z").isNone())
    assert(counts.keyAt(1) == "b")
    counts.at("a") += 10
    assert(counts.remove("a") == .Some(11))

    var seen = Set<i64>()
    assert(seen.add(1))
    assert(!seen.add(1))
    assert(seen.contains(1))
    assert(seen.len() == 1 && seen[0] == 1)
    assert(seen.remove(1))
}
```

`get` answers the value by reference, or nothing, and borrows the map and
not the key; `at` lends it to read or write, and panics
where there is none. `remove` keeps the order and costs the entries after
it; `swapRemove` moves the last into the gap and costs nothing; `values()`
lends a map's values to walk or to write.

`Deque<T>` is pushed and taken at either end at the same small cost, where
a `Vec` is quick at its back alone: a queue, a stack, or a list pushed at
its front. It is read and written by position, and walked
from front to back:

```wip,run
import std::collections::{Deque}

fn main() = {
    var waiting: Deque<i64> = Deque()
    waiting.pushBack(2)
    waiting.pushBack(3)
    waiting.pushFront(1)
    assert(waiting.len() == 3 && waiting[0] == 1)
    assert(waiting.popFront() == .Some(1))
    assert(waiting.popBack() == .Some(3))
    assert(waiting == Deque::of([2]))
}
```

`Arena<T>` holds values that link to each other — a tree's parents, a list
linked both ways, a graph — where no one owner could hold the links.
`add` answers a `Handle<T>`, a slot and a generation:
plain data, kept in a field, a `Vec` or a map, and not taken where a
handle of another type is. `arena[handle]` lends the value to read or to
write, and panics where the value was removed, even once another has taken
its slot; `get` and `contains` ask instead:

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
    val leaf = tree.add(Node(value: 2, parent: .Some(root)))
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

`Arena<T>` is `Arena<T, K = T>`, and its handles are `Handle<K>`.
Where the code that keeps handles cannot name the values'
type — a game's rules, which know a ball by its handle, in a module below
the one that says what a ball is — the second type is one that code can
name: `Arena<Ball, Balls>` gives `Handle<Balls>`, with `Balls` declared
beside the rules.

## Values several owners hold: `std::shared`

`Shared<T>` is a value on the heap that several owners hold and read.
`share()` makes another owner, and is the one place a
count goes up; `value()` lends the value to read; the last owner to let go
drops it. `edit()` writes it where `T` clones: in place where nothing else
holds it, and otherwise a copy first, which the editing owner holds alone.
A `Weak<T>` links to the value without keeping it, as a child to its
parent: `upgrade()` answers an owner while one remains. The counts are
atomic, so owners may be on different threads.

```wip,run
import std::shared::{Shared, Weak}

fn main() = {
    val first = Shared::of(Vec::of(own [1, 2, 3]))
    var second = first.share()
    assert(first.owners() == 2 && second.value().len() == 3)
    second.edit().push(4) // a copy: `first` is shared
    assert(first.value().len() == 3 && second.value().len() == 4)

    val link: Weak<Vec<i64>> = first.downgrade()
    assert(link.upgrade().isSome())
    destroy(move first)
    assert(link.upgrade().isNone())
}
```

## Connections: `std::net`

A `Listener` listens on a port or a Unix-domain socket and accepts
connections; a `Stream` is one connection, accepted or made; an `Address`
says where one is, and prints as `127.0.0.1:8080` or `[::1]:8080`.
Each owns its descriptor and closes it where it ends, and
what fails is an `io::IoError`, as a file's failures are.

```wip,run
import std::future
import std::net::{Address, Listener, Stream}

fn main() = {
    // Port 0: the system chooses one, and `localAddress` says which.
    val listener = Listener::tcp(.Some("127.0.0.1"), 0).unwrap()
    val .Ok(.V4(port: port, ..)) = listener.localAddress() else {
        return
    }
    val (_, answer) = future::together(
        () => {
            val (stream, from) = listener.accept().unwrap()
            var request: [u8; 64] = [0; 64]
            val count = stream.read(&var request).unwrap()
            stream.writeText("got \(count) bytes from \(from.host())").unwrap()
        },
        () => {
            val stream = Stream::tcp("127.0.0.1", port).unwrap()
            stream.writeText("hello").unwrap()
            var reply: [u8; 64] = [0; 64]
            val count = stream.read(&var reply).unwrap()
            return String::of(str::fromBytes(&reply[0..count]))
        },
    )
    assert(answer == "got 5 bytes from 127.0.0.1")
}
```

`Listener::tcp(.None, port)` listens on the loopback, and `"0.0.0.0"` or
`"::"` on every address. `read` answers `0` once the other end has closed;
`write` may write fewer bytes than it was given, and `writeAll` and
`writeText` write them all. A write to a connection the other end has
closed answers `.BrokenPipe`, and no signal ends the program.
`setBlocking(false)` makes `accept`, `read` and `write` answer
`.WouldBlock` where they would wait, and `descriptor()` is the number an
event queue watches; the queue is the program's own. `accept`, `read` and
`write` take `&self`, so several threads may accept from one listener.

## Regular expressions: `std::regex`

A pattern is compiled once and then matched, searched and replaced with.
Matching takes time in proportion to the text times the pattern, whatever
either holds: no pattern backtracks without end.

```wip,run
import std::regex::{Regex}

fn main() = {
    val .Ok(numbered) = Regex::new("(\\d+)") else {
        return
    }
    assert(numbered.isMatch("img12.jpg"))
    assert(numbered.replaceAll("img12-3.jpg", "<$1>") == "img<12>-<3>.jpg")
    val .Some(found) = numbered.find("a1b22") else {
        return
    }
    assert(found.start == 1 && found.group(1) == .Some((1, 2)))
    assert(Regex::new("(unclosed").isErr())
}
```

The syntax is Rust's, in part: `.`, classes such as `[a-z]`, `[^0-9]`,
`\d`, `\w` and `\s`, the anchors `^` and `$`, groups `(…)` and `(?:…)`,
`|`, and `*`, `+`, `?` and `{n,m}`, with a `?` after them for as few as can
be. A replacement names what matched as `$0` and a group as `$1` or
`${1}`. What is not a pattern is a `RegexError`, which says what and at
which byte. `Regex::anyCase(pattern)` compiles one that matches ASCII
letters in either case, as POSIX's `REG_ICASE` does, so a host name
matches however it is written.

## JSON: `std::json`

`json::parse(text)` reads a whole document into a `Json`, and a `Json`
interpolated is its JSON, compact.

```wip,run
import std::json
import std::json::{Json}

fn main() = {
    val .Ok(doc) = json::parse(
        "{\"name\": \"wip\", \"tags\": [1, 2.5, null]}",
    ) else {
        return
    }
    assert("\(doc)" == "{\"name\":\"wip\",\"tags\":[1,2.5,null]}")
    val .Object(fields) = doc else {
        return
    }
    assert(
        fields[0].0 == "name" && fields[0].1 == .Text(text: String::of("wip")),
    )
    assert(fields[1].0 == "tags" && "\(fields[1].1)" == "[1,2.5,null]")

    val .Err(error) = json::parse("[1,\n 2,]") else {
        return
    }
    assert("\(error)" == "expected a value, at line 2, column 4")

    // A field and an element, by reference, and what a value is.
    assert(doc.get("name").flatMap((name) => name.asText()) == .Some("wip"))
    val second = doc.get("tags").flatMap((tags) => tags.item(1))
    assert(second.flatMap((tag) => tag.asFloat()) == .Some(2.5))
    assert(doc.get("size").isNone())
}
```

A number is an `Int` where it is written without a fraction or an
exponent and fits an `i64`, and a `Float` otherwise; a `Float` is written
with a `.0` where it would read back as an `Int`, and one that is not a
number or is infinite is written `null`. An object keeps its fields in
the order written, a name written twice included. What is not JSON is a
`JsonError`, which says what, and where by line and column; arrays and
objects nested deeper than `json::MOST_NESTED`, 512, are one.

What is there is read with `get(name)`, a field of an object, and
`item(index)`, an element of an array, each by reference or nothing,
and with `asText`, `asInt`, `asFloat`, `asBool`,
`asArray` and `isNull`, which answer where the value is of that kind. A
name written twice answers the one written last.

## Files and directories

`std::fs` reads and writes a whole file, lists a directory, and answers
what the filesystem says about a path. `struct stat` and `struct dirent`
are laid out differently on every system, so neither reaches Wip: what
comes back is a `Meta` and a `Vec<Entry>`.

```wip,run
import std::fs

fn main() = {
    val .Ok(entries) = fs::readDir("std/collections") else {
        // Run from somewhere else, there is nothing to list.
        return
    }
    assert(entries.len() >= 2)
    for entry in entries.items() {
        assert(entry.kind == .File || entry.isDirectory())
    }

    val .Ok(about) = fs::meta("std/prelude/option.wip") else {
        return
    }
    assert(about.isFile())
    assert(about.size > 0)
    assert(about.modified > 0) // seconds since 1970
    assert(about.mode > 0) // the permission bits, as `chmod` writes them
    assert(about.inode > 0) // with `device`, which file it is, whatever its name
}
```

A `Meta`'s and an `Entry`'s `kind` is a `File`, a `Directory`, a `Link`, or
one of the special files — a `Pipe`, a `Socket`, a `CharDevice`, a
`BlockDevice` — or `Other`; `kind.isSpecial()` says whether it is none of
the first three. `meta` follows a symbolic link and `linkMeta` looks at the
link itself; `exists` asks whether there is anything there; `removeFile`
removes a file and refuses a directory; `makeDir` makes a directory and
fails where one already is, and `File::createNew` makes a file the same way,
where `File::create` would empty one that is there. An open `File` reads by
lines or whole, and `readAt(offset, into)` reads from where it is asked
without moving where the next read starts, as C's `pread` does; `size()`
says how long the file is. A `Meta`'s `device` and `inode` say which file a
path names, so a program can tell, before it acts, that a name it listed
still means what it did. Entries come in the order the filesystem gives
them, without `.` and `..`, and sorting is the caller's. `realPath` answers
where a path really is: absolute, with no `.`, `..` or link on the way.

`std::path` takes a path apart and puts one together, as text: a `str`
of names between `/`s, which is what `std::fs` takes. Nothing in it reads
the disk:

```wip,run
import std::path

fn main() = {
    assert(path::join("/a", "b") == "/a/b")
    assert(path::name("/a/b/c.tar.gz") == "c.tar.gz")
    assert(path::parent("/a/b/c.tar.gz") == "/a/b")
    assert(path::parent("c") == ".")
    assert(path::extension("c.tar.gz") == .Some("gz"))
    assert(path::extension(".bashrc").isNone())
    assert(path::stem("c.tar.gz") == "c.tar")
    assert(path::isWithin("/a/b", "/a") && !path::isWithin("/ab", "/a"))
}
```

## Files embedded: `std::embed`

`embed::bytes(path)` and `embed::text(path)` read a file when the program
is compiled, and its contents are a constant of the program, kept once in
memory nothing writes: a `&[u8]`, or a `str`, which must be UTF-8.
The path is a string literal, relative to the directory
of the module that names it, and may not leave the package — `..` past its
root, a whole path, or a link that leads out of it is refused. They are
written where a constant's value is worked out, a top-level `val` or
`assert`, and a function reads the file through that constant, which
stands for the bytes as a reference parameter does: `FONT.len()`,
`FONT[0]`, `FONT.toVec()`. A program built this way carries its assets in
itself:

```wip,ignore
import std::embed

val FONT: &[u8] = embed::bytes("assets/font.png")
val HELP: str = embed::text("help.txt")

@comptime
val LEVELS: [Level; 12] = parseLevels(embed::text("levels.txt"))

assert(FONT.len() > 0)
```

## What the target is

Three constants the compiler writes say what machine the program is being
compiled for: `TARGET_OS` is `macos`, `linux` or
`windows`; `TARGET_ARCH` is `arm64` or `x86_64`; `TARGET_VENDOR` is `apple`
on a Mac and empty elsewhere.

```wip,run
fn main() = {
    val separator = if TARGET_OS == "windows" then "\\" else "/"
    assert(separator == "/" || TARGET_OS == "windows")
    assert(TARGET_VENDOR == "apple" || TARGET_OS != "macos")
}
```

They are for the places where splitting an item with `@target` is more than
the difference is worth. The branch that cannot be taken is folded away,
but it still has to compile — which is the opposite trade to `@target`, and
the reason both exist.

## The other modules

| Module | What is in it |
|---|---|
| `std::args` | `Arguments`, `Argument` and `ArgumentError`: the program's arguments, read as options |
| `std::io` | `print`, `println`, `printInt`, `eprintln`, `flush`, `stdin`, `Reader`, `env`, `IoError` |
| `std::fs` | whole files, reading a directory, and what the filesystem says about a path |
| `std::path` | a path joined, and taken apart: its name, parent, extension and stem |
| `std::c` | a value lent to C as a callback's `void *`, and had back in the callback; `Pinned`, a value C holds until it lets go; a slice's address for C to read; C's `sizeof` and `_Alignof` |
| `std::embed` | a file's bytes or text, read when the program is compiled, as a constant |
| `std::sync` | `Atomic<T>` and `Mutex<T>`: what several threads may change at once |
| `std::shared` | `Shared<T>` and `Weak<T>`: a value several owners hold and read |
| `std::net` | `Listener`, `Stream` and `Address`: TCP and Unix-domain connections |
| `std::math` | `PI`, `TAU` and `E`; the functions are methods of `f64` and `f32`, `x.sin()`, written in Wip so that a `@comptime` constant computes the program's bits |
| `std::text` | `ParseError`, and the `Split`, `Lines` and `Chars` types the prelude's methods answer |
| `std::iter` | `Walk`, `Taking` (what a `Vec` gives up to `for x in move v`), and the adapter types `Mapped`, `Filtered`, `Enumerated`, `Taken`, `Skipped`, `Zipped` |
| `std::collections` | `Map`, `Set`, `MapEntry`, `Deque`, and `Arena` with its `Handle`, `ArenaEntry` and `ArenaValues` |
| `std::time` | `Duration`, a length of time; `Instant`, a moment on a clock that only goes forward; `sleep`; `now`, the date it is, and `Utc`, a moment as a calendar has it |
| `std::libc` | the C library as std calls it: `printf`, `fwrite`, `stdout`, `read`, `open` and its flags, `errno()` — C's, promising nothing C does not |

```wip,run
import std::io
import std::math

fn main() = {
    io::println("a line on standard output")
    assert(math::TAU > 6.28)
    assert((2.0).sqrt() > 1.41)
}
```

`std::time` measures and waits: a `Duration` is made in
any unit and read back in any, adds and compares, and is written in the
largest unit it fills; an `Instant` is a moment on a clock that only goes
forward, which means nothing alone and something against another:

```wip,run
import std::time::{Duration, Instant, sleep}

fn main() = {
    val start = Instant::now()
    sleep(Duration::millis(5))
    assert(start.elapsed() >= Duration::millis(5))
    assert("\(Duration::millis(1500))" == "1.5s")
}
```

The date it is is another clock: `time::now()` answers the seconds since
1970 as the system says, which may be set back, and a `Utc` is such a
moment as a calendar has it — year, month, day, hour, minute, second and
weekday, in UTC, with no time zone and no leap second. It
is written as HTTP and as ISO 8601 write it, and HTTP's form is read back:

```wip,run
import std::time
import std::time::{Utc}

fn main() = {
    assert(time::now() > 0)
    val moment = Utc::of(784111777)
    assert(
        moment.year() == 1994 && moment.month() == 11 && moment.weekday() == 0,
    )
    assert(moment.httpDate() == "Sun, 06 Nov 1994 08:49:37 GMT")
    assert(moment.iso() == "1994-11-06T08:49:37Z")
    assert(Utc::parseHttpDate("Sun, 06 Nov 1994 08:49:37 GMT") == .Some(moment))
    assert(Utc::parseHttpDate("Sunday, 06-Nov-94 08:49:37 GMT").isNone())
}
```
