#!/bin/bash
# Builds `wip` from this checkout with `clang` alone: no Rust, and no
# binary but what `clang` and the system's linker make.
#
#   scripts/bootstrap.sh [--out DIR] [--kept]
#
# `--kept` keeps a `wip` it built before, in the same `--out`, unless the
# seed is newer than it: what `scripts/build.sh` asks for, which builds the
# compiler with it where it is given no other `wip`.
#
# 1. Stage A: this system's seed, `bootstrap/<system>/wip.bc.gz`, compiled
#    by `clang` and linked as its `link.txt` says, with an empty table of
#    the calls a panic lists, since that table is made after `clang`
#    compiles: a `wip` whose only work is to build the next.
# 2. Stage B: stage A builds the compiler from `compiler/`, a release
#    build. A seed may be older than the source; B is the source's own
#    compiler, built by whatever the seed was.
# 3. Stage C: B builds it again, and B and C must write the same bitcode
#    for the compiler, which is the compiler compiling itself faithfully.
#
# C is the `wip` it leaves, laid out as an installed compiler is, in `--out`
# (`$WIP_TARGET_DIR/bootstrap` where none is named, `target/bootstrap`
# where that is not set): `bin/wip`, and beside it the checkout's `std/`
# and `tools/`. `clang` is `$WIP_CLANG`, or `clang`, 15 or newer; the
# linker is `$CC`, or `cc`.
set -euo pipefail
cd "$(dirname "$0")/.."

out=${WIP_TARGET_DIR:-target}/bootstrap
kept=false
while [ $# -gt 0 ]; do
    case $1 in
    --out)
        out=$2
        shift
        ;;
    --kept) kept=true ;;
    *)
        echo "usage: scripts/bootstrap.sh [--out DIR] [--kept]" >&2
        exit 2
        ;;
    esac
    shift
done

case "$(uname -s)-$(uname -m)" in
Darwin-arm64) system=macos-arm64 ;;
Linux-aarch64 | Linux-arm64) system=linux-arm64 ;;
Linux-x86_64) system=linux-x86_64 ;;
*)
    echo "error: there is no seed for $(uname -s) on $(uname -m)" >&2
    exit 2
    ;;
esac
seed=bootstrap/$system
if [ ! -f "$seed/wip.bc.gz" ]; then
    echo "error: there is no seed in $seed" >&2
    exit 2
fi
if $kept && [ -x "$out/bin/wip" ] && [ ! "$seed/wip.bc.gz" -nt "$out/bin/wip" ]; then
    exit 0
fi
clang=${WIP_CLANG:-clang}
work="$out/stages"
mkdir -p "$work"
started=$SECONDS
say() { printf '%-34s %ss\n' "$1" $((SECONDS - started)); }

# Stage A. The table of calls is a list of none: one word, its count.
gunzip -c "$seed/wip.bc.gz" >"$work/seed.bc"
printf '@"wip.frame_tables" = hidden global [1 x i64] zeroinitializer, align 8\n' >"$work/tables.ll"
"$clang" -O2 -Wno-override-module -c "$work/seed.bc" -o "$work/seed.o"
"$clang" -Wno-override-module -c "$work/tables.ll" -o "$work/tables.o"
link=()
while IFS= read -r line; do
    # A framework is two words, `-framework Name`.
    [ -n "$line" ] && read -r -a words <<<"$line" && link+=("${words[@]}")
done <"$seed/link.txt"
mkdir -p "$work/A/bin"
"${CC:-cc}" -o "$work/A/bin/wip" "$work/seed.o" "$work/tables.o" ${link[@]+"${link[@]}"}
say "stage A, from the seed"

# Stages B and C, each laid out as an installed compiler, the library the
# checkout's.
scripts/build.sh --release --with "$work/A/bin/wip" --out "$work/B" >/dev/null
say "stage B, built by A"
scripts/build.sh --release --with "$work/B/bin/wip" --out "$out" >/dev/null
say "stage C, built by B"

"$work/B/bin/wip" build --release --emit llvm-bc compiler/main.wip -o "$work/B.bc"
"$out/bin/wip" build --release --emit llvm-bc compiler/main.wip -o "$work/C.bc"
if ! cmp -s "$work/B.bc" "$work/C.bc"; then
    echo "error: stages B and C write different bitcode for the compiler" >&2
    exit 1
fi
say "B and C write the same bitcode"
echo "wip is $out/bin/wip: $("$out/bin/wip" --version)"
