#!/bin/bash
# Checks that nothing published cites a design record by its number, as in
# "decision 0252": the records are not published, so the number would point
# at nothing. A comment, a message or a document says why in words.
#
#   scripts/citations.sh [DIR]    check DIR, this repository by default
#
# It prints each line that cites one, and fails where there is any.
set -uo pipefail
cd "${1:-$(dirname "$0")/..}"

published=(README.md CHANGELOG.md THIRD-PARTY-NOTICES.md docs/language docs/grammar.md
    crates std tools tests editors examples scripts .github)
skip=(--exclude-dir=target --exclude-dir=node_modules --exclude-dir=bench
    --exclude=publish.sh --exclude=speed.sh --exclude=citations.sh)

found=$(
    grep -rIn -E '[Dd]ecisions? [0-9]{4}' "${published[@]}" "${skip[@]}" 2>/dev/null
    # The word at the end of one line, and the number starting the next.
    grep -rIn -A1 -E '[Dd]ecisions?[^A-Za-z0-9]*$' "${published[@]}" "${skip[@]}" 2>/dev/null |
        grep -E '^[^:]+-[0-9]+-[^A-Za-z0-9]*[0-9]{4}'
)
if [ -n "$found" ]; then
    echo "$found"
    exit 1
fi
exit 0
