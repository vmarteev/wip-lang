; The outline panel: declarations, and what they contain.

(function_declaration
  (visibility)? @context
  receiver: (receiver)? @context
  "fn" @context
  name: (_) @name) @item

(struct_declaration
  (visibility)? @context
  ["struct" "union"] @context
  name: (_) @name) @item

(enum_declaration
  (visibility)? @context
  "enum" @context
  name: (_) @name) @item

(interface_declaration
  (visibility)? @context
  "interface" @context
  name: (_) @name) @item

(type_alias
  (visibility)? @context
  "type" @context
  name: (_) @name) @item

(extern_type
  "type" @context
  name: (_) @name) @item

(val_declaration
  (visibility)? @context
  "val" @context
  name: (_) @name) @item

(extern_variable
  ["val" "var"] @context
  name: (_) @name) @item

(extend_block
  "extend" @context
  type: (_) @name) @item

(extern_block
  "extern" @context
  abi: (_) @name) @item

(field_declaration
  name: (_) @name) @item

(enum_variant
  name: (_) @name) @item
