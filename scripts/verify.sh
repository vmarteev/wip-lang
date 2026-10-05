#!/bin/bash
# Everything that must pass before a commit: the Rust in `cargo fmt`'s
# layout and the standard library in `wip fmt`'s, the generated tables as
# their scripts write them, the editors' grammar, no decision cited, every
# test, and clippy with warnings denied. Prints ALL OK when they all pass.
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

say() { printf '%-34s %s\n' "$1" "$2"; }

# The C the tests compile is kept in a cache of the repository's own, under
# target/, rather than the user's. A caller that sets it — scripts/linux.sh,
# which keeps a cache per platform — is left alone.
export WIP_CACHE_DIR="${WIP_CACHE_DIR:-$PWD/target/wip-cache}"

# Every program the tests build checks its moves: a value dropped after it
# was moved away panics where it is dropped. A caller may say otherwise.
export WIP_CHECK_MOVES="${WIP_CHECK_MOVES:-1}"

if ! cargo fmt --all -- --check >"$out/fmt" 2>&1; then
    say "formatting" "FAILED"
    cat "$out/fmt"
    failed=1
fi

if [ "$fast" = "--fast" ]; then
    if cargo test -q -p wip-lang --test cases >"$out/cases" 2>&1; then
        say "cases" "ok"
    else
        say "cases" "FAILED"
        cat "$out/cases"
        failed=1
    fi
    [ $failed -eq 0 ] && echo "ALL OK"
    exit $failed
fi

# Build every test binary, and collect what was built.
if ! cargo test -q --workspace --no-run >"$out/build" 2>&1; then
    say "building the tests" "FAILED"
    cat "$out/build"
    exit 1
fi
# `mapfile` is bash 4; macOS ships bash 3, so read the list the old way.
# Each binary runs in its own crate's directory, as `cargo test` runs it:
# a test that reads `../../tests/cases` means it from there.
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
    if message.get('executable') and message.get('profile', {}).get('test'):
        home = os.path.dirname(message.get('manifest_path', ''))
        print(home + chr(9) + message['executable'])
"
)

# The standard library is written in `wip fmt`'s layout.
if ! cargo run -q --bin wip -- fmt --check std >"$out/wip-fmt" 2>&1; then
    say "wip fmt (std)" "FAILED"
    cat "$out/wip-fmt"
    failed=1
else
    say "wip fmt (std)" "ok"
fi

# The table of character widths is what its script writes.
if ! python3 scripts/char-widths.py --check >"$out/widths" 2>&1; then
    say "character widths" "FAILED"
    cat "$out/widths"
    failed=1
else
    say "character widths" "ok"
fi

# The case tables are what their script writes.
if ! python3 scripts/char-case.py --check >"$out/case" 2>&1; then
    say "character case" "FAILED"
    cat "$out/case"
    failed=1
else
    say "character case" "ok"
fi

# The editors' grammar parses every file, where tree-sitter is installed.
if command -v tree-sitter >/dev/null; then
    if ! scripts/tree-sitter.sh --check >"$out/tree-sitter" 2>&1; then
        say "editors' grammar" "FAILED"
        grep -v -E 'parser directories|init-config|configuration file|language grammars' "$out/tree-sitter" | tail -20
        failed=1
    else
        say "editors' grammar" "ok"
    fi
else
    say "editors' grammar" "skipped: no tree-sitter"
fi

# Nothing published cites a design record by its number.
if ! scripts/citations.sh >"$out/citations" 2>&1; then
    say "citations" "FAILED"
    cat "$out/citations"
    failed=1
else
    say "citations" "ok"
fi

# The binaries need no lock; clippy does, and nothing else wants it now.
cargo clippy --all-targets --quiet -- -D warnings >"$out/clippy" 2>&1 &
clippy=$!

pids=()
for i in "${!binaries[@]}"; do
    name=$(basename "${binaries[$i]}")
    ( cd "${homes[$i]}" && "${binaries[$i]}" ) </dev/null >"$out/$name" 2>&1 &
    pids+=($!)
done

for i in "${!pids[@]}"; do
    if ! wait "${pids[$i]}"; then
        name=$(basename "${binaries[$i]}")
        say "$name" "FAILED"
        # What failed, not the hundreds of lines that passed around it.
        grep -v ' \.\.\. ok$' "$out/$name" | tail -40
        failed=1
    fi
done
[ $failed -eq 0 ] && say "tests (${#binaries[@]} binaries, at once)" "ok"

if wait $clippy; then
    say "clippy" "ok"
else
    say "clippy" "FAILED"
    cat "$out/clippy"
    failed=1
fi

[ $failed -eq 0 ] && echo "ALL OK"
exit $failed
