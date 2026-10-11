# Wip

A systems language, and its compiler, written in itself. Wip is an experiment
in how much of what a borrow checker buys can be had without lifetimes:
writing is second-class — a `&var` exists only while a call runs — and
reading borrows are tracked without being written down, so ownership is
decided where a value is written, nothing is counted at run time unless
a program shares a value and says so, and no lifetime is ever written
down.

```wip,run
struct Board {
    cells: Vec<i64>
}

extend Board {
    static fn of(size: i64): Board = Board(cells: Vec::filled(size, 0))

    // One body, two halves: `board.at(i)` reads, and `board.at(i) = x`
    // writes.
    lend fn at(index: i64): &i64 = lend self.cells[index]

    fn firstEmpty(): Option<i64> = {
        var i = 0
        while i < self.cells.len() {
            if self.cells[i] == 0 then return .Some(i)
            i += 1
        }
        return .None
    }
}

fn main() = {
    var board = Board::of(9)
    board.at(0) = 7
    assert(board.at(0) == 7)

    val .Some(free) = board.firstEmpty() else {
        panic("a fresh board has room")
    }
    assert(free == 1)
}
```

## Where it stands

It is a prototype, and it is finished enough to write programs in: the
compiler type-checks, move-checks and borrow-checks, generates native code
through LLVM, with a `clang` 15 or newer, links C, and compiles a program
in a fraction of a second; a release build is optimised across the whole
program. It has been tried on real programs by porting
them: a C++23 game — raylib built from source with it, a frame identical
to the C++ build's, pixel for pixel, compiled in **0.19 s** — a file
manager, a web server, a Lox interpreter, and miniz's compressor, whose
output is byte for byte the C library's.

What it does not have yet:

- **Targets** beyond Apple arm64 and Linux on arm64 and x86-64. There is
  no Windows.
- **Packages fetched from anywhere:** a program depends on another by its
  path, and nothing is downloaded.
- **Asynchronous input and output:** threads and futures are there, an
  event loop is not; nor UDP or TLS.

## Installing

Wip runs on macOS on Apple silicon and on Linux on arm64 and x86-64. Every
program it builds goes through a `clang` 15 or newer, which it looks for
as `clang`, or as `WIP_CLANG` names it, and is linked by the system's C
compiler, `cc`. On a Mac, the Command Line Tools have both
(`xcode-select --install`); Debian 12's own `clang` is 14, so install
`clang-15` or newer there and set `WIP_CLANG=clang-15`.

`wip` is a directory: `bin/wip`, and beside it the standard library,
`std/`, and the tools, `tools/`, which it finds from where `bin/wip` is.
Keep the three together; a link to `bin/wip` from elsewhere on the `PATH`
works, a copy of it alone does not.

**From a release.** Each version has an archive for each system,
`wip-<version>-<system>.tar.gz`, on [the releases
page](https://github.com/vmarteev/wip-lang/releases). Unpack it where it
is to live, and put its `bin` on the `PATH` in your shell's profile:

```sh
curl -LO https://github.com/vmarteev/wip-lang/releases/download/v0.3.1/wip-0.3.1-macos-arm64.tar.gz
tar -xzf wip-0.3.1-macos-arm64.tar.gz
mv wip-0.3.1-macos-arm64 ~/wip
echo 'export PATH="$HOME/wip/bin:$PATH"' >> ~/.zshrc   # ~/.bashrc for bash
```

The systems are `macos-arm64`, `linux-arm64` and `linux-x86_64`. A Mac
refuses to run a program downloaded through a browser until it is told
otherwise: `xattr -dr com.apple.quarantine ~/wip`. To upgrade, put the
new version's directory in the old one's place.

**From a checkout of this repository,** with nothing but `clang`:

```sh
scripts/bootstrap.sh --out ~/wip
echo 'export PATH="$HOME/wip/bin:$PATH"' >> ~/.zshrc
```

The compiler is written in Wip, and `bootstrap/` holds it for each system
as LLVM's bitcode, which `clang` compiles into a first `wip`; that one
builds the compiler from its source, and the compiler built builds itself
again, a few minutes in all. The `std/` and `tools/` it leaves beside
`bin/wip` are links to the checkout's own, so a change to the library is
seen by the next build of a program, and the checkout must stay where it
is. After pulling a change to the compiler, build it again from its
source:

```sh
scripts/build.sh --release --out ~/wip
```

`wip --version` says which you have: `wip 0.3.1` for a release, and with
the commit and its day after it for a build from a checkout.

## Try it

```wip
// hello.wip
import std::io

fn main() = {
    val name = "world"
    io::println("hello, \(name)")
}
```

```sh
wip run hello.wip     # compile a program and run it
wip build hello.wip   # write an executable
wip test hello.wip    # run the program's tests
wip check hello.wip   # report everything wrong, write nothing
wip fmt hello.wip     # lay the file out in the one way Wip is written
```

The entry file's directory is the program's module, and every directory it
imports is another: there is no build file, no manifest and no module
manager. The standard library is read from beside the compiler, so
nothing is fetched.

For Zed and Neovim — highlighting, and a language server with errors as
you type, hover, definitions, completion and rename — see
[editors](editors/README.md).

## What is here

| Directory | What is in it |
|---|---|
| `compiler/` | the compiler, in Wip, a module a phase — `syntax`, `hir` (checking), `analysis` (moves and borrows), `mir`, `llvm` (LLVM's IR and bitcode, through `clang`), `fmt` — and `lang`, the driver and the language server; `main.wip` is the `wip` command |
| `bootstrap/` | the compiler as LLVM's bitcode, for each system: what builds the first `wip` with `clang` |
| `std/` | the standard library, in Wip, which the compiler reads from beside it |
| `tools/` | tools written in Wip that `wip` carries and builds when first asked: `wip bindgen`, a Wip module from a C header |
| `editors/` | Zed and Neovim: a Tree-sitter grammar, and the language server `wip lsp` |
| `tests/cases/` | the language's own test programs, each with its output or its diagnostics as a snapshot |
| `tests/std/` | the standard library's own tests, written in Wip |
| `tests/gate/` | checks of the `wip` command, written in Wip: given a `wip`, it runs it as a reader does: the cases against their snapshots, every example in the documents, calls across the C boundary, packages, the driver and the debugger |
| `docs/language/` | the language reference, by topic |
| `docs/grammar.md` | the syntax specification the parser must agree with |
| `examples/` | small, complete programs, each in a directory of its own, from `hello` to a web server and a C library |
| `scripts/` | everything done to the repository: the gate, snapshots, Linux, the editors' grammar |

## Reading it

Start with **[the language reference](docs/language/README.md)** — twelve
pages that say what the language is, by topic, each rule with an example
that runs. Then **[the grammar](docs/grammar.md)** for exactly what the
parser accepts.

**[The examples](examples/README.md)** are whole programs, each showing a
part of the language at work: options, files, JSON, threads, a web server,
dynamic dispatch, packages, and C.

## A few things that make it what it is

- **Second-class writing, tracked reading.** `&var x` is written at a call
  and lives for that call, so two writers of one place are only ever two
  arguments of one call. A `&x` may also be kept where a view is —
  `Option<&Node>`, a local — and what it borrows is worked out from where
  it came from and what the types can hold, so nothing outlives what it
  points at, and there are no lifetimes.
- **Ownership without a garbage collector.** A value is freed where its
  owner ends, decided while compiling; `Destroy` says what else to do. A
  value several owners read is a `std::shared::Shared<T>`, counted where a
  line calls `share()`, and never otherwise.
- **Lending pairs.** `lend fn at(i): &T` declares the reading and the
  writing half from one body, so `v[i]` and `v[i] = x` come from one place.
- **Errors are values.** `Result`, `?`, and conversions only where a
  program wrote a `From`. A panic is for mistakes, not for failure the
  caller should handle.
- **The compiler explains itself.** Diagnostics carry a code, a place, a
  fix, and a note that says why the rule is there.

## Development

Everything you can do with the language is `wip --help`; everything you can
do to the repository is a script in `scripts/`:

```sh
scripts/bootstrap.sh           # build the compiler from the seed, with clang
scripts/build.sh [--release]   # build it from its source, into target/wip
scripts/verify.sh              # the gate: the compiler built twice, formatting, every check
scripts/verify.sh --fast       # just the language's own cases, in a second
scripts/snapshots.sh [filter]  # regenerate what the cases expect, then read the diff
scripts/linux.sh [--fast]      # the same verification on Linux, in a container
scripts/clangs.sh              # the LLVM backend through clang 15, 16 and 19, on Linux
scripts/tree-sitter.sh         # the editors' grammar: regenerate, test, parse everything
```

`scripts/verify.sh` is the definition of green: it must print `ALL OK`
before a commit. It builds the compiler from its source with the `wip` the
seed builds, has the compiler built build itself, the two writing the same
bitcode for it, and runs `tests/gate` with the second, a program in Wip
that checks `wip` by running it: the case programs and their snapshots, the standard library's
own tests, the driver, the compiler's own tests — among them a robustness
pass over mutilated input — and every example in this file and in the
language reference, each of which must compile and be laid out as `wip
fmt` lays it out. A `run` example must run to a successful end
and an `error=` example must be refused with the code it names, so a
document that stops being true fails the build.

The scripts are the interface: a script that says what it does outlives
whichever tool is underneath it.

Apple arm64 is what this is developed on, and Linux is tested beside it:
`scripts/linux.sh` runs the same verification in a container. That is where
the differences that bite turn up — the C ABI, the width of `c_long`, what
the linker wants. It uses [Apple's
`container`](https://github.com/apple/container) where that is installed and
Docker otherwise, builds into a volume of the guest's own so the host's
artifacts are untouched and a second run is warm, and takes
`WIP_LINUX_ARCH=amd64` for an x86-64 guest instead of the arm64 one — the
other ABI, through Rosetta. One run at a time: the build volume attaches to
one guest, and a second run is told so rather than failing inside the
virtual machine.

A release build is LLVM's where there is a `clang` 15 or newer, and a
check stands beside the gate for it, too slow to be in it:
`scripts/clangs.sh` runs the cases' release builds and the C ABI's corpus
through every `clang` the backend claims, in the Linux guest. It runs
before a change to the backend is committed.

`wip bindgen`'s test asks clang for a header's syntax tree, and says it
skipped where clang is not installed.

## License

MIT; see [`LICENSE`](LICENSE). Some of the standard library is translated
from others' work, under the licenses in
[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).
