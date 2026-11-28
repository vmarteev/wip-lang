# Wip

[![CI](https://github.com/vmarteev/wip-lang/actions/workflows/ci.yml/badge.svg)](https://github.com/vmarteev/wip-lang/actions/workflows/ci.yml)

Wip is a compiled systems language with memory safety and no garbage
collector, and no lifetimes to write. Its compiler is written in Wip.

The question it explores is how much of what a borrow checker gives can be
had without lifetime annotations:

- **Writing is second-class.** A `&var` exists only while a call runs, so
  two writers of one place can only be two arguments of one call.
- **Reading is tracked for you.** A `&x` may be kept, in a local or an
  `Option<&Node>`, and the compiler works out what it borrows from where
  it came from. Nothing outlives what it points at.
- **Ownership is decided where a value is written.** Nothing is counted at
  run time unless a program shares a value and says so.

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
            if self.cells[i] == 0 then
                return i

            i += 1
        }
        return .None
    }
}

fn main() = {
    var board = Board::of(9)
    board.at(0) = 7
    assert(board.at(0) == 7)

    val .Some(free) = board.firstEmpty() else panic("a fresh board has room")
    assert(free == 1)
}
```

## Features

- **Borrow checking without lifetimes**, as above.
- **Ownership without a garbage collector.** A value is freed where its
  owner ends, decided at compile time; `Destroy` says what else to do. A
  value with several owners is a `std::shared::Shared<T>`, counted only
  where a line calls `share()`.
- **Lending pairs.** `lend fn at(i): &T` declares the reading and the
  writing half from one body, so `v[i]` and `v[i] = x` come from one place.
- **Errors are values:** `Result`, `?`, and conversions only where a
  program wrote a `From`. A panic is for mistakes, not for failures the
  caller should handle.
- **Diagnostics that explain themselves:** a code, a place, a fix, and a
  note saying why the rule exists.
- **Fast builds.** Native code through LLVM. A debug build of ten
  thousand lines takes half a second, and a release build is optimised
  across the whole program.
- **C, directly.** Call C libraries, compile a module's C and C++ beside
  it, export functions to C, and generate bindings from a header with
  `wip bindgen`.
- **No build files.** A program's directory is its module and every
  directory it imports is another; there is no manifest and nothing is
  downloaded.
- **Tools included:** a formatter, a test runner, a fixer for what the
  warnings point out, and a language server for Zed and Neovim.

## Status

Wip is young: the language may still change from one version to the next.
It is complete enough for real programs, and has been tried by porting
existing ones to it: a C++23 game, built with raylib compiled from source,
which draws frames identical to the C++ build's, pixel for pixel, and
compiles in 0.19 s; a file manager; a web server; an interpreter; and a
compressor whose output matches the C original's byte for byte.

Not there yet:

- **Platforms** other than those under [Install](#install).
- **Packages fetched from anywhere:** a program depends on another by its
  path.
- **Asynchronous I/O:** threads, futures and an event queue that waits on
  many connections are there; a runtime that runs tasks on them is not,
  and neither is UDP.

See [`CHANGELOG.md`](CHANGELOG.md) for what each version changed.

## Install

Wip is built from source with `clang`, in a few minutes; no binaries are
published. It runs on:

| System | What it needs |
|---|---|
| macOS on Apple silicon | the Command Line Tools: `xcode-select --install` |
| iOS, iPadOS and the simulator | built from a Mac with Xcode |
| Linux on arm64 and x86-64 | `clang` 15 or newer and a C compiler, `cc`; for `std::tls`, `std::http` and `std::compress`, OpenSSL 3 and zlib (`libssl-dev`, `zlib1g-dev` on Debian) |
| Windows on arm64 and x86-64 | Visual Studio's Build Tools, with the C++ tools and "C++ Clang Compiler for Windows" |

Wip uses the `clang` on the `PATH`, or the one `WIP_CLANG` names. Debian 12
ships `clang` 14, so install `clang-15` or newer and set
`WIP_CLANG=clang-15`. On Windows, Wip finds Visual Studio's `clang` itself
and links with Microsoft's linker. Windows has no zlib, so `std::compress`
is not available there and `std::http` asks for bodies uncompressed;
`std::users`, Unix signals and `fork` are Unix-only.

### macOS and Linux

```sh
git clone https://github.com/vmarteev/wip-lang && cd wip-lang
git checkout "$(git describe --tags --abbrev=0)"      # the latest version, or stay on main
scripts/bootstrap.sh --out ~/wip
echo 'export PATH="$HOME/wip/bin:$PATH"' >> ~/.zshrc   # ~/.bashrc for bash
```

### Windows

In PowerShell:

```powershell
git clone https://github.com/vmarteev/wip-lang; cd wip-lang
git checkout (git describe --tags --abbrev=0)
powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1 -Out $HOME\wip
```

Then add `wip\bin` in your home directory to the `Path` in the system's
settings. `-ExecutionPolicy Bypass` is needed because Windows runs no
script it was not told to.

### What the install is

`bootstrap/` holds the compiler for each system as LLVM bitcode. `clang`
compiles it into a first `wip`, which builds the compiler from source, and
that compiler builds itself once more.

The result is a directory: `bin/wip`, with the standard library, `std/`,
and the tools, `tools/`, beside it. Keep them together: a link to
`bin/wip` from elsewhere on the `PATH` works, a copy of it alone does not.
`std/` and `tools/` link back to the checkout, so leave the checkout where
it is; a change to the library is seen by the next build.

To update, pull, then rebuild from source: `scripts/build.sh --release
--out ~/wip`, or `scripts\bootstrap.ps1` again on Windows. `wip --version`
gives the version, the commit it was built from, and that commit's date.

## Quick start

```wip
// hello.wip
import std::io

fn main() = {
    val name = "world"
    io::println("hello, \(name)")
}
```

```sh
wip run hello.wip     # compile and run
wip build hello.wip   # write an executable
wip test hello.wip    # run the program's tests
wip check hello.wip   # report every error, write nothing
wip fmt hello.wip     # format the file
wip fix hello.wip     # remove a needless clone() or move, as the warnings say
```

`wip --help` lists every command and option. For highlighting, errors as
you type, hover, go to definition, completion and rename in Zed and
Neovim, see [editors](editors/README.md).

## Documentation

- **[The cheat sheet](docs/language/cheat-sheet.md):** the language on one
  page.
- **[The language reference](docs/language/README.md):** twelve pages, by
  topic, each rule with an example that runs, and a
  [glossary](docs/language/glossary.md) of the words they use in a sense
  of their own.
- **[The grammar](docs/grammar.md):** exactly what the parser accepts.
- **[The examples](examples/README.md):** whole programs, from options and
  files to JSON, threads, a web server, packages and C.

Every Wip example in the reference and in this README is compiled with
each change to the compiler, so they stay true.

## Repository layout

| Directory | What is in it |
|---|---|
| `compiler/` | the compiler, in Wip: `syntax`, `hir` (type checking), `analysis` (moves and borrows), `mir`, `llvm`, `fmt`, and `lang`, the driver and the language server; `main.wip` is the `wip` command |
| `bootstrap/` | the compiler as LLVM bitcode for each system, which builds the first `wip` |
| `std/` | the standard library |
| `tools/` | tools `wip` builds when first asked, such as `wip bindgen` |
| `editors/` | Zed and Neovim support, and the Tree-sitter grammar |
| `examples/` | small, complete programs |
| `docs/` | the language reference and the grammar |
| `tests/` | the language's test cases, the standard library's tests, and the gate that checks `wip` |
| `scripts/` | building, testing and releasing |

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for how to build the compiler,
run the tests, and check a change on Linux and Windows.

## License

MIT; see [`LICENSE`](LICENSE). Parts of the standard library are
translated from others' work, under the licenses in
[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).
