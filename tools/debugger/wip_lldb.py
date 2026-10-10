"""Wip's values as lldb shows them.

`wip debug` loads this into lldb; an editor's debugger loads it with
`command script import <path>`, the path `wip debug --scripts` prints. It
reads the layouts of the compiler that carries it: a `str` and a `String`
are shown as their text, a slice, a `Vec`, a `Set`, a `Map` and a `Deque` as
their elements, and an enum as the variant it holds.

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

import lldb

# ---------------------------------------------------------------------------
# What a type is.


def _fields(sbtype):
    return [sbtype.GetFieldAtIndex(i) for i in range(sbtype.GetNumberOfFields())]


def _field_names(sbtype):
    return [field.GetName() for field in _fields(sbtype)]


def _plain(sbtype):
    return sbtype.GetCanonicalType().GetUnqualifiedType()


def kind_of(sbtype):
    """What Wip value a type is: 'char', 'str', 'String', 'slice', 'Vec',
    'Set', 'Map', 'Deque', 'enum', 'niche', or None."""
    sbtype = _plain(sbtype)
    # A Unicode scalar value, in 32 bits.
    if sbtype.GetBasicType() == lldb.eBasicTypeChar32:
        return "char"
    name = sbtype.GetName() or ""
    names = _field_names(sbtype)
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
    return _enum_kind(sbtype, name)


def _enum_kind(sbtype, name):
    fields = _fields(sbtype)
    if not fields or not name:
        return None
    variants = fields
    tagged = fields[0].GetName() == "variant"
    if tagged:
        if fields[0].GetType().GetTypeClass() != lldb.eTypeClassEnumeration:
            return None
        variants = fields[1:]
    for field in variants:
        if field.GetType().GetName() != name + "::" + field.GetName():
            return None
    if tagged:
        return "enum"
    if len(variants) == 2 and variants[1].GetType().GetByteSize() == 0:
        return "niche"
    return None


def _recognizer(kind):
    def recognize(sbtype, _internal_dict):
        return kind_of(sbtype) == kind

    return recognize


is_char = _recognizer("char")
is_str = _recognizer("str")
is_string = _recognizer("String")
is_slice = _recognizer("slice")
is_vec = _recognizer("Vec")
is_set = _recognizer("Set")
is_map = _recognizer("Map")
is_deque = _recognizer("Deque")


def is_enum(sbtype, _internal_dict):
    return kind_of(sbtype) in ("enum", "niche")


# ---------------------------------------------------------------------------
# Reading.


def _int(value):
    return value.GetValueAsSigned(0)


GONE = "<optimized out>"


def _in_part(value):
    """Whether a part of `value` is optimized out: its own members, not what
    its pointers reach."""
    plain = value.GetNonSyntheticValue()
    if plain.GetError().Fail():
        return True
    sbtype = plain.GetType()
    if sbtype.IsPointerType() or not sbtype.IsAggregateType():
        error = lldb.SBError()
        plain.GetData().GetUnsignedInt8(error, 0)
        return error.Fail() and sbtype.GetByteSize() > 0
    return any(_in_part(plain.GetChildAtIndex(i)) for i in range(plain.GetNumChildren()))


def _text(pointer, length):
    """The UTF-8 text of `length` bytes at `pointer`, quoted."""
    if length == 0:
        return '""'
    address = pointer.GetValueAsUnsigned(0)
    if address == 0 or length < 0:
        return "<no text>"
    error = lldb.SBError()
    shown = min(length, 4096)
    data = pointer.GetProcess().ReadMemory(address, shown, error)
    if not error.Success():
        return "<unreadable>"
    text = data.decode("utf-8", "replace")
    text = text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
    more = "..." if shown < length else ""
    return '"' + text + more + '"'


def _element(pointer, index, name):
    elem_type = pointer.GetType().GetPointeeType()
    address = pointer.GetValueAsUnsigned(0) + index * elem_type.GetByteSize()
    return pointer.CreateValueFromAddress(name, address, elem_type)


def _variant(value):
    """The member of the variant an enum holds, or its name alone where it
    carries nothing: (name, member or None)."""
    kind = kind_of(value.GetType())
    plain = value.GetNonSyntheticValue()
    if kind == "enum":
        name = plain.GetChildAtIndex(0).GetValue() or "?"
        return name, plain.GetChildMemberWithName(name) or None
    fields = _fields(_plain(value.GetType()))
    empty = fields[1]
    offset = empty.GetOffsetInBytes()
    error = lldb.SBError()
    word = plain.GetData().GetUnsignedInt64(error, offset)
    if error.Success() and word == 0:
        return empty.GetName(), None
    carried = fields[0].GetName()
    return carried, plain.GetChildMemberWithName(carried)


def _summary_of(value):
    summary = value.GetSummary()
    if summary:
        return summary
    shown = value.GetValue()
    return shown if shown is not None else "..."


# ---------------------------------------------------------------------------
# Summaries.


def char_summary(value, _internal_dict):
    if _in_part(value):
        return GONE
    code = value.GetValueAsUnsigned(0)
    if code > 0x10FFFF or 0xD800 <= code <= 0xDFFF:
        return "<not a char: %#x>" % code
    return "'" + chr(code).replace("\\", "\\\\").replace("'", "\\'") + "'"


def str_summary(value, _internal_dict):
    if _in_part(value):
        return GONE
    plain = value.GetNonSyntheticValue()
    return _text(plain.GetChildMemberWithName("ptr"), _int(plain.GetChildMemberWithName("len")))


def string_summary(value, _internal_dict):
    if _in_part(value):
        return GONE
    bytes_ = value.GetNonSyntheticValue().GetChildMemberWithName("bytes")
    pointer = bytes_.GetChildMemberWithName("slots").GetChildMemberWithName("ptr")
    return _text(pointer, _int(bytes_.GetChildMemberWithName("length")))


def length_summary(value, _internal_dict):
    if _in_part(value):
        return GONE
    return "len %d" % value.GetNumChildren()


def enum_summary(value, _internal_dict):
    if _in_part(value):
        return GONE
    name, member = _variant(value)
    if member is None:
        return "." + name
    fields = [member.GetChildAtIndex(i) for i in range(member.GetNumChildren())]
    inner = ", ".join("%s: %s" % (f.GetName(), _summary_of(f)) for f in fields)
    return ".%s(%s)" % (name, inner)


# ---------------------------------------------------------------------------
# Children.


class _Elements:
    """Children `[0]`, `[1]`, ... of a run of values in memory."""

    def __init__(self, value, _internal_dict):
        self.value = value
        self.pointer = None
        self.count = 0

    def where(self):
        """The pointer to the first value and how many there are."""
        raise NotImplementedError

    def update(self):
        if _in_part(self.value):
            self.pointer, self.count = None, 0
        else:
            self.pointer, self.count = self.where()
        return False

    def num_children(self):
        return self.count

    def get_child_index(self, name):
        try:
            return int(name.lstrip("[").rstrip("]"))
        except ValueError:
            return -1

    def get_child_at_index(self, index):
        if index < 0 or index >= self.count:
            return None
        return _element(self.pointer, index, "[%d]" % index)

    def has_children(self):
        return True


class SliceChildren(_Elements):
    def where(self):
        plain = self.value.GetNonSyntheticValue()
        return plain.GetChildMemberWithName("ptr"), _int(plain.GetChildMemberWithName("len"))


def _vec_where(vec):
    vec = vec.GetNonSyntheticValue()
    pointer = vec.GetChildMemberWithName("slots").GetChildMemberWithName("ptr")
    return pointer, _int(vec.GetChildMemberWithName("length"))


class VecChildren(_Elements):
    def where(self):
        return _vec_where(self.value)


class SetChildren(_Elements):
    def where(self):
        return _vec_where(self.value.GetNonSyntheticValue().GetChildMemberWithName("keys"))


class MapChildren(_Elements):
    """Each value, named by its key, in the order the keys went in."""

    def where(self):
        row = self.value.GetNonSyntheticValue().GetChildMemberWithName("row")
        return _vec_where(row.GetChildMemberWithName("entries"))

    def get_child_index(self, name):
        for index in range(self.count):
            if self.get_child_at_index(index).GetName() == name:
                return index
        return -1

    def get_child_at_index(self, index):
        if index < 0 or index >= self.count:
            return None
        entry = _element(self.pointer, index, "[%d]" % index)
        key = entry.GetChildMemberWithName("key")
        value = entry.GetChildMemberWithName("value")
        name = "[%s]" % _summary_of(key)
        return value.CreateValueFromAddress(name, value.GetLoadAddress(), value.GetType())


class DequeChildren(_Elements):
    """Each value, from the front: the ring's slots from `head` on."""

    def where(self):
        plain = self.value.GetNonSyntheticValue()
        self.slots, self.capacity = _vec_where(plain.GetChildMemberWithName("slots"))
        self.head = _int(plain.GetChildMemberWithName("head"))
        return self.slots, _int(plain.GetChildMemberWithName("length"))

    def get_child_at_index(self, index):
        if index < 0 or index >= self.count or self.capacity <= 0:
            return None
        slot = _element(self.slots, (self.head + index) % self.capacity, "[%d]" % index)
        _name, member = _variant(slot)
        if member is None or member.GetNumChildren() == 0:
            return slot
        held = member.GetChildAtIndex(0)
        return held.CreateValueFromAddress("[%d]" % index, held.GetLoadAddress(), held.GetType())


class EnumChildren:
    """The fields of the variant an enum holds."""

    def __init__(self, value, _internal_dict):
        self.value = value
        self.member = None

    def update(self):
        self.member = None if _in_part(self.value) else _variant(self.value)[1]
        return False

    def num_children(self):
        return 0 if self.member is None else self.member.GetNumChildren()

    def get_child_index(self, name):
        if self.member is None:
            return -1
        return self.member.GetIndexOfChildWithName(name)

    def get_child_at_index(self, index):
        if self.member is None:
            return None
        return self.member.GetChildAtIndex(index)

    def has_children(self):
        return self.member is not None


# ---------------------------------------------------------------------------
# Loading.


def __lldb_init_module(debugger, _internal_dict):
    module = __name__

    def run(command):
        debugger.HandleCommand(command)

    def summary(recognizer, function, extra=""):
        run(
            "type summary add -w wip %s --recognizer-function %s.%s -F %s.%s"
            % (extra, module, recognizer, module, function)
        )

    def children(recognizer, provider):
        run(
            "type synthetic add -w wip --recognizer-function %s.%s -l %s.%s"
            % (module, recognizer, module, provider)
        )

    # The character alone, not its number too.
    summary("is_char", "char_summary", "--no-value")
    summary("is_str", "str_summary")
    summary("is_string", "string_summary")
    for recognizer, provider in (
        ("is_slice", "SliceChildren"),
        ("is_vec", "VecChildren"),
        ("is_set", "SetChildren"),
        ("is_map", "MapChildren"),
        ("is_deque", "DequeChildren"),
    ):
        summary(recognizer, "length_summary", "--expand")
        children(recognizer, provider)
    summary("is_enum", "enum_summary")
    children("is_enum", "EnumChildren")
    run("type category enable wip")
