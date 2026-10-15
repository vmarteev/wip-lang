# Glossary

The words these pages use in a sense of their own, each in one line, then
what goes with it, and where the rule is.

**answer** — what a function or an expression evaluates to: `fn area():
i64` answers an `i64`, and a block answers its last expression. Wip says
"answers" where other languages say "returns", since an `if`, a `match`
and a block answer too. [Page 4](04-expressions.md#everything-answers-something)

**binding** — a name a pattern gives to part of a value. A `match`'s, an
`is`'s and a `for`'s binding is the place it matched, so writing through it
writes the value; one of plain data that is only read is a copy, and a
`var` pattern's are variables of their own. [Page 5](05-patterns.md#what-a-binding-is)

**borrow** — to refer to something without owning it, as a `str` refers to
the bytes of a `String`. What borrows is checked against what it borrows:
it is refused where that has changed or ended. [Page 7](07-references.md#views)

**box** — an `own<T>`: one value on the heap, owned by whatever holds the
box, and freed where that ends. [Page 6](06-ownership.md)

**closure** — a lambda that uses names from around it. One **lent** for a
call reads what is around it while the call runs, `&(x: i64) => i64`, or
reads and changes it, `&var (x: i64) => i64`; one kept in a `val` does the
same, to its last use, and an iterator's adapter keeps the one it is lent
as a view; one **owned**, `own<(x: i64) => i64>`, owns its own copies of
what it captured and may be kept anywhere, as another thread runs it. A
lambda that captures nothing is a **function value**, `(x: i64) => i64`,
code alone. [Page 3](03-functions.md#functions-as-values)

**constant** — a `val` at the top level of a file: worked out by the
compiler, kept once, and never written. One worked out by running the
program's own code says `@comptime`. A local `val` is a variable that is
not assigned again. [Page 2](02-modules.md#constants)

**constraint** — what a type parameter must implement, `T: Ord`; the
generic code may use that and nothing else. [Page 8](08-interfaces.md#constraints)

**debug build, release build** — the two ways a program is built. A debug
build, the default, checks moves and keeps an error's trace; `--release`
optimises. Both check arithmetic for overflow, indices against lengths,
and `assert`. [Page 12](12-tools.md#the-commands)

**`defer`** — an expression run when its block ends, however it ends. [Page 6](06-ownership.md#defer)

**derive** — `@derive(Eq, Ord, Hash, Text, Clone, Encode, Decode)`: the
compiler writes those interfaces for a type, field by field. [Page 8](08-interfaces.md#derive)

**`dyn`** — `&dyn Shape`, a reference to a value of some type that
implements `Shape`, called through a table of that type's methods. [Page 8](08-interfaces.md#dyn)

**`empties`** — `var fn clear() empties`: the call leaves the receiver
holding no view, so it borrows nothing until something is put in it again;
a list of views is cleared and filled while what it borrowed changes. [Page 7](07-references.md#empties-what-a-call-leaves)

**end** — a value ends where its owner does, in the reverse of the order
values were declared; what it owns is freed then, and a type that must do
something more implements `Destroy`. [Page 6](06-ownership.md#ending-destroy)

**error** — what may go wrong and is expected to sometimes: a value of
`Result`'s `.Err`, passed up with `?`. A **panic** is what must not
happen. [Page 9](09-errors.md)

**expected type** — the type a place asks of what is written there: a
`val`'s written type, a parameter's, a function's result, the other
branch of an `if`. A literal, a `.Variant`, a lambda's parameters, text
that becomes a `String`, a list that becomes a `Vec` and an integer that
widens all take their type from it. [Page 1](01-types.md)

**extend** — `extend Rect { … }` adds methods to a type, and `extend
Rect: Shape { … }` implements an interface for it. [Page 8](08-interfaces.md#declaring-and-implementing-one)

**generator** — an `Iterator` whose values a loop or a function hands over
with `yield`, one each time it is asked; it runs nothing until then, and
allocates nothing. [Page 4](04-expressions.md#generators)

**give** — to move a value somewhere: as an argument, into a field,
`return`ed, assigned. A place given away holds nothing after, and using it
again is refused at the place it was given. [Page 6](06-ownership.md#copies-and-moves)

**held to** — a body is held to what its signature says, `from`, `keeps`
and `empties`: one that does otherwise is refused, so that a caller may
rely on the signature alone. [Page 7](07-references.md#what-a-signature-says-about-borrows)

**`from`** — `fn text(i: i64): str from self.ast`, after a result type:
what the result borrows, where the signature alone would say more. [Page 7](07-references.md#from-what-a-call-answers)

**implicit parameter** — `implicit dice: &var Dice`: filled, where a call
leaves it out, from the caller's implicit of the same name. [Page 3](03-functions.md#implicit-parameters)

**interface** — a set of methods a type may implement; what a constraint
asks for, and what `&dyn` calls through. Rust's trait, Swift's protocol. [Page 8](08-interfaces.md)

**`keeps`** — `var fn add(word: str) keeps word`: the call may store that
parameter's value in one of its `&var` arguments, which then borrows it. A
call keeps nothing it does not say. [Page 7](07-references.md#keeps-what-a-call-stores)

**lend** — to let a call use a place for as long as it runs: `&place` for
reading, `&var place` for reading and writing. While a place is lent for
writing nothing else reaches it; while it is lent for reading nothing
writes it. [Page 7](07-references.md#references)

**`lend fn`, lending pair** — two methods of one name, one answering `&T`
and one `&var T`, where the call decides which: `board.at(0)` reads and
`board.at(0) = 10` writes. `lend fn` writes both from one body. [Page 7](07-references.md#lend-fn-the-pair-from-one-body)

**list** — `[a, b, c]`: an array where nothing expects more, a `Vec`
where one is expected, and a buffer, `own<[T]>`, where one is.
[Page 1](01-types.md#arrays-and-slices)

**method** — a function of a type, called with a dot. The word before
`fn` says what it is given: `fn` a `&Self`, `var fn` a `&var Self`, `move
fn` the value itself, `static fn` nothing, `lend fn` both in turn. [Page 3](03-functions.md#methods)

**module** — a directory of `.wip` files that share their names; a
**package** is a module with a `package.wip`, which a program may depend
on by path. [Page 2](02-modules.md)

**move** — a value that is not plain data moves where it is used as a
value: it is given, not copied. `move` is written only where a `match`,
`for` or pattern takes a value apart, `match move v`. [Page 6](06-ownership.md#copies-and-moves)

**`own`** — five things, all about owning what is made:

| Written | Is |
|---|---|
| `own<T>`, `own Node(…)` | a box, and making one |
| `own [1, 2, 3]` | a list on the heap, `own<[T]>`, where nothing expects a `Vec` or a buffer |
| `own for x in xs { yield … }` | the list a loop makes, made now |
| `own (x) => …` | a closure that owns what it captures |
| `for x in own leaves(…)` | a generator kept in a box, where a generator walks one of its own kind |

**place** — what can be named, lent and written: a variable, a field, an
element, or what a reference refers to. A **temporary** — what a call
answers, a literal — is not one, and is lent or moved where it is used.
[Page 7](07-references.md#what-may-happen-while-a-place-is-lent)

**plain data** — a value that owns no memory and cleans up after nothing:
numbers, `bool`, `char`, `str`, and arrays, tuples and structs of plain
data. It is copied where anything else moves; `T: copy` asks for it. [Page 6](06-ownership.md#copies-and-moves)

**prelude** — the names every file has without an import: `Option`,
`Result`, `Vec`, `String`, the interfaces the operators use, and the rest
of [page 8's list](08-interfaces.md#the-interfaces-the-prelude-declares).
A program may not declare one of them again. [Page 2](02-modules.md#the-prelude)

**projection** — a function that answers a reference into one of its own
reference parameters, written with `lend`: `fn at(i: i64): &i64 = lend
self.cells[i]`. [Page 7](07-references.md#projections-lending-a-place-out-of-a-call)

**second-class reference** — a `&var` can only be an argument: never a
variable's type, a field's, or what a function answers but a projection's,
and a kept closure that writes, which holds its `&var`s in its block.
So it never outlives the call it was lent for, and there is no lifetime to
write. A `&` may also be kept where a view is. [Page 7](07-references.md#references)

**stale** — a view whose owner has changed or ended since it was made;
using it is refused. [Page 7](07-references.md#views)

**text** — `str`, a view of bytes something else owns, and `String`, which
owns its bytes and grows. A literal is a `str`, and becomes a `String`
where one is expected. [Page 1](01-types.md#text)

**`.Variant`** — a variant written without its enum, which comes from the
expected type: `.Some(3)`, `.None`, `.Circle(r: 1.0)`. Where nothing is
expected it is written whole, `Shape::Circle(r: 1.0)`. [Page 1](01-types.md#enums)

**view** — a value that borrows: a `str`, a `&` kept in a variable, a
`view struct` or `view enum`, which may hold those. It is used only while
what it borrows stays as it was. [Page 7](07-references.md#views)

**walk** — to go through a container's elements. `for x in c` walks what
implements `Items`; `c.forwards()` and `c.backwards()` lend each element
where it lies, `c.copies()` copies each out into an `Iterator`, and `for x
in move c` takes them. [Page 10](10-library.md#walking-items-iterator-and-the-adapters)

**widen** — an integer taken as a wider integer type that holds every
value of it, with no `as`: a `u8` where an `i64` is expected, or beside
one. Every other conversion of a number is `as`. [Page 1](01-types.md#numbers)
