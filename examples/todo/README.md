# todo

A to-do list on the command line, kept in a text file.

```sh
wip build examples/todo/main.wip -o todo
./todo add buy milk
./todo add write the report
./todo done 1
./todo list --all
./todo clear
wip test examples/todo/main.wip
```

The list is kept in `todo.txt` in the current directory, or in the file
`-f` or `$TODO_FILE` names: one item to a line, `[x] ` before one that is
done.

What it shows:

- **Subcommands with `std::args`:** the options before the command are
  read, the command is a positional argument, and the same `Arguments`
  then reads what that command takes — `list --all`, `done 2`, or, for
  `add`, the rest of the words as they are.
- **Structs and enums of the program's own,** an error type for each
  part, and `From` so that `?` turns one into the other.
- **A `match` on text,** for the command.
- **A module with no input or output,** `list.wip`, which the tests in
  `list.test.wip` use directly.
