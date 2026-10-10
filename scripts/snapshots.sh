#!/bin/bash
# Regenerates the snapshots of the language's own cases — what each program
# in tests/cases prints, and what each one that must be refused reports —
# and the gate's others, as what `wip bindgen` writes for its header.
# Run it after a change that means to change them, read the diff, and keep
# what is right.
#
#   scripts/snapshots.sh          every case
#   scripts/snapshots.sh 185      the cases whose name holds "185"
#
# The gate's check of the cases, `tests/gate`, run with the compiler built
# from this checkout as `scripts/verify.sh` builds its stage 1, writes what
# it found beside each snapshot that differs, `*.snap.new`; this puts each
# in its snapshot's place.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

export WIP_CACHE_DIR="${WIP_CACHE_DIR:-$PWD/target/wip-cache}"
export WIP_CHECK_MOVES="${WIP_CHECK_MOVES:-1}"

filter=${1:-}
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

stage1="${WIP_TARGET_DIR:-target}/stages/1"
if ! scripts/build.sh --release --out "$stage1" >"$out/build" 2>&1; then
    cat "$out/build"
    exit 1
fi
wip="$stage1/bin/wip"
if ! "$wip" build --release tests/gate/main.wip -o "$out/gate" >"$out/build" 2>&1; then
    cat "$out/build"
    exit 1
fi
"$out/gate" "$wip" cases bindgen --only "$filter" >"$out/cases"
status=$?

kept=0
while IFS= read -r new; do
    mv "$new" "${new%.new}"
    kept=$((kept + 1))
done < <(find tests -name '*.snap.new')

# A case that failed for a reason no snapshot holds — a program that does
# not build, or a release build that prints something else — is not mended
# by keeping one: the check runs again, and says so.
if [ $status -ne 0 ] && [ $kept -gt 0 ]; then
    "$out/gate" "$wip" cases bindgen --only "$filter" >"$out/cases"
    status=$?
fi
if [ $status -ne 0 ]; then
    echo "the cases did not all pass:"
    cat "$out/cases"
fi

changed=$(git status --porcelain tests/cases tests/bindgen | wc -l | tr -d ' ')
if [ "$changed" = "0" ]; then
    echo "no snapshot changed"
else
    echo "$changed snapshot file(s) changed — read the diff before committing:"
    git status --short tests/cases tests/bindgen
fi
exit $status
