; Copied from editors/tree-sitter-wip/queries by scripts/tree-sitter.sh; edit that.

; Neovim's indentation: a line inside brackets is one level in, and the
; closing bracket comes back out.

[
  (block)
  (declaration_list)
  (enum_variant_list)
  (extern_item_list)
  (match_block)
  (arguments)
  (parameters)
  (array_expression)
  (tuple_expression)
  (parenthesized_expression)
] @indent.begin

[
  "}"
  ")"
  "]"
] @indent.branch @indent.end

(comment) @indent.auto

(string) @indent.ignore
