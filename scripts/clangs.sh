#!/bin/bash
# Checks the LLVM backend against every major `clang` it says it builds with
# — 15 and newer — on Linux, where Debian packages them side by side: for
# each, the case suite, whose release builds are LLVM's and are compared
# with its debug builds, and `c_abi`, whose third way is a release build.
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

if ! cargo test -q -p wip-lang --test cases --test c_abi --no-run >"$out/build" 2>&1; then
    say "building" "FAILED"
    cat "$out/build"
    exit 1
fi
# The compiler the tests run.
wip=${CARGO_TARGET_DIR:-target}/debug/wip
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
    # The release builds below fall back to Cranelift, saying so, where
    # the `clang` does not read the IR; insisting on LLVM first makes that
    # a failure here.
    if ! "$wip" build --release --backend llvm "$out/main.wip" -o "$out/main" >"$out/probe" 2>&1; then
        say "$clang" "FAILED: does not build with it"
        cat "$out/probe"
        failed=1
        continue
    fi
    if cargo test -q -p wip-lang --test cases --test c_abi >"$out/tests" 2>&1; then
        say "$clang" "ok"
    else
        say "$clang" "FAILED"
        grep -E "^test .* FAILED|^---- |test result" "$out/tests"
        failed=1
    fi
    find tests -name '*.snap.new' -delete
done

[ $failed -eq 0 ] && echo "ALL OK"
exit $failed
