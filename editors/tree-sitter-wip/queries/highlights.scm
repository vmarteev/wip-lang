; Wip's highlighting. Capture names follow Neovim's conventions, which Zed
; reads too, falling back to the part before a dot where a theme has no
; more specific colour. Later patterns win in Neovim, so the general ones
; come first.

; ---- Names ---------------------------------------------------------------

(identifier) @variable

(type_identifier) @type

; The built-in type names are identifiers, not keywords (grammar §1).
((type_identifier) @type.builtin
  (#match? @type.builtin "^(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64|bool|str|char|cstring|void|never|ptr|c_char|c_schar|c_uchar|c_short|c_ushort|c_int|c_uint|c_long|c_ulong|c_longlong|c_ulonglong|c_float|c_double|size_t|ssize_t|intptr_t|uintptr_t)$"))

; A name in capitals is a constant, as `TARGET_OS` or `O_CREAT` is.
((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))

(field_identifier) @property

(parameter
  name: (identifier) @variable.parameter)

(lambda_parameter
  (identifier) @variable.parameter)

(self) @variable.builtin

(label
  (identifier) @label)

(break_statement
  (identifier) @label)

(continue_statement
  (identifier) @label)

; ---- Modules and paths -----------------------------------------------------

(import_declaration
  path: (identifier) @module)

(import_declaration
  alias: (identifier) @module)

(import_item
  (identifier) @variable)

; `io::println`: what comes before `::` is a module or a type.
(scoped_identifier
  (identifier) @module
  "::")

(scoped_type_identifier
  (identifier) @module
  "::")

(package_declaration
  name: (identifier) @module)

; ---- Declarations -----------------------------------------------------------

(function_declaration
  name: (identifier) @function)

(enum_variant
  name: (identifier) @constructor)

(variant_expression
  name: (identifier) @constructor)

(variant_pattern
  name: (identifier) @constructor)

(val_declaration
  name: (identifier) @constant)

(extern_variable
  name: (identifier) @variable)

(named_argument
  name: (identifier) @variable.parameter)

; `width:`, `fill:` and `align:` in `\(value, width: 4)`.
(interpolation
  option: (identifier) @variable.parameter)

(binder
  field: (identifier) @property)

; ---- Calls ------------------------------------------------------------------

(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (scoped_identifier
    (identifier) @function.call .))

(call_expression
  function: (generic_expression
    name: (identifier) @function.call))

(call_expression
  function: (field_expression
    field: (field_identifier) @function.method.call))

; `Shape::Circle(…)` and `.Some(…)` build a variant.
(call_expression
  function: (variant_expression
    name: (identifier) @constructor))

((call_expression
  function: (scoped_identifier
    (identifier) @constructor .))
  (#match? @constructor "^[A-Z]"))

; `Row(name: "a")` and `Vec<i64>()` build a struct.
((call_expression
  function: (identifier) @constructor)
  (#match? @constructor "^[A-Z]"))

((call_expression
  function: (generic_expression
    name: (identifier) @constructor))
  (#match? @constructor "^[A-Z]"))

; ---- Annotations ------------------------------------------------------------

(annotation
  "@" @attribute
  name: (identifier) @attribute)

(annotation_argument
  name: (identifier) @variable.parameter)

; ---- Literals ---------------------------------------------------------------

(integer) @number

(float) @number.float

(boolean) @boolean

(null) @constant.builtin

(string) @string

(string_content) @string

(text_block_content) @string

(escape_sequence) @string.escape

(char) @character
(byte) @character

(interpolation
  "\\(" @punctuation.special
  ")" @punctuation.special)

(comment) @comment @spell

; ---- Keywords ---------------------------------------------------------------

[
  "val"
  "var"
  "type"
  "struct"
  "union"
  "enum"
  "interface"
  "extend"
  "extern"
  "view"
  "package"
  "static"
  "dyn"
] @keyword

(visibility) @keyword.modifier

(receiver) @keyword.modifier

"fn" @keyword.function

"from" @keyword
"keeps" @keyword

[
  "import"
] @keyword.import

[
  "if"
  "then"
  "else"
  "match"
] @keyword.conditional

[
  "while"
  "for"
  "in"
  "break"
  "continue"
] @keyword.repeat

[
  "return"
  "lend"
  "yield"
] @keyword.return

[
  "defer"
  "assert"
] @keyword

[
  "as"
  "is"
  "!is"
  "own"
  "move"
] @keyword.operator

; ---- Operators and punctuation ------------------------------------------------

[
  "="
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  "+"
  "-"
  "*"
  "/"
  "%"
  "+%"
  "-%"
  "*%"
  "!"
  "&"
  "&&"
  "|"
  "||"
  "^"
  "<<"
  ">>"
  "+="
  "-="
  "*="
  "/="
  "%="
  "&="
  "|="
  "^="
  "<<="
  ">>="
  "+%="
  "-%="
  "*%="
  "=>"
  "?"
  ".."
  "..="
] @operator

(variadic) @punctuation.special

(rest_pattern) @punctuation.special

(wildcard_pattern) @variable.builtin

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

(type_arguments
  [
    "<"
    ">"
  ] @punctuation.bracket)

(type_parameters
  [
    "<"
    ">"
  ] @punctuation.bracket)

(own_type
  [
    "<"
    ">"
  ] @punctuation.bracket)

[
  ","
  ";"
  ":"
  "::"
  "."
] @punctuation.delimiter
