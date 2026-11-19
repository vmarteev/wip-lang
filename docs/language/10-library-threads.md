# 10.1. Threads

Part of [the standard library](10-library.md): work run on another
thread, and work that borrows what the calling thread holds, with what
several threads may change at once.

## Work on another thread: `std::future`

`Future::run` runs an owned closure on a thread of its own, and hands
back a `Future<T>` of what it answers: `get()` waits for it, `poll()`
takes it if it is there without waiting, `pollWithin(duration)` waits for
it that long at most, and `isDone()` says whether the work has finished.
A channel moves values between threads; its receiver is an `Iterator`,
which a loop reads until every sender is gone. `tryReceive()` answers a
`Received` — a `Value`, `Empty` or `Closed` — without waiting, and
`receiveWithin(duration)` the same once a value comes or the time is up:

```wip,run
import std::future::{Future, channel}

fn main() = {
    val data = Vec::of([1, 2, 3, 4])
    val sum = Future::run(own () => {
        var total = 0
        for x in data do
            total += x

        total
    })
    assert(sum.get() == 10)

    val (tx, rx) = channel<String>()
    val producer = Future::run(own () => {
        for i in 0..3 do
            tx.send("line \(i)")
    })
    var lines = 0
    for line in move rx do
        lines += 1

    assert(lines == 3)
}
```

What the closure captured is moved into it, and is the thread's alone: an
owned closure cannot hold a reference or a view, so nothing two threads
could both reach is ever shared, and nothing needs a lock. A future that is
dropped waits for its thread, so work never outlives what started it.

## Work that borrows: `both`, `each`, `map` and `std::sync`

Work may also borrow what is around it, where the call that runs it waits
for it. `future::both(first, second)` runs `first` on a thread of its own
and `second` on this one, and answers both results once both are done;
`future::each(items, work)` and `future::map(items, work)` split a slice
among the processors, as many as `future::cores()` says are working. The
closures are lent for the call, so what they capture is checked as the
call's arguments are: what one writes, no other touches, and several may
read one place. `each` and `map` call one closure from every thread at
once, so it reads, `&(…) => R`, and one that writes is refused there.

Two types change through a `&`, safely from any number of threads:
`std::sync::Atomic<T>` — `load`, `store`, `swap`, `add`, `subtract` and
`replaceIf`, each one step no thread sees half of — for `bool`, the
integers and floats, and C pointers, and `Mutex<T>`, whose `lock` lends
what is inside as a `&var` for its call alone, and whose `lockWhen` waits
until it is ready:

```wip,run
import std::future
import std::sync::{Atomic, Mutex}

fn sum(values: &[i64]): i64 = {
    var total = 0
    for value in values do
        total += value

    return total
}

fn main() = {
    val numbers = [1, 2, 3, 4, 5, 6, 7, 8]
    val (low, high) = future::both(
        () => sum(&numbers[0..4]),
        () => sum(&numbers[4..]),
    )
    assert(low + high == 36)

    val counted = Atomic::of(0 as i64)
    var cells = [1, 1, 1, 1]
    future::each(&var cells, (cell) => {
        cell *= 2
        counted.add(1)
    })
    assert(counted.load() == 4 && cells[0] == 2)
    assert(future::cores() >= 1)

    val names: Mutex<Vec<String>> = Mutex::of(Vec())
    future::both(
        () => names.lock((all) => all.push("ann")),
        () => names.lock((all) => all.push("bob")),
    )
    assert(names.lock((all) => all.len()) == 2)
}
```

A wait may give up. `lockWhenWithin(ready, duration, body)` waits as
`lockWhen` does, for that long at most, and answers what `body` answered,
or nothing where it was not ready in time; a wait woken before its time,
and still not ready, waits out the rest. The time is measured on the clock
that only goes forward, so setting the time of day changes no wait.
`tryLock(body)` runs `body` only where no other thread holds the lock, for
a thread that has other work meanwhile. `sync::Lazy<T>` is a value made
by the first thread that asks for it, by the closure it was made with,
and lent to every thread after: a field of what the threads share, made
only where one of them needs it. `get()` answers it, and `made()` answers
it only where it is made already:

```wip,run
import std::future
import std::sync::{Lazy, Mutex}
import std::time::{Duration}

struct Shared {
    words: Lazy<Vec<String>>
}

fn main() = {
    val queue: Mutex<Vec<i64>> = Mutex::of(Vec())
    val waited = queue.lockWhenWithin(
        (jobs) => !jobs.isEmpty(),
        Duration::millis(10),
        (jobs) => jobs.pop(),
    )
    assert(waited.isNone())
    assert(queue.tryLock((jobs) => jobs.len()) == .Some(0))

    val shared = Shared(
        words: Lazy::of(own () => {
            var words: Vec<String> = Vec()
            for word in "a b c".split(" ") do
                words.push(word)

            return words
        }),
    )
    assert(shared.words.made().isNone())
    val (first, second) = future::both(
        () => shared.words.get().len(),
        () => shared.words.get().len(),
    )
    assert(first == 3 && second == 3)
}
```

An atomic operation is sequentially consistent unless it is given an
order: every thread agrees on one order of every such operation, which is
what a counter or a flag needs and the easiest to get right. Code written
lock-free names the weaker order it relies on, which costs less where the
processor orders less by itself — on x86-64 a sequentially consistent
store is a locked exchange, and a release store a plain one. A load takes
a `LoadOrder` — `.Relaxed`, `.Acquire` or `.SeqCst` — a store a
`StoreOrder` — `.Relaxed`, `.Release` or `.SeqCst` — and the rest an
`Order`, any of the five, `.AcqRel` among them; an order an operation
cannot take is a type error. `sync::fence(order)` orders this thread's
reads and writes against other threads' atomics with nothing read or
written. An `Atomic` also holds `f32` and `f64`, added to and taken from
as floats, and a C pointer, `ptr<T>`; `replaceIf` compares what it holds
bit for bit, so a NaN is the NaN it was:

```wip,run
import std::future
import std::sync::{Atomic}

// One writer and one reader: the writer fills the slot, then publishes it
// with a release store; the reader acquires the count, then reads the slot.
struct Mailbox {
    slot: Atomic<i64> = Atomic::of(0)
    sent: Atomic<i64> = Atomic::of(0)
    read: Atomic<i64> = Atomic::of(0)
}

extend Mailbox {
    fn send(value: i64) = {
        val count = self.sent.load(order: .Relaxed)
        while self.read.load(order: .Acquire) != count {}
        self.slot.store(value, order: .Relaxed)
        self.sent.store(count + 1, order: .Release)
    }

    fn receive(): i64 = {
        val count = self.read.load(order: .Relaxed)
        while self.sent.load(order: .Acquire) == count {}
        val value = self.slot.load(order: .Relaxed)
        self.read.store(count + 1, order: .Release)
        return value
    }
}

fn main() = {
    val mailbox = Mailbox()
    val (_, total) = future::both(
        () => {
            for i in 1..101 do
                mailbox.send(i)
        },
        () => {
            var total = 0
            for _ in 0..100 do
                total += mailbox.receive()

            return total
        },
    )
    assert(total == 5050)

    val seconds = Atomic::of(0.0)
    var frames = [0.25, 0.5, 0.25, 1.0]
    future::each(&var frames, (frame) => { seconds.add(frame) })
    assert(seconds.load() == 2.0)
    assert(!seconds.replaceIf(-2.0, 0.0) && seconds.replaceIf(2.0, 0.0))
}
```

A thread that must never wait — an audio callback, which a late answer
makes a click — is handed its work through a ring,
`sync::ring<T>(capacity)`: a fixed number of slots, allocated once, between
one thread that fills it and one that drains it, answered as the two ends,
a `Filler<T>` and a `Drainer<T>`. `push` puts a value in, or, where every
slot is full, hands it back as `Err(value)`; `pop` takes the oldest, or
answers nothing; and neither waits, locks or allocates, so a channel, which
does all three, is for what may wait. Neither end is copied or shared, and
both `push` and `pop` take their end `var`, so one thread at a time fills
and one drains; the values still in the ring end with it, once both ends
are gone:

```wip,run
import std::future
import std::sync

fn main() = {
    var (sounds, mixer) = sync::ring<String>(2)
    assert(sounds.push("coin").isOk() && sounds.push("door").isOk())
    assert(sounds.push("step") == .Err("step")) // full
    assert(mixer.pop() == String::of("coin") && mixer.len() == 1)

    var (samples, played) = sync::ring<i64>(8)
    val (_, total) = future::both(
        () => {
            for i in 1..1001 {
                while samples.push(i).isErr() {}
            }
        },
        () => {
            var total = 0
            var heard = 0
            while heard < 1000 {
                if played.pop() is .Some(sample) {
                    total += sample
                    heard += 1
                }
            }
            return total
        },
    )
    assert(total == 500500)
}
```

A slice lends two parts of itself at once, which do not overlap, with
`split(at, (first, second) => …)`: what `each` gives each thread.

`future::each` and `future::map` start a thread for each part of every
call, some tens of microseconds a call, which a game that splits its work
every frame pays every frame. A `future::Pool` starts its threads once and
keeps them: its `each` and `map` take and answer what the functions do,
under the same rules on what the closures borrow, and hand the parts to
threads that are already waiting, spinning a while before they sleep. Its
end tells its threads to stop, and waits for them:

```wip,run
import std::future
import std::future::{Pool}

struct Body {
    height: f64
    speed: f64
}

fn main() = {
    val pool = Pool::of(future::cores() - 1)
    var bodies: Vec<Body> = Vec()
    for i in 0..100 do
        bodies.push(Body(height: i as f64, speed: 0.0))

    for _ in 0..60 do
        pool.each(&var bodies.items(), (body) => {
            body.speed -= 9.8 / 60.0
            body.height += body.speed / 60.0
        })

    val heights = pool.map(&bodies.items(), (body) => body.height)
    assert(heights.len() == 100 && heights[99] < 99.0)
}
```
