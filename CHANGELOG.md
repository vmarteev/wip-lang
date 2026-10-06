# Changelog

Each version published, newest first.

## 0.1.1 — 2026-10-05

- **The language server survives a line half typed.** It died at
  `String::of` typed without its call, and an editor went on showing the
  diagnostics of the keystroke before. Four panics of the checker are
  fixed, three of which a finished program could reach too:
  - `Type::name` not called is refused, as a method named as a value is
    (E0337), with the call to write and a lambda that stands in for it;
  - an enum's name interpolated, `"\(Json)"`, is a type used as a value;
  - a lambda in a `@comptime` constant runs:
    `@comptime val SQUARES = table((i) => i * i)`;
  - a module whose last line is half written is reported at its end.
- **`std::future`'s channels** hold their queue as a `Shared`, and have
  tests of their own.
- **CI passes on Rust 1.99,** whose clippy found two lints.
- **The scripts start with their interpreter,** and `scripts/snapshots.sh`
  runs on Linux.

## 0.1.0 — 2026-10-05

The first public version: the prototype compiler for Wip, its standard
library, `wip fmt`, `wip doc`, `wip test`, `wip bindgen`, the language server
`wip lsp`, and support for Zed and Neovim. Apple arm64, and Linux on arm64
and x86-64.
