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

An interface may take types of its own — `From<T>`, `Items<Item>`,
`Iterator<Item>` — and a constraint names them: `C: Items<i64>` above.
Several are joined with `+`: `<K: Hash + Eq>`.

The last of an interface's types may have defaults, which may name `Self`,
the type that implements it, and the types before them: the operators'
interfaces are declared `interface Add<Rhs = Self, Out = Self>`, so
`extend Money: Add` is `Add<Money, Money>`, and so is the constraint
`T: Add`, while `extend Vec2: Multiply<f32>` scales a vector.

Every built-in number implements the operators' interfaces, each answered
by the operator itself and checked as it is, and `Zero` and `One`, what
adding and multiplying start from. `a + b` between two numbers is still
the machine's instruction; the implementations are what a type parameter
reaches, so code generic over numbers is written once:

```wip,run
fn total<T: Add + Zero + copy>(items: &[T]): T = {
    var sum = T::zero()
    for x in items {
        sum = sum + x
    }
    return sum
}

fn main() = {
    val counts = [1, 2, 3]
    val weights = [0.5, 0.25]
    assert(total(&counts) == 6 && total(&weights) == 0.75)
}
```

`copy` is a constraint too, and holds for a type that is plain data:
`fn pair<T: copy>(value: T): (T, T) = (value, value)`.

## Types an implementation decides

Some of an interface's types are not for whoever names it to choose: a
container has one kind of element, and its implementation says which. The
interface marks such a type `type`, and each implementation decides it
where it names the interface, as it names any other type:

```wip,run
interface Shelf<type Book> {
    fn first(): Book
}

struct Range {
    from: i64
    to: i64
}

extend Range: Shelf<i64> {
    fn first(): i64 = self.from
}

// Any shelf: what it holds is read as `S::Book`.
fn firstOf<S: Shelf>(shelf: &S): S::Book = shelf.first()

// A shelf of numbers, pinned, so that it can be added to.
fn firstPlusOne<S: Shelf<i64>>(shelf: &S): i64 = shelf.first() + 1

fn main() = {
    val range = Range(from: 3, to: 9)
    assert(firstOf(&range) == 3)
    assert(firstPlusOne(&range) == 4)
    val book: Range::Book = 7
    assert(book == 7)
}
```

- **A constraint may leave it out.** `S: Shelf` asks for a shelf of
  anything; `S: Shelf<i64>` pins what it holds, positionally. So decided
  types come after the ones a constraint names, and have no default
  (E0365).
- **It is read as `S::Book`,** wherever a type is written, by the name the
  interface gives it; a concrete type's is what its implementation says:
  `Range::Book` is `i64`, and `Chars::Item` is `char`. Where two interfaces
  a parameter is constrained by each decide a `Book`, which one `S::Book`
  means is in question, and it is refused (E0365).
- **A type implements the interface once.** It is the same implementation
  whatever it decides, so a second, deciding another type, is refused where
  it is declared (E0340): which one applies is never in question.
- **What is asked of it is said of a parameter.** A constraint is written
  on a parameter, never on `S::Book`, so code that needs the element to be
  something names it, `fn largest<I: Iterator<T>, T: Ord>(values: I)`,
  where code that only passes it on reads `I::Item`.

```wip,error=E0340
interface Shelf<type Book> {
    fn count(): i64
}

struct Library

extend Library: Shelf<String> {
    fn count(): i64 = 0
}

extend Library: Shelf<i64> {
    fn count(): i64 = 1
}

fn main() = {}
```

The prelude's `Iterator`, `Items`, `Sequence`, `IntoIterator` and
`Index` decide their element, so what an adapter holds names only the
iterator it was made from: `it.map(f)` is a `Mapped<I, U>`, with `f`
taking an `I::Item`.

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

## Methods for every implementer, where its types allow

An interface that takes types may be extended under a condition on them:
the block's methods are every implementer's whose types meet it, and no
other's. The prelude gives an iterator `sum` where its element adds up and
`max` where it is ordered this way:

```wip,run
interface Source<T> {
    var fn take(): Option<T>
}

extend Source<T: Add + Zero> {
    var fn total(): T = {
        var sum = T::zero()
        while self.take() is .Some(x) {
            sum = sum + x
        }
        return move sum
    }
}

struct Countdown {
    left: i64
}

extend Countdown: Source<i64> {
    var fn take(): Option<i64> = {
        if self.left == 0 then return .None
        self.left -= 1
        return .Some(self.left + 1)
    }
}

fn through<S: Source<i64>>(source: &var S): i64 = source.total()

fn main() = {
    var countdown = Countdown(3)
    assert(countdown.total() == 6)
    var more = Countdown(4)
    assert(through(&var more) == 10)
}
```

The body is a default's, with the condition in scope: `self` is whatever
implements the interface, and `T::zero()` and `+` are what `T: Add + Zero`
promises. A call on a type whose types do not meet the condition says
which lacks what, and so does one through a constraint that does not
promise it.

- **It adds methods, and implements nothing.** No type comes to implement
  an interface by it, so a constraint asks what it always asked.
- **It is written in the interface's module,** as an implementation is
  written with its type or its interface: a method comes with its type or
  its interface, and is never imported on its own.
- **It has a condition.** A method every implementer has is a default, and
  is written in the interface.
- **Its names are its own.** A method the interface or another of its
  extensions has is refused, whatever the conditions, and an
  implementation does not write one: it is the same for every implementer.
- **A type's own method of the name is its own,** as it is over a default.
  A name that two interfaces give a type, each a default or an extension,
  is not chosen between: the call is refused, and says which (E0363).

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
| `Items<type Item>` | a container walked as a slice: `for x in container` |
| `Iterator<type Item>` | values one at a time, with the adapters ([page 10](10-library.md)) |
| `IntoIterator<type Iter>` | a container that gives its elements up, as the iterator `Iter`: `for x in move container` ([page 4](04-expressions.md)) |
| `Sequence<type Item>` | `len()` and `at(i)`, both constant time: `x[i]` and `for` |
| `Index<K, type V>` | a lookup whose key is not a position |
| `From<T>` | a value made from another, which `?` uses ([page 9](09-errors.md)) |
| `Add`, `Subtract`, `Multiply`, `Divide`, `Remainder` | `+`, `-`, `*`, `/` and `%`, with what is on the other side and what the answer is: every number has them, and a type of a program's own may |
| `Negate`, `Not` | `-x` and `!x`: every signed number and float negates |
| `Zero`, `One` | what adding and multiplying start from: every number has them, and `sum` and `product` ask for them |
