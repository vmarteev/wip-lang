# 6. Ownership

Every value has one owner, and ends when its owner does. Nothing is counted
at run time and nothing is collected: where a value ends is decided while
the program is compiled. The one exception is one a program
asks for: a value several owners read is a `std::shared::Shared<T>`,
counted where a line calls `share()` and freed with its last owner
([page 10](10-library.md)).

## Copies and moves

A value that is **plain data** is copied: it owns no memory, directly or in
its fields, and nothing in it cleans up after itself. Numbers, `bool`,
`char`, `str`, arrays and structs whose fields are all plain data are plain
data.

Everything else **moves**: giving it to something else hands over the
ownership, and the place it came from no longer holds it. A move out of a
place is written `move`, so that reading a name never quietly empties it:

```wip,run
fn main() = {
    // Plain data is copied, and both names hold a value.
    val first = [1, 2, 3]
    val second = first
    assert(first[0] == second[0])

    // A `String` owns its bytes, so handing it over is a move.
    var text = String::of("held")
    val taken = move text
    assert(taken == "held")
}
```

Reading a moved-out place is refused, and the message says where it was
moved. A temporary — the result of a call, a literal — is already moved
where it is used, so `move` is not written on one.

## `own<T>`

`own<T>` is a value on the heap that this value owns. It is written with
`own` before what is being made, and is what a type that holds something of
its own size holds:

```wip,run
struct Node {
    value: i64
    next: Option<own<Node>>
}

fn main() = {
    val list = Node(value: 1, next: .Some(own Node(value: 2, next: .None)))
    assert(list.value == 1)
    val .Some(rest) = list.next else {
        assert(false, "there is a second node")
        return
    }
    assert(rest.value == 2)
}
```

`Option<own<T>>` is one word wide: nothing is the null pointer. The memory
is freed when the value that owns it ends.

A box clones where what it holds does: its clone is a box of its own,
holding a clone of the value, so `@derive(Clone)` clones a recursive type
whose children are boxed, and nothing is shared between the two.

A method called on an `own<T>` is `T`'s, and a field read through it is
`T`'s, as through a reference. An interface is another matter: an
`own<T>` is not a `T`, so it implements what an `extend own<T>` block
implements for a box, and nothing of `T`'s. A module may write one for an
interface of its own, which holds where its condition on `T` does:

```wip,run
interface Describe {
    fn describe(): String
}

struct Leaf {
    name: String
}

extend Leaf: Describe {
    fn describe(): String = "leaf \(self.name)"
}

extend own<T: Describe>: Describe {
    fn describe(): String = "boxed \(self.describe())"
}

fn told<T: Describe>(x: &T): String = x.describe()

fn main() = {
    val boxed = own Leaf(name: String::of("a"))
    assert(told(&boxed) == "boxed leaf a")
    // Called on the box, the method is the leaf's.
    assert(boxed.describe() == "leaf a")
}
```

## Ending: `Destroy`

A type that must do something when its values end implements `Destroy`,
whose method the compiler calls at the end of the value's life:

```wip,run
import std::io

struct Loud {
    id: i64
}

extend Loud: Destroy {
    var fn destroy() = io::printInt(self.id)
}

fn main() = {
    val first = Loud(1)
    val second = Loud(2)
    // `second` ends first: values end in the reverse of the order they
    // were declared.
    assert(first.id + second.id == 3)
}
```

A value may be ended early with `destroy(move value)`, and what it holds is
ended then and there. A type with a `Destroy` is never plain data, so it is
moved rather than copied, and it cannot be copied out of a place that is
only borrowed.

## `defer`

`defer expression` runs that expression when the block it stands in ends,
however it ends — falling off the end, `return`, `break`, or a panic that is
handled by nothing. Deferred expressions run in the reverse
of the order they were written:

```wip,run
fn main() = {
    var order = Vec<i64>()
    {
        defer order.push(1)
        defer order.push(2)
        order.push(3)
    }
    assert(order == Vec::of(own [3, 2, 1]))
}
```

`defer` is for what a type cannot own: a C handle closed by a function, a
lock released by name. What a value owns is `Destroy`'s business instead,
since that runs wherever the value ends rather than only here.

A `return` may not stand inside a deferred expression, and neither may
`break` or `continue` leaving the loop it is in.

## Where memory comes from: `mem::using`

Every `own`, `Vec` and `String` takes its memory from the heap, unless a
call says otherwise. `std::mem::using(&var allocator, body)` runs `body`
with every allocation it makes on this thread — in its own code and in
every function it calls, the library's included — taken from the
allocator, and the one before it back when the call ends. Each value
remembers where it was made and is freed there, whenever that is: a value
made under `using` and dropped after it is still given back to its
allocator.

A game keeps a frame's short-lived values — what is visible, the draw
commands, the text of an overlay — in an `Arena`: memory handed out by
moving a pointer forward, and taken back all at once by `reset`. A value
the arena holds that is still alive when it is reset or dropped is a
panic, so no value outlives the memory it lives in; in a debug build the
panic first says where the first few such values were made. What a frame
makes to keep is made under `mem::onHeap`, which is `using` with the heap —
and growing a list that outlives the frame is making something to keep:

```wip,run
import std::mem
import std::mem::{Arena}

struct Asset {
    name: String
}

/// Kept past the frame: the asset, and the list's room for it.
fn load(into: &var Vec<own<Asset>>) = {
    mem::onHeap(() => into.push(own Asset(name: "box")))
}

fn main() = {
    var frame = Arena::of(65536)
    var loaded: Vec<own<Asset>> = Vec()
    for round in 0..3 {
        val drawn = mem::using(&var frame, () => {
            var visible: Vec<i64> = Vec()
            for i in 0..100 {
                if i % 3 == 0 then visible.push(i)
            }
            if round == 1 then load(&var loaded)
            return visible.len()
        })
        assert(drawn == 34 && frame.live() == 0)
        frame.reset()
    }
    assert(loaded.len() == 1 && loaded[0].name == "box")
}
```

- **A thread has its own:** `using` reaches the allocations of the thread
  that calls it. A thread its body starts — `future::each`, `together` —
  allocates from the heap, since an arena reached from two threads at
  once would need a lock on every allocation.
- **Calls nest,** and the innermost gives.
- **A program's own allocator** implements `mem::Allocator` —
  `allocate(size, align)` and `free(at)` — and is kept by `mem::Custom::of`
  where `using` can reach it: a pool of fixed-size slots, or memory a GPU
  maps. Dropping it while something it made is alive is a panic, as an
  arena's is.
- **It costs code that never says `using` one load** of a word the thread
  keeps, before `malloc` as before.

## What this adds up to

- A value is freed exactly once, where its owner ends.
- Nothing is freed while something still refers to it, because references
  are second-class and cannot outlive the call they are lent for — which is
  [page 7](07-references.md).
- No garbage collector, no `free` written by hand, and no reference
  counting but where a program shares a value with `share()`.
