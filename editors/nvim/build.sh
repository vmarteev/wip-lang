#!/bin/bash
# Compiles the grammar in editors/tree-sitter-wip to parser/wip.so, where
# Neovim looks for a language's parser. Needs the `tree-sitter` command and
# a C compiler. Run it again after the grammar changes.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p parser
tree-sitter build -o parser/wip.so ../tree-sitter-wip
echo "built $(pwd)/parser/wip.so"
