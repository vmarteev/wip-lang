# Wip — the language

This is what the language is today, by topic. It is the place to look when
the question is "what does Wip do here"; the other documents answer other
questions:

| Document | Answers |
|---|---|
| this reference | what the language is now |
| [`../grammar.md`](../grammar.md) | what the parser accepts, exactly |
| [the README](../../README.md#where-it-stands) | what it cannot do yet |

Each page states the rule and shows it in an example. Only the rule that
holds today is written down, not how it came to be.

## The pages

| Page | What is in it |
|---|---|
| [1. Values and types](01-types.md) | numbers, `bool`, `char`, text, arrays and slices, tuples, structs, enums, aliases, generics |
| [2. Files, modules and names](02-modules.md) | what a file is, how modules are found and imported, what `pub` means, packages and `package.wip`, the prelude |
| [3. Functions](03-functions.md) | declaring, calling, defaults and named arguments, methods and their receivers, closures |
| [4. Expressions and statements](04-expressions.md) | literals, operators, blocks, `if`, `match`, `is`, loops, interpolation, casts |
| [5. Patterns](05-patterns.md) | what a pattern matches, nesting, exhaustiveness, guards, `val … else` |
| [6. Ownership](06-ownership.md) | moves and copies, `own`, drops, `defer` |
| [7. References and lending](07-references.md) | `&` and `&var`, second-class references, projections, `lend fn`, views |
| [8. Interfaces](08-interfaces.md) | `interface`, `extend`, `@derive`, constraints, `&dyn`, conditional implementations |
| [9. Failure](09-errors.md) | `Result` and `?`, `panic`, `assert`, `never` |
| [10. The standard library](10-library.md) | the prelude, `Vec`, `String`, collections, iterators, `std::io` |
| [11. C](11-c.md) | `extern` blocks, `@link` and `@header`, C files in a module and their settings, `cstring` |
| [12. The tools](12-tools.md) | `wip build`, `run`, `test`, `check`, `mir`, and what they read from disk |

## The examples are compiled

Every Wip example on these pages — and in the repository's
[README](../../README.md) — is a program the compiler is run against by the
repository's gate, `tests/gate`, so an example cannot quietly stop being
true. The fence says what is expected:

- **`wip`** — it compiles. A block that declares no items is read as the
  body of a `main`, so an example can be a few statements.
- **`wip,run`** — it compiles and runs to a successful end. Such an example
  says what it claims with `assert`, and running it is what checks the
  claim.
- **`wip,error=E0303`** — it is refused, with that code. This is how a page
  shows what the language does not allow.
- **`wip,ignore`** — a fragment that stands for something rather than a
  program.

Each example is also laid out as `wip fmt` lays it out, four spaces to an
indent and in 80 columns, since what a page shows is what a reader copies.

## Keeping it true

A change to the language changes this reference in the same commit, and
`../grammar.md` where its syntax changes. The reference is not a history:
the page says what holds now.
