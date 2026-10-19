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
share everything: a name declared in one is a name in all of them. A test
file, `name.test.wip`, is the one exception: it sees all its module
declares, and what it declares is its own (see "Tests", page 12). The
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

A file whose name ends `.macos.wip`, `.linux.wip`, `.windows.wip`,
`.ios.wip` — or with an architecture, `.arm64.wip`, `.x86_64.wip`, or with
`.apple.wip`, which is a Mac's and iOS's together — is compiled only for
that target, as a `.test.wip` file is compiled only by `wip test`. The two
compose: `board.linux.test.wip` is a test for Linux. `.macos.wip` is a
Mac's alone, so what a Mac and an iPad share, as Metal, is `.apple.wip`. A
name whose last piece is not a target the compiler knows says nothing, so
`v1.2.wip` is an ordinary file.

An item may say the same thing with `@target(os = "linux")`, which [page
11](11-c.md) shows; a file is for the places where most of it differs. What
another target takes is checked by asking for that target: `wip check
--target linux` on a Mac ([page 12](12-tools.md)).

## Constants a build defines

A build is given settings with `--define NAME=VALUE`: a release without
its editor, a renderer built with its validation, a demo with half its
world. What a build may set is a constant that says `@define`, of a
`bool`, an integer, a float or a `str`, with the value it has where no
build sets it written where it is; a program reads it as any constant.
Where a condition is a constant — a define, `TARGET_OS`, `!`, `&&`, `||`,
`==` and `!=` of those, and a constant written as one of them — the branch
it never takes is checked, as all code is, and not compiled: what only it
calls is not in the program, in a debug build as in a release.

```wip,run
import std::io

@define
val EDITOR: bool = true

@define
val ZONE_SIZE: i64 = 32

// Built of a define and a target: a constant, which decides as they do.
val EDITING: bool = EDITOR && TARGET_OS != "ios"

fn openEditor(): str = "the editor"

fn main() = {
    if EDITING then io::println(openEditor())
    assert(ZONE_SIZE == 32)
}
```

```sh
wip build --release --define EDITOR=false main.wip
```

A define names a constant by its name, or by its module's path where two
declare one, `--define world::DEBUG=true`; a library's are set the same
way. A name no constant declares, a value not of its type — `true` or
`false`, an integer in its range, a float, any text for a `str` — and a
name given twice are refused (E0368), so a misspelt define is never
quietly the default. A define decides branches, not which items there are:
both sides are always checked, which is what keeps the one a build leaves
out from rotting, where `@target` drops what another machine would not
compile.

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

An item is a function, a type, an interface or a constant, and a path names
any of them where its kind is expected: `shapes::Square` as a type,
`shapes::area(…)` as a call, `T: shapes::Area` as an interface ([page
8](08-interfaces.md)). A full path needs no import: `std::math::PI_F32`
names the constant in any file, and the module is read for it as an
imported one is. An import binds names for the file it is written in, not
the module: each file says what it uses by a shorter name.

## What `pub` means

Everything is private to its module unless it says `pub`. That holds for
functions, types, interfaces, constants, fields, methods and variants of the
types that have them. A private name is reachable from every
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

A private field is often named as the method that reads it. In its module,
`self.count` is the field; everywhere, `c.count()` is the method. Another
module has no field to read, so `c.count` is refused there, with the fix to
call `count()`:

```wip,run
import counter::{Counter}

fn main() = {
    var c = Counter::new()
    c.advance()
    assert(c.count() == 1)
}

// file: counter/counter.wip
pub struct Counter {
    count: i64
}

extend Counter {
    pub static fn new(): Counter = Counter(count: 0)

    pub fn count(): i64 = self.count

    pub var fn advance() = {
        self.count += 1
    }
}
```

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
- **`@files("levels", "help.txt")`** names the files and directories,
  from the package's root, that the package's code reads while the
  program runs, a library's as a program's: each build puts them beside
  the program, and `package::besideProgram("levels")` is where the
  package's code finds them ([page 12](12-tools.md)). A program's own are
  at the same paths beside it as in its root, and a dependency's under a
  directory of its name, `engine/shaders/…`, which `package::besideProgram`
  knows: the engine's code asks for `"shaders"`, whether the engine is the
  program, as its tests are, or what a game depends on. Each must be the
  package's own — no whole path, nothing outside its root — and hold no
  `.wip` file, since what a package reads is not its code; a program's
  own may not begin with a dependency's name, the directory beside it
  that is the dependency's. A path written out to
  `package::besideProgram` must be one `@files` names, or below one. A
  file read when the program is compiled is embedded instead ([page
  10](10-library.md)): what is part of the code, as a table it is written
  against; a file, for what is changed apart from the code, as a game's
  pictures and shaders.
- **`@app(id = "com.example.game", name = "The Game")`** says the package
  is an app, which a build for iOS makes a bundle of
  ([page 12](12-tools.md#ios)): `id`, which it must say, is its
  identifier, reverse-DNS; `name` is what the home screen shows, the
  package's name if not said; `devices` is `ipad`, `iphone` or `both`;
  `orientation` is `landscape`, `portrait` or `any`; `icon` is a square
  PNG of the package, which the build makes each size of; `minimum` is the
  least iOS version it runs on, `"17.0"` if not said. Its version is the
  package's `@version`. A build for a Mac or Linux does not read it.
- **`wip build game`** builds the package whose root is `game/`, with what
  it depends on, from source, into one program; `wip test game` runs
  `game`'s own tests. A package with a `main` is a program, and one
  without is a library; a program may depend on another's modules.
- **A directory with no `package.wip`** builds as a program of its entry
  file's directory, depending on `std` alone.

## The prelude

Some names are in scope in every file without an import: the types
`Option`, `Result`, `Ordering`, `Vec`, `String`, `InlineVec`, `Range`,
`Hasher`, `Align`, `InRadix`, `WithPrecision` and the tuple structs; the
interfaces `Eq`, `Ord`, `Hash`, `Text`, `Clone`, `Items`, `Iterator`,
`IntoIterator`, `Sequence`, `Index`, `Destroy` and `From`, the operators'
`Add`, `Subtract`, `Multiply`, `Divide`, `Remainder`, `Negate` and `Not`,
the numbers' `Zero` and `One`, and `Encode`, `Decode`, `Encoder` and
`Decoder`; the functions `destroy` and `cloneOf`; and `assert` and
`panic` as forms of the language.

The prelude is the outermost scope, read only for a name found nowhere
nearer: a name a module declares, or imports, is the module's own, of
whatever kind, and the prelude's of that name is reached there by its
path, `std::prelude::Text`. What the compiler writes — an interpolation, a
tuple, `x[i]`, a `for`, an operator, `?`, a derive — is the prelude's in
every module, whatever the module names. So the prelude grows without
breaking a program, and a program may use the words it happens to hold:

```wip,run
/// A line of glyphs: this module's `Text`.
struct Text {
    glyphs: i64
}

extend Text: std::prelude::Text {
    fn appendTo(out: &var String) = out.push("\(self.glyphs) glyphs")
}

fn main() = {
    val line = Text(glyphs: 3)
    assert("\(line)" == "3 glyphs") // interpolation: the prelude's `Text`
}
```

The tuples are the one exception: `(a, b)` is written as the prelude's
`Tuple2` by its name, so `Tuple2`, `Tuple3` and `Tuple4` may not be a
module's own.

```wip,error=E0345
struct Tuple2 {
    first: i64
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

A row of a table may hold a list whose length differs from row to row. An
array written in a constant where a slice is wanted — a view's field, a
constant of type `&[T]`, an element of an array or a tuple — is kept once,
beside the table, and the slice points at it. It borrows nothing, as a
literal's text does, so a row and its list may be lent and walked anywhere.
A list holds what a constant may, lists among them, and not what owns
memory:

```wip,run
view struct Command {
    name: str
    keys: &[str]
}

val COMMANDS: [Command; 3] = [
    Command(name: "quit", keys: ["q", "Q"]),
    Command(name: "top", keys: ["g g"]),
    Command(name: "redraw", keys: []),
]

val ARROWS: &[str] = ["up", "down", "left", "right"]

fn commandOf(key: str): Option<str> = {
    for command in COMMANDS {
        if command.keys.contains(&key) then return .Some(command.name)
    }
    return .None
}

fn main() = {
    assert(commandOf("Q") == .Some("quit"))
    assert(commandOf("x") == .None)
    assert(COMMANDS[2].keys.isEmpty() && ARROWS.len() == 4)
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
`str`, and tables of them, not what owns memory, and not a slice, which
would point into memory the compiler used while it ran; a list is kept only
where it is written. Such a constant is not known while types are checked,
so it cannot be an array's length.

A constant may belong to a type: a `val` written among the type's
members — in its body, beside its fields and methods, or in an `extend`
block — is reached through the type, `Type::NAME`, as a static function
is, as the numbers' limits are, `i64::MAX`. It is a constant as one at the top level is, with the same rules,
`pub` to be used outside its module. Its name is one of the type's
members, which no method, static function or variant of it may share:

```wip,run
struct Money {
    cents: i64

    /// None at all.
    pub val ZERO: Money = Money(cents: 0)

    pub fn plus(other: Money): Money = Money(cents: self.cents + other.cents)
}

extend Money {
    pub val DOLLAR: Money = Money(cents: 100)
    val COINS: [i64; 4] = [1, 5, 10, 25]
}

fn main() = {
    val total = Money::ZERO.plus(Money::DOLLAR)
    assert(total.cents == 100 && Money::COINS[3] == 25)
}
```

A block that implements an interface holds only what the interface
declares, and an interface declares no constant; and a type with
parameters has none, since `Pair::ORIGIN` would not say which `Pair` it
is (E0370):

```wip,error=E0370
struct Pair<T> {
    a: T
    b: T

    val ORIGIN: i64 = 0
}

fn main() = {}
```
