# Changelog

Each version published, newest first.

## 0.5.0 — 2026-10-13

Programs written for 0.4 compile as they did; where they write `move`
that is now inferred, a warning says so, with the fix that removes it.

**The language**

- **A value that owns memory moves where it is used as a value:** an
  argument, a field, `return`, an arm's value. `move` is no longer
  written there, and a use after the move is refused at the place it was
  given, "`report` is given here, and used again". `move` stays where a
  place is only lent or aliased, in `match move`, `for … in move` and a
  pattern's value; where it is written and inferred, it is a warning
  (E0451) whose fix removes it.
- **`when`, the first condition that holds:** `when { c.isDigit() =>
  number() … else => unexpected() }`, a choice among conditions, each
  arm a line, an `is` binding for its own arm. A choice among the values
  of one thing stays a `match`.
- **A local's type arguments from its first use:** `var shapes = Vec()`
  and then `shapes.push(circle)` is a `Vec<Shape>`, decided by the next
  place that names it — a method's arguments, an expected type, an
  assignment.
- **`become f(x)`** leaves the function and calls `f` in its place, a
  tail call that is guaranteed, so functions that call each other run in
  one frame: an interpreter's dispatch, a state machine. The callee has
  the caller's signature, and nothing owned or deferred is left to end.
- **A method may say `keeps self`:** a call may keep, in its `&var`
  arguments, what the receiver holds or borrows, as a render queue keeps
  views of a scene: `scene.drawn(camera, &var draws)`.
- **A literal in a tuple takes the type of the tuples beside it** in an
  array, as a literal element does: `[(0.0, -reach), (-reach, 0.0)]`.
- **A module named by a full path needs no import:**
  `std::math::PI_F32` is found whether or not another file imports
  `std::math`.
- **A struct's, an enum's and an interface's constraints** may name an
  interface imported by name, `struct Held<S: Shelf>`, as a function's
  could.
- **Type arguments are inferred through what an iterator hands out:**
  `Map::fromPairs(…)` needs no type written.

**C**

- **A plain value's bytes:** `c::bytesOf(&value)`, borrowing it, for a
  type with no padding and no address in it — a GPU's constants, a hash,
  a file.
- **A C struct a Wip function answers borrows what its signature lends,**
  so a function may return C structs that point into its arguments, as a
  list of draws or descriptor writes built for a call does.
- **Assembly in a file:** `@source` takes `.s` files, and `.S` files
  given the module's `@define`s.
- **`wip bindgen`** types an enum's enumerators alike whichever clang it
  asks, and asks the compiler `$CC` or `$WIP_CLANG` names, as `wip`
  builds with.

**The library**

- **Results collected:** `Result::collect(results)` and
  `Option::collect(options)`, every value or the first failure.

**Tools**

- **Hover** on a place given away says so: it owns memory, so it is
  moved, never copied.
- **The compiler and the library are written with `when`** and without
  the `move`s now inferred.

## 0.4.0 — 2026-10-12

Programs written for 0.3 may need changes where marked **breaking**.

**The language**

- **Implicit parameters.** `fn roll(implicit dice: &var Dice)` is given,
  where a call leaves it out, the caller's own implicit of that name: one
  of its implicit parameters, or an `implicit val` or `var` in scope. What
  a function needs is still in its signature; what is no longer written
  is the same name at every call that only passes it on.
- **A struct taken apart without its name,** by its fields' names in
  parentheses, `val (x, y) = here`, as a tuple is by position.
- **A field and a method may share a name:** `x.name` is the field, and
  `x.name()` the method.
- **A guard's `else` may be one jump or expression,** as an `if`'s
  `then`: `val .Some(x) = found else return .None`.
- **A literal takes the type of what is beside it** in an `if`'s other
  branch, a `match`'s other arms and an array's other elements, as it
  already did beside an operator: `if h == 0 then 1.0 else w as f32 / h
  as f32` is an `f32`.
- **An integer constant takes the integer type expected of it** where its
  value fits, and is refused where it does not: a C library's `c_int`
  flags are given where a field takes a `u32`.
- **What a `for` loop's head makes lives as long as the loop,** whatever
  it walks: `for (i, x) in TABLE.forwards().enumerate()` and
  `for w in words().forwards()`. **Breaking:** an iterator over a
  temporary, or over what a method of one lends, kept in a variable past
  its statement is refused; it compiled before, and walked freed memory.
- **A test may expect a panic:** `@test(panics)`, or `@test(panics =
  "text")` for one whose message holds the text.
- **`@align(n)` and `@packed`** lay out a struct, or a field, as a GPU's
  buffers or a file's format do, and `c::offsetOf<T>("field")` says where
  a field lies.
- **A list in a constant** is kept once beside it, where a slice is
  wanted.
- **A reference parameter passed to a generic function** says what its
  type argument is, as it does to any other function.
- **Soundness. Breaking:** each of these compiled before, and read or
  wrote memory it did not own. A `&` of a place behind a reference
  parameter, stored in a `&var` one, holds what that reference holds; a
  `var` method called on what a call answered through a `&` is refused,
  and through a `&var` lends it again.

**C**

- **A call lends a place where C takes a pointer:** `&var count` for C to
  write, `&x` for it to read, an array or a slice as its first element's
  address, and `null` where C allows none. Out-parameters need no array
  of one.
- **A C struct's pointer field takes `&x` in its literal,** and the
  struct borrows `x`: one used after `x` changed, or returned pointing at
  a local, is refused.
- **A `cstring` may be null,** as C's may: `null` is one, `isNull` and
  `== null` ask, and reading a null one as text panics. **Breaking:**
  `libc::getenv` answers a `cstring`.
- **`&p[a..b]` is a slice of what a pointer points at,** as many elements
  as the range says: memory a GPU maps, an image a decoder answers.
- **An opaque C type may be a type argument,** of a generic that only
  points at it; an instance that would hold a value of it is refused.
- **C++ is built with a module:** a module's `.cpp` files are compiled as
  C++, and `.mm` files as Objective-C++ on Apple's systems, with the C++
  library linked.
- **C is written only for what a program calls or names,** so a binding
  of a newer header builds against an older one.
- **A module's directory is searched for `#include "…"` alone.**
  **Breaking:** C that included a module's own header with `<…>` names it
  with quotes; a system's header is never a module's file of the same
  name.
- **`wip bindgen`** reads a pointer that is itself `const`, arrays of
  arrays, a `#define` past an `i64`, `_Nullable` and `_Nonnull`, and the
  structs another header declares; takes `-I` for the header's own
  includes and `--pointer NAME` for a parameter that is a pointer for a
  reason the header does not say; and writes an enumerator past an `int`
  and a `static const` number as constants. **Breaking:** a binding
  written again takes a struct C writes, or an array after its count, as
  a `ptr<T>`, which a call gives `&var x`, an array or `null`.

**The library**

- **Encoding:** `@derive(Encode, Decode)` writes a value to JSON or to
  `std::bytes`' binary format, and reads it back by its fields' names.
- **Maths:** `std::math`'s vectors, matrices and quaternions, laid out as
  a GPU reads them, with projections for each depth range; `Noise`;
  `lerp`, `smoothstep` and the rest on `f32` and `f64`; `Random` draws
  floats, directions, points, weighted choices and gaussians.
- **A game's loop:** `time::FixedStep`, updates at a fixed rate whatever
  the frames are; `future::Pool`, threads started once.
- **Memory:** an allocator for a call, `mem::using`, and `mem::Arena`, a
  frame's arena; `mem::onHeap`.
- **Atomics** in every order, of floats and of pointers, and fences.
- **`own<T>`** is extended as `T` is, and clones where `T` does;
  `Vec.splitOff` and `Vec.append`.

**Tools**

- **`wip build -o`** makes the directory it writes into.
- **The compiler is faster** in a release build: a cycle of calls is not
  inlined into itself, and a struct taken by value is read where it is.

## 0.3.1 — 2026-10-10

Programs written for 0.3.0 may need changes where marked **breaking**.

- **The gate passes on Linux on x86-64.** Its check of `wip check
  --dump-hir` counted, as the program's own, the line saying C's maths
  library is linked, which only Linux on x86-64 links; 0.3.0's
  verification failed there.
- **A C string made from a `String` is checked as a `str` is.**
  **Breaking:** `s.toCstring()` borrows `s`, and one used after `s`
  changed — another `toCstring()` among the changes, since it may move
  the bytes — kept past a `String` that was a temporary, or returned
  borrowing a `String` that ends, is refused; each compiled before and
  read freed memory. One from C, or a literal, is free as before, and
  a value that holds a C string borrows what it does.
- **A struct pattern's binding is written through,** as a variant's is:
  `match p { Point(x, ..) => x += 10 }` writes `p.x`, where it was
  refused.
- **A byte answers what C's `<ctype.h>` does,** by the names a `char`
  has: `isDigit`, `isHexDigit`, `isAsciiLetter`, `isAsciiAlphanumeric`,
  `isAsciiUppercase`, `isAsciiLowercase`, `isAsciiWhitespace`,
  `isAsciiPunctuation`, `isAsciiPrintable`, `digitValue(radix)`,
  `toAsciiUpper` and `toAsciiLower`; a `char` has `digitValue` too.
  **Breaking:** `isAsciiWhitespace` takes the vertical tab and the form
  feed, as C's `isspace` and `char.isWhitespace` do, and so does `trim`.
- **Text and containers:** `str.startsWithAnyCase`, `toAsciiUpper` and
  `toAsciiLower`; `atLast()`, the last value of a `Vec` or a slice to
  read or to write; `takeAll()`, everything a `Vec`, `String`, `Map`,
  `Set` or `Deque` holds, leaving it empty.
- **Local time:** `time::Local`, a moment in the time zone the program
  runs in, read as `Utc` is and written as ISO 8601 writes it with its
  offset.
- **Text compared for equality by its length first,** so that most
  comparisons that are false call nothing, and a `match` on text becomes
  a switch on its length.
- **The compiler is faster:** checking itself takes a quarter of the time
  it did, and a debug build after an edit a fifth less, with each
  function's MIR made on threads ahead of its code.
- **`wip fmt` counts the columns a line takes,** not its characters, so a
  line of wide characters is broken where it reaches the width.
- **The README says how to install Wip,** from a release or from a
  checkout.

## 0.3.0 — 2026-10-10

Programs written for 0.2 may need changes where marked **breaking**.

- **The compiler is written in Wip.** It is built from its source with a
  `clang` 15 or newer and nothing else: the repository keeps it for each
  system as LLVM's bitcode, which `clang` compiles into a first `wip`, and
  that one builds the compiler, which builds itself again
  (`scripts/bootstrap.sh`). No Rust is needed to build or to work on Wip.
- **`wip` is a directory:** `bin/wip`, and beside it the standard library,
  `std/`, and the tools, `tools/`, which it reads from where it is
  installed rather than carrying them inside. Each system's is published
  as an archive, `wip-0.3.0-<system>.tar.gz`; `WIP_HOME` names another
  home.
- **Every build is LLVM's.** **Breaking:** a `clang` 15 or newer is
  needed for a debug build too, and `--backend` is gone. A debug build is
  compiled in parts at `-O0`, at once and kept in the cache, so that a
  build after an edit compiles only what the edit changed.
- **What a call keeps:** a call keeps nothing of its arguments unless its
  signature says `keeps`, `var fn add(word: str) keeps word`, and a body
  is held to it. **Breaking:** a function that stores a parameter in a
  `&var` one says so; the library's methods that store, `Vec`'s `push`
  among them, do.
- **Another module's interface,** imported or named by its path: `T:
  shapes::Area`, `&dyn shapes::Area`, `extend Rect: shapes::Area`.
- **A line that begins with `|` continues a pattern's alternatives.**
- **A `str` field of a view lends as a `&` field does,** so a method may
  answer the text it reads `from self.source`.
- **A variable holding a function value is captured** by a closure as any
  variable is; `&` of a function value is refused, since a closure is
  expected there.
- **Programs talked to:** `Command::connect()` starts a program with its
  input and output piped, written and read as it runs; `Command.unset`
  leaves a variable out of its environment; `io::Reader.setTimeout` says
  how long a read waits. A program started is given none of the files,
  sockets and pipes this one has open.
- **`embed::textOr(path, otherwise)`,** a file's text where it is there;
  **`future::cores()`,** how many processors the work is spread over.
- **The tools:** `wip build --emit llvm-ir` and `--emit llvm-bc` write the
  program's LLVM IR, as text or as bitcode; `--no-debug-info` writes no
  debug information; `wip cache clean` empties the cache; `wip check
  --dump-diagnostics` and `--dump-hir` write what was found and what was
  checked for a tool; `wip mir` prints what a build compiles, `--release`
  as a release build does.
- **Fixes:** what a match guard makes is dropped where the guard ran; a
  reference loaded from memory is no longer promised to LLVM as one it may
  read early; an `i128` product's overflow is checked without a helper
  GCC's runtime lacks; a generic function given `void` for a type
  compiles, where the backend panicked; an `is` binding after `&&` in a
  loop is bound afresh on each pass, not moved on the pass before; a
  `return move` in a `for` body is not a move on the next turn;
  alternatives on a tuple matched in place are each tried; a `String`
  lent through a reference lends what it refers to.

## 0.2.0 — 2026-10-07

Programs written for 0.1 may need changes where marked **breaking**.

- **Types an implementation decides:** `interface Iterator<type Item>`.
  A constraint may leave such a type out, `I: Iterator`, and generic code
  reads it as `I::Item`; a concrete type's is its implementation's,
  `Chars::Item`. A type implements such an interface once, and a second
  implementation is refused where it is declared. `Iterator`, `Items`,
  `Sequence`, `IntoIterator` and `Index` decide their element.
  **Breaking:** `std::iter`'s adapters hold only the iterator,
  `Mapped<I, U>` where it was `Mapped<I, T, U>`.
- **Every field named where there are two or more.** A struct or a
  variant is built under a call's rules; a bare variable no longer stands
  for `field: variable`. **Breaking:** `File(fileNo, name: n)` is
  `File(fileNo: fileNo, name: n)`. A struct of one field may still be
  built by position, `Path(text)`.
- **A result that says what it borrows:** `fn text(i: i64): str from
  self.ast`, so a caller may change the rest while it holds the answer. A
  place reached through a view's `&` field borrows what that field does,
  not the view.
- **Alternatives that bind:** `.Circle(radius: n) | .Square(side: n) =>
  n`, after `val` too.
- **A pattern tests through a reference at any depth,** so a `&Option<T>`
  or a `&(A, B)` matches as the value would.
- **Text where a `String` is expected is copied into one,** wherever a
  literal already is: arguments, fields, results and variables.
  `String::of` is needed less.
- **An interface extended under a condition on its types,** `extend
  Iterator<T: Ord> { … }`: `sum`, `product`, `min`, `max` and `toSet` are
  written so.
- **A range of a vector,** `v[a..b]`, as of an array.
- **A type's functions as values,** `Type::name`, and a plain function
  where an owned closure is expected.
- **A place is lent where it is used,** at the write or when the call
  begins, so `self.nodes[n].cond = .Some(self.expr(&var t))` is accepted
  though `expr` may grow `nodes`.
- **A struct with no fields is written without braces,** `struct Csv`, and
  `wip fmt` writes it so; a file that says `// wip fmt: off` is left as
  written.
- **Arguments passed in each other's places are warned of:**
  `draw(height, width)` to a `draw(width, height)`; naming them says it
  is meant.
- **The library:** an iterator's `flatten`, `flatMap`, `filterMap`,
  `chain`, `takeWhile`, `position`, `nth`, `last`, `findMap`, `forEach`
  and `toSet`, `Map::fromPairs`, and `forwards()` beside `backwards()`; a
  stable `sort`, `sortBy` and `sortByKey`, and `dedup`; checked and
  saturating arithmetic; maps and sets compared; faster hashing; `replace`
  and `swap` in `std::mem`; files, processes and the terminal; reading
  exactly so many bytes; more of text; SHA-256; `std::random`; numbers
  written to a precision or in a radix; `void` compares, hashes and is
  written.
- **C:** a function pointer that may be null, as a parameter or a result,
  and a `bool` that C hands over read as C reads one, nonzero true.
- **Fixes:** a guard's parentheses before an arm's `=>` are not a lambda;
  `wip fmt` keeps a comment in an `if` given as a value; a debug build on
  macOS packs its debug information.

## 0.1.1 — 2026-10-05

- **The language server survives a line half typed.** It died at
  `String::of` typed without its call, and an editor went on showing the
  diagnostics of the keystroke before. Four panics of the checker are
  fixed, three of which a finished program could reach too:
  - `Type::name` not called is refused, as a method named as a value is
    (E0337), with the call to write and a lambda that stands in for it;
  - an enum's name interpolated, `"\(Json)"`, is a type used as a value;
  - a lambda in a `@comptime` constant runs:
    `@comptime val SQUARES = table((i) => i * i)`;
  - a module whose last line is half written is reported at its end.
- **`std::future`'s channels** hold their queue as a `Shared`, and have
  tests of their own.
- **CI passes on Rust 1.99,** whose clippy found two lints.
- **The scripts start with their interpreter,** and `scripts/snapshots.sh`
  runs on Linux.

## 0.1.0 — 2026-10-05

The first public version: the prototype compiler for Wip, its standard
library, `wip fmt`, `wip doc`, `wip test`, `wip bindgen`, the language server
`wip lsp`, and support for Zed and Neovim. Apple arm64, and Linux on arm64
and x86-64.
