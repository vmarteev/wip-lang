# Examples

Small, complete programs, each a directory with a `main.wip` and a
`README.md` saying what it shows and how to run it. Each is checked, and
its tests run, with every change to the compiler.

| Example | What it shows |
|---|---|
| [hello](hello) | the smallest program: `main`, and `std::io` |
| [wordcount](wordcount) | options with `std::args`, reading files, a generator, a `Map`, an error type of the program's own |
| [json-pretty](json-pretty) | `std::json`, recursion over an enum with `match`, text blocks |
| [todo](todo) | subcommands, a list kept in a text file, structs, enums and tests of a module |
| [life](life) | Conway's Life: a lending `at`, a generator, a constant worked out while compiling, terminal frames |
| [report-formats](report-formats) | dynamic dispatch with `&dyn`, beside the same code generic, and an interface's default method |
| [http-hello](http-hello) | a tiny web server: `std::net`, threads that borrow with `future::each`, an `Atomic` |
| [parallel-sum](parallel-sum) | work on several threads three ways: `future::map`, `together` and `each` |
| [shapes-package](shapes-package) | a program that is a package, depending on a library package, with `internal/` |
| [sqlite](sqlite) | calling a C library directly: `@link`, `extern "C"`, opaque types, `defer` |

Run one with `wip run examples/<name>/main.wip`, and its tests with
`wip test examples/<name>/main.wip`.
