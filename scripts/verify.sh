#!/bin/bash
# Everything that must pass before a commit: the Rust in `cargo fmt`'s
# layout and the Wip that is ours to lay out in `wip fmt`'s, the
# generated tables as their scripts write them, the editors' grammar,
# nothing pointing at a private document, every test, and clippy with
# warnings denied. Prints ALL OK when they all pass, and how long each step
# took, so that a run that has grown slower says where.
#
# The tests are run by starting every test binary at once rather than
# letting `cargo test` walk them one at a time, which is what it does. On
# this repo that is about twice as fast warm, and nearer three times after
# a change, because the binaries wait on each other for nothing. Clippy
# runs beside them: by then the binaries are built, so they need no lock
# and it can have one.
#
#   scripts/verify.sh          everything
#   scripts/verify.sh --fast   the language's own cases, and nothing else
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
fast=${1:-}
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

step=$SECONDS
if ! cargo fmt --all -- --check >"$out/fmt" 2>&1; then
    say "formatting" "FAILED" $((SECONDS - step))
    cat "$out/fmt"
    failed=1
else
    say "formatting" "ok" $((SECONDS - step))
fi

if [ "$fast" = "--fast" ]; then
    step=$SECONDS
    if cargo test -q -p wip-lang --test cases >"$out/cases" 2>&1; then
        say "cases" "ok" $((SECONDS - step))
    else
        say "cases" "FAILED" $((SECONDS - step))
        cat "$out/cases"
        failed=1
    fi
    [ $failed -eq 0 ] && echo "ALL OK in $(duration $SECONDS)"
    exit $failed
fi

# Build every test binary, and collect what was built.
step=$SECONDS
if ! cargo test -q --workspace --no-run >"$out/build" 2>&1; then
    say "building the tests" "FAILED" $((SECONDS - step))
    cat "$out/build"
    exit 1
fi
say "building the tests" "ok" $((SECONDS - step))
# `mapfile` is bash 4; macOS ships bash 3, so read the list the old way.
# Each binary runs in its own crate's directory, as `cargo test` runs it:
# a test that reads `../../tests/cases` means it from there. The `wip`
# command the tests run is noted too, and runs `wip fmt` below, so it is
# not built a second time.
binaries=()
homes=()
while IFS=$'\t' read -r home binary; do
    homes+=("$home")
    binaries+=("$binary")
done < <(
    cargo test -q --workspace --no-run --message-format=json 2>/dev/null |
        python3 -c "
import json, os, sys
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if not message.get('executable'):
        continue
    if message.get('profile', {}).get('test'):
        home = os.path.dirname(message.get('manifest_path', ''))
        print(home + chr(9) + message['executable'])
    elif message.get('target', {}).get('name') == 'wip':
        open(sys.argv[1], 'w').write(message['executable'])
" "$out/wip"
)
wip=$(cat "$out/wip" 2>/dev/null)

# The Wip that is ours to lay out is in `wip fmt`'s layout: the standard
# library and its tests, the examples, and bindgen. Not the cases in
# tests/cases, which test the parser on layouts the formatter rewrites —
# trailing commas, line breaks, comments — and on text that does not parse.
step=$SECONDS
[ -x "$wip" ] || echo "the build of the tests made no \`wip\` command" >"$out/wip-fmt"
if [ ! -x "$wip" ] || ! "$wip" fmt --check std tests/std examples tools/bindgen >"$out/wip-fmt" 2>&1; then
    say "wip fmt" "FAILED" $((SECONDS - step))
    cat "$out/wip-fmt"
    failed=1
else
    say "wip fmt" "ok" $((SECONDS - step))
fi

# The table of character widths is what its script writes.
step=$SECONDS
if ! python3 scripts/char-widths.py --check >"$out/widths" 2>&1; then
    say "character widths" "FAILED" $((SECONDS - step))
    cat "$out/widths"
    failed=1
else
    say "character widths" "ok" $((SECONDS - step))
fi

# The tables of letters and numbers are what their script writes.
step=$SECONDS
if ! python3 scripts/char-category.py --check >"$out/category" 2>&1; then
    say "character category" "FAILED" $((SECONDS - step))
    cat "$out/category"
    failed=1
else
    say "character category" "ok" $((SECONDS - step))
fi

# The case tables are what their script writes.
step=$SECONDS
if ! python3 scripts/char-case.py --check >"$out/case" 2>&1; then
    say "character case" "FAILED" $((SECONDS - step))
    cat "$out/case"
    failed=1
else
    say "character case" "ok" $((SECONDS - step))
fi

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

# The binaries need no lock; clippy does, and nothing else wants it now.
# Each notes when it ended, as `$SECONDS` counts, which a subshell goes on
# counting from its parent's.
step=$SECONDS
(
    cargo clippy --all-targets --quiet -- -D warnings >"$out/clippy" 2>&1
    status=$?
    echo $SECONDS >"$out/clippy.end"
    exit $status
) &
clippy=$!

pids=()
for i in "${!binaries[@]}"; do
    name=$(basename "${binaries[$i]}")
    (
        cd "${homes[$i]}" && "${binaries[$i]}"
        status=$?
        echo $SECONDS >"$out/$name.end"
        exit $status
    ) </dev/null >"$out/$name" 2>&1 &
    pids+=($!)
done

tests_failed=0
for i in "${!pids[@]}"; do
    if ! wait "${pids[$i]}"; then
        name=$(basename "${binaries[$i]}")
        ended=$SECONDS
        [ -f "$out/$name.end" ] && ended=$(cat "$out/$name.end")
        say "${name%-*}" "FAILED" $((ended - step))
        # What failed, not the hundreds of lines that passed around it.
        grep -v ' \.\.\. ok$' "$out/$name" | tail -40
        tests_failed=1
        failed=1
    fi
done
[ $tests_failed -eq 0 ] && say "tests (${#binaries[@]} binaries, at once)" "ok" $((SECONDS - step))
# The three that took longest, which is where the time of the tests goes;
# a binary's name is its crate's test, without the hash cargo gives it.
slowest=""
for i in "${!binaries[@]}"; do
    name=$(basename "${binaries[$i]}")
    [ -f "$out/$name.end" ] && echo "$(($(cat "$out/$name.end") - step)) ${name%-*}"
done | sort -rn | head -3 >"$out/slowest"
while read -r seconds name; do
    slowest+="${slowest:+, }$name $(duration "$seconds")"
done <"$out/slowest"
[ -n "$slowest" ] && printf '%-34s %s\n' "  the slowest" "$slowest"

# Clippy's own time, from when it started beside the tests to when it ended.
wait $clippy
clippy_status=$?
clippy_took=$((SECONDS - step))
[ -f "$out/clippy.end" ] && clippy_took=$(($(cat "$out/clippy.end") - step))
if [ $clippy_status -eq 0 ]; then
    say "clippy" "ok" $clippy_took
else
    say "clippy" "FAILED" $clippy_took
    cat "$out/clippy"
    failed=1
fi

if [ $failed -eq 0 ]; then
    echo "ALL OK in $(duration $SECONDS)"
else
    echo "FAILED in $(duration $SECONDS)"
fi
exit $failed
