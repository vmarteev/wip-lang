#!/bin/bash
# Regenerates the snapshots of the language's own cases: what each program
# in tests/cases prints, and what each one that must be refused reports.
# Run it after a change that means to change them, read the diff, and keep
# what is right.
#
#   scripts/snapshots.sh          every case
#   scripts/snapshots.sh 185      the cases whose name holds "185"
#
# `insta` writes the new snapshots in place when INSTA_UPDATE says so. The
# `script` around it gives cargo a terminal, which it wants before it will
# print while a test runs, and the redirection keeps that terminal from
# swallowing this script's own input. Linux's `script` is util-linux's,
# which takes the command as one argument and passes its status on only
# when asked; the BSD one on a Mac takes it as it is and always does.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

export WIP_CACHE_DIR="${WIP_CACHE_DIR:-$PWD/target/wip-cache}"
export INSTA_UPDATE=always

filter=${1:-}
if script --version 2>/dev/null | grep -q util-linux; then
    script -q -e -c "cargo test -q -p wip-lang --test cases -- $(printf %q "$filter")" /dev/null </dev/null
else
    script -q /dev/null cargo test -q -p wip-lang --test cases -- "$filter" </dev/null
fi
status=$?

if [ $status -ne 0 ]; then
    echo "the cases did not all pass; the snapshots that were written are still in the tree"
    exit $status
fi

changed=$(git status --porcelain tests/cases | wc -l | tr -d ' ')
if [ "$changed" = "0" ]; then
    echo "no snapshot changed"
else
    echo "$changed snapshot file(s) changed — read the diff before committing:"
    git status --short tests/cases
fi
