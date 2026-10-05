#!/bin/bash
# Checks the editors' grammar, editors/tree-sitter-wip: regenerates the
# parser from grammar.js, runs its test corpus, compiles its queries, and
# parses every Wip file in the repository, which must give no ERROR or
# MISSING node (except the cases in tests/cases/err, which are wrong on
# purpose). Needs the `tree-sitter` command (0.25 or later).
#
#   scripts/tree-sitter.sh           regenerate, check, and copy
#   scripts/tree-sitter.sh --check   change nothing: fail where src/ is not
#                                    what grammar.js makes, or where a test,
#                                    a query or a file fails; the gate runs
#                                    this where tree-sitter is installed
#
# It also copies the queries to the editors that read them: Neovim's
# (editors/nvim/queries/wip) as they are, and Zed's highlights
# (editors/zed/languages/wip) with the one capture name Zed has no colour
# for, `@character`, as `@string`. Edit the queries here, never the copies.
#
# Run it after a change to the language's syntax, together with
# docs/grammar.md, and commit the regenerated src/ and the copies with it:
# Zed builds the grammar from those files.
set -uo pipefail
cd "$(dirname "$0")/../editors/tree-sitter-wip" || exit 1
check=false
[ "${1:-}" = "--check" ] && check=true

if ! command -v tree-sitter >/dev/null; then
    echo "tree-sitter is not installed; see https://tree-sitter.github.io"
    exit 1
fi

if $check; then
    # What grammar.js makes, in a copy, against what is committed.
    echo "== src/ is what grammar.js makes"
    fresh=$(mktemp -d)
    cp grammar.js tree-sitter.json "$fresh"/
    (cd "$fresh" && tree-sitter generate >/dev/null 2>&1) || { echo "grammar.js does not generate"; exit 1; }
    for file in parser.c grammar.json node-types.json; do
        if ! cmp -s "$fresh/src/$file" "src/$file"; then
            echo "src/$file is not what grammar.js makes: run scripts/tree-sitter.sh"
            rm -rf "$fresh"
            exit 1
        fi
    done
    rm -rf "$fresh"
    echo "ok"
else
    echo "== generate"
    tree-sitter generate || exit 1
fi

echo "== test corpus"
tree-sitter test || exit 1

echo "== queries"
for query in queries/*.scm; do
    tree-sitter query -p . "$query" ../../std/prelude/runtime.wip >/dev/null 2>&1 ||
        { echo "$query does not compile against the grammar"; tree-sitter query -p . "$query" ../../std/prelude/runtime.wip; exit 1; }
done
echo "ok"

if ! $check; then
    echo "== copies for the editors"
    header="; Copied from editors/tree-sitter-wip/queries by scripts/tree-sitter.sh; edit that."
    mkdir -p ../nvim/queries/wip
    for query in highlights folds indents locals; do
        { echo "$header"; echo; cat "queries/$query.scm"; } >"../nvim/queries/wip/$query.scm"
    done
    { echo "$header"; echo; sed 's/@character/@string/' queries/highlights.scm; } >../zed/languages/wip/highlights.scm
    echo "ok"
fi

echo "== every .wip file in the repository"
list=$(mktemp)
trap 'rm -f "$list"' EXIT
if git -C ../.. rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git -C ../.. ls-files '*.wip' | grep -v '^tests/cases/err/' | sed 's|^|../../|' >"$list"
else
    # An export, which has no history: every file there is.
    (cd ../.. && find . -name '*.wip' -not -path './target/*' -not -path './tests/cases/err/*' |
        sed 's|^\./|../../|') >"$list"
fi
count=$(wc -l <"$list" | tr -d ' ')
if ! tree-sitter parse -p . --quiet --paths "$list"; then
    echo "some of the $count files did not parse cleanly"
    exit 1
fi
echo "$count files, no errors"
