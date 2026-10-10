#!/usr/bin/env python3
# Writes std/prelude/category.wip: whether a character is a letter or a
# number, by Unicode's general categories, from the Unicode data Python
# carries. The compiler's lexer asks the same table whether what it cannot
# read begins with a letter, so that it answers as the library does.
#
#   scripts/char-category.py          write it
#   scripts/char-category.py --check  fail if it is not what would be written
#
# A Python with another version of Unicode would write another table, so
# `--check` checks only against the version the file was written from.
#
# A letter is a character of a category `L…`: upper, lower and title case,
# modifier and other letters, `é`, `я`, `中`. A number is one of `N…`:
# decimal digits of every script, letters that are numbers, as `Ⅻ`, and
# other numbers, as `½`.
import re
import sys
import unicodedata


def ranges(prefix):
    """The characters whose category starts with `prefix`, as runs."""
    found = []
    for cp in range(0x110000):
        if not unicodedata.category(chr(cp)).startswith(prefix):
            continue
        if found and found[-1][1] == cp - 1:
            found[-1] = (found[-1][0], cp)
        else:
            found.append((cp, cp))
    return found


def set_table(name, table, what):
    rows = "\n".join(f"\t(0x{f:X}, 0x{l:X})," for f, l in table)
    return f"""/// {what}: the first and the last of each run.
val {name}: [(u32, u32); {len(table)}] = [
{rows}
]
"""


def tester(name, table, count, what):
    return f"""/// {what}: in one of {table}'s runs.
fn {name}(c: char): bool = {{
	val code = c as u32
	var lo = 0
	var hi = {count} - 1
	while lo <= hi {{
		val middle = (lo + hi) / 2
		val (first, last) = {table}[middle]
		if code < first then hi = middle - 1
		else if code > last then lo = middle + 1
		else return true
	}}
	return false
}}
"""


def text():
    letters = ranges("L")
    numbers = ranges("N")
    return f"""// Whether a character is a letter or a number, by Unicode's general
// categories: a letter is `L…`, a number `N…`.
// Written by scripts/char-category.py from Unicode {unicodedata.unidata_version}; edit that, not this.

extend char {{
	/// Whether it is a letter of any script: `a`, `é`, `я`, `中`. Not a
	/// digit, nor a mark that goes with a letter. ASCII is answered without
	/// the table, since most text a program reads is.
	pub fn isLetter(): bool = if self.isAscii() then self.isAsciiLetter() else inLetters(self)

	/// Whether it is a number of any script: `7`, `٣`, `Ⅻ`, `½`. What a
	/// program parsing a number asks is `isDigit`, `0` to `9`.
	pub fn isNumeric(): bool = if self.isAscii() then self.isDigit() else inNumbers(self)

	/// Whether it is a letter or a number.
	pub fn isAlphanumeric(): bool =
		if self.isAscii() then self.isAsciiAlphanumeric() else inLetters(self) || inNumbers(self)
}}

{tester("inLetters", "LETTERS", len(letters), "Whether `c` is a letter")}
{tester("inNumbers", "NUMBERS", len(numbers), "Whether `c` is a number")}
{set_table("LETTERS", letters, "The letters")}
{set_table("NUMBERS", numbers, "The numbers")}"""


if __name__ == "__main__":
    outputs = [("std/prelude/category.wip", text())]
    if "--check" in sys.argv:
        for path, wanted in outputs:
            with open(path) as f:
                found = f.read()
            written = re.search(r"from Unicode ([0-9.]+);", found)
            if written is None or written.group(1) != unicodedata.unidata_version:
                print(f"{path}: not checked, as this Python has Unicode {unicodedata.unidata_version}")
                continue
            if found != wanted:
                sys.exit(f"{path} is not what scripts/char-category.py writes")
        sys.exit(0)
    for path, wanted in outputs:
        with open(path, "w") as f:
            f.write(wanted)
