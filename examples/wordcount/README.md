# wordcount

Counts the words of files, or of standard input, and prints the ones used
most.

```sh
wip run examples/wordcount/main.wip -n 5 README.md
wip run examples/wordcount/main.wip -i --top=3 - < README.md
wip test examples/wordcount/main.wip
```

What it shows:

- **Options read with `std::args`:** `-n 5`, `-n5`, `--top 5` and
  `--top=5` all work, `-in5` is a cluster, and a mistake such as `-x` or
  `-n ten` ends the program with a message saying so.
- **An error of the program's own.** `main` answers `Result<i64,
  Failure>`, which is printed after `error: ` with exit code 1, and
  `From<ArgumentError>` lets `?` turn an option's error into one.
- **A generator,** `words`, which hands out each word of a text as a
  `str` of it, as it is asked for, allocating nothing.
- **A `Map` and sorting by a closure,** and `\(count, width: 7)` lining
  the numbers up.
- **Tests beside the code** they test, in `words.test.wip`, which only
  `wip test` compiles.
