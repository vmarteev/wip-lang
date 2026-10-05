# json-pretty

Reads a JSON document and writes it again, indented, with each object's
keys in order where asked.

```sh
echo '{"name": "wip", "tags": [1, 2.5, null]}' | wip run examples/json-pretty/main.wip
wip run examples/json-pretty/main.wip --sort-keys --indent=4 data.json
wip test examples/json-pretty/main.wip
```

What it shows:

- **`std::json`:** a document parsed into a `Json`, an enum, and a parse
  error that says where, by line and column.
- **Recursion over an enum** with `match`, each case writing into a
  `String` it is lent with `&var`.
- **A value's own `Text`:** a leaf, and a key, are written as `Json`
  writes them compact, escapes and all.
- **A struct with defaults,** `Layout()`, that options change field by
  field.
- **Text blocks** for the usage, and for what a test expects.
