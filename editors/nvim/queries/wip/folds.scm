; Copied from editors/tree-sitter-wip/queries by scripts/tree-sitter.sh; edit that.

; What folds: a body, a list of members, a long call or literal.

[
  (block)
  (declaration_list)
  (enum_variant_list)
  (extern_item_list)
  (match_block)
  (arguments)
  (parameters)
  (array_expression)
] @fold

(import_declaration) @fold
