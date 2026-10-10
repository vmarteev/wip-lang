#!/bin/bash
# Checks the LLVM backend against every major `clang` it says it builds with
# — 15 and newer — on Linux, where Debian packages them side by side: for
# each, the cases, each case's release build compared with its debug
# build; the calls across the C boundary, whose third way is a release
# build; and the bitcode of every case, which each must read to the module
# it reads the text to. The compiler is built from this checkout as
# `scripts/verify.sh` builds its stage 1.
# Run before a release, and whenever the IR the backend writes changes; each
# `clang` is a build of every case, minutes in the guest, which is why the
# gate leaves it to this.
#
#   scripts/clangs.sh                  15, 16 and 19
#   WIP_CLANGS="16 19" scripts/clangs.sh
#
# Started on a machine that is not Linux, it runs itself in the guest
# scripts/linux.sh starts (WIP_LINUX_ARCH and the rest apply).
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

if [ "$(uname -s)" != "Linux" ]; then
    WIP_LINUX_RUN=scripts/clangs.sh exec scripts/linux.sh "$@"
fi

versions=${WIP_CLANGS:-15 16 19}
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
failed=0
say() { printf '%-34s %s\n' "$1" "$2"; }

export WIP_CACHE_DIR="${WIP_CACHE_DIR:-$PWD/target/wip-cache}"
export WIP_CHECK_MOVES="${WIP_CHECK_MOVES:-1}"

# The compiler, and the gate's checks, which run with each `clang`.
stage1="${WIP_TARGET_DIR:-target}/stages/1"
wip="$stage1/bin/wip"
if ! { scripts/build.sh --release --out "$stage1" &&
    "$wip" build --release tests/gate/main.wip -o "$out/gate"; } >"$out/build" 2>&1; then
    say "building" "FAILED"
    cat "$out/build"
    exit 1
fi
printf 'fn main() = {}\n' >"$out/main.wip"

for version in $versions; do
    clang=clang-$version
    if ! command -v "$clang" >/dev/null; then
        mkdir -p /work/apt/partial 2>/dev/null
        { apt-get update -qq && apt-get install -y -qq -o Dir::Cache::Archives=/work/apt "$clang"; } \
            >"$out/apt" 2>&1
    fi
    if ! command -v "$clang" >/dev/null; then
        say "$clang" "FAILED: not installable"
        cat "$out/apt"
        failed=1
        continue
    fi
    export WIP_CLANG=$clang
    # A `clang` that does not read the IR builds nothing: a release build
    # first says so here.
    if ! "$wip" build --release "$out/main.wip" -o "$out/main" >"$out/probe" 2>&1; then
        say "$clang" "FAILED: does not build with it"
        cat "$out/probe"
        failed=1
        continue
    fi
    if "$out/gate" "$wip" cases abi bitcode --read-only >"$out/tests" 2>&1; then
        say "$clang" "ok"
    else
        say "$clang" "FAILED"
        grep -v -E " ok$" "$out/tests" | head -40
        failed=1
    fi
done

[ $failed -eq 0 ] && echo "ALL OK"
exit $failed
