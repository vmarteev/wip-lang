#!/bin/bash
# Everything that must pass before a commit: the generated tables as their
# scripts write them, the editors' grammar, nothing pointing at a private
# document, the compiler built twice — by the `wip` it is given, then by
# itself — the two writing the same bitcode for it, the Wip that is ours to
# lay out in `wip fmt`'s layout, and the gate's checks of `wip`,
# `tests/gate`, on the second. Prints ALL OK when they all pass, and how
# long each step took, so that a run that has grown slower says where.
#
#   scripts/verify.sh              everything
#   scripts/verify.sh --fast       the language's own cases, and nothing else
#   scripts/verify.sh --with WIP   stage 1 built by WIP
#
# Stage 1 is built by the `wip` `--with` names, or by the one
# `scripts/bootstrap.sh` builds from this system's seed, which is kept and
# built again only when the seed changes. Stage 1 is the source's compiler
# built by another; stage 2, which stage 1 builds, is the source's compiler
# built by itself, and the one the gate checks. The stages are kept under
# `$WIP_TARGET_DIR`, `target` unless it says otherwise, which a Linux guest
# keeps apart from the Mac's.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
fast=false
with=()
while [ $# -gt 0 ]; do
    case $1 in
    --fast) fast=true ;;
    --with)
        with=(--with "$2")
        shift
        ;;
    *)
        echo "usage: scripts/verify.sh [--fast] [--with WIP]" >&2
        exit 2
        ;;
    esac
    shift
done
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
failed=0

# A step's name, how it went, and how many seconds it took, written as
# minutes and seconds.
say() { printf '%-34s %-8s %s\n' "$1" "$2" "${3:+$(duration "$3")}"; }
duration() {
    if [ "$1" -ge 60 ]; then printf '%dm%02ds' $(($1 / 60)) $(($1 % 60)); else printf '%ds' "$1"; fi
}

# The C the tests compile is kept in a cache of the repository's own, under
# target/, rather than the user's. A caller that sets it — scripts/linux.sh,
# which keeps a cache per platform — is left alone.
export WIP_CACHE_DIR="${WIP_CACHE_DIR:-$PWD/target/wip-cache}"

# Every program the tests build checks its moves: a value dropped after it
# was moved away panics where it is dropped. A caller may say otherwise.
export WIP_CHECK_MOVES="${WIP_CHECK_MOVES:-1}"

target=${WIP_TARGET_DIR:-target}
stages="$target/stages"

# The `wip` that builds stage 1: the seed's, built here where it is not
# kept, said apart from stage 1 since it takes minutes.
if [ ${#with[@]} -eq 0 ]; then
    step=$SECONDS
    if scripts/bootstrap.sh --kept --out "$target/bootstrap" >"$out/bootstrap" 2>&1; then
        say "the seed's compiler" "ok" $((SECONDS - step))
    else
        say "the seed's compiler" "FAILED" $((SECONDS - step))
        cat "$out/bootstrap"
        exit 1
    fi
    with=(--with "$target/bootstrap/bin/wip")
fi

# Stage 1, a release build, as every stage is.
step=$SECONDS
if scripts/build.sh --release "${with[@]}" --out "$stages/1" >"$out/stage1" 2>&1; then
    say "the compiler, built (stage 1)" "ok" $((SECONDS - step))
else
    say "the compiler, built (stage 1)" "FAILED" $((SECONDS - step))
    cat "$out/stage1"
    exit 1
fi

if $fast; then
    step=$SECONDS
    wip="$stages/1/bin/wip"
    if "$wip" build --release tests/gate/main.wip -o "$out/gate" >"$out/cases" 2>&1 &&
        "$out/gate" "$wip" cases >>"$out/cases" 2>&1; then
        say "cases" "ok" $((SECONDS - step))
    else
        say "cases" "FAILED" $((SECONDS - step))
        cat "$out/cases"
        failed=1
    fi
    [ $failed -eq 0 ] && echo "ALL OK in $(duration $SECONDS)"
    exit $failed
fi

# Stage 2: stage 1 builds the compiler again, and each writes the
# compiler's bitcode. They are one source compiled by two compilers meant to
# be the same, so the bitcode is the same bytes, or the compiler does not
# compile itself faithfully.
step=$SECONDS
if {
    scripts/build.sh --release --with "$stages/1/bin/wip" --out "$stages/2" &&
        "$stages/1/bin/wip" build --release --emit llvm-bc compiler/main.wip -o "$out/1.bc" &&
        "$stages/2/bin/wip" build --release --emit llvm-bc compiler/main.wip -o "$out/2.bc"
} </dev/null >"$out/stage2" 2>&1 && cmp -s "$out/1.bc" "$out/2.bc"; then
    say "the compiler, built by itself" "ok" $((SECONDS - step))
else
    say "the compiler, built by itself" "FAILED" $((SECONDS - step))
    tail -20 "$out/stage2"
    [ -f "$out/2.bc" ] && echo "stage 1 and stage 2 write different bitcode for the compiler"
    exit 1
fi
wip="$stages/2/bin/wip"

# The gate's checks of `wip`, a program in Wip, which runs below.
step=$SECONDS
if "$wip" build --release tests/gate/main.wip -o "$out/gate" >"$out/gate-build" 2>&1; then
    say "the gate, built" "ok" $((SECONDS - step))
else
    say "the gate, built" "FAILED" $((SECONDS - step))
    cat "$out/gate-build"
    failed=1
fi

# The Wip that is ours to lay out is in `wip fmt`'s layout: the standard
# library and its tests, the gate, the examples, bindgen, and the compiler.
# Not the cases in tests/cases, which test the parser on layouts the
# formatter rewrites — trailing commas, line breaks, comments — and on text
# that does not parse.
step=$SECONDS
if ! "$wip" fmt --check std tests/std tests/gate examples tools/bindgen compiler >"$out/wip-fmt" 2>&1; then
    say "wip fmt" "FAILED" $((SECONDS - step))
    cat "$out/wip-fmt"
    failed=1
else
    say "wip fmt" "ok" $((SECONDS - step))
fi

# The tables of character widths, letters and numbers, and case are what
# their scripts write.
for table in widths category case; do
    step=$SECONDS
    if ! python3 "scripts/char-$table.py" --check >"$out/$table" 2>&1; then
        say "character $table" "FAILED" $((SECONDS - step))
        cat "$out/$table"
        failed=1
    else
        say "character $table" "ok" $((SECONDS - step))
    fi
done

# The editors' grammar parses every file, where tree-sitter is installed.
step=$SECONDS
if command -v tree-sitter >/dev/null; then
    if ! scripts/tree-sitter.sh --check >"$out/tree-sitter" 2>&1; then
        say "editors' grammar" "FAILED" $((SECONDS - step))
        grep -v -E 'parser directories|init-config|configuration file|language grammars' "$out/tree-sitter" | tail -20
        failed=1
    else
        say "editors' grammar" "ok" $((SECONDS - step))
    fi
else
    say "editors' grammar" "skipped: no tree-sitter"
fi

# Nothing published points at a document that is not.
step=$SECONDS
if ! scripts/citations.sh >"$out/citations" 2>&1; then
    say "citations" "FAILED" $((SECONDS - step))
    cat "$out/citations"
    failed=1
else
    say "citations" "ok" $((SECONDS - step))
fi

# The gate, on stage 2: every check, each saying how many items it ran,
# and what was found writes itself beside a snapshot that differs.
if [ -x "$out/gate" ]; then
    step=$SECONDS
    if "$out/gate" "$wip" </dev/null >"$out/checks" 2>&1; then
        say "the gate's checks" "ok" $((SECONDS - step))
        sed 's/^/  /' "$out/checks"
    else
        say "the gate's checks" "FAILED" $((SECONDS - step))
        cat "$out/checks"
        failed=1
    fi
fi

if [ $failed -eq 0 ]; then
    echo "ALL OK in $(duration $SECONDS)"
else
    echo "FAILED in $(duration $SECONDS)"
fi
exit $failed
