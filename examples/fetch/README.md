# fetch

Fetches a URL and prints its body, or writes it to a file as it comes:

```sh
wip run examples/fetch/main.wip https://example.com/
wip run examples/fetch/main.wip -i https://example.com/
wip run examples/fetch/main.wip -o page.html https://example.com/
```

It shows `std::http`: a `Client` with a timeout, `open`, whose answer's
body is an `io::Read` that comes as it is read — undone where the server
sent it gzipped — and `io::copy`, which writes it to any `io::Write`, a
file or standard output, so that a body larger than memory is never held
whole. `https` is the system's TLS, which checks the server's certificate.

Its tests, `wip test examples/fetch/main.wip`, fetch from a server they
start themselves, and need no network.
