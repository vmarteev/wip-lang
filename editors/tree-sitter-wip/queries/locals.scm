; Scopes and definitions, so an editor can tell a parameter from a global.

[
  (function_declaration)
  (block)
  (lambda_expression)
  (match_arm)
  (for_statement)
] @local.scope

(parameter
  name: (identifier) @local.definition.parameter)

(lambda_parameter
  (identifier) @local.definition.parameter)

(val_statement
  name: (identifier) @local.definition.var)

(identifier_pattern
  (identifier) @local.definition.var)

(binder
  field: (identifier) @local.definition.var)

(function_declaration
  name: (identifier) @local.definition.function)

(identifier) @local.reference
