# life

Conway's Game of Life, drawn in the terminal a generation at a time.

```sh
wip run examples/life/main.wip
wip run examples/life/main.wip -g 30 -d 150 pulsar
wip test examples/life/main.wip
```

What it shows:

- **A lending pair from one body:** `lend fn at(x, y): &bool` reads a
  cell, `board.at(x, y)`, and writes one, `board.at(x, y) = true`, the
  edges wrapping around either way.
- **A constant worked out while compiling:** `@comptime val NEIGHBOURS`
  is what `around()` answers, run by the compiler and kept as a table.
- **A generator:** `alive()` is a loop that yields the live cells, one at
  a time as they are asked for; `count()` and `toVec()` walk it.
- **Tables of text:** each pattern is an array of rows, chosen by a
  `match` on the pattern's name.
- **`Text` written for a type,** which is what `"\(board)"` and the tests
  use, and **`std::time`'s `sleep`** between frames.
