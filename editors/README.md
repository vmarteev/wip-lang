# Editors

Highlighting, folding, indenting and an outline for `.wip` files in Zed and
Neovim. Both are built on one grammar, [tree-sitter-wip](tree-sitter-wip),
and read the same queries, so they colour a program the same way.

Both also start Wip's language server, `wip lsp`. It shows
what `wip check` would as you type, in unsaved files too, offers a
diagnostic's fix where it has one, formats, gives an outline, shows what a
name is on hover, goes to its definition, completes fields, methods, a
module's items and the names in scope, finds references, renames, and
shows a call's signature as you type its arguments.

## The `wip` command

Both editors start the server as `wip lsp`, so `wip` must be on your
`PATH`. From the repository, with a `clang` 15 or newer:

```sh
scripts/bootstrap.sh --out ~/wip
export PATH="$HOME/wip/bin:$PATH"
wip --version
```

This builds the compiler into `~/wip`, `bin/wip` beside the standard
library it reads, which is the repository's own: a change to the library
is seen at once. After the compiler changes, build it again from its
source, so the editors serve the new one:

```sh
scripts/build.sh --release --out ~/wip
```

## Neovim

This needs Neovim 0.10 or later, the `tree-sitter` command and a C
compiler, for the grammar the plugin compiles.

### With LazyVim, or lazy.nvim

Add a plugin file, `~/.config/nvim/lua/plugins/wip.lua`:

```lua
return {
  {
    dir = "/path/to/wip-lang/editors/nvim",
    name = "wip",
    build = "./build.sh",
  },
}
```

On the first start, lazy.nvim runs `build.sh`, which compiles the grammar
to `editors/nvim/parser/wip.so`. After the grammar changes, run
`:Lazy build wip`.

### Without a plugin manager

Compile the grammar, then put the plugin on the runtime path in
`init.lua`:

```sh
editors/nvim/build.sh
```

```lua
vim.opt.rtp:prepend("/path/to/wip-lang/editors/nvim")
```

### What a `.wip` buffer gets

- Highlighting and folding from the grammar, tabs four wide, and `// `
  comments. With nvim-treesitter, which LazyVim includes, indentation from
  the grammar too.
- The language server, started for every Wip buffer. In LazyVim: hover
  `K`, definition `gd`, references `gr`, rename `<leader>cr`, fixes
  `<leader>ca`, and completion and signature help as you type.
- Formatting on save through the server, where the configuration formats
  through a language server, as LazyVim does; `vim.lsp.buf.format()`
  formats at any time. `:WipFmt` formats with `wip fmt` itself, with or
  without the server, and leaves a buffer that does not parse as it is.

`:LspInfo` lists the `wip` client when the server runs, and `:InspectTree`
shows the grammar's tree.

## Zed

In Zed, run **zed: install dev extension** from the command palette and
choose `editors/zed` in a clone of this repository. Zed fetches the
grammar from GitHub, at the commit `editors/zed/extension.toml` names,
and builds it and the extension's Rust part itself, which needs Rust
installed with `rustup`. It finds `wip` on your shell's `PATH`.

The extension sets up `wip fmt`'s layout: tabs, four columns wide, and a
guide at 100 columns. **editor: format** lays a file out through the
language server. Zed formats on save only for the languages it names in
its own settings, so for Wip say so in `~/.config/zed/settings.json`:

```json
"languages": {
    "Wip": {
        "format_on_save": "on",
        "formatter": "language_server"
    }
}
```

Zed does not format what it saves by itself when `autosave` is set to
save after a delay.

Where something goes wrong, **zed: open log** says what.

While changing the grammar, `editors/zed/dev.sh` makes a copy of the
extension that builds it from this checkout instead, at the last commit
that changed it, in `target/zed-wip`: install that directory instead, and
after committing a change to the grammar, run it again and reinstall.

## Changing the grammar

Edit `tree-sitter-wip/grammar.js` or its queries, never the copies in
`zed/` or `nvim/`, then run:

```sh
scripts/tree-sitter.sh
```

It regenerates the parser, runs the grammar's tests, parses every `.wip`
file in the repository, and copies the queries to both editors. The gate,
`scripts/verify.sh`, checks the same where `tree-sitter` is installed. Commit the result, then set `rev` in
`zed/extension.toml` to that commit.
