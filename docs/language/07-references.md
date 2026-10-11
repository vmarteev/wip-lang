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

A place given to a call is lent when the call starts, once every argument
is worked out, in the order written. So a later argument may change it:
a method's receiver may be changed by the method's own argument.

```wip,run
struct Counter {
    items: Vec<i64> = Vec()
    seen: i64 = 0
}

extend Counter {
    var fn add(n: i64): i64 = {
        self.items.push(n)
        return self.items.len()
    }

    var fn record(n: i64) = {
        self.seen = n
    }
}

fn main() = {
    var counter = Counter()
    counter.record(counter.add(7))
    assert(counter.seen == 1)
}
```

What borrows where it is made — a `str`, a view, a closure — borrows from
there, so a later argument may not change what it borrows; and one that
moves a place an earlier argument names leaves nothing to lend:

```wip,error=E0414
struct Tag {
    name: String
}

extend Tag {
    var fn rename(): i64 = {
        self.name = String::of("renamed")
        return 0
    }
}

fn show(text: str, n: i64) = {}

fn main() = {
    var tag = Tag(String::of("first"))
    show(tag.name.toStr(), tag.rename())
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
    var board = Board(Vec::of(own [1, 2, 3]))
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
    var board = Board(Vec::of(own [1, 2, 3]))
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
and using it is refused. A C string made from a `String` is checked the
same way ([page 11](11-c.md)), though it may be held wherever a `cstring`
may:

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
    val tree = Tree(Vec::of(own [Node(size: 3)]))
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
    val tree = Tree(Vec::of(own [Node(size: 1), Node(size: 2)]))
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

A place read through a view's `&` field — `self.ast.names[0]` in a
checker that holds the syntax tree it checks — borrows what that field
borrows, not the view. A view holds no `&var`, so nothing done to the view
changes what its `&` points at: in its own methods that is the caller's,
as a reference parameter's referent is, so an element read through the
field is kept across a call that writes the view, given to one, or walked
while the view changes, and text read through it is answered as the
view's. A place the view owns is part of it, as before:

```wip,run
struct Ast {
    names: Vec<String>
}

view struct Lowerer {
    ast: &Ast
    errors: Vec<String>
}

extend Lowerer {
    var fn report(text: str) = self.errors.push("unknown: \(text)")

    var fn check() = {
        for name in self.ast.names.items() {
            if name.isEmpty() then self.report(self.ast.names[0].toStr())
        }
        val first = &self.ast.names[0]
        self.errors.clear()
        assert(first == "a")
    }
}

fn main() = {
    val ast = Ast(names: Vec::of(own ["a", ""]))
    var lowerer = Lowerer(ast: &ast, errors: Vec())
    lowerer.check()
}
```

What a call answers borrows what its arguments' types can hold, which is
read from the signature alone; a helper of a view that owns as well as
borrows answers what borrows the whole view, and a function of two texts
answers what borrows both. `from` after the result type says what it
borrows instead, and nothing else: a parameter, whose place and what it
borrows, or `self.ast`, a `&`, `str` or view field of one, what the
parameter borrows and not its place. The body is held to it (E0437), so changing the
body never breaks a caller, and the caller keeps only what it names:

```wip,run
struct Ast {
    names: Vec<String>
}

view struct Lowerer {
    ast: &Ast
    errors: Vec<String>
}

extend Lowerer {
    fn text(i: i64): str from self.ast = self.ast.names[i].toStr()
}

fn label(name: str, fallback: str): str from name =
    if name.isEmpty() then "?" else name

fn main() = {
    val ast = Ast(names: Vec::of(own ["ab"]))
    var lowerer = Lowerer(ast: &ast, errors: Vec())
    val text = lowerer.text(0)
    lowerer.errors.push("the view changes; the text is the tree's")
    assert(text == "ab")
    var fallback = String::of("none")
    val named = label("given", fallback.toStr())
    fallback.push("!")
    assert(named == "given")
}
```

A `str` field is named as a `&` field is, since its bytes are never what
holds it: a reader that holds its source answers the source's text, which
stays good while the reader changes and goes bad where the source does:

```wip,run
view struct Reader {
    source: str
    notes: Vec<String>
}

extend Reader {
    fn text(lo: i64, hi: i64): str from self.source = self.source[lo..hi]
}

fn main() = {
    val source = String::of("fn main")
    var reader = Reader(source: source.toStr(), notes: Vec())
    val word = reader.text(0, 2)
    reader.notes.push("the reader changes; the text is the source's")
    assert(word == "fn")
}
```

`from` is a word only after a result type, so a method may still be named
`from`.

A `&var` is kept nowhere: a view may be copied, and two copies of a
`&var` would each write one place. A struct or an enum that holds a `&`
is declared `view`, as one that holds a `str` is.

Nor is what a `&var` reaches kept by another argument: a
call that writes a context — a parser stepped with the machine it fills —
leaves a view argument borrowing what it borrowed, and the caller goes on
writing the context. A body that would keep a view of it is refused
(E0447); a value argument, and what a `&` reaches, may be kept where the
signature says so, as the next section shows:

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
        vm.names.push(self.source[self.at..self.at + 1])
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

## What a call keeps

A call keeps nothing of its arguments unless its signature says `keeps`
and the parameters whose values it may keep in its `&var` arguments, the
receiver of a `var fn` among them. A view's method given text it only
reads leaves the view borrowing what it borrowed, so the text may end
while the view goes on; a method that keeps says so, and what it is given
stays borrowed by what keeps it:

```wip,run
view struct Printer {
    src: str
    out: String
}

extend Printer {
    var fn line(text: str) = self.out.push(text)
}

view struct Words {
    words: Vec<str>
}

extend Words {
    var fn add(word: str) keeps word = self.words.push(word)
}

fn main() = {
    val source = String::of("a b")
    var printer = Printer(src: source.toStr(), out: String())
    var words = Words(words: Vec())
    for part in source.toStr().split(" ") {
        // `text` ends with each turn, and nothing kept it.
        val text = String::of(part)
        printer.line(text.toStr())
        // `part` borrows `source`, which lasts.
        words.add(part)
    }
    assert(printer.out == "ab" && words.words.len() == 2)
}
```

The body is held to it: one that stores a parameter its `keeps` does not
name in a `&var` parameter's place, by assignment or by a call that keeps
it, is refused, with a fix that names it (E0449). A value of a type
parameter counts, since an instance may make it a `str`, which is why
`Vec`'s `push` says `keeps value`:

```wip,error=E0449
struct Bag<T> {
    items: Vec<T>
}

extend Bag<T> {
    var fn put(item: T) = self.items.push(move item)
}

fn main() = {}
```

`keeps` names parameters that are not `&var`, in a function that has a
`&var` parameter to keep them in, and follows the result type and `from`:
`fn longer(a: str, b: str, into: &var Vec<str>): str from a, b keeps a, b`.
An interface's method says what it keeps, and an implementation keeps no
more; a call through a function value says nothing, and is taken to keep
every argument. As `from` is, `keeps` is a word only there.

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
