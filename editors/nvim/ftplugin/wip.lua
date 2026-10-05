-- Wip buffers: `wip fmt`'s layout, and the grammar in parser/wip.so, which
-- build.sh makes, for highlighting, folding and indenting.

vim.bo.commentstring = "// %s"
vim.bo.comments = ":///,://"
vim.bo.expandtab = false
vim.bo.tabstop = 4
vim.bo.shiftwidth = 4
vim.bo.textwidth = 100

-- :WipFmt lays the whole buffer out with `wip fmt`, as the file's package
-- says. What does not parse is left as it is, and why is shown.
vim.api.nvim_buf_create_user_command(0, "WipFmt", function()
  local buf = vim.api.nvim_get_current_buf()
  local text = table.concat(vim.api.nvim_buf_get_lines(buf, 0, -1, false), "\n") .. "\n"
  local cmd = { "wip", "fmt", "-", "--stdin-path", vim.api.nvim_buf_get_name(buf), "--color", "never" }
  local ok, done = pcall(function() return vim.system(cmd, { stdin = text }):wait() end)
  if not ok then
    vim.notify("wip fmt: " .. tostring(done), vim.log.levels.ERROR)
  elseif done.code ~= 0 then
    vim.notify(done.stderr, vim.log.levels.ERROR)
  elseif done.stdout ~= text then
    local view = vim.fn.winsaveview()
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, vim.split(done.stdout:gsub("\n$", ""), "\n"))
    vim.fn.winrestview(view)
  end
end, { desc = "Format the buffer with wip fmt" })

-- What this file set, undone when the buffer stops being Wip or is read
-- again. `:lua` takes the rest of its line, so it comes last.
local undo = { vim.b.undo_ftplugin, "setl commentstring< comments< expandtab< tabstop< shiftwidth< textwidth<" }
table.insert(undo, "silent! delcommand -buffer WipFmt")
vim.b.undo_ftplugin = table.concat(vim.tbl_filter(function(u) return u and u ~= "" end, undo), " | ")

-- The language server, `wip lsp`, where `wip` is on the PATH: what is
-- wrong as you type, fixes, formatting and an outline. One serves every
-- Wip buffer.
if vim.fn.executable("wip") == 1 then
  vim.lsp.start({
    name = "wip",
    cmd = { "wip", "lsp" },
    root_dir = vim.fs.root(0, { "package.wip", ".git" }) or vim.fn.expand("%:p:h"),
  })
end

if not pcall(vim.treesitter.start) then
  vim.notify_once("wip: no parser; run editors/nvim/build.sh", vim.log.levels.WARN)
  return
end

vim.wo[0][0].foldmethod = "expr"
vim.wo[0][0].foldexpr = "v:lua.vim.treesitter.foldexpr()"
vim.wo[0][0].foldlevel = 99

-- Neovim itself does not indent with a grammar; nvim-treesitter does.
local ok, ts = pcall(require, "nvim-treesitter")
if ok and ts.indentexpr then
  vim.bo.indentexpr = "v:lua.require'nvim-treesitter'.indentexpr()"
end

vim.b.undo_ftplugin = vim.b.undo_ftplugin
  .. " | setl indentexpr< | setl foldmethod< foldexpr< foldlevel< | lua vim.treesitter.stop()"
