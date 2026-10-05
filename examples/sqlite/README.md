# sqlite

A query against [SQLite](https://sqlite.org), a C library, called directly:
no bindings layer and no generated glue.

```sh
wip run examples/sqlite/main.wip
```

It needs SQLite installed: on macOS it is part of the system, and on
Debian or Ubuntu it is `apt install libsqlite3-dev`.

What it shows:

- `@link("sqlite3")` names the library, and an `extern "C"` block declares
  what is used of it, in C's types.
- `type sqlite3` is an opaque C type, only ever held through `ptr`, and a
  `&var ptr<sqlite3>` is C's `sqlite3 **` out-parameter.
- A string literal is a `cstring` where C wants one, and
  `cstring.toStr()` reads the text of a column.
- `defer` closes what was opened, however the function ends.

It prints:

```
ada is 36
alan is 41
grace is 45
total 122
```
