# http-hello

A tiny web server that answers every request with a line of text, on
several threads.

```sh
wip run examples/http-hello/main.wip            # then: curl localhost:8080/hi
wip run examples/http-hello/main.wip -p 9000 -n 3
wip test examples/http-hello/main.wip
```

What it shows:

- **`std::net`:** a `Listener` that accepts connections, and a `Stream`
  read from and written to, each closed where it ends.
- **Threads that borrow:** `future::each` runs the workers, and lends
  them the listener and the count of requests for its call alone, so
  nothing is reference-counted. `accept` takes `&self`, so every worker
  accepts from the one listener.
- **An `Atomic`,** the count, which threads change through a `&` without
  a lock.
- **A text block with values in it** for the response, `\r` written where
  HTTP wants it.
- **A test that is a client:** `future::together` runs the server for one
  request beside a client that sends it, on a port the system chooses.
