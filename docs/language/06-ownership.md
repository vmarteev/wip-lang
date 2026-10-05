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
    val first = Loud(id: 1)
    val second = Loud(id: 2)
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

## What this adds up to

- A value is freed exactly once, where its owner ends.
- Nothing is freed while something still refers to it, because references
  are second-class and cannot outlive the call they are lent for — which is
  [page 7](07-references.md).
- No garbage collector, no `free` written by hand, and no reference
  counting but where a program shares a value with `share()`.
