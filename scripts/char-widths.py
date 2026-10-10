#!/usr/bin/env python3
# Writes std/prelude/width.wip: how many columns a character takes on a
# terminal, from the Unicode data Python carries. The compiler lays out
# diagnostics and the syntax tree's dump by the same table, the library's,
# so that it measures text as a program in Wip does.
#
#   scripts/char-widths.py          write it
#   scripts/char-widths.py --check  fail if it is not what would be written
#
# A Python with another version of Unicode would write another table, so
# `--check` checks only against the version the file was written from.
#
# A character is 0 columns where it draws nothing of its own — a combining
# mark, a format character, a control, the medial and final Hangul jamo — 2
# where Unicode says it is wide (East Asian Width W or F), and 1 otherwise.
# Where termbox2's table, which some terminal libraries draw by, said
# otherwise for a printable character, its answer is taken, so that what is
# laid out is what is drawn: the format characters that show a mark, eight
# circled numbers, and the Hangul fillers. They are listed below by name.
import re
import sys
import unicodedata

# Format characters that show a mark, and so take a column.
SHOWN_FORMAT = {0xAD, 0x6DD, 0x70F, 0x8E2, 0x110BD, 0x110CD}
SHOWN_FORMAT |= set(range(0x600, 0x606))
SHOWN_FORMAT |= {0x890, 0x891, 0xFFF9, 0xFFFA, 0xFFFB}
SHOWN_FORMAT |= set(range(0x13430, 0x13440))
# Circled numbers on black squares, drawn wide.
WIDE_AMBIGUOUS = set(range(0x3248, 0x3250))
# Fillers, which draw nothing.
FILLERS = {0x3164, 0xFFA0}


def width(cp):
    c = chr(cp)
    category = unicodedata.category(c)
    if cp < 0x20 or 0x7F <= cp < 0xA0:
        return 0
    if cp in FILLERS:
        return 0
    if category in ("Mn", "Me"):
        return 0
    if category == "Cf" and cp not in SHOWN_FORMAT:
        return 0
    if 0x1160 <= cp <= 0x11FF or 0xD7B0 <= cp <= 0xD7FF:
        return 0
    if category == "Cn":
        return 1
    if unicodedata.east_asian_width(c) in ("W", "F") or cp in WIDE_AMBIGUOUS:
        return 2
    return 1


def ranges():
    """The ranges, from U+0300, whose width is not 1."""
    found = []
    start, current = None, None
    for cp in range(0x300, 0x110000):
        w = width(cp)
        if current is not None and w == current and cp == end + 1:
            end = cp
            continue
        if current is not None and current != 1:
            found.append((start, end, current))
        start, end, current = cp, cp, w
    if current != 1:
        found.append((start, end, current))
    return found


def text():
    table = ranges()
    rows = "\n".join(f"\t(0x{lo:X}, 0x{hi:X}, {w})," for lo, hi, w in table)
    return f"""// How many columns a character takes on a terminal: 0
// where it draws nothing of its own, 2 where it is wide, 1 otherwise.
// Written by scripts/char-widths.py from Unicode {unicodedata.unidata_version}; edit that, not this.

extend char {{
	/// How many columns it takes on a terminal: 0 for a combining mark, a
	/// format character or a control, 2 for a wide one — most of Chinese,
	/// Japanese and Korean, and emoji — and 1 for the rest. A cluster of
	/// several, as a flag or a family, is not counted as one.
	pub fn width(): i64 = {{
		val code = self as u32
		// Everything before the combining marks is one column, but the
		// controls, which a terminal does not show.
		if code < 0x300 then return if code < 0x20 || (code >= 0x7F && code < 0xA0) then 0 else 1
		var lo = 0
		var hi = {len(table)} - 1
		while lo <= hi {{
			val middle = (lo + hi) / 2
			val (first, last, columns) = WIDTHS[middle]
			if code < first then hi = middle - 1
			else if code > last then lo = middle + 1
			else return columns
		}}
		return 1
	}}
}}

extend str {{
	/// How many columns it takes on a terminal: its characters' widths
	/// together.
	pub fn width(): i64 = self.chars().map(own (c) => c.width()).sum()
}}

/// The characters from U+0300 whose width is not 1: the first and the last
/// of each run, and the width.
val WIDTHS: [(u32, u32, i64); {len(table)}] = [
{rows}
]
"""


if __name__ == "__main__":
    outputs = [("std/prelude/width.wip", text())]
    if "--check" in sys.argv:
        for path, wanted in outputs:
            with open(path) as f:
                found = f.read()
            written = re.search(r"from Unicode ([0-9.]+);", found)
            if written is None or written.group(1) != unicodedata.unidata_version:
                print(f"{path}: not checked, as this Python has Unicode {unicodedata.unidata_version}")
                continue
            if found != wanted:
                sys.exit(f"{path} is not what scripts/char-widths.py writes")
        sys.exit(0)
    for path, wanted in outputs:
        with open(path, "w") as f:
            f.write(wanted)
    print(f"wrote {', '.join(path for path, _ in outputs)}: {len(ranges())} ranges, Unicode {unicodedata.unidata_version}")
