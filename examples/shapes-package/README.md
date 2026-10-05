# shapes-package

A program that is a package, depending on a library package, `geometry`,
which lives in `libs/` beside it.

```sh
wip run examples/shapes-package/main.wip
wip test examples/shapes-package/libs/geometry   # the library's own tests
```

What it shows:

- **`package.wip`:** a name, a version, and `@depends("geometry", path =
  "libs/geometry")`, which says where the dependency's root is. There is no
  build file and nothing is fetched: the directories are the build.
- **A dependency's name begins a path,** as `std` does: `import
  geometry::{Shape, largest}`.
- **`internal/`:** `libs/geometry/internal/measure` is `geometry`'s own. Its
  modules import it; the program cannot.
- **`package::NAME` and `package::VERSION`,** constants of the package
  the code is written in, so `geometry::version()` answers `0.2.0`.
- **`@derive(Eq, Text)`** on an enum, which compares and prints it field
  by field, and a lending function, `largest`, that answers a shape in
  the slice it was given.
