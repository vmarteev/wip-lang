# The seeds

Wip's compiler is written in Wip, and a compiler is needed to build it.
These are what builds the first one, on a machine with `clang` 15 or newer
and nothing of Wip's: for each system, the compiler as LLVM bitcode.

| Directory | System |
|---|---|
| `macos-arm64/` | macOS on Apple silicon |
| `linux-arm64/` | Linux on 64-bit Arm |
| `linux-x86_64/` | Linux on x86-64 |

Each holds:

- **`wip.bc.gz`** — the compiler, a release build without debug
  information, optimised for size by `clang` 15 and compressed. LLVM reads
  bitcode that an older LLVM wrote, so every `clang` from 15 on reads it.
- **`link.txt`** — the libraries and frameworks it is linked with, one a
  line; empty where the C library is all it needs.

## Building from them

```sh
scripts/bootstrap.sh
```

compiles this system's seed with `clang` into a `wip` (stage A), which
builds the compiler from `compiler/` (stage B), which builds it again
(stage C). B and C must write the same bitcode for the compiler: that is
the compiler compiling itself faithfully. C is the `wip` it leaves, in
`target/bootstrap/bin/wip`, beside the library it reads; `--out` puts it
elsewhere. A seed may be older than the source; stage B is always the
source's own compiler. `scripts/build.sh` and the gate build the compiler
with it where they are given no other `wip`, and build it first where it
is not there.

## Making them again

A seed is made again only when the compiler's source uses something the
seed's compiler cannot compile, so that the repository does not grow with
every version. `scripts/seed.sh`, run on a Mac with Apple silicon, makes
all three: the Mac's there, and the Linux ones in the Linux guests that
`scripts/linux.sh` starts.

A seed frees the build from Rust and from binaries, not from trusting
what made it. The longer way is the chain it shortens: the compiler in
Rust that Wip was first written in, which the versions before 0.3.0 are,
building the first compiler in Wip, and each version the next.
