# The checker's fuzzer

A program that writes test cases for the compiler's checks of moves and
borrows by the thousand, builds them with the `wip` it is given, and runs
those the checker accepts with every check a program has. A case the
checker accepts must run clean; one that does not is a finding.

```sh
wip build --release tests/fuzz/main.wip -o target/fuzz
target/fuzz target/wip/bin/wip --rounds 20
```

## What a case is

A test function of the program `world.wip` declares: values that own
memory in each way a program's do — a `String`, a `Vec<String>`, an
`Option<String>`, a struct of two, an enum, an `own<Node>`, a node that
holds one, a list of boxes and an owned closure, `own<() => i64>`, which
owns what it captured — a view, `Seen`, that borrows, and `Tracked`, a
value with a `destroy` of its own that a pair and a node hold, and a case
keeps alone too. `generate.wip`
writes its statements:

- values made, moved, cloned, and moved out of a field, a list, an option
  and a box; a struct taken apart by a `move fn` of its own; a value moved
  into an owned closure, which is called, passed on and ended;
- variables given new values, fields given new values after they were
  moved out, and values and fields given new values through a `&var`, by a
  function or by a `var fn`;
- values lent, `&` and `&var`, and lent out of a call;
- views kept — a `str`, a `&`, an `Option<&String>`, a view struct, a
  closure that reads what it captures — and used after what they borrow
  may have changed; a closure that writes what it captures, which holds it
  until its last call;
- a value matched where it is, its arms reading what they bind, and, as
  often as `--risk` says, moving it out, which must be refused;
- values given to generic functions, which give them back or end them;
  a pair given to a function whose `?` leaves with a field moved out of
  it;
- a map of pairs by a text — put, taken out, looked up, a lookup kept as a
  view, walked — and a set of texts;
- `if` and `else`, `for` over a range, over a list where it is, over a list
  taken apart, and over what a generator lends of a list, `while`,
  `match move`, blocks, `destroy`, `defer` of a value given away when the
  block ends, and `return`, `break` — out of a loop further out too, by
  its label — and `continue` taken on some runs;
- a block under an arena, as `mem::using` runs a closure: what it makes is
  the arena's, and must end before the arena is reset. It writes nothing
  from before it, whatever the risk: what it put there would outlive the
  arena, which the arena refuses as it is reset, not the checker.

The generator keeps a model of what each variable holds and which views
are still good, and writes mostly what the model says is sound, so that
most cases are accepted and run. As often as `--risk` says, a choice
ignores the model, and the checker must refuse what is not sound.

## What is checked

Each round writes a batch of cases and builds it with `wip test --emit
objects`. A case the checker refuses is counted by its error's code and
left out, and the batch is built again until the checker accepts it all.
The runner is linked, and each case is run in a process of its own, as
`wip test` runs a test, with:

- **its moves checked,** as a test's are: a value whose type cleans up after
  itself is poisoned where it is moved from, and dropping it after that
  panics;
- **its leaks checked,** `WIP_CHECK_LEAKS=1`: what it allocated and did not
  free ends it with 70;
- **its memory checked,** by the address sanitizer: the cases are built
  with `--sanitize address`, and a read or write of memory that is not
  alive stops the case with the sanitizer's report — freed heap memory, and
  a variable whose block has ended, which Wip marks in every build. With
  `--guard`, on a Mac, Guard Malloc checks it instead: each allocation on
  pages of its own, taken away when it is freed, which sees the heap alone.

- **every end of a tracked value,** from the log the case writes: making a
  `Tracked` writes `made N`, `N` the place in the case where it is made
  (a clone's is 100000 more), and ending one writes `ended N`. Each must
  end as often as it was made, and never before. At the end of each block
  the case writes `end of B: …`: the tracked values the block declared and
  kept, newest first, which must end right after it, in that order — the
  language's promise that a block's values end in the reverse of the order
  they were declared. A value that may have been moved, or whose contents
  the generator cannot say, is not named, and a block whose values it
  cannot say is marked `?`.

- **both builds:** each batch is built as a test is and again with
  `--release`, and a case that passes in the first must pass in the second,
  with every check above, and print the same: its log, its marks, and the
  sum it worked out, which it says as it ends. A release build checks no
  moves and is optimised, so a second drop there is a double free, and an
  optimisation that loses a drop, or makes one twice, prints otherwise.

A case has ten seconds, far more than one takes, and is ended past them.

A case that is accepted and then panics, leaks, or is stopped by the
sanitizer or a fault, in either build, runs out of its time, or that the
two builds print differently, is a finding. So is a compiler that ends without
saying why it refused, which is found by building each case of the batch
alone.

## A finding

A finding is kept once for each kind: what failed, and the form of the
statement of the case it names, `vN = …` or `destroy(…)`, with numbers
made `N`, so that the same failure through the same kind of statement is
one finding however many cases find it. The first of each kind is made
small and kept; the others are counted, and the run ends with
`target/fuzz/findings/summary.txt`, each kind with its count and where it
is kept.

It is made as small as it stays the same finding:

- its statements left out one at a time, from the last, where the case
  without one still builds and fails the same way, so that a cut that
  leaves a loop without its step, which then runs out of time, is not
  kept;
- then each block tried as one of its bodies alone, where it was, so a
  branch the failure does not need goes;
- and each use of a variable tried as a plain value of its type, written
  where the use was, `given(Tracked::of(0))` for `given(v3)`, which lets
  go of what made the variable;

and after each of the last two that keeps the finding, its statements left
out again. It is kept in `target/fuzz/findings/SEED-CASE/`: the world as
`main.wip`, the case as `cases.test.wip`, and `found.txt`, which says what
was found and what the run printed. `wip test main.wip` there shows it
again. The same seed writes the same cases, so a round is found again by
its seed, which each round prints.

`--verbose` prints each refusal with its case, which is how a refusal of a
case the model says is sound is looked at: either the model is simpler
than the language, or the checker refuses what it should accept.

## The programs changed: `--mutate`

```sh
target/fuzz target/wip/bin/wip --mutate --rounds 50
```

Run in the checkout, it takes every program the repository keeps — each
case of `tests/cases/ok` and `tests/cases/err`, and each example — copies
it under `target/fuzz/mutants`, and each round changes it in one to three
places, by its seed: a token dropped, doubled, swapped with the next, or,
most often, put in the place of another of its kind — a name by another
the file uses, a keyword by one near it in meaning (`val` by `var`, `if`
by `while`, `break` by `return`), a number by an edge of a type, a text by
another, a mark by another, `&` by `&var`. Its first file is changed most
often, and its others sometimes. Each is checked with `wip check`, as the
language server checks what is being typed, many at once, each given
twenty seconds of a processor.

Whatever the text, the check must accept it or refuse it with an error's
code. Past the check, each change goes through two stages more:

- **built and run,** where the check accepts it: built with
  `--sanitize address` and run, given nothing, with five seconds of a
  processor and fifteen of the clock. It may panic, print anything, loop
  or ask for more memory than there is; what a program of Wip cannot do —
  read or write memory it does not own, or end by a signal — is a finding,
  and so is a build that fails other than by refusing with an error's
  code. A program that calls C, holds a `ptr`, or reaches the network or
  another process is built and not run: its memory is its own to get
  wrong.
- **formatted,** accepted or not: `wip fmt -` must not panic; what it
  writes, formatted again, must be what it wrote; and it must hold every
  token the text held, but for what the formatter lays out itself —
  commas, semicolons, the braces of a body of one statement it writes
  after `then` or `do`, a text block's indentation, and a `>=` that
  closes a type's arguments, which it writes `> =`;
- **asked of the language server,** an eighth of the changes a round, a
  different eighth each, since opening a file checks its whole program: a
  server for each processor's share opens each changed file, and is asked
  at each place changed for what is there, what completes it, the
  signature around it, its definition, its references, a rename and the
  fixes there, and of the whole file for its symbols, its hints and its
  layout. It must answer each — an error is an answer — and go on; a server
  that ends or hangs is a finding, and another takes its place.

A stage's finding is made smaller by trying that stage again, but the
server's, which is kept as it was found: a line left out would move the
places it was asked about.

With `--sources`, more programs are changed too: the tool,
`tools/bindgen`, and each directory a checkout lists in
`scripts/ports.txt`, first on a line, that is there and holds a program,
copied without its history, what it built, or a file larger than 2 MB. One that panics, is ended by a signal, runs out of its time, or
refuses without an error's code is a finding: the changed file's lines are
left out, many at a time and then one at a time, while the check fails the
same way, and the program is kept in
`target/fuzz/findings/mutate-STAGE-SEED-NAME/`
with `found.txt`, which says what was changed and what the check said;
`wip check main.wip` there shows it again. A finding of a kind found
already in the run is counted and not kept again: a panic is its message,
with numbers made `N`, the first place in the compiler it names, and the
first function of the compiler that called the one there, as one helper
called from many places panics for as many reasons. The run ends with
`summary.txt`, as the other mode does. The run ends with the refusals counted by their first error's
code, which says how far into the compiler the changes reach.

## The engines compared: `--engines`

```sh
target/fuzz target/wip/bin/wip --engines --rounds 50
```

A function is worked out by two engines: the evaluator, which runs a
`@comptime` constant while the program is compiled, and the program built,
whose optimizer works out what it can of a release build itself. Each
round writes functions of numbers by its seed — `caseN(a: i64, b: u64, c:
f64, d: u8)` answering a number of any width, of the arithmetic that
panics where it does not fit and the one that wraps, shifts, the bit
methods, the `checked` and `saturating` ones, casts both ways and from a
float, comparisons and `if`, the float methods, `f64::fromBits`, and a
number's text — each given inputs at the edges of their types. Each is
worked out three ways: in a constant; called when the program runs with
its inputs made where the compiler cannot see them; and called with them
written where it is called. A debug build and a release one are built and
run.

All must answer the same, every `NaN` as one, since a `NaN`'s bits are the
machine's. A constant that panics while the program is compiled is a case
of its own: both calls must panic, in both builds, saying the same. A case
the checker refuses — a shift by a constant past its width, a constant
cast that does not fit — was the generator's mistake, and is counted by
its code and left out.

A finding is kept once for each kind and type in
`target/fuzz/findings/engines-SEED-CASE/`: the program of its case alone,
which `wip run main.wip -- CASE` runs again, and `found.txt`.
`--verbose` prints each case refused with what refused it.
