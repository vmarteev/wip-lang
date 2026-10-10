#!/bin/bash
# Makes the seeds, `bootstrap/<system>/`, which `scripts/bootstrap.sh`
# builds a `wip` from with `clang` alone: for each system, the compiler as
# LLVM bitcode, `wip.bc.gz`, and the libraries it links, `link.txt`.
#
#   scripts/seed.sh [--with WIP]
#
# Run on a Mac with Apple silicon: it makes the Mac's seed here, and the
# Linux ones in the guests `scripts/linux.sh` starts, arm64 and x86-64, the
# second through Rosetta and slow. In each, the compiler is built from
# `compiler/` by the `wip` there — `--with` here, where it is given one,
# and otherwise, and in the guests, the one that system's seed builds — and
# that compiler writes its own bitcode, a release build without debug
# information, which a seed has no use for. The three are then optimised
# for size by `clang-15`, the oldest `clang` Wip builds with, in the arm64
# guest: LLVM reads bitcode older versions wrote, so every `clang` it
# supports reads a seed, on every system.
#
# A seed is made again only when the compiler's source uses what the seed's
# compiler cannot compile; the commit that changes it says which commit's
# compiler it is. Inside a guest it is run with `--step`, which it gives
# itself.
set -euo pipefail
cd "$(dirname "$0")/.."

with=""
step=""
while [ $# -gt 0 ]; do
    case $1 in
    --with)
        with=$2
        shift
        ;;
    --step)
        step=$2
        shift
        ;;
    *)
        echo "usage: scripts/seed.sh [--with WIP]" >&2
        exit 2
        ;;
    esac
    shift
done

# The system a seed is for, as `bootstrap/` names it.
system() {
    case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) echo macos-arm64 ;;
    Linux-aarch64 | Linux-arm64) echo linux-arm64 ;;
    Linux-x86_64) echo linux-x86_64 ;;
    *)
        echo "error: no seed is made for $(uname -s) on $(uname -m)" >&2
        exit 2
        ;;
    esac
}

raw=target/seed

# This system's compiler as bitcode, unoptimised, and the libraries it
# links: `raw/<system>.bc` and `raw/<system>.link`. The compiler is built
# from the source by `$with`, or the seed's compiler, through a C compiler
# that writes down how it is called, so that what the link names past its
# objects is known: the last call is the compiler's own link.
make_raw() {
    local name out log
    name=$(system)
    out=$(mktemp -d)
    log="$out/calls.txt"
    printf '#!/bin/sh\necho "$*" >> "%s"\nexec %s "$@"\n' "$log" "${CC:-cc}" >"$out/cc.sh"
    chmod +x "$out/cc.sh"
    CC="$out/cc.sh" WIP_CACHE_DIR="$out/cache" scripts/build.sh --release ${with:+--with "$with"} --out "$out/stage"
    mkdir -p "$raw"
    WIP_HOME="$PWD" "$out/stage/bin/wip" build --release --no-debug-info --emit llvm-bc \
        compiler/main.wip -o "$raw/$name.bc"
    # The link is the last call: past `-o <program>` and the objects, the
    # libraries and frameworks, one a line.
    tail -1 "$log" | tr ' ' '\n' | awk '
        skip { skip = 0; next }
        $0 == "-o" { skip = 1; next }
        /\.o$/ || $0 == "" { next }
        $0 == "-framework" { framework = 1; next }
        framework { print "-framework " $0; framework = 0; next }
        { print }' >"$raw/$name.link"
    rm -rf "$out"
    echo "$name: $(du -h "$raw/$name.bc" | cut -f1) of bitcode"
}

# Every raw seed, optimised for size by clang-15 and compressed, into
# `bootstrap/<system>/`.
optimise() {
    if ! command -v clang-15 >/dev/null; then
        mkdir -p /work/apt/partial
        { apt-get update -qq && apt-get install -y -qq -o Dir::Cache::Archives=/work/apt clang-15; } >/dev/null
    fi
    local bc name
    for bc in "$raw"/*.bc; do
        name=$(basename "$bc" .bc)
        mkdir -p "bootstrap/$name"
        clang-15 -Oz -c -emit-llvm -Wno-override-module "$bc" -o "$raw/$name.oz.bc"
        gzip -9 -n -c "$raw/$name.oz.bc" >"bootstrap/$name/wip.bc.gz"
        cp "$raw/$name.link" "bootstrap/$name/link.txt"
        echo "$name: $(du -h "bootstrap/$name/wip.bc.gz" | cut -f1) gzipped"
    done
}

case "$step" in
raw)
    make_raw
    exit 0
    ;;
optimise)
    optimise
    exit 0
    ;;
"") ;;
*)
    echo "error: no step \`$step\`" >&2
    exit 2
    ;;
esac

if [ "$(system)" != macos-arm64 ]; then
    echo "error: the seeds are made from a Mac with Apple silicon, which starts the Linux guests" >&2
    exit 2
fi
rm -rf "$raw"
make_raw
WIP_LINUX_RUN=scripts/seed.sh scripts/linux.sh --step raw
WIP_LINUX_RUN=scripts/seed.sh WIP_LINUX_ARCH=amd64 scripts/linux.sh --step raw
WIP_LINUX_RUN=scripts/seed.sh scripts/linux.sh --step optimise
echo "the seeds are in bootstrap/, made by the compiler of $(git --no-pager rev-parse --short=12 HEAD)"
