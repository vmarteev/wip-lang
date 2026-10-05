"""Wip's values as gdb shows them.

`wip debug` loads this into gdb; an editor's debugger loads it with
`source <path>`, the path `wip debug --scripts` prints. It reads the layouts
of the compiler that carries it: a `str` and a `String` are shown as their
text, a slice, a `Vec`, a `Set`, a `Map` and a `Deque` as their elements,
and an enum as the variant it holds.

A type is known by its name and by the members read from it, so a program's
own type of the same name is left alone; an enum by its shape, as the
compiler describes one: a structure whose members are its variants, each a
structure named `Enum::Variant`, and first its tag, `variant`, unless the
enum is a niche, where the variant that carries nothing is an empty
structure where the null would be.

A release build keeps a variable where it can, and may keep only part of
it: a `String`'s length but not where its bytes are. A value with a part
gone is shown as `<optimized out>`, rather than read as what it does not
hold.
"""

import gdb

# ---------------------------------------------------------------------------
# What a type is.


def _plain(gdbtype):
    return gdbtype.strip_typedefs().unqualified()


def _name(gdbtype):
    return gdbtype.tag or gdbtype.name or ""


def _fields(gdbtype):
    if gdbtype.code not in (gdb.TYPE_CODE_STRUCT, gdb.TYPE_CODE_UNION):
        return []
    return gdbtype.fields()


def kind_of(gdbtype):
    """What Wip value a type is: 'char', 'str', 'String', 'slice', 'Vec',
    'Set', 'Map', 'Deque', 'enum', 'niche', or None."""
    gdbtype = _plain(gdbtype)
    name = _name(gdbtype)
    # A Unicode scalar value, in 32 bits.
    if name == "char" and gdbtype.sizeof == 4:
        return "char"
    names = [field.name for field in _fields(gdbtype)]
    if name == "str" and names == ["ptr", "len"]:
        return "str"
    if name == "String" and names == ["bytes"]:
        return "String"
    if (name.startswith("&[") or name.startswith("&var [")) and names == ["ptr", "len"]:
        return "slice"
    if name.startswith("Vec<") and names == ["slots", "length"]:
        return "Vec"
    if name.startswith("Set<") and names == ["table", "keys"]:
        return "Set"
    if name.startswith("Map<") and names == ["table", "row"]:
        return "Map"
    if name.startswith("Deque<") and names == ["slots", "head", "length"]:
        return "Deque"
    return _enum_kind(gdbtype, name)


def _enum_kind(gdbtype, name):
    fields = _fields(gdbtype)
    if not fields or not name:
        return None
    variants = fields
    tagged = fields[0].name == "variant"
    if tagged:
        if _plain(fields[0].type).code != gdb.TYPE_CODE_ENUM:
            return None
        variants = fields[1:]
    for field in variants:
        if _name(_plain(field.type)) != name + "::" + field.name:
            return None
    if tagged:
        return "enum"
    if len(variants) == 2 and _plain(variants[1].type).sizeof == 0:
        return "niche"
    return None


# ---------------------------------------------------------------------------
# Reading.


def _vec(vec):
    """A `Vec`'s first value and how many it holds."""
    return vec["slots"]["ptr"], int(vec["length"])


def _text(pointer, length):
    if length <= 0:
        return ""
    return pointer.lazy_string(encoding="utf-8", length=min(length, 4096))


def _variant(value):
    """The name of the variant an enum holds, and its member, or None where
    it carries nothing."""
    gdbtype = _plain(value.type)
    fields = gdbtype.fields()
    if kind_of(gdbtype) == "enum":
        tag = str(value["variant"])
        # A C enumerator may print qualified by its type.
        tag = tag.rsplit("::", 1)[-1]
        for field in fields[1:]:
            if field.name == tag:
                return tag, value[field.name]
        return tag, None
    empty = fields[1]
    address = value.address
    if address is not None:
        start = int(address) + empty.bitpos // 8
        word = gdb.selected_inferior().read_memory(start, 8).tobytes()
        if int.from_bytes(word, "little") == 0:
            return empty.name, None
    return fields[0].name, value[fields[0].name]


# ---------------------------------------------------------------------------
# Printers.


class CharPrinter:
    def __init__(self, value):
        self.value = value

    def to_string(self):
        return _quoted_char(int(self.value) & 0xFFFFFFFF)


def _quoted_char(code):
    if code > 0x10FFFF or 0xD800 <= code <= 0xDFFF:
        return "<not a char: %#x>" % code
    return "'" + chr(code).replace("\\", "\\\\").replace("'", "\\'") + "'"


class StrPrinter:
    def __init__(self, value):
        self.value = value

    def to_string(self):
        return _text(self.value["ptr"], int(self.value["len"]))

    def display_hint(self):
        return "string"


class StringPrinter:
    def __init__(self, value):
        self.value = value

    def to_string(self):
        pointer, length = _vec(self.value["bytes"])
        return _text(pointer, length)

    def display_hint(self):
        return "string"


class ElementsPrinter:
    """A run of values in memory, as an array."""

    def __init__(self, pointer, count):
        self.pointer = pointer
        self.count = max(count, 0)

    def to_string(self):
        return "len %d" % self.count

    def children(self):
        for index in range(self.count):
            yield "[%d]" % index, (self.pointer + index).dereference()

    def display_hint(self):
        return "array"


class MapPrinter:
    """Each key and its value, in the order the keys went in."""

    def __init__(self, value):
        self.entries, self.count = _vec(value["row"]["entries"])

    def to_string(self):
        return "len %d" % self.count

    def children(self):
        for index in range(self.count):
            entry = (self.entries + index).dereference()
            yield "key%d" % index, entry["key"]
            yield "value%d" % index, entry["value"]

    def display_hint(self):
        return "map"


class DequePrinter:
    """Each value, from the front: the ring's slots from `head` on."""

    def __init__(self, value):
        self.slots, self.capacity = _vec(value["slots"])
        self.head = int(value["head"])
        self.count = int(value["length"])

    def to_string(self):
        return "len %d" % self.count

    def children(self):
        if self.capacity <= 0:
            return
        for index in range(self.count):
            slot = (self.slots + (self.head + index) % self.capacity).dereference()
            _name, member = _variant(slot)
            if member is None or not _plain(member.type).fields():
                yield "[%d]" % index, slot
            else:
                yield "[%d]" % index, member[_plain(member.type).fields()[0].name]

    def display_hint(self):
        return "array"


class EnumPrinter:
    """`.Variant`, with the fields of the variant it holds."""

    def __init__(self, value):
        self.name, self.member = _variant(value)

    def to_string(self):
        return "." + self.name

    def children(self):
        if self.member is None:
            return
        for field in _plain(self.member.type).fields():
            yield field.name, self.member[field.name]


def _in_part(value):
    """Whether a part of `value` is optimized out: its own members, not what
    its pointers reach. A part the location does not describe at all, gdb
    reads as 0 and does not call optimized out, and prints as
    `<synthetic pointer>`, which is how it is known."""
    try:
        if value.is_optimized_out:
            return True
        gdbtype = _plain(value.type)
        if gdbtype.code in (gdb.TYPE_CODE_STRUCT, gdb.TYPE_CODE_UNION):
            return any(_in_part(value[field.name]) for field in gdbtype.fields() if field.name)
        return str(value) in ("<synthetic pointer>", "<optimized out>")
    except gdb.error:
        return True


class GonePrinter:
    """A value a release build keeps only in part."""

    def __init__(self, _value):
        pass

    def to_string(self):
        return "<optimized out>"


def lookup(value):
    kind = kind_of(value.type)
    if kind is None:
        return None
    plain = value.cast(_plain(value.type)) if value.type != _plain(value.type) else value
    if _in_part(plain):
        return GonePrinter(plain)
    if kind == "char":
        return CharPrinter(plain)
    if kind == "str":
        return StrPrinter(plain)
    if kind == "String":
        return StringPrinter(plain)
    if kind == "slice":
        return ElementsPrinter(plain["ptr"], int(plain["len"]))
    if kind == "Vec":
        return ElementsPrinter(*_vec(plain))
    if kind == "Set":
        return ElementsPrinter(*_vec(plain["keys"]))
    if kind == "Map":
        return MapPrinter(plain)
    if kind == "Deque":
        return DequePrinter(plain)
    return EnumPrinter(plain)


gdb.pretty_printers.append(lookup)
