# 8. Interfaces

An interface is a set of methods a type may implement. It is how generic
code says what it needs of a type, and how a value of some unknown type is
called through a reference.

## Declaring and implementing one

```wip,run
interface Shape {
    fn area(): i64

    // A method may have a body, which a type takes unless it writes
    // its own.
    fn label(): i64 = self.area() + 100
}

struct Rect {
    width: i64
    height: i64
}

struct Circle {
    radius: i64
}

extend Rect: Shape {
    fn area(): i64 = self.width * self.height
}

extend Circle: Shape {
    fn area(): i64 = self.radius * 3
    fn label(): i64 = 1
}

fn main() = {
    val rect = Rect(width: 2, height: 3)
    assert(rect.area() == 6)
    assert(rect.label() == 106)
    assert(Circle(radius: 2).label() == 1)
}
```

An `extend Type: Interface` block holds exactly the interface's methods:
each is as visible as the interface, so none of them says `pub`. An
`extend Type` block without an interface holds methods of the type's own
([page 3](03-functions.md)).

Two defaults may each be written with the other, so that a type writes
whichever is natural for it. `@oneOf` on the interface names them, and a
type that writes none of them is refused, rather than calling one from
the other until the stack runs out:

```wip,run
@oneOf(area, side)
interface Square {
    fn area(): i64 = self.side() * self.side()
    fn side(): i64 = {
        var side = 0
        while side * side < self.area() {
            side += 1
        }
        return side
    }
}

struct Tile {
    length: i64
}

struct Plot {
    size: i64
}

extend Tile: Square {
    fn side(): i64 = self.length
}

extend Plot: Square {
    fn area(): i64 = self.size
}

fn main() = {
    assert(Tile(length: 3).area() == 9)
    assert(Plot(size: 16).side() == 4)
}
```

`@oneOf` names two of the interface's methods or more, each with a
default; an interface may say it more than once, each a group of its own.
`Text` is the prelude's: a type writes `toString` or `appendTo`
([page 4](04-expressions.md)).

## Constraints

A type parameter says which interfaces its type must implement. The code
inside may then call those methods, and nothing else:

```wip,run
fn largest<T: Ord>(values: &[T]): &T = {
    var best = 0
    var i = 1
    while i < values.len() {
        if values[i] > values[best] then best = i
        i += 1
    }
    lend values[best]
}

fn counted<C: Items<i64>>(container: &C): i64 = {
    var count = 0
    for value in container.items() {
        count += value
    }
    return count
}

fn main() = {
    val numbers = [3, 9, 4]
    assert(largest(&numbers) == 9)
    assert(counted(Vec::of(own [1, 2, 3])) == 6)
}
```

An interface may take types of its own — `Items<T>`, `Iterator<T>`,
`Sequence<T>`, `From<E>` — and a constraint names them: `C: Items<i64>`
above. Several are joined with `+`: `<K: Hash + Eq>`.

The last of an interface's types may have defaults, which may name `Self`,
the type that implements it, and the types before them: the operators'
interfaces are declared `interface Add<Rhs = Self, Out = Self>`, so
`extend Money: Add` is `Add<Money, Money>`, and so is the constraint
`T: Add`, while `extend Vec2: Multiply<f32>` scales a vector.

`copy` is a constraint too, and holds for a type that is plain data:
`fn pair<T: copy>(value: T): (T, T) = (value, value)`.

## `@derive`

Five interfaces are written by the compiler, field by field and variant by
variant, where a type asks for them. What it writes is code of the module's
own, checked as the program's is:

| Derived | Gives |
|---|---|
| `Eq` | `==` and `!=` |
| `Ord` | `<`, `<=`, `>`, `>=`, and sorting |
| `Hash` | a key for `Map` and `Set` |
| `Text` | what `\(value)` appends, and what printing uses |
| `Clone` | `clone()`: a copy that owns what it holds |

```wip,run
@derive(Eq, Ord, Hash, Text)
struct Version {
    major: i64
    minor: i64
}

fn main() = {
    val old = Version(major: 1, minor: 2)
    val new = Version(major: 1, minor: 9)
    assert(old < new)
    assert("\(old)" == "Version(major: 1, minor: 2)")
}
```

What `compare` answers is the prelude's `Ordering` — `.Less`, `.Same` or
`.More` — which compares, hashes and prints as a derived enum does:

```wip,run
fn main() = {
    assert(1.compare(2) == .Less)
    assert("\(2.compare(1))" == ".More")
}
```

A field compares by whatever its type implements — a `Vec<String>`, an
array — and a generic type compares where its parameters do: a
`Pair<T>` that derives `Eq` is equal where `T` is. Two
values of an enum are ordered by the order its variants are written in,
and then by their fields.

`Eq` is needed beside `Hash`, since a map is wrong the moment two equal
values hash differently. A type whose equality is not its fields writes
`extend …: Eq` itself instead.

`clone()` answers a copy that owns what it holds: each field cloned in
turn, a `String`'s bytes and a `Vec`'s elements copied, so that changing
the copy leaves the original as it was. What owns no memory — a number,
a `bool`, a `char`, a `str`, a plain struct, a tuple or an array of them —
is its own clone without being told: its clone is a copy, and it is taken
where `Clone` is asked for. A generic type clones where
its parameters do, so a `Vec<T>` field clones where `T` does.
Each field is cloned as a value of its own type —
`cloneOf(&self.field)`, which the prelude has — so a view's field that is
a `&T` is the reference copied, and what the view owns is cloned.
A field whose type does not clone — an `own<T>` among
them — is reported where it is written:

```wip,run
@derive(Clone)
struct Row {
    name: String
    tags: Vec<String>
}

fn main() = {
    var tags: Vec<String> = Vec()
    tags.push("new")
    val row = Row(name: "a", tags: move tags)
    var copy = row.clone()
    copy.tags.push("changed")
    assert(row.tags.len() == 1 && copy.tags.len() == 2)
}
```

## Implementations that hold only sometimes

An implementation may be written for a generic type under a condition on its
arguments. `Option<T>` compares where `T` compares, and no further:

```wip,ignore
extend Option<T: Eq>: Eq {
    fn equals(other: &Option<T>): bool = …
}
```

Where the condition does not hold, the type does not implement the
interface, and the operator or the call that wanted it says which argument
is in the way:

```wip,error=E0303
enum Wrong {
    Empty
}

fn main() = {
    val a: Option<Wrong> = .None
    val b: Option<Wrong> = .None
    assert(a == b)
}
```

## What a reference implements

A `&T` answers what `T` answers about itself: an interface whose methods
read `Self` through `&` alone — `Eq`, `Ord`, `Hash`, `Text` — is `T`'s
implementation, given what the reference points to. Two references to equal
values are equal, wherever they point, and a `Vec<&Node>` sorts, searches
and prints as its nodes do. A reference is also `Clone`, as all plain data
is: its clone is the reference. An interface that takes a `Self` by value,
changes it or answers one, as `Add` does, is not about the value alone, and
a reference does not implement it.

```wip,run
fn same<T: Eq>(a: &T, b: &T): bool = a == b

fn main() = {
    val one = 1
    val also = 1
    val first: &i64 = &one
    val second: &i64 = &also
    assert(same<&i64>(&first, &second))
    val found: Option<&i64> = .Some(&one)
    assert(found == .Some(&also))
}
```

## `&dyn`

`&dyn Interface` is a reference to a value of some type that implements it,
whichever type that is. The call is made through a table of the type's
methods, made once, at compile time:

```wip,run
interface Shape {
    fn area(): i64
}

struct Rect {
    width: i64
    height: i64
}

struct Circle {
    radius: i64
}

extend Rect: Shape {
    fn area(): i64 = self.width * self.height
}

extend Circle: Shape {
    fn area(): i64 = self.radius * 3
}

fn areaOf(shape: &dyn Shape): i64 = shape.area()

fn main() = {
    assert(areaOf(Rect(width: 2, height: 3)) == 6)
    assert(areaOf(Circle(radius: 2)) == 6)
}
```

`&dyn` is a reference, so it is second-class like any other: it is passed to
a call and not kept ([page 7](07-references.md)). A generic parameter is the
other way to write the same thing, and compiles to a direct call per type;
`&dyn` compiles to one function for all of them.

## The interfaces the prelude declares

| Interface | For |
|---|---|
| `Eq`, `Ord` | `==` and `<` and their siblings |
| `Hash` | keys of `Map` and `Set` |
| `Text` | `\(value)`, printing, and `toString()`: a type writes `toString` or `appendTo` |
| `Clone` | `clone()`, a copy that owns what it holds |
| `Destroy` | what to do when a value ends ([page 6](06-ownership.md)) |
| `Items<T>` | a container walked as a slice: `for x in container` |
| `Iterator<T>` | values one at a time, with the adapters ([page 10](10-library.md)) |
| `IntoIterator<I>` | a container that gives its elements up, as the iterator `I`: `for x in move container` ([page 4](04-expressions.md)) |
| `Sequence<T>` | `len()` and `at(i)`, both constant time: `x[i]` and `for` |
| `Index<K, V>` | a lookup whose key is not a position |
| `From<T>` | a value made from another, which `?` uses ([page 9](09-errors.md)) |
| `Add`, `Subtract`, `Multiply`, `Divide`, `Remainder` | `+`, `-`, `*`, `/` and `%`, for a type of a program's own, with what is on the other side and what the answer is |
| `Negate`, `Not` | `-x` and `!x`, for a type of a program's own |
