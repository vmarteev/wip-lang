# 12. The tools

One command does everything: `wip`.

```sh
wip run main.wip                 # compile and run, with the program's exit code
wip build main.wip -o game       # write an executable
wip test main.wip                # compile the tests and run them
wip check main.wip               # report every error, and write nothing
wip check --target all main.wip  # the same, as every target compiles it
wip build game                   # a package: its directory, with what it depends on
wip fmt .                        # lay every .wip file out in one way
wip debug game                   # the debugger, showing Wip's values as they are
```

## What is compiled

`run`, `build`, `test`, `check`, `doc` and `mir` read a *program*, not a
file. The entry file's directory is the root module, so every `.wip` file
beside it is compiled too, and each import pulls in the directory it names
([page 2](02-modules.md)). A module's `.c` files — and its `.m` files on an
Apple target — are compiled and linked with it ([page 11](11-c.md)).

A package — a directory with a `package.wip` — is built by naming it:
`wip build game` compiles `game` and every package it depends on, from
source, into one program named `game`, written in the package's root:
`game/game`, which is `./game` when the command is typed there. A
repository of several packages is built one package at a time, from its
root:

```sh
wip build game                   # writes game/game
wip test engine                  # each package's tests, in its own root
wip test game
```

The standard library is inside the compiler, so nothing is installed and
nothing is fetched. There is no package manager and no build file: the
directories are the build, and a `package.wip` only names a package and
says which others it uses.

## The commands

| Command | Does |
|---|---|
| `wip run file.wip [args]` | compiles to a temporary place and runs it, passing the arguments on and exiting with its code |
| `wip build file.wip` | writes an executable, named after the file unless `-o` says otherwise |
| `wip build dir` | builds the package whose root is `dir`, named as the package names itself, into that root |
| `wip test file.wip [filter]` | compiles the program with its `.test.wip` files and runs the `@test` functions; given a package, only that package's |
| `wip check file.wip` | reports everything wrong, and writes nothing; given a package, every module of it, imported or not; `--target` checks as another target compiles the program |
| `wip fmt [paths]` | lays `.wip` files out in one way; `--check` writes nothing and fails if any would change; `-` formats standard input |
| `wip doc [program]` | writes the program's documentation as pages, into `doc` unless `-o` says otherwise, with its packages' and the standard library's; `--std` the standard library's alone |
| `wip doc path::to::item` | prints one item's documentation: `std::collections::Map`, `str::findLast` |
| `wip debug program [args]` | starts lldb on a Mac, gdb elsewhere, or the one `--debugger` names, on a program `wip build` wrote, with the scripts that show Wip's values; `--scripts` prints where the scripts are |
| `wip lsp` | serves an editor: the Language Server Protocol on standard input and output |
| `wip bindgen header.h -o out.wip` | writes the `extern "C"` block for a C header, with clang ([page 11](11-c.md)) |
| `wip mir file.wip` | prints the mid-level IR of every function |
| `wip parse file.wip` | prints the syntax tree of one file |
| `wip lex file.wip` | prints the tokens of one file |

A program is built for the machine `wip` runs on, and may be checked for any
target the standard library is written for: `wip check --target linux` reads
the items under `@target(os = "linux")` and the `.linux.wip` files ([page
2](02-modules.md)) on a Mac, and checks them as Linux would. The names are
`macos-arm64`, `macos-x86_64`, `linux-arm64` and `linux-x86_64`; `macos` and
`linux` alone are on this machine's processor; `all` is the four, and
`--target` may be given more than once. A mistake several targets find is
said once, and one that not every target finds says which did. What a check
cannot see is what only the target's machine knows — how its C lays a struct
out, which symbols its libraries have — and building for another target is
not yet there.

`--color auto|always|never` and `--threads N` work everywhere; compiling
uses one thread per core by default. `wip --version` names the compiler
and, where it was built from a git checkout, the commit and its day:
`wip 0.1.0 (a1b2c3d4e5f6 2026-10-05)`.

`wip build`, `wip run` and `wip test` make a **debug build** unless
`--release` asks for a **release build**. A debug build
is not optimised, compiles a module's C `-O0 -g`, and checks moves;
a release build is optimised, compiles the C `-O2 -g`,
and does not check moves. The program does the same either way: arithmetic
that overflows, an index out of bounds and an `assert` that fails panic in
both.

A debug build's code is **Cranelift's**, which compiles fast. A release
build's is **LLVM's** where there is a `clang` of version 15 or newer to
compile it with — the one `WIP_CLANG` names, or `clang` — and Cranelift's
where there is not, which the build says in a note:

```
note: built by Cranelift: `clang` is not a clang 15 or newer, or is not there; WIP_CLANG names one
```

LLVM's code runs some two times faster, and takes longer to build: a
program of eleven thousand lines takes six seconds, where Cranelift takes a
quarter of one. `--backend cranelift` or `--backend llvm` chooses, for
`build`, `run` and `test`; `llvm` is refused without a `clang` for it, and
for a debug build, whose variables its code does not describe. The program
does the same whichever builds it, and a panic lists the same calls.

Every build is for a **processor**, which `--cpu` names.
By default it is the baseline of the machine's kind — x86-64-v2, which
every x86-64 processor since 2008's Nehalem and 2011's Bulldozer has; the
M1 on a Mac with Apple silicon; and Armv8.0 elsewhere, which is a
Raspberry Pi 4 — so that a program built on one machine runs on another.
`--cpu native` builds for this machine alone, with every feature it has,
and a level may be named, as `x86-64-v3` or `armv8.1`. The program does
the same on each, only faster where the processor has an instruction for
what it would otherwise do in several, or call C for; its C is compiled
for the same processor.

Both builds write **debug information**: the line each
piece of code was written at, and each function's name and place, so that
lldb and gdb stop at `main.wip:12`, step by line, and name the function
and line of every frame, and a profiler gives time to lines. A debug build
describes its variables and parameters too, with their types: each is
kept in memory while it is in scope, where the debugger shows it and may
change it. A release build by LLVM describes them as well,
where the optimised code keeps them: a variable whose value is in a
register for part of its scope is shown there, one LLVM computed away is
`<optimized out>`, and so is one it kept only in part — a `String` whose
length it kept and whose bytes' address it did not — rather than shown as
what it does not hold.

`wip debug program` starts the debugger with **formatters**, small scripts
the compiler carries, that show a value as Wip means it rather than as it
is laid out:

```
(lldb) frame variable
(Vec<i64>) numbers = len 2 {
  [0] = 4
  [1] = 5
}
(String) name = "hello"
(Map<str, i64>) ages = len 1 {
  ["ann"] = 31
}
(Shape) shape = .Circle(radius: 2)
(Option<i64>) none = .None
(char32_t) letter = 'x'
```

A `str` and a `String` are their text, a slice, a `Vec`, a `Set`, a `Map`
and a `Deque` their elements, and an enum the variant it holds. An editor's
debugger loads the same scripts: `wip debug --scripts` prints where they
are, for lldb's `command script import` or gdb's `source`. On a Mac it is
gathered into `program.dSYM`, beside the program, where lldb looks for it.
On Linux a debug build keeps it in the program, and a release build moves it
into `program.debug`, beside the program, which the program names and gdb
reads; it takes `objcopy`, which comes with the C toolchain.

`wip build` takes more:

| Option | For |
|---|---|
| `-o name` | where to write it |
| `-I dir`, `-L dir` | where C headers and libraries are on this machine |
| `--emit program\|static\|dynamic` | an executable, or a library C links against |
| `--header board.h` | write the C declarations of what the library exports |
| `--cpu baseline\|native\|level` | the processor it is for, as above |
| `--time` | print how long each phase took |

`--cpu` goes with `wip run` and `wip test` too, as `--release` does.

## Formatting

`wip fmt` prints each file again from its syntax tree, in one layout, to a
width, as Prettier does. What fits on a line is written on one; what does
not is broken where Wip allows — after `(`, `[`, `{`, a comma, an operator,
`=` and `=>` — with a list one element to a line and a trailing comma.
Declarations — a struct's fields, an enum's variants, a match's arms — are
one to a line with no comma, which a line break makes unneeded. A call's
last argument hugs its parentheses where the others are short and it is a
literal of several lines — a struct, an array, a lambda, a text block — so
`io::println("""` opens a block of lines and `""")` closes it. Comments are
kept where they were, one blank line between things is kept where the file
had one, and literals are written as they were.

```wip,ignore
// before
enum Shape { Circle(r:f64), Rect(w:f64,h:f64), }
fn area( s:&Shape ):f64=match s{
  .Circle(r)=>3.14*r*r
    .Rect(w,h) => w*h
}

// after
enum Shape {
	Circle(r: f64)
	Rect(w: f64, h: f64)
}
fn area(s: &Shape): f64 = match s {
	.Circle(r) => 3.14 * r * r
	.Rect(w, h) => w * h
}
```

The width is 100 and indentation is a tab, counted as four columns. A
package says otherwise in its `package.wip`, and the command line says
otherwise again:

```wip,ignore
@format(width = 120, indent = "spaces", size = 4)
package game
```

```sh
wip fmt --width 120 --spaces 2 main.wip
```

A file that does not parse is left as it is, with its errors shown. Before
anything is written the result is parsed again, and it must be the same
program with the same comments; where it is not, the file is left as it
was and the formatter reports its own bug.

An editor formats what it has not saved: `wip fmt -` reads standard input
and writes the result to standard output, laid out as the package of the
file `--stdin-path` names says. When the input does not parse it writes
nothing and exits 1, so the editor keeps what it had.

```sh
wip fmt - --stdin-path src/main.wip < buffer.wip
```

## Documentation

`wip doc` writes a program's documentation as pages, from what its source
says of itself: every `pub` item's declaration as it is
written, and the `///` lines above it. A page is a module's, or a type's
with its methods — those of every `extend` block for it — and the
interfaces it implements; a module's files are its sections, each with
the `//` paragraph the file begins with. A type's name in a declaration
links to its page. The pages are plain HTML, read from the disk, with an
index of every module and every item by name.

A comment is prose: a blank `///` line parts paragraphs, `` `code` `` is
code, and a line indented four spaces more is an example. Nothing else is
markup.

```wip,ignore
/// A point on the plane.
pub struct Point {
    /// Across.
    pub x: f64
}

extend Point {
    /// How far it is from the origin:
    ///
    ///     val far = point.length() > 10.0
    pub fn length(): f64 = …
}
```

Given a path of names instead of a program, `wip doc` prints that item —
`wip doc std::collections::Map`, `wip doc str::findLast` — looking in the
program in the current directory, and in the standard library where there
is none. Nothing is type-checked, so a program with errors still has its
documentation.

## Editors

`wip lsp` is a language server, which the Zed extension
and the Neovim plugin in `editors/` start. It shows what `wip check`
would as you type, in the files you have not saved too; offers a
diagnostic's fix where it has one; formats as `wip fmt` does; gives each
file an outline of its declarations; shows what a name is, with the `///`
comment above its declaration; goes to where it was declared, in the
standard library's sources too; and completes what you are typing: a
value's fields and methods after `.`, a module's items after `::`, and
the names in scope. It finds every reference to a name, across the
packages that use it, renames one everywhere it is written, and shows a
call's signature as its arguments are typed.

A file is checked as part of its program: the package whose
`package.wip` is nearest above it; or else the program of the highest
directory above it whose modules import the file's, which is its own
directory when none does. Every edit checks that program again,
whole. A file no program reaches, such as one of the standard library's,
is only parsed.

## Tests

A test file is `name.test.wip`, beside the code it tests, in the same
module. Only `wip test` compiles those files, so tests see what their
module keeps to itself and a built program carries none of them:

```wip,ignore
// board/board.test.wip
@test
fn theBoardStartsEmpty() = {
    val board = Board::new(9)
    assert(board.isEmpty())
    assert(board.count() == 0, "a new board holds nothing")
}
```

A test takes nothing and answers nothing. `wip test main.wip` builds one
program, runs each test in the order it is declared, and prints a line per
test. Given a package, `wip test` and `wip check` load every module of it,
whether anything imports it yet or not — each directory below the root
holding `.wip` files, but not a hidden one, `target`, or another package
— so a module is tested, and its errors reported, before the program uses
it; `wip build` compiles what the program imports:

```
running 3 tests
test theBoardStartsEmpty ... ok
test board::aBallCanBePlaced ... ok
test board::aFullBoardEndsTheGame ... ok

3 tests passed
```

A test outside the root module is named by its module. `wip test main.wip
board` runs only the tests whose name holds `board`. A package's tests run
in the package's root, so a path a test names — a fixture beside it — is
the package's wherever the command is typed; a program without a
`package.wip` runs them where the command is typed. A test that fails
panics, which prints the message and ends the run: the unfinished line names
the test, and the exit code is 101.

Tests check more than a program does. The program they are built into
checks its moves: a value whose type has a `destroy` of its own is left
poisoned where it was moved from, and dropping it after the move panics
with `a value was dropped after it was moved away` — a bug in the
compiler that would otherwise run a `destroy` on nothing.
`WIP_CHECK_MOVES=1` checks moves in any build, and `WIP_CHECK_MOVES=0`
turns it off in a test's. `WIP_CHECK_LEAKS=1` in the environment of any
program makes it report, as it ends, what it allocated and never freed;
only then, or while a test traces its allocations, does a program count
them, which threads would otherwise wait on one another to do.

## Exit codes

| Code | Means |
|---|---|
| 0 | it worked |
| 1 | the program was refused, or `main` answered an error |
| 2 | the entry could not be read |
| 101 | it panicked, or a test failed |

## The C cache

C compiled with a program — a module's own files, and the shims the compiler
writes — is cached by the compiler's identity, the flags, the file and the
headers it reads. A second build compiles none of it again. What LLVM makes
of a release build is kept the same way, by this compiler, the `clang`, the
flags and the program's IR, so that a program built again unchanged takes no
longer than Cranelift's. The cache is `$WIP_CACHE_DIR`, or the platform's
cache directory when that is not set.

## Diagnostics

Everything reported has a code, a place, and usually a fix. `wip check` is
the fastest way to see them all; the compiler stops nothing early, so one
run reports everything it can. A diagnostic reads like this:

```
[E0303] Error: cannot apply `==` to `Result<i64, Wrong>`
    │ Help 1: `Result<i64, Wrong>` implements `Eq` where `Wrong` does, and `Wrong` does not
    │ Help 2: write `@derive(Eq)` on `Wrong`, and the compiler writes it field by field
    │ Note: an implementation may hold only for some type arguments
```

The note says why the rule is there, which is the fastest way from an
error to the reasoning.
