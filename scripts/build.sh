#!/bin/bash
# Builds the compiler from its source, `compiler/`, with a `wip` it is
# given, into a directory laid out as an installed compiler is: `bin/wip`,
# and beside it its home, the library `std/` and the tools `tools/`, which
# here are the checkout's own. Every script that builds the compiler builds
# it here, so that each is the same build.
#
#   scripts/build.sh [--release] [--with WIP] [--out DIR]
#
# `--with` names the `wip` that builds it: a `wip` installed, or one built
# before. Where none is named it is the one `scripts/bootstrap.sh` builds
# from this system's seed, which is built first where it is not there yet or
# the seed is newer than it: a `wip` older than the source may not compile
# it, and the seed's always does. `--out` is where the directory goes,
# `$WIP_TARGET_DIR/wip` where none is named, `$WIP_TARGET_DIR` being
# `target` unless it says otherwise.
#
# Before the build it writes the commit checked out and its day to
# `compiler/lang/commit.txt`, which git ignores and the compiler embeds, so
# that `wip --version` names them; a copy of the source with no history
# writes none, and the compiler says its version alone.
set -euo pipefail
cd "$(dirname "$0")/.."

target=${WIP_TARGET_DIR:-target}
release=""
with=""
out=$target/wip
while [ $# -gt 0 ]; do
    case $1 in
    --release) release=--release ;;
    --with)
        with=$2
        shift
        ;;
    --out)
        out=$2
        shift
        ;;
    *)
        echo "usage: scripts/build.sh [--release] [--with WIP] [--out DIR]" >&2
        exit 2
        ;;
    esac
    shift
done

commit=compiler/lang/commit.txt
if hash=$(git --no-pager rev-parse --short=12 HEAD 2>/dev/null) &&
    day=$(git --no-pager log -1 --format=%cs HEAD 2>/dev/null); then
    # Written again only where it changes, so that a build kept in a cache
    # is not made again for nothing, and under a name of its own first, since
    # a Linux guest may build in this checkout at the same time.
    partial="$commit.$$.partial"
    printf '%s %s' "$hash" "$day" >"$partial"
    if cmp -s "$partial" "$commit"; then rm "$partial"; else mv "$partial" "$commit"; fi
else
    rm -f "$commit"
fi

if [ -z "$with" ]; then
    scripts/bootstrap.sh --kept --out "$target/bootstrap" >&2
    with=$target/bootstrap/bin/wip
fi

mkdir -p "$out/bin"
# The home is the checkout's: a change to the library is seen by the next
# run, as an installed compiler's would be by the next version. The links
# are made again each time, since a Linux guest's build directory outlives
# the guest, and the next may mount the checkout elsewhere.
for dir in std tools; do
    ln -sfn "$PWD/$dir" "$out/$dir"
done

# The compiler that builds it is told that its library is the checkout's,
# which the source is written against, rather than its own.
WIP_HOME="$PWD" "$with" build $release compiler/main.wip -o "$out/bin/wip"
