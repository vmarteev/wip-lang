#!/bin/bash
# Makes the directory a version of Wip is installed as, for this system,
# and its archive: `wip-<version>-<system>/`, which holds `bin/wip`, the
# standard library `std/`, the tools `tools/`, the license and the notices
# of the work the library is translated from; and beside it
# `wip-<version>-<system>.tar.gz`.
#
#   scripts/package.sh [--out DIR]    DIR is target/dist where none is named
#
# The compiler is built from this system's seed with `clang` alone, by
# `scripts/bootstrap.sh`, in a directory of its own made afresh, whatever
# was built before: a version whose source the seed cannot build is not
# published. Built where git knows no commit, it says its version alone.
# The directory is then tried as a user has it, from elsewhere and with no
# `WIP_HOME`: it runs a program.
#
# `scripts/publish.sh` runs it on a Mac and, through `scripts/linux.sh`, in
# each Linux guest.
set -euo pipefail
cd "$(dirname "$0")/.."

out=target/dist
while [ $# -gt 0 ]; do
    case $1 in
    --out)
        out=$2
        shift
        ;;
    *)
        echo "usage: scripts/package.sh [--out DIR]" >&2
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
    echo "error: Wip is not packaged for $(uname -s) on $(uname -m)" >&2
    exit 2
    ;;
esac
version=$(sed -n 's/^pub val VERSION: str = "\(.*\)"$/\1/p' compiler/lang/version.wip)
name=wip-$version-$system

build="${WIP_TARGET_DIR:-target}/package"
rm -rf "$build"
scripts/bootstrap.sh --out "$build"

# The checkout's library and tools, copied: what the build has beside it
# are links to them.
mkdir -p "$out"
out=$(cd "$out" && pwd)
rm -rf "${out:?}/$name" "$out/$name.tar.gz"
mkdir -p "$out/$name/bin"
cp "$build/bin/wip" "$out/$name/bin/wip"
cp -R std tools "$out/$name/"
cp LICENSE THIRD-PARTY-NOTICES.md "$out/$name/"

# Tried as installed: from another directory, with nothing of the checkout's.
try=$(mktemp -d)
printf 'import std::io\n\nfn main() = io::println("hello from \\(1 + 2)")\n' >"$try/main.wip"
said=$(cd "$try" && env -u WIP_HOME WIP_CACHE_DIR="$try/cache" "$out/$name/bin/wip" run main.wip)
rm -rf "$try"
if [ "$said" != "hello from 3" ]; then
    echo "error: $name/bin/wip, installed, does not run a program: it said \`$said\`" >&2
    exit 1
fi

tar -C "$out" -czf "$out/$name.tar.gz" "$name"
echo "$out/$name.tar.gz: $("$out/$name/bin/wip" --version), $(du -h "$out/$name.tar.gz" | cut -f1)"
