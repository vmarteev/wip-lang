#!/bin/bash
# Makes a copy of the extension that builds the grammar from this checkout
# instead of GitHub, for "zed: install dev extension". The copy goes to
# target/zed-wip; install that directory. Its `rev` is the last commit
# that changed the grammar, so commit a grammar change before running this.
set -euo pipefail
cd "$(dirname "$0")/../.."
root=$(pwd)
rev=$(git log -1 --format=%H -- editors/tree-sitter-wip)
out=target/zed-wip
rm -rf "$out"
mkdir -p "$out"
cp -R editors/zed/languages editors/zed/src editors/zed/Cargo.toml editors/zed/Cargo.lock "$out/"
sed -e "/^\[grammars.wip\]/,/^path/{s|^repository = .*|repository = \"file://$root\"|;s|^rev = .*|rev = \"$rev\"|;}" \
    editors/zed/extension.toml >"$out/extension.toml"
echo "install $root/$out with \"zed: install dev extension\""
