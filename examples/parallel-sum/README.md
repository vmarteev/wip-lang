# parallel-sum

Counts the primes below a limit on one thread, then on several in three
ways, and says how long each took.

```sh
wip run --release examples/parallel-sum/main.wip
wip run --release examples/parallel-sum/main.wip -n 20000000 -p 32
wip test examples/parallel-sum/main.wip
```

What it shows:

- **`future::map`:** each part on a thread of its own, their answers in
  a `Vec`, in order.
- **`future::together`:** two pieces of work at once, both answers back.
- **`future::each` with an `Atomic`:** every thread adds to one total,
  through a `&`, without a lock.
- **Threads that borrow.** Each call waits for its threads before it
  answers, so the closures may borrow what is around them — the ranges,
  the total — and the checker treats their captures as a call's
  arguments: what one writes, no other reaches.
- **A closure lent to a function,** `timed`, and `Instant` from
  `std::time`, which prints its `Duration` in the largest unit it fills.
