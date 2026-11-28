# Contributing to Wip

Everything the language does is `wip --help`; everything done to the
repository is a script in `scripts/`. The scripts are the interface: a
script that says what it does outlives whichever tool is underneath it.

## Building

```sh
scripts/bootstrap.sh           # build the compiler from the seed, with clang
scripts/build.sh [--release]   # build it from its source, into target/wip
```

`target/wip/bin/wip` uses the checkout's standard library, so a change to
`std/` needs no rebuild; a change to `compiler/` does.

## Testing

```sh
scripts/verify.sh              # the gate: the compiler built twice, formatting, every check
scripts/verify.sh --fast       # just the language's own cases, in a second
scripts/snapshots.sh [filter]  # regenerate what the cases expect, then read the diff
scripts/tree-sitter.sh         # the editors' grammar: regenerate, test, parse everything
```

A change is ready when `scripts/verify.sh` prints `ALL OK`. It:

1. builds the compiler from source with the `wip` the seed builds, then
   has that compiler build itself, and checks that the two write the same
   bitcode;
2. checks that `wip fmt` leaves every file as it is;
3. runs `tests/gate` with the second compiler: a program in Wip that
   checks `wip` by running it, on the test cases and their snapshots, the
   standard library's tests, the driver, the compiler's own tests
   (including a robustness pass over mangled input), and every example in
   the README and the language reference.

An example in a document must compile and be formatted as `wip fmt` lays
it out. A ```` ```wip,run ```` example must run to a successful end, and a
```` ```wip,error=E0303 ```` example must be refused with that code, so a
document that stops being true fails the gate.

### Test cases

A case is a directory in `tests/cases/ok/` (a program that must compile
and run, with its output as a snapshot) or `tests/cases/err/` (one that
must be refused, with its diagnostics as a snapshot). After a change,
`scripts/snapshots.sh` rewrites the snapshots that differ; read the diff
before keeping it.

## Other systems

Wip is developed on Apple arm64. The differences that bite turn up on
other systems: the C ABI, the width of `c_long`, what the linker wants.

```sh
scripts/linux.sh [--fast]      # the same verification on Linux, in a container
scripts/windows.sh gate        # the same on a Windows machine, over ssh
scripts/clangs.sh              # the LLVM backend through clang 15, 16 and 19, on Linux
```

- **Linux:** `scripts/linux.sh` runs the verification in a container. It
  uses [Apple's `container`](https://github.com/apple/container) where
  that is installed, and Docker otherwise. It builds into a volume of its
  own, so the host's build is untouched and a second run is warm.
  `WIP_LINUX_ARCH=amd64` runs an x86-64 guest instead of the arm64 one,
  through Rosetta. One run at a time: the build volume attaches to one
  guest.
- **Windows:** `scripts\verify.ps1` is the same verification in
  PowerShell: the compiler built from the seed by `scripts\bootstrap.ps1`,
  its stages compared, and every check of the gate but the iOS
  simulator's and the debugger's. `scripts/windows.sh gate` runs it on the
  machine `WIP_WINDOWS_HOST` names, over ssh.
- **Every `clang` the backend supports:** `scripts/clangs.sh` runs the
  cases' release builds and the C ABI's tests through `clang` 15, 16 and
  19 in the Linux guest. It is too slow for the gate; run it after a
  change to the IR the backend writes.

CI runs the gate on each push: `verify.sh` on Linux x86-64 and on a Mac,
and `verify.ps1` on Windows arm64 and x86-64.

`wip bindgen`'s test asks `clang` for a header's syntax tree, and is
skipped where `clang` is not installed.

## Writing Wip

`wip fmt` decides the layout; run it on what you write. A change to the
syntax also changes [the grammar](docs/grammar.md), the Tree-sitter
grammar in `editors/tree-sitter-wip`, and the page of
[the reference](docs/language/README.md) it touches.
