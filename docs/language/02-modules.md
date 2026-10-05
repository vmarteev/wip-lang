# 2. Files, modules and names

## A file

A file is a list of items — functions, types, interfaces, `extend` blocks,
constants, `extern` blocks and imports — in any order. Nothing is declared
before it is used, since a file is read as a whole: a function may call one
written below it.

Statements do not stand at the top level; a program starts at `main`,
which is [page 12](12-tools.md).

## A module is a directory

Every `.wip` file in a directory is one module, and the files of a module
share everything: a name declared in one is a name in all of them. The
program's own module is the directory the entry file is in; every other
module is a directory below it.

```wip,run
import shapes::{Square, area}

fn main() = {
    assert(area(Square(side: 3)) == 9)
}

// file: shapes/square.wip
pub struct Square {
    pub var side: i64
}

// file: shapes/area.wip
pub fn area(square: Square): i64 = square.side * square.side
```

A module below another is named through it: `geometry::shapes`. Modules
may not import each other in a circle, and the compiler says where the
circle is.

## A file may be for one target

A file whose name ends `.macos.wip`, `.linux.wip`, `.windows.wip` — or with
an architecture, `.arm64.wip`, `.x86_64.wip` — is compiled only for that
target, as a `.test.wip` file is compiled only by `wip test`. The two
compose: `board.linux.test.wip` is a test for Linux. A name whose last piece
is not a target the compiler knows says nothing, so `v1.2.wip` is an
ordinary file.

An item may say the same thing with `@target(os = "linux")`, which [page
11](11-c.md) shows; a file is for the places where most of it differs. What
another target takes is checked by asking for that target: `wip check
--target linux` on a Mac ([page 12](12-tools.md)).

## Imports

An import names a module, or items from one:

```wip,run
import shapes // the module, used as `shapes::…`
import shapes::{Square} // the item, used by its name
import shapes::{area as areaOf} // renamed here
import shapes::sizes::{self} // the module below it, as `sizes::…`

fn main() = {
    assert(areaOf(Square(side: 2)) == 4)
    assert(shapes::area(Square(side: 2)) == 4)
    assert(sizes::biggest() == 9)
}

// file: shapes/shapes.wip
pub struct Square {
    pub var side: i64
}

pub fn area(square: Square): i64 = square.side * square.side

// file: shapes/sizes/sizes.wip
pub fn biggest(): i64 = 9
```

An import is for the file it is written in, not the module: each file says
what it uses.

## What `pub` means

Everything is private to its module unless it says `pub`. That holds for
functions, types, constants, fields, methods and variants of the types that
have them. A private name is reachable from every
file of its own module and from nowhere else:

```wip,error=E0210
import counter::{Counter}

fn main() = {
    val c = Counter(count: 1)
    assert(c.count == 1)
}

// file: counter/counter.wip
pub struct Counter {
    count: i64
}
```

`pub` on a field exports the *reading* of it. A module that wants everyone
to be able to write one says `pub var`, and a field with only `pub` is
written by the module that declares it — including where another module
builds the value from a literal:

```wip,error=E0210
import counter::{Counter}

fn main() = {
    var c = Counter::new()
    c.count += 1
    assert(c.count == 1)
}

// file: counter/counter.wip
pub struct Counter {
    pub count: i64
}

extend Counter {
    pub static fn new(): Counter = Counter(count: 0)
}
```

A `pub` type with private fields is made by the module that declares it and
used by everyone else through its methods, which is how the standard
library's `Vec`, `Map` and `String` are written.

## Packages

A package is a tree of modules with a `package.wip` at its root, which
names it, versions it and says what it depends on. Two programs share a
library by both depending on it:

```wip,run
import std::io
import engine
import engine::render

fn main() = {
    assert(package::NAME == "game")
    assert(package::VERSION == "1.4.0")
    assert(render::frame(4) == 40)
    // `package::` in `engine`'s code means `engine`.
    assert(engine::version() == "0.3.0")
    io::println("\(package::NAME) \(package::VERSION)")
}

// file: package.wip
@version("1.4.0")
@depends("engine", path = "libs/engine")
package game

// file: libs/engine/package.wip
@version("0.3.0")
package engine

// file: libs/engine/engine.wip
pub fn version(): str = package::VERSION

// file: libs/engine/render/render.wip
import internal::pool

pub fn frame(n: i64): i64 = n * pool::size()

// file: libs/engine/internal/pool/pool.wip
pub fn size(): i64 = 10
```

- **`package.wip`** is annotations and `package NAME`, and nothing else.
  `@version` is a semantic version, `MAJOR.MINOR.PATCH`. `@depends` names a
  dependency and says where its root is, relative to this package's; the
  dependency's own `package.wip` must give it that name.
- **A dependency's name begins a path,** as `std` does: `import
  engine::render`. A package's own modules are named from its root, as
  every program's are.
- **`internal/`:** a module under a directory named `internal` is its
  package's own. `engine`'s modules import `internal::pool`; no other
  package may.
- **`package::NAME` and `package::VERSION`** are constants of the package
  the code is written in, known when it is compiled.
- **`wip build game`** builds the package whose root is `game/`, with what
  it depends on, from source, into one program; `wip test game` runs
  `game`'s own tests. A package with a `main` is a program, and one
  without is a library; a program may depend on another's modules.
- **A directory with no `package.wip`** builds as a program of its entry
  file's directory, depending on `std` alone.

## The prelude

Some names are in scope in every file without an import: the types
`Option`, `Result`, `Ordering`, `Vec`, `String`, `Slots`, `Hasher`,
`Align` and the tuple structs; the interfaces `Eq`, `Ord`, `Hash`, `Text`,
`Clone`, `Items`, `Iterator`, `IntoIterator`, `Sequence`, `Index`,
`Destroy` and `From`, and the operators' `Add`, `Subtract`, `Multiply`,
`Divide`, `Remainder`, `Negate` and `Not`; and `assert` and `panic` as
forms of the language. That list is closed and the compiler knows it: a
module may not declare or import a name the prelude has.

```wip,error=E0345
struct Option {
    taken: bool
}
```

Everything else the library offers is imported: `std::io`, `std::math`,
`std::collections`, `std::iter`, `std::text`, `std::fs`. [Page
10](10-library.md) is what is in them.

## Constants

A `val` at the top level is a constant: it is computed where it is used, and
its value must be one the compiler can work out. A constant
is private unless it says `pub`.

```wip,run
pub val LINE_LENGTH: i64 = 5
val NAME = "wip"

fn main() = {
    assert(LINE_LENGTH == 5)
    assert(NAME.len() == 3)
}
```

A constant may be a table: an array, a struct or tuple, or a variant, of
constants, to any depth. It cannot own memory. The
program keeps a table once, in memory nothing writes, and a use of it
reads it where it lies, so a large table costs nothing to use. It is
indexed, searched and lent as any array is; to change one, copy it first:

```wip,run
val DIRECTIONS: [(i64, i64); 4] = [(0, 1), (1, 0), (1, 1), (1, -1)]
val PRIMES: [i64; 6] = [2, 3, 5, 7, 11, 13]

fn main() = {
    assert(DIRECTIONS[3].1 == -1)
    assert(PRIMES.binarySearch(7) == .Ok(3))
    var changed = PRIMES
    changed[0] = 1
    assert(changed[0] == 1 && PRIMES[0] == 2)
}
```

A constant may be computed by the program's own code: one that calls a
function, loops or builds a value is run by the compiler, once the program
is checked, and the program keeps what it came to, as it keeps a table. It
says so with `@comptime`, which such a constant needs and one known where it
is written may not have. Any function may be run — generic ones, closures, a
`Vec` or a `String` built and dropped on the way — but not C: printing, the
time, files and threads are refused, each an error at the constant. A panic
while it runs is a compile error, with the calls that led to it.

```wip,run
@comptime
val SQUARES: [i64; 8] = squares()

@comptime
val LIMIT: i64 = nextPowerOfTwo(100)

fn squares(): [i64; 8] = {
    var table: [i64; 8] = [0; 8]
    for i in 0..8 {
        table[i] = i * i
    }
    return table
}

fn nextPowerOfTwo(n: i64): i64 = {
    var p = 1
    while p < n {
        p *= 2
    }
    return p
}

fn main() = {
    assert(SQUARES[7] == 49)
    assert(LIMIT == 128)
}
```

What it computes is what a constant may hold: numbers, `bool`, `char`,
`str`, and tables of them, not what owns memory. Such a constant is not
known while types are checked, so it cannot be an array's length.
