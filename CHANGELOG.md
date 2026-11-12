# Changelog

Each version published, newest first.

## 0.16.0 — 2026-11-12

Programs written for 0.15 compile as they did: an interpolated literal
builds the `String` it built wherever no other type is expected of it.

**The language**

- **An interpolated literal builds the type expected of it.** Where a type
  that implements the prelude's new `Template` is expected, the literal's
  own text goes to its `pushLiteral` and each hole's value to its
  `pushValue`, lent, with the hole's options as named arguments; a hole
  of the type itself is joined by `pushBuilt`. So the type decides what a
  value becomes in it: a query that binds each value as a parameter, a
  page that escapes it. A literal with no holes where a template is
  expected builds one too. `String` is a template, the one built where no
  other is expected. A hole's option or value that its `pushValue` does
  not take is refused at the hole (E0134, E0342).

**The library**

- **HTML and XML, `std::markup`.** `Html` and `Xml` are templates: every
  value written into one is escaped as its place in the page needs, with
  no call to say so. `Html` reads where each hole is: it escapes text and
  quoted attributes, writes a link's `:` as `%3A` where it would make a
  scheme that runs a script, and refuses a hole in a `<script>`, a
  `<style>`, an `on…` attribute, an unquoted value, a tag or a comment,
  with a panic at the hole. `Html` is joined only between tags;
  `Html::joined` and `Html::trusted` are there for lists and for HTML
  another writer made.
- **Secrets, `std::secret`:** `bytes` and `fill` from the system's
  generator for secrets, and `equal`, two secrets compared in a time that
  does not say where they differ, which the optimiser is kept from ending
  early.
- **Digests:** `sha1`, `sha512` and their running `Sha1` and `Sha512`
  beside SHA-256, and HMAC over each: `hmacSha1`, `hmacSha256`,
  `hmacSha512`.
- **Sorted collections:** `SortedMap` and `SortedSet`, `Map`'s and `Set`'s
  functions kept in the order of their keys, with `first`, `last`,
  `positionOf` and `within`.
- **Dates:** `Utc::at(year:, month:, day:, …)`, which refuses a day the
  month does not have; `Utc::parseIso`, RFC 3339 with any offset; `after`,
  `before` and `since`, as `Instant` has; and `addDays`, `addMonths`,
  which keeps a month's end, and `daysSince`.
- **The environment:** `io::setEnv` and `io::unsetEnv`, which panic where
  a second thread runs, since another may be reading it, and
  `io::environment()`, every variable.
- **Paths and trees:** `path::clean`, `relative`, `withExtension` and
  `matches`, a glob as git matches one; and `fs::walk`, every entry under
  a directory, with a closure that says which directories to enter.

**The tools**

- **`wip doc`** writes its pages as `Html`.

**Examples**

- **`bookmarks`:** a page and an RSS feed of bookmarks read from a file,
  built as `Html` and `Xml`, a link that would run a script among them.

## 0.15.0 — 2026-11-09

Programs written for 0.14 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead, and `wip
fix` writes the rest.

**The language**

- **A named argument whose value is the variable of its name is written
  once:** `Point(x:, y:)`, `Log(least:, write:)`, in a call, a struct's
  and a variant's build and `panic`'s `at`. `x: x` is a warning (E0485),
  and `wip fix` writes `x:`; so do the fixes that build a struct.
- **A closure lent for the call may have a default:** a parameter of
  type `&(…) => R` or `&var (…) => R` may default to a lambda that
  captures nothing or a function's name, so a builder's children, where
  most elements have none, say so once. Any other default for a
  reference is still refused (E0330).
- **A `dyn` is only behind a reference, wherever it is written:** inside
  a type's arguments, `Vec<own<dyn Shape>>`, and in a function type's
  parts, as well as in a field, a parameter or a result, and in a local's,
  a constant's and a type argument's type. The error points at the `dyn`
  and says what to write instead: an enum of the kinds, or a type
  parameter. **Breaking:** these were accepted before (E0344).

**The library**

- **A module for logs, `std::log`.** A `Log` keeps records at its least
  level or above and writes them to standard error (`Log::toStderr()`),
  to the system's log through `syslog` (`Log::toSystem()`), or to a
  closure of the program's own (`Log::to`); `Log::none()` keeps nothing.
  A function that logs takes the program's log as `implicit log: &Log`,
  and each record knows the file and line of the call that made it. One
  log is shared by every thread it is lent to.
- **A walk with positions,** `values.indexed()`, of a slice, an array
  and a `Vec`: each element lent with its index. `forwards().enumerate()`
  on a list is a warning (E0482), and `wip fix` writes `indexed()`.
- **The binary format spells each name once:** a field's or a variant's
  name is written in full the first time it comes, and as the place it
  was written each time after, so a list of a thousand records spells its
  fields' names once. Bytes written by an earlier version are read as
  they were.

**The tools**

- **Shorter forms:** `String::of(text)` of a `str` where a `String` is
  expected (E0483) and `Vec::of` of a list written where a `Vec` is
  (E0484) are warnings, and `wip fix` takes them away.
- **An empty list whose annotation was refused** is no longer also told
  it needs one (E0315).

## 0.14.0 — 2026-11-06

Programs written for 0.13 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead, and `wip
fix` writes most of it.

**The language**

- **A loop of one statement is written after `do`,** with no braces:
  `for name in names do` and the body on the next line, `while i < n do`,
  and a list's loop, `[for e in elements[1..] do yield analyze(&e)?]`.
  `do` takes a call, an assignment, a `yield`, a jump or an `if … then`
  without an `else`; a loop, an `if` with an `else` or a `match` after it
  is refused, with the fix that writes the braces. **Breaking:** `do` is
  a keyword.
- **A reference is passed on with its `&`:** a name that holds a `&T`,
  given to a call, is lent with `&` as a value of the caller's own is,
  `size(&values)`. **Breaking:** one passed by its name alone is refused
  (E0476), and `wip fix` writes the `&`.
- **`yield` is a branch,** `if c then yield x` and `.Some(v) => yield v`,
  and a list that yields where nothing expects a collection is a `Vec`;
  `own [ … ]` and `own for` still make a buffer. A value beside a branch
  that yields, which the statement would drop, is refused (E0477).
- **A list is any collection:** `val primes: Set<i64> = [2, 3, 5]`, a
  `Map` of pairs, a `String` of characters, and a type of a program's own
  that implements `Collect<T>`; `T::collect(iterator)` makes each.
  **Breaking:** `Map::fromPairs` and `PriorityQueue::fromPairs` are
  `collect`, and `Iterator.toSet()` is gone.
- **A variant of one field is a function of it,**
  `result.mapErr(HttpError::Io)`, `map(Option::Some)`; and **a generic
  function goes where a closure is expected,** its type arguments learnt
  from the closure's type.
- **A struct's fields as a value,** `c::fieldsOf<T>()`: each field's name,
  where it lies, its size and the numbers it is made of, as C lays it
  out, which a top-level `assert` checks while the program is built.

**The tools**

- **`wip fmt` puts every body of one statement on the line below its
  `then` or `do`,** and a blank line after it before the next statement;
  a guard's `else` and an `if … else` inside an expression stay on one
  line. Every program is laid out again.
- **What is never used is said:** a `var` nothing writes, a local, a
  binding and a parameter nothing reads, an import nothing names and a
  module's function nothing calls are warnings, E0470 to E0475; a program
  with errors is told none of them.
- **Shorter forms:** a lambda that only calls a function is its name
  (E0478); a `Vec` declared empty and filled by the loop after it is the
  list that yields (E0479); a list built only for `sum`, `max` or `min` is
  the generator (E0480); a loop over a slice or a `Vec` that only searches
  is `any`, `all` or `find` (E0481).
- **`wip fix` applies them all,** with what is never used but a function,
  and the `&` of E0476; `--target` fixes code for another system with the
  rest. A fix that would take a comment with a loop is not offered.

**The library**

- **A tuple of four is encoded and decoded,** as tuples of two and three
  are.

**Speed**

- **A generator walked by a loop no longer tests where it stopped on each
  step** in a release build: `zip` and `chain` written as generators are
  as fast as the library's adapters, or faster.

## 0.13.0 — 2026-11-03

Programs written for 0.12 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead.

**The language**

- **A function is told where it was called from.** The prelude's
  `Location` is a place in a program: its file, line, column and
  function. An implicit parameter of that type, `implicit at: Location`,
  is filled where nothing of its name is with the place of the call; a
  helper that takes one and calls another hands its own on, so the place
  is the first call into the chain. `panic(message, at: at)` reports that
  place, so a helper panics at the line that called it.
- **A pattern tests what a box holds, at any depth:** `.Node(left:
  .Leaf(n), ..)` where the node holds its children in `own` fields,
  `.Some(.Leaf(n))` of an `Option<own<E>>`, in `match`, `is` and `val …
  else`. A binding in a box refers to the value where it lies, and `match
  move` takes it out, freeing the box.
- **What a guard's `is` binds reaches its arm:** `.Some(entry) if
  entry.next is .Some(n) => n + 1`.
- **`match move` of a box** took what it held and then dropped it again,
  which crashed where it held a `String`; it frees the box and drops only
  what the pattern left.

**The library**

- **`unwrap`, `get`, `atLast`, `remove`, `insert`, `copyFrom` and the
  library's other checks panic at the line that called them,** with the
  library's line among the calls below. **Breaking:** a function with an
  implicit parameter is not a value, so these are no longer passed as
  one; a lambda that calls one, `(o) => o.unwrap()`, is.
- **An element or nothing,** `values.get(index)`, where `values[index]`
  would panic, of a slice, an array and a vector; and **a byte of text or
  nothing,** `text.get(index)`, by value, `line.get(at) == b'/'`.
- **A value taken out of its box,** `unbox(boxed)`, which frees the box.

**The tools**

- **`wip test` runs every test,** each in a process of its own, so a
  failure ends only its test: the run says `FAILED` under it with what it
  printed and its panic, goes on, and names the tests that failed at the
  end. With `WIP_CHECK_LEAKS`, a leak fails the test that left it.
  **Breaking:** what a test that passes prints is no longer shown, and
  tests no longer share a process, so one does not see what another left.
- **Completion after a leading dot** offers what the type expected has:
  an enum's variants, and, in a value, the type's functions that answer
  it.
- **Clearer help:** a `?` in a lambda says it leaves the lambda, not the
  function around it, and how `Result::collect` gathers many values for
  one `?`; a value behind a reference that cannot be moved is told to
  copy, where it was told to `match move`, which cannot take it either.

## 0.12.0 — 2026-10-31

Programs written for 0.11 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead.

**The language**

- **A value where an `Option` or a `Result` is expected is its `.Some`
  or its `.Ok`:** `return found` from a function answering `Option<T>`,
  `val limit: Option<i64> = 10`; a function answering `Result<void, E>`
  ends with nothing, or `return`. `.None` and `.Err(…)`
  are still written. A `.Some(…)` or `.Ok(…)` the type expected says is a
  warning, and `wip fix` takes them away.
- **An integer literal where a float is expected is that float:**
  `Color(r: 255, g: 140, b: 0, a: 255)`, `width / 2`, `if held then 1
  else 0` of an `f32`, and `[0, 0.5, 1]`. A literal the float cannot
  hold exactly, as `16_777_217` is for an `f32`, is refused.
- **A type's function named from the type expected,** `.name(…)`, as a
  variant is: `Sizing(width: .grow(), height: .fixed(60.0))`,
  `Mat4::lookAt(eye, look, .unitY())`, `val every: Duration =
  .millis(500)`.
- **An enum whose variants carry nothing compares and hashes** without
  `@derive(Eq, Hash)`, and is a map's key as it is; a `@derive` that says
  so is a warning `wip fix` takes away. It also answers **`name()`**, the
  name its variant is written with, and **`fromName(text)`**, the variant
  of that name or nothing.
- **A box compares, orders, hashes and is written as what it holds,** so
  a recursive type whose children are `own` derives `Eq`, `Ord`, `Hash`
  and `Text`.
- **A line that starts with `-`** and whose value nothing uses is
  refused as a line cut short, with the fix that joins it to the line
  above: `total = a` then `- b` on the next line computed `a`.
  **Breaking:** such a statement compiled, and did not subtract.
- **`own f` of a function's name** is refused, as `&f` is, with the fix
  that takes the `own` away; it crashed the compiler.

**The tools**

- **`wip fix`** makes three more fixes: a byte written as a character
  cast, `'[' as u8`, becomes `b'['`; a `@derive(Clone)` on plain data,
  which clones without it, goes; and so do the wrappers and the
  `@derive`s above.

**The library**

- **`std::text::Scanner`:** text read from the front by position — the
  next byte or character looked at, taken, taken where it is the one
  expected, skipped or taken while a test holds, what was read cut out,
  and a line and a column worked out where an error needs one.
  `std::json` reads with it.
- **A vector of plain data is emptied in one step:** `clear`,
  `truncate` and its drop set its length, where they touched every
  element.

## 0.11.0 — 2026-10-24

Programs written for 0.10 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead.

**The language**

- **A narrowing that does not fit panics.** `as` between integers gives
  the same number in the new type, or panics with the value and the type,
  `300 does not fit in a u8`, as arithmetic that does not fit does; a
  constant that does not fit is refused where it is written. `as%` keeps
  the low bits, which is what `as` did: for a byte taken out of a word,
  or the same bits read with the other sign. An `as%` of a value the type
  holds is a warning whose fix writes `as`. **Breaking:** where a program
  cuts on purpose, it writes `as%`; a narrowing that cuts by mistake now
  panics where it happens rather than going on with a wrong number.
- **An `assert` in a generic's signature**, after its result: a fact
  about its type and length parameters, `assert(c::sizeOf<C>() <= ROOM)`,
  checked while the program compiles for each set of type arguments a
  call gives, and an error at that call where it does not hold.
- **A generator's type named.** A function that yields may name the type
  it answers, `fn split(text: str, separator: str): Iterator<str> as
  Split`, which a field, a result or an argument may then be.
- **Implicit parameters in generic code:** an implicit's type may name
  the function's type parameters, and those of the type a method extends.
- **A generic struct built without its type arguments** is told them by
  the arguments beside it, `written(Writer(scale: 10), draw)`.
- **An `Option` in the room of what it holds:** `Option<&T>`,
  `Option<own<T>>` and an optional function are one pointer, an optional
  `bool` one byte, an optional `char` four; an `Option<&T>` crosses to C
  as the pointer, null for nothing.
- **A closure as a block,** `c::Block::of(own (…) => …)`, for what
  Apple's systems call back.
- **Generators run as fast as a loop written out:** a generator keeps
  between calls only what it needs then, and is spliced where it is
  walked.

**The tools**

- **`wip fix`** makes the fixes of the warnings that change no meaning —
  a copy where a move would do, a `move` the compiler infers, a `clone()`
  of plain data, now a warning of its own — over a tree, round after
  round; `--check` writes nothing.

**The library**

- **`madeBy((i) => …)`:** an array made by a function of each index, its
  length and element type the type expected's, of any element type.
- **`std::text`'s pieces, lines, words and characters** are generators,
  `text::split(t, ",")` and the rest, which the methods `split`, `lines`
  and the like answer; `rsplit` is several times faster. **Breaking:**
  `Split::of(t, ",")` and its kind are `text::split(t, ",")` and the like.
- **The library's files for one system** are named for it, and read for
  that system alone.

**Fixed**

- A library module's file named for a system was read for every system.

## 0.10.0 — 2026-10-19

Programs written for 0.9 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead.

**The language**

- **A method's type arguments** are written after its name where the
  call's `(` follows, `response.json<Vec<i64>>()`, as a function's are.
  And what is expected of a value says them through `?` and through a
  method called on it: `val numbers: Vec<i64> = response.json()?` and
  `val numbers: Vec<i64> = json::decode(text).unwrap()` compile, where
  they asked for the type to be written. **Breaking:** `f(a.x < b, c >
  (d))`, comparisons after a field in an argument list, reads as type
  arguments, and is refused with a fix that adds the parentheses.

**The library**

- **`std::http`:** an HTTP/1.1 client, in Wip over `std::net`.
  `http::get(url)`, and a `Client` that sends any `Request`, keeps its
  connections, follows redirects and stops waiting after its timeout;
  `https` through the system's TLS; gzip asked for and undone; a body
  read as it comes, `client.open(request)`, so that one larger than
  memory is never held whole, and sent from a file or from any
  `io::Read`, `client.upload(request, &var source)`; JSON both ways; a
  Unix-domain socket.
- **`std::tls`:** `TlsStream`, a TLS connection whose server's
  certificate the system checks: OpenSSL 3 on Linux, Network.framework
  on a Mac and iOS, its blocks laid out in Wip.
- **`std::compress`:** gzip, zlib and raw deflate, at once or in pieces
  through an `Inflater` and a `Deflater`, over the system's zlib.
- **`io::Read` and `io::Write`,** what bytes are read from and written
  to: a `File`, standard input, a connection, a program's input, a
  response's body; `io::copy(&var from, &var to)`; `io::stdout()` and
  `io::stderr()`, in order with `print`. **Breaking:** `File.writeBytes`
  is `writeAll`; a `net::Stream` is read, and it and a
  `process::Connected` written, through `var`; `IoError` has
  `InvalidData` and `TimedOut`, and `EPERM` is `.Denied`.
- **`std::base64`, `std::hex` and `std::url`:** bytes as text, each an
  `encode` and a `decode` that says where text is wrong: base64 in both
  alphabets, padded or not; two hexadecimal digits a byte; and
  percent-encoding for any part of a URL.
- **`digest::crc32` and `digest::adler32`,** and `Crc32` and `Adler32`
  for bytes in pieces.
- **What a server asks of the system:** `std::users`, a user or a group
  by name or number; a file's owner, `fs::setOwner` and `fs::setMode`;
  `process::fork`, `newGroup`, `sendToGroup`, `becomeUser`,
  `changeRoot`, `setCurrentDir`, `limit` and `setLimit`.
- **`net::Stream::tcpWithin`,** a connection that gives up after a time.
- **`future::both`.** **Breaking:** it was `future::together`.

**Fixed**

- A binding of an `is` whose test failed, followed by a `return`, could
  drop what it never held: a variable's drop flag is false from the start.
- A `&var dyn` parameter lent on to another call, `inner(&var source)`.
- A release build in parts, where a type's interface table named a
  small function another part had.
- On a Mac, a write to a program that had ended ended this one where
  another thread was running; on Linux, a TLS write to a server that had
  gone could too.

## 0.9.0 — 2026-10-18

Programs written for 0.8 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead.

**The language**

- **A range is a value:** `lo..hi` and `lo..=hi` stand wherever an
  expression may, of the prelude's `Range<T>`, which is an iterator:
  `(0..4).any((i) => taken[i])`, `(1..=n).sum()`, `val counted = 0..n`.
  A `for` counts a range and a slice takes one, as before; the operator
  binds looser than every other but assignment.
- **Constants of a type:** a `val` among a type's members — in a
  struct's or an enum's body, or an `extend` block — is a constant of
  the type, reached as `Type::NAME`, as its static functions are. Every
  integer type has `MIN` and `MAX`, `i64::MAX`; `f32` and `f64` have
  `INFINITY`, `NAN`, `MAX`, `MIN` and `EPSILON`. A type with
  parameters, and a block that implements an interface, hold none
  (E0370).
- **A module's own names come before the prelude's:** a name a module
  declares or imports is the module's, and the prelude's item of that
  name is reached by its path, `std::prelude::Text`. A program may name
  its own `Index` or `Range`, and the prelude can grow without taking a
  program's names.
- **An array maps to an array:** `xs.map(f)` of a `[T; N]` answers a
  `[U; N]`. **Breaking:** where it was kept as a `Vec`, write
  `xs.copies().map(f).toVec()`.

**The library**

- **`std::watch`:** a `Watcher` says which of the files and directories
  it watches changed since it was last asked — FSEvents on a Mac,
  inotify on Linux — and its `descriptor()` is ready while a change
  waits.
- **`io::EventQueue`:** a program waits on many descriptors at once — a
  listener, its connections, a watcher, a pipe — each with a number of
  its own, until one is ready or a time passes. kqueue on a Mac, epoll
  on Linux.
- **`process::Signals`:** Ctrl-C, a request to end, the terminal going,
  Ctrl-\ and a child's ending, caught and told when the program asks,
  with no handler running at a moment of the system's choosing.
- **`sync::ring<T>(capacity)`:** a ring between two threads, one filling
  and one draining, without waiting, a lock or an allocation: what an
  audio callback is handed its work through.
- **`PriorityQueue<T, P: Ord>`** in `std::collections`, the smallest
  priority first, equal ones in the order pushed.
- **`std::fs`:** `writeBytes`; `replaceFile` and `replaceBytes`, which
  replace a file so that a reader, or a program stopped while writing,
  finds the old file or the new one and never part of either; and
  `TempDir`, a directory of the system's temporary files removed with
  what it holds where the value ends.
- **Numbers at a place in bytes:** `bytes::u32le(&header, 4)` and
  `bytes::putU32le(&var header, 4, size)`, for every number a
  `bytes::Reader` reads, each checked once and one load or one store.
  A `Reader` reads through the same, faster.
- **`InlineVec`'s `readInto`** lends the room past what it holds to a
  read, and holds as many as the read filled.

**Faster**

- A slice's, a text's and a vector's length is known never to be
  negative, so the optimiser drops the bounds checks it implies.
- `str.indexOf` and a byte's search go through C's `memchr`.
- A copy of a few bytes is a load and a store: `std::libc`'s `memmove`
  answers its destination, as C declares it, so the optimiser knows it.

**Tools**

- **A release build describes every variable** to a debugger, even one
  the optimiser dropped every use of; a value held in pieces may show as
  it was.
- **`wip fmt`**, completion after `Type::`, `wip doc` and the editors'
  grammar know a type's constants; the grammar knows a range as a value.

## 0.8.0 — 2026-10-17

Programs written for 0.7 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead.

**The language**

- **Generics over a length:** a struct, an enum or a function may take
  `val N` among its type parameters, an `i64` known where the program
  compiles. It stands wherever an array's length may, `[T; N]`, and in
  the code it is the number each use gives. A use gives a number in a
  type, `Ring<f32, 4>`, a constant's name, or leaves it to be inferred
  from an array, `total([1, 2, 3])`. A type does no arithmetic on a
  length, `[T; N + 1]`, and an interface takes none (E0369, which also
  refuses a length where a type is expected, and a type where a length
  is). `extend [T; N]` gives every array the methods of a module's own
  interface.
- **A local of a literal takes its type from its first use:** `var sum =
  0.0`, then `sum += sample` of an `f32`, is an `f32`; so is `val side =
  if dx >= 0.0 then 1.0 else -1.0` before `side * halfX`. The first use
  decides as the literal written there would — an operand, an argument,
  a field, a `return`, a place lent to Wip or to C, a value assigned —
  and where it says nothing, the local is `i64` or `f64` as before, and a
  later use that wants another says where the first use was and what to
  write. Nothing that compiled changes its meaning.
- **On an operator's left as on its right,** what takes its type from
  beside it takes it: `(if c then 1.0 else -1.0) * x` and `(1.0) * x`,
  which were refused, are of `x`'s type.
- **A copy with fields changed takes what it owns:** `Name(field: value,
  ..base)` moves out of `base` each field it does not name that owns
  memory, as writing the field out would, where it copied plain data
  alone; and `.Variant(field: value, ..self)` builds a variant from the
  one a `match` or an `is` around it matched.
- **A binding only read holds its place to its last use,** not to the end
  of its arm or block: after `if node is .Leaf(value)` and the last use of
  `value`, the place may be assigned, moved or lent `&var`.
- **A keyword names a field where a field's name is expected:** `when`,
  `then`, `type` and the rest, declared, after a `.`, in a literal and in
  a pattern; a derived `Decode` reads a format whose keys are keywords.
- **A closure given to a call lends the result only what calling it may
  give back,** so `self.items.find((x) => x.name == name)` borrows `self`
  and not `name`.
- **A reference chosen by a branch** is held in a `val` without its type
  written, as one `&place` alone was.
- **A lambda's result may be a tuple:** `(n: i64): (i64, i64) => (n, n +
  1)`.
- **A guard broken after any operator** goes on to its next line, where
  only `&&`, `||` and `|` let it.

**The library**

- **`InlineVec<T, N>`:** at most `N` values held where the vector is —
  in a local, a field, an element — never allocating, with a `Vec`'s
  methods but those that would grow it: `push` panics where it is full,
  `tryPush` answers `false`. It holds what owns memory as well as plain
  data, and is plain data where its element is.
- **What keeps elements clones them:** `filter`, `partition`, `min` and
  `max` of a slice or a `Vec` are there for any element that clones,
  answering clones, where they were for plain data alone.

**Errors**

- A method not found names the methods named like it, not every method
  of its type.
- A list asked for `count` or `toVec` is told it has `len`, or is one
  already, with the fix.
- A variant's `..base` refused is the one error, the values it names
  still checked.

**Tools**

- **`wip fmt`** does not break a guard's chain before its dots, where a
  line that starts with `.` would begin the next arm.
- **`wip bindgen`** writes a C field named like a keyword as C names it.
  **Breaking:** a binding written again names it `type` where it named it
  `type_`.
- **iOS:** a program in the simulator is started by a launcher `wip`
  builds with Xcode, where the Mac's shell, which an iOS 26 simulator
  refuses, started it.

## 0.7.0 — 2026-10-15

Programs written for 0.6 compile as they did, but where marked
**breaking**; the compiler's errors say what to write instead.

**The language**

- **A name a pattern binds may be a variable:** `.Ok(var gpu) =>
  run(&var gpu)`, in a `match`, an `is` test or a `for`, where the part
  was copied into a `var` on the next line before.
- **A method may say it empties what it is called on:** `var fn clear()
  empties`. After the call the receiver borrows nothing it borrowed, so a
  list of views kept from one line, one frame or one round to the next is
  cleared and filled again while what it borrowed changes. The library's
  `clear` of `Vec`, `Deque`, `Map`, `Set` and `Arena`, and `takeAll`,
  say so.
- **A type borrows what its fields hold,** read through the types they
  name: a type argument makes a generic type a view only where a field
  holds a value of it. A `Handle<str>` is plain data, a struct of handles
  generic over a GPU borrows nothing, and a body is asked to name in
  `keeps` only what it keeps.
- **What `@derive` asks of a type's parameters is what its fields need of
  them,** worked out through each field's type, not every parameter for
  every interface. A `Handle<T>` compares, orders, hashes and prints
  whatever `T` is, so it keys a map of values that do not compare; a
  struct of a GPU's handles clones and compares whatever the GPU is. A use
  that fails a derived condition is told which field asks it, and hover on
  `@derive` shows the conditions.
- **Constants a build defines:** `@define val EDITOR: bool = true`, set
  by `wip build --define EDITOR=false`, read as any constant. A branch a
  constant condition never takes is checked in every build and compiled
  in none, so a release without the editor carries none of its code.
  Text compared in a constant folds, as numbers do: `EDITOR && TARGET_OS
  != "ios"` is known where it is written.
- **A test file's names are its own:** what a `.test.wip` file declares
  is seen in that file alone, so two test files of one module may each
  have a `WIDTH` and a `pixel`, and the module's own code never sees a
  test's names. A test file sees all its module declares, and may declare
  none of it again. **Breaking:** a helper one test file uses from another
  is marked `pub`, which shares it with the module's test files and
  nothing else; a test file extends only a type a test file declares
  (E0338); two tests of one module may not share a name.
- **Breaking:** a `for` over `&var c` or `&c` is refused, with the fix
  that removes the borrow: a `for` walks a place, writing its elements
  where it can be written (E0411).
- A slice read through a reference and given for a type parameter is held
  as the reference: `Option::Some(bytes)` of a `&[u8]` is an
  `Option<&[u8]>`.

**Building**

- **iOS:** `wip build`, `wip run` and `wip test` take `--target ios-arm64`
  and `--target ios-arm64-simulator` from a Mac with Xcode.
  `@target(os = "ios")` and `TARGET_OS == "ios"` say iOS's code. A package
  that says `@app(id = …, name = …)` is built into an app, signed, and run
  on the device or on a simulator of its kind.
- **The files a program reads, beside it:** a package names them in its
  `package.wip`, `@files("levels")`, and every build puts them beside the
  program; a library's go under a directory of its name, so a program
  using the engine finds `engine/shaders/…` there. A package's code finds
  its own with `package::besideProgram(path)`. **Breaking:** `@files` may
  not name a `.wip` file, and a literal path given to `besideProgram` must
  be one it names, or below one.
- **Release builds in parts:** a release build is optimised in parts, on
  every core, each part with a copy of what it calls from the others, and
  runs as fast as one optimised whole; the linker strips what every call
  inlined. A debug build of the compiler takes 2.0 seconds where it took
  3.3, and a release build a quarter less.

**Errors**

- A stale borrow (E0436) names the word a signature could have said —
  `from` on a result, `empties` on a method — and offers it.
- A borrow left out of a generic call is said to be left out, rather than
  its type parameter not inferable.
- A field refused for what it holds is said once, not again where a value
  of its type is built.

**The library**

- `fs::readBytes`, a whole file as bytes.
- `Handle` and the arenas derive what they implement.

**Tools**

- **`wip fmt`** keeps a variant written alone under a statement on its own
  line.
- **The editor** offers a test file's names in it, and none in its
  module's own files.

## 0.6.0 — 2026-10-13

Programs written for 0.5 may need changes where marked **breaking**; the
compiler's errors carry the fixes, and the `wip fmt` of this version
writes the new forms.

**The language**

- **A chain goes on with a `.` at the start of a line,** as in Swift and
  Kotlin: `query()` and then `.every()` on the next line is one chain. A
  `match` arm, read as a pattern before `=>`, is never a chain.
  **Breaking:** a line that ends with `.` is refused (E0113), with the
  fix that moves it; and a variant meant as a block's value under a
  statement is written with its type, `Option::None`, since a `.None`
  there continues the line above (E0126, with the fix that writes the
  type).
- **Each branch of an `if` in its own form:** one expression after `then`
  or `else`, statements in braces, mixed in one `if` as a `when`'s arms
  are — `if xs.isEmpty() then return .None else { … }`, `} else 0`, and a
  `match` after `else` without braces round it.
- **A list literal takes the type expected of it:** `args: [id]` where a
  `Vec` is expected is one, `[]` an empty one, and a list where an
  `own<[T]>` is expected is built on the heap; text in a `Vec<String>`
  becomes `String`s. A `Vec` compared with a list, an array or a slice
  compares their elements and builds nothing: `order == [3, 2, 1]`.
- **Integers widen where nothing is lost:** a `u8` where an `i64` is
  expected, or beside one in an operator, is taken as one, as are the
  branches of an `if` and the elements of an array of the type that holds
  them all. `as` is left to what may lose something. What an operator
  works out in a narrow type and is then widened is warned of (E0366),
  with the fix that widens an operand. **Breaking:** a shift by a constant
  at least as wide as the value is refused (E0367), since the amount is
  taken modulo the width and it leaves the value as it was.
- **Closures that write:** what calls a closure one call at a time — a
  slice's `map`, `fold`, `filter`, `find`, `any`, an `Option`'s and a
  `Result`'s, an iterator's consumers — takes one that may change what it
  captures, `self` among them: `names.map((n) => self.intern(n))`. A
  closure that reads goes where one that writes is taken. A lambda kept
  in a `val` may write what it captures, which it holds until its last
  use; reading it before then is refused (E0414).
- **An iterator's adapters borrow:** `map`, `filter`, `filterMap`,
  `flatMap` and `takeWhile` take a closure lent to read, and the chain is
  a view of what it captured, so it may read a `str`, a reference or
  `self`: `text.split(" ").filter((w) => w.startsWith(prefix))`.
  **Breaking:** `own` at an adapter is refused, with the fix that leaves
  it out.
- **Where an error was made, and the way it came:** in a debug build, an
  error that ends the program, or is `unwrap`ped, is followed by where it
  was made and each `?` that passed it on, as a panic's calls are.
- **Breaking:** a closure that reads, `&(…) => R`, no longer writes
  through a `&var` it captured, which let `future::map` race on `self`.

**The library**

- **Breaking — walks and lookups named for what they do:** the copying
  walk is `copies()`, and its iterator `std::iter::Copies` (was `walk()`
  and `Walk`); a position is `indexOf` and `lastIndexOf` on every type,
  text's and a `Map`'s and a `Set`'s among them (was `find` and
  `findLast`); a `Deque`'s `first()` and `last()` lend, as a slice's do.
  `find` answers what it finds.
- The functions that take a closure that writes are `@inline`, so it
  costs what the loop it replaces costs; an `@inline` function is not a
  value, so `Option::map` is passed as a lambda.

**Tools**

- **`wip fmt`** writes each branch of an `if` in its form and a long chain
  with a `.` at the start of each line.
- **The reference** has a glossary and a one-page cheat sheet.
- **The compiler and the library** are written with all of the above:
  lists for vectors, closures for loops, without the casts and the
  `own`s no longer needed.

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
