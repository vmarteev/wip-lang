# report-formats

Writes a small table as plain text, CSV, Markdown or JSON lines, in the
format an option names.

```sh
wip run examples/report-formats/main.wip
wip run examples/report-formats/main.wip --format markdown
wip run examples/report-formats/main.wip -f csv
wip test examples/report-formats/main.wip
```

What it shows:

- **Dynamic dispatch with `&dyn`:** which format writes the table is
  decided while the program runs, by a `match` on the option. `render`
  takes a `&dyn Format`, so it is one function, compiled once, calling
  each format's methods through a table of them.
- **The same, generic:** `renderEach<F: Format>` is compiled once for each
  format it is called with, and calls each directly. The answers are the
  same; the cost is in code size and in the indirect call, and the choice
  is the program's.
- **An interface with a default method:** `begin` writes nothing unless a
  format says otherwise, and `JsonLines` keeps it.
- **A `&dyn` is a reference,** so it is lent to a call and not kept: each
  arm of the `match` lends `render` a value of another type, made where it
  is written.
- **Tables of text** as constants, and a struct with a default,
  `Plain(width: 6)`.
