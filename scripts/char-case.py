#!/usr/bin/env python3
# Writes std/prelude/case.wip: a character's case, from the Unicode data
# Python carries.
#
#   scripts/char-case.py          write it
#   scripts/char-case.py --check  fail if it is not what would be written
#
# A Python with another version of Unicode would write another table, so
# `--check` checks only against the version the file was written from.
#
# The mappings are Unicode's simple ones: one character to one. A
# character whose lower or upper case is several — `İ`, `ß` — keeps
# itself. `folded` puts together every character that the simple mappings
# join, as Rust's regular expressions do without case: `K`, `k` and the
# Kelvin sign fold alike, and so do `Σ`, `σ` and `ς`. What a class folds
# to is the lower case of its capital — `σ` — as Unicode's own folding has
# it.
import re
import sys
import unicodedata


def single(text):
    return ord(text) if len(text) == 1 else None


def lower(cp):
    mapped = single(chr(cp).lower())
    return mapped if mapped is not None and mapped != cp else None


def upper(cp):
    mapped = single(chr(cp).upper())
    return mapped if mapped is not None and mapped != cp else None


def folds():
    """Each character that folds to another, and what it folds to."""
    parent = {}

    def find(cp):
        while parent.get(cp, cp) != cp:
            cp = parent[cp]
        return cp

    def join(a, b):
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[max(ra, rb)] = min(ra, rb)

    for cp in range(0x110000):
        for other in (lower(cp), upper(cp)):
            if other is not None:
                join(cp, other)
    classes = {}
    for cp in list(parent):
        classes.setdefault(find(cp), set()).add(cp)
    for root in list(classes):
        classes[root].add(root)
    mapping = {}
    for members in classes.values():
        # The lower case of a capital in the class, as Unicode folds: `σ`
        # for `Σ`, `σ` and `ς`; or else its first lower case member.
        capitals = sorted({lower(m) for m in members if chr(m).isupper() and lower(m) is not None})
        lowers = sorted(m for m in members if chr(m).islower())
        target = capitals[0] if capitals else lowers[0] if lowers else min(members)
        for m in members:
            if m != target:
                mapping[m] = target
    return mapping


def runs(mapping):
    """A mapping as runs: the first and the last, the difference each maps
    by, and the step between them — 1, or 2 where upper and lower case
    alternate, as in Latin Extended-A."""
    found = []
    for cp in sorted(mapping):
        delta = mapping[cp] - cp
        if found:
            first, last, d, step = found[-1]
            if d == delta:
                if step == 0 and cp - last in (1, 2):
                    found[-1] = (first, cp, d, cp - last)
                    continue
                if step != 0 and cp - last == step:
                    found[-1] = (first, cp, d, step)
                    continue
        found.append((cp, cp, delta, 0))
    return [(f, l, d, s if s else 1) for f, l, d, s in found]


def ranges(test):
    """The characters `test` holds for, as runs."""
    found = []
    for cp in range(0x110000):
        if not test(chr(cp)):
            continue
        if found and found[-1][1] == cp - 1:
            found[-1] = (found[-1][0], cp)
        else:
            found.append((cp, cp))
    return found


def mapping_table(name, table, what):
    rows = "\n".join(f"\t(0x{f:X}, 0x{l:X}, {d}, {s})," for f, l, d, s in table)
    return f"""/// {what}: the first and the last of each run, what each maps by, and
/// the step between them.
val {name}: [(u32, u32, i64, i64); {len(table)}] = [
{rows}
]
"""


def set_table(name, table, what):
    rows = "\n".join(f"\t(0x{f:X}, 0x{l:X})," for f, l in table)
    return f"""/// {what}: the first and the last of each run.
val {name}: [(u32, u32); {len(table)}] = [
{rows}
]
"""


def mapper(name, table, count, what):
    return f"""/// {what}, by {table}'s runs.
fn {name}(c: char): char = {{
	val code = c as u32
	var lo = 0
	var hi = {count} - 1
	while lo <= hi {{
		val middle = (lo + hi) / 2
		val (first, last, delta, step) = {table}[middle]
		if code < first {{
			hi = middle - 1
		}} else if code > last {{
			lo = middle + 1
		}} else {{
			if ((code - first) as i64) % step != 0 then return c
			return char::fromScalar(((code as i64) + delta) as u32)
		}}
	}}
	return c
}}
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
    lowers = runs({cp: lower(cp) for cp in range(0x110000) if lower(cp) is not None})
    uppers = runs({cp: upper(cp) for cp in range(0x110000) if upper(cp) is not None})
    folded = runs(folds())
    upper_set = ranges(str.isupper)
    lower_set = ranges(str.islower)
    return f"""// A character's case: Unicode's simple mappings, one
// character to one, and a fold that puts together the characters they
// join, for comparing without case.
// Written by scripts/char-case.py from Unicode {unicodedata.unidata_version}; edit that, not this.

extend char {{
	/// Its lower case: `A` is `a`, `Σ` is `σ`. A character whose lower
	/// case is several, as `İ`'s is, and one that has none, are themselves.
	pub fn toLowercase(): char = lowered(self)

	/// Its upper case: `a` is `A`, `ς` is `Σ`. A character whose upper
	/// case is several, as `ß`'s is, and one that has none, are themselves.
	pub fn toUppercase(): char = uppered(self)

	/// What it is without its case, for comparing two without it: `K`, `k`
	/// and `K` (the Kelvin sign) fold alike, and so do `Σ`, `σ` and `ς`.
	pub fn folded(): char = foldedChar(self)

	/// Whether it is an upper case letter, or another character Unicode
	/// counts as upper case, as `Ⓐ`.
	pub fn isUppercase(): bool = inUppercase(self)

	/// Whether it is a lower case letter, or another character Unicode
	/// counts as lower case, as `ª`.
	pub fn isLowercase(): bool = inLowercase(self)
}}

extend str {{
	/// Each of its characters in lower case.
	pub fn toLowercase(): String = {{
		var out = String()
		for c in self.chars() {{
			out.pushChar(c.toLowercase())
		}}
		return move out
	}}

	/// Each of its characters in upper case.
	pub fn toUppercase(): String = {{
		var out = String()
		for c in self.chars() {{
			out.pushChar(c.toUppercase())
		}}
		return move out
	}}

	/// Whether the two are the same text in any case: what
	/// `self.folded() == other.folded()` answers, a character at a time,
	/// making nothing.
	pub fn equalsAnyCase(other: str): bool = {{
		var theirs = other.chars()
		for c in self.chars() {{
			val .Some(d) = theirs.next() else {{
				return false
			}}
			if c.folded() != d.folded() then return false
		}}
		return theirs.next().isNone()
	}}

	/// Whether it begins with `prefix` in any case, character by character
	/// as `equalsAnyCase` compares: an HTTP header's name, a keyword C
	/// writes in either case.
	pub fn startsWithAnyCase(prefix: str): bool = {{
		var mine = self.chars()
		for c in prefix.chars() {{
			val .Some(d) = mine.next() else {{
				return false
			}}
			if c.folded() != d.folded() then return false
		}}
		return true
	}}

	/// Each of its characters folded, for comparing without case:
	/// `a.folded() == b.folded()`.
	pub fn folded(): String = {{
		var out = String()
		for c in self.chars() {{
			out.pushChar(c.folded())
		}}
		return move out
	}}
}}

{mapper("lowered", "LOWER", len(lowers), "What `c` is in lower case")}
{mapper("uppered", "UPPER", len(uppers), "What `c` is in upper case")}
{mapper("foldedChar", "FOLD", len(folded), "What `c` folds to")}
{tester("inUppercase", "UPPERCASE", len(upper_set), "Whether `c` is upper case")}
{tester("inLowercase", "LOWERCASE", len(lower_set), "Whether `c` is lower case")}
{mapping_table("LOWER", lowers, "Lower case, where it is another character")}
{mapping_table("UPPER", uppers, "Upper case, where it is another character")}
{mapping_table("FOLD", folded, "What a character folds to, where it is another")}
{set_table("UPPERCASE", upper_set, "The upper case characters")}
{set_table("LOWERCASE", lower_set, "The lower case characters")}"""


if __name__ == "__main__":
    path = "std/prelude/case.wip"
    wanted = text()
    if "--check" in sys.argv:
        with open(path) as f:
            found = f.read()
        written = re.search(r"from Unicode ([0-9.]+);", found)
        if written is None or written.group(1) != unicodedata.unidata_version:
            print(f"{path}: not checked, as this Python has Unicode {unicodedata.unidata_version}")
            sys.exit(0)
        if found != wanted:
            sys.exit(f"{path} is not what scripts/char-case.py writes")
        sys.exit(0)
    with open(path, "w") as f:
        f.write(wanted)
