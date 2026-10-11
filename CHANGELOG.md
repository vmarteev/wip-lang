# Changelog

Each version published, newest first.

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
