#!/bin/bash
# Checks that nothing published points at the documents that are not: a
# design record, an answer or a question by its number ("decision 0252",
# "answer 5"), a milestone of the plan ("plan M7"), a review or the plan by
# name, or a path to any of them. They are not published, so the pointer
# would point at nothing. A comment, a message or a document says why in
# words.
#
#   scripts/citations.sh [DIR]    check DIR, this repository by default
#
# It prints each line that points at one, and fails where there is any.
set -uo pipefail
cd "${1:-$(dirname "$0")/..}"

published=(README.md CHANGELOG.md THIRD-PARTY-NOTICES.md docs/language docs/grammar.md
    compiler bootstrap std tools tests editors examples scripts .github)
skip=(--exclude-dir=target --exclude-dir=node_modules --exclude-dir=bench
    --exclude=publish.sh --exclude=speed.sh --exclude=citations.sh)

# A record by its number. "Answers 1", what a function gives, is not one.
numbered='[Dd]ecisions?|[Aa]nswer|[Qq]uestions?'
# The rest, by name or by path.
named='plan M[0-9]|[Ss]yntax reviews?|syntax-review|[Pp]rototype plan|prototype-plan|code-quality|docs/(decisions|answers|roadmap)|[Rr]oadmap'

found=$(
    grep -rIn -E "\\b($numbered) [0-9]+\\b|$named" "${published[@]}" "${skip[@]}" 2>/dev/null
    # The word at the end of one line, and the number starting the next;
    # not a word that ends a sentence.
    grep -rIn -A1 -E "\\b($numbered)[^A-Za-z0-9.]*\$" "${published[@]}" "${skip[@]}" 2>/dev/null |
        grep -E '^[^:]+-[0-9]+-[^A-Za-z0-9]*[0-9]+\b'
)
if [ -n "$found" ]; then
    echo "$found"
    exit 1
fi
exit 0
