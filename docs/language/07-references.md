# 7. References and lending

## References

A reference is written at a call, and is a parameter's type. A `&var`
reference exists only while the call it was passed to is running: it is
never a variable's type, a field's, or what a function answers, except for a
projection, below. A `&` reference may also be kept where a view is kept,
which the section on views says.

```wip,run
fn total(values: &[i64]): i64 = {
    var sum = 0
    for value in values {
        sum += value
    }
    return sum
}

fn bump(value: &var i64) = {
    value += 1
}

fn main() = {
    val numbers = [1, 2, 3]
    assert(total(&numbers) == 6)

    var count = 1
    bump(&var count)
    assert(count == 2)
}
```

`&place` lends it for reading, `&var place` for reading and writing. An
argument that is not a place — a literal, a call's result — is already lent
where it is used, and takes no `&`:

```wip,run
fn total(values: &[i64]): i64 = {
    var sum = 0
    for value in values {
        sum += value
    }
    return sum
}

fn main() = {
    assert(total([1, 2, 3]) == 6)
}
```

A block that is an argument is lent as the argument would be: its value,
once it has left the block, is what is lent, so a `String` it answers is
passed where a `str` is taken, and a value where a `&` is. An `&` written
as a block's value borrows what the block holds, and is refused where it
would outlive it:

```wip,run
fn total(values: &[i64]): i64 = {
    var sum = 0
    for value in values {
        sum += value
    }
    return sum
}

fn main() = {
    val sum = total({
        val first = 1
        [first, 2, 3]
    })
    assert(sum == 6)
}
```

This is what makes ownership cheap to check: since no `&var` outlives the
call it was made for, two writers of one place are only ever two arguments
of one call, and there is no lifetime to write down.

## What may happen while a place is lent

While a place is lent for writing, nothing else may read or write it; while
it is lent for reading, nothing may write it. The check is on the place, so
lending two different fields of one struct at once is fine, and lending the
same one twice is not:

```wip,error=E0413
fn twice(a: &var i64, b: &var i64) = {
    a += b
}

fn main() = {
    var count = 1
    twice(&var count, &var count)
}
```

## Projections: lending a place out of a call

A function may answer a reference where that reference is a place inside one
of its own reference parameters. The body says which place with `lend`, and
the caller gets a reference that lives as long as the call it stands in:

```wip,run
struct Board {
    cells: Vec<i64>
}

extend Board {
    fn at(index: i64): &i64 = lend self.cells[index]

    var fn at(index: i64): &var i64 = lend self.cells[index]
}

fn main() = {
    var board = Board(cells: Vec::of(own [1, 2, 3]))
    assert(board.at(0) == 1)
    board.at(0) = 10
    board.at(0) += 5
    assert(board.at(0) == 15)
}
```

A projection may also lend a constant table — a top-level `val` that is an
array, a struct, a tuple or a variant — or part of one, for reading. A
table lives as long as the program and nothing writes it, so a call that
lends only tables holds nothing of its arguments:

```wip,run
val LOW: [i64; 2] = [1, 2]
val HIGH: [i64; 2] = [30, 40]

struct Mode {
    high: bool = false
}

fn levels(mode: &Mode): &[i64] = if mode.high then lend HIGH else lend LOW

fn raise(mode: &var Mode, seen: &[i64]) = {
    mode.high = seen[0] < 10
}

fn main() = {
    var mode = Mode()
    // `levels` holds nothing of `mode`, which may be lent as `&var` beside it.
    raise(&var mode, &levels(&mode))
    assert(mode.high && levels(&mode)[1] == 40)
}
```

A projection that lends a parameter's place on any path holds that
argument, as before; a number has no place to lend, and is answered by
value.

The two declarations above are a **lending pair**: one for reading, one for
writing, and where the call stands decides which is called. A `Vec`'s `at`,
a `Map`'s `at` and `values` are written this way.

## `lend fn`: the pair from one body

Where both halves would say the same thing, `lend fn` declares them both
from one body. `self` is `&Self` in one half and `&var Self` in the other,
and the result `&T` in one and `&var T` in the other; the body is checked
twice, once each way:

```wip,run
struct Board {
    cells: Vec<i64>
}

extend Board {
    lend fn at(index: i64): &i64 = lend self.cells[index]
}

fn main() = {
    var board = Board(cells: Vec::of(own [1, 2, 3]))
    assert(board.at(2) == 3)
    board.at(2) = 30
    assert(board.at(2) == 30)
}
```

Its result is written `&T`: a result that is not a reference lends nothing,
and one written `&var T` is told that the writing half makes it `&var` by
itself.

## Views

A `str` is a view of bytes that belong to something else, and so is a
`view struct` — a struct that may hold a `str` or a reference. A view is
checked against what it borrows: where the owner changes, the view is stale,
and using it is refused:

```wip,error=E0436
fn main() = {
    var text = String::of("hello")
    val view = text.toStr()
    text.push(" again")
    assert(view.len() == 5)
}
```

A view struct is declared with `view` before `struct`, and may be used
wherever the thing it borrows outlives it:

```wip,run
view struct Words {
    text: str
    at: i64
}

extend Words {
    static fn of(text: str): Words = Words(text: text, at: 0)

    fn rest(): str = self.text[self.at..]
}

fn main() = {
    val whole = "one two"
    val words = Words::of(whole)
    assert(words.rest() == "one two")
}
```

What a call answers borrows an argument only where the argument's type
can hold what the answer refers to, by the callee's signature: a `&Node`
cannot point into a `str`, so a lookup by name borrows the tree and not
the name. A reference argument whose type holds it as
its own lends its place; one that reaches it through a borrow lends what
it borrows, so `words.next()` borrows the text and not `words`:

```wip,run
struct Node {
    size: i64
}

struct Tree {
    nodes: Vec<Node>
}

view struct Found {
    node: &Node
}

extend Tree {
    fn first(name: str): Found = Found(node: &self.nodes[0])
}

fn main() = {
    val tree = Tree(nodes: Vec::of(own [Node(size: 3)]))
    var name = String::of("a")
    val found = tree.first(name.toStr())
    name.clear()
    assert(found.node.size == 3)
}
```

A `view enum` is an enum whose variants may borrow, as a view struct's
fields may. It is checked the same way, and what a `match`
binds from one borrows what the enum borrowed:

```wip,run
view enum Token {
    Word(text: str)
    Number(value: i64)
    End
}

fn first(text: str): Token = {
    val trimmed = text.trim()
    if trimmed.isEmpty() then return .End
    return match trimmed.toInt() {
        .Ok(value) => .Number(value)
        .Err(..) => .Word(trimmed)
    }
}

fn main() = {
    val line = String::of("  hello ")
    val word = match first(line.toStr()) {
        .Word(text) => text
        _ => ""
    }
    assert(word == "hello")
}
```

A tree of views — a screen's, built from a model for one frame — is a
`view enum` whose variants hold the others in a `Vec`, and the model
cannot change while the tree is in use.

## A reference kept as a view

A `&` reference is a view. It may be a type argument — `Option<&Node>`,
`Vec<&Node>` — a local, a view's field and a variant's payload, and it
borrows what it points into, as a `str` borrows its bytes. `&place` is
written wherever a reference is expected: a local's value, an assignment to
one, a variant's payload. A local whose whole value is `&place` holds a
reference without its type written; a `&` that is part of a value — a
tuple's element — needs the type said. A slice has nothing of fixed size to
copy, so a local or an array given a slice read through a reference — a
parameter `bytes: &[u8]`, a constant of embedded bytes, a call that lends
one — holds the reference: `val all = bytes`, `[bytes, rest]`, `val view =
list.items()`, which borrows `list` as `&list.items()` would. A `var`
holding one may be pointed somewhere else. Reading it reads what it refers
to, as reading a reference parameter does:

```wip,run
struct Node {
    size: i64
}

struct Tree {
    nodes: Vec<Node>
}

extend Tree {
    fn find(size: i64): Option<&Node> = {
        for i in 0..self.nodes.len() {
            if self.nodes[i].size == size then return .Some(&self.nodes[i])
        }
        return .None
    }
}

fn main() = {
    val tree = Tree(nodes: Vec::of(own [Node(size: 1), Node(size: 2)]))
    val found = tree.find(2)
    assert(found.isSome())
    var at = &tree.nodes[0]
    assert(at.size == 1)
    at = &tree.nodes[1]
    assert(at.size == 2)
}
```

What it points into must not change while it is used, and it cannot
leave the function or block that holds that place (E0436, E0437):

```wip,error=E0436
struct Node {
    size: i64
}

fn main() = {
    var nodes = Vec::of(own [Node(size: 1)])
    val first = &nodes[0]
    nodes.clear()
    assert(first.size == 1)
}
```

A reference passes on as the value it is: `?` on an
`Option<&T>` answers the `&T`, a method of `T` is given the reference
itself, and what is reached through one kept in a variable — the elements
of a `&Vec<T>`, walked — borrows what that variable points into. What a
function a call is given makes of a reference borrows what the reference
did: `map` on an `Option<&String>`, making a `str` of it, answers an
`Option<str>` that borrows the `String`. So a chain of
lookups, and a list of references gathered from one, leave the function
that received the reference:

```wip,run
import std::json
import std::json::{Json}

fn kind(node: &Json): Option<str> = node.get("kind")?.asText()

fn named(node: &Json, wanted: str): Vec<&Json> = {
    var found: Vec<&Json> = Vec()
    if node.get("inner").flatMap((inner) => inner.asArray()) is .Some(nodes) {
        for child in nodes.items() {
            if kind(child) == .Some(wanted) then found.push(child)
        }
    }
    return move found
}

fn main() = {
    val .Ok(tree) = json::parse(
        "{\"inner\": [{\"kind\": \"Fn\"}, {\"kind\": \"Var\"}]}",
    ) else {
        return
    }
    assert(named(&tree, "Fn").len() == 1)
}
```

A `&var` is kept nowhere: a view may be copied, and two copies of a
`&var` would each write one place. A struct or an enum that holds a `&`
is declared `view`, as one that holds a `str` is.

Nor is what a `&var` reaches kept by another argument: a
call that writes a context — a parser stepped with the machine it fills —
leaves a view argument borrowing what it borrowed, and the caller goes on
writing the context. A body that would keep a view of it is refused
(E0447); a value argument, and what a `&` reaches, may be kept as before:

```wip,run
struct Vm {
    names: Vec<String>
}

view struct Parser {
    source: str
    at: i64
}

extend Parser {
    var fn step(vm: &var Vm) = {
        vm.names.push(String::of(self.source[self.at..self.at + 1]))
        self.at += 1
    }
}

fn main() = {
    var vm = Vm(names: Vec())
    val text = String::of("ab")
    var parser = Parser(source: text.toStr(), at: 0)
    parser.step(&var vm)
    parser.step(&var vm)
    assert(vm.names.len() == 2)
}
```

## What a thread may borrow

A reference lives for the call it is lent to, so a thread may borrow
where the call that starts it does not answer until it has ended:
`future::together`, `each` and `map`. The closures they
run are checked as a call's arguments are, which is what keeps threads
from racing: a place one writes, no other reaches, and a place several
reach, all only read. The one way to change a place through a `&` is
`std::sync`'s `Atomic` and `Mutex`, each safe from any number of threads.

## What is still to come

Generic code writes through `&var [T]` rather than through `c[i]`, because
the interfaces that answer `at` ask for the reading half alone. An interface
that asks for both halves of a lending pair is the next step.
