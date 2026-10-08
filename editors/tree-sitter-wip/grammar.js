/**
 * Tree-sitter grammar for Wip, after docs/grammar.md.
 *
 * Line breaks end statements, fields, variants and arms where the language
 * says they do: the external scanner in src/scanner.c emits
 * a `_terminator` at a line break wherever the grammar could end one there,
 * and never before `else`, which continues the line above. Inside `( )` and
 * `[ ]` nothing can end there, so a line break is only space.
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const PREC = {
  assign: 1,
  lend: 2,
  or: 3,
  and: 4,
  equality: 5,
  comparison: 6,
  bit_or: 7,
  bit_xor: 8,
  bit_and: 9,
  shift: 10,
  additive: 11,
  multiplicative: 12,
  cast: 13,
  unary: 14,
  postfix: 15,
  path: 16,
};

/** `rule`, separated by commas, with a trailing comma allowed. */
function commaSep(rule) {
  return optional(seq(rule, repeat(seq(',', rule)), optional(',')));
}

function commaSep1(rule) {
  return seq(rule, repeat(seq(',', rule)), optional(','));
}

module.exports = grammar({
  name: 'wip',

  // `_error_sentinel` is never used by a rule: it is valid only while the
  // parser recovers from an error, which is how the scanner tells.
  // `text_block_content` is a text block's text up to a `\\` or its
  // closing `"""`, which a rule cannot say.
  externals: $ => [$._terminator, $._error_sentinel, $.text_block_content],

  extras: $ => [/\s/, $.comment],

  word: $ => $.identifier,

  supertypes: $ => [$._expression, $._type, $._pattern, $._statement],

  conflicts: $ => [
    // `f<i64>(x)`, or `i < n` (R7).
    [$._expression, $.generic_expression],
    // `(x) => …` or `(x)`: a lambda's parameter, or an expression.
    [$._expression, $.lambda_parameter],
    // `a::b<T>` or `a::b < x` (R7).
    [$.scoped_identifier, $.generic_segment],
    // `Some(x)`: a name bound, or a variant taken apart; the checker knows.
    [$.identifier_pattern, $.variant_pattern],
    [$.identifier_pattern, $.binder],
    [$.variant_pattern],
  ],

  rules: {
    source_file: $ => seq(
      optional($._separators),
      repeat(seq($._item, optional($._separators))),
    ),

    _separators: $ => repeat1(choice(';', $._terminator)),

    // ---- Items -------------------------------------------------------

    _item: $ => choice(
      $.package_declaration,
      $.import_declaration,
      $.function_declaration,
      $.struct_declaration,
      $.enum_declaration,
      $.interface_declaration,
      $.extend_block,
      $.extern_block,
      $.val_declaration,
      $.type_alias,
      $.assert_item,
    ),

    // An assert at the top level, checked while the program is compiled.
    assert_item: $ => seq(
      repeat($.annotation),
      $.assert_expression,
    ),

    annotation: $ => seq(
      '@',
      field('name', $.identifier),
      optional(seq('(', commaSep($.annotation_argument), ')')),
    ),

    annotation_argument: $ => seq(
      optional(seq(field('name', $.identifier), '=')),
      field('value', choice($.integer, $.string, $.boolean, $.identifier)),
    ),

    visibility: _ => 'pub',

    // `package game` in a package.wip.
    package_declaration: $ => seq(
      repeat($.annotation),
      'package',
      field('name', $.identifier),
    ),

    // The path is written out rather than a rule of its own, so that the
    // token after a `::` decides: a name continues it, `{` names items.
    import_declaration: $ => seq(
      'import',
      field('path', $.identifier),
      repeat(seq('::', field('path', $.identifier))),
      optional(choice(
        seq('as', field('alias', $.identifier)),
        seq('::', '{', commaSep($.import_item), '}'),
      )),
    ),

    import_item: $ => seq(
      choice($.identifier, $.self),
      optional(seq('as', field('alias', $.identifier))),
    ),

    val_declaration: $ => seq(
      repeat($.annotation),
      optional($.visibility),
      'val',
      field('name', $.identifier),
      optional(seq(':', field('type', $._type))),
      '=',
      field('value', $._expression),
    ),

    type_alias: $ => seq(
      repeat($.annotation),
      optional($.visibility),
      'type',
      field('name', $._type_identifier),
      optional(field('type_parameters', $.type_parameters)),
      '=',
      field('type', $._type),
    ),

    function_declaration: $ => seq(
      repeat($.annotation),
      optional($.visibility),
      optional(field('receiver', $.receiver)),
      'fn',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      field('parameters', $.parameters),
      optional(seq(
        ':',
        field('return_type', $._type),
        optional(field('lends_from', $.lends_from)),
      )),
      optional(seq('=', field('body', $._expression))),
    ),

    // What a result borrows: `from a, self.ast`. A word only here.
    lends_from: $ => prec.right(seq('from', commaSep1($.lend_path))),

    lend_path: $ => seq(
      choice($.self, $.identifier),
      repeat(seq('.', field('field', $.identifier))),
    ),

    receiver: _ => choice('var', 'move', 'static', 'lend'),

    parameters: $ => seq('(', commaSep(choice($.parameter, $.variadic)), ')'),

    variadic: _ => '...',

    parameter: $ => seq(
      optional('var'),
      field('name', $.identifier),
      ':',
      field('type', $._type),
      optional(seq('=', field('default', $._expression))),
    ),

    type_parameters: $ => seq('<', commaSep1($.type_parameter), '>'),

    // `T: Ord + Hash`, and `K = T`, a default naming the parameters
    // before it; `type Item`, an interface's, which each implementation
    // decides.
    type_parameter: $ => seq(
      optional('type'),
      field('name', $._type_identifier),
      optional(seq(':', $.constraint, repeat(seq('+', $.constraint)))),
      optional(seq('=', field('default', $._type))),
    ),

    constraint: $ => seq($._type_identifier, optional($.type_arguments)),

    struct_declaration: $ => seq(
      repeat($.annotation),
      optional($.visibility),
      optional(choice('extern', 'view')),
      choice('struct', 'union'),
      field('name', $._type_identifier),
      optional(field('type_parameters', $.type_parameters)),
      // A struct with no fields is its name alone: `struct Csv`.
      optional(field('body', $.declaration_list)),
    ),

    // `view enum`: its variants may borrow.
    enum_declaration: $ => seq(
      repeat($.annotation),
      optional($.visibility),
      optional('view'),
      'enum',
      field('name', $._type_identifier),
      optional(field('type_parameters', $.type_parameters)),
      field('body', $.enum_variant_list),
    ),

    /// `{ … }` of fields and methods, separated by line breaks or commas.
    declaration_list: $ => seq(
      '{',
      optional($._list_separators),
      repeat(seq(
        choice($.field_declaration, $.function_declaration),
        optional($._list_separators),
      )),
      '}',
    ),

    enum_variant_list: $ => seq(
      '{',
      optional($._list_separators),
      repeat(seq(
        choice($.enum_variant, $.function_declaration),
        optional($._list_separators),
      )),
      '}',
    ),

    _list_separators: $ => repeat1(choice(',', ';', $._terminator)),

    field_declaration: $ => seq(
      optional(seq($.visibility, optional('var'))),
      field('name', $._field_identifier),
      ':',
      field('type', $._type),
      optional(seq('=', field('default', $._expression))),
    ),

    enum_variant: $ => seq(
      field('name', $.identifier),
      optional(field('fields', $.parameters)),
    ),

    interface_declaration: $ => seq(
      repeat($.annotation),
      optional($.visibility),
      'interface',
      field('name', $._type_identifier),
      optional(field('type_parameters', $.type_parameters)),
      field('body', $.declaration_list),
    ),

    // `extend Type: Interface { … }`, `extend [T: Ord] { … }`.
    extend_block: $ => seq(
      repeat($.annotation),
      'extend',
      field('type', choice(
        seq($.scoped_type_identifier, optional($.type_parameters)),
        seq($._type_identifier, optional($.type_parameters)),
        seq('[', $.type_parameter, ']'),
      )),
      optional(seq(':', field('interface', $._type_identifier), optional($.type_arguments))),
      field('body', $.declaration_list),
    ),

    extern_block: $ => seq(
      repeat($.annotation),
      'extern',
      field('abi', $.string),
      field('body', $.extern_item_list),
    ),

    extern_item_list: $ => seq(
      '{',
      optional($._list_separators),
      repeat(seq(
        choice(
          $.function_declaration,
          $.extern_type,
          $.extern_variable,
          $.struct_declaration,
        ),
        optional($._list_separators),
      )),
      '}',
    ),

    extern_type: $ => seq(
      repeat($.annotation),
      optional($.visibility),
      'type',
      field('name', $._type_identifier),
    ),

    extern_variable: $ => seq(
      repeat($.annotation),
      optional($.visibility),
      choice('val', 'var'),
      field('name', $.identifier),
      ':',
      field('type', $._type),
    ),

    // ---- Types -------------------------------------------------------

    _type: $ => choice(
      $._type_identifier,
      $.scoped_type_identifier,
      $.generic_type,
      $.reference_type,
      $.own_type,
      $.array_type,
      $.slice_type,
      $.dyn_type,
      $.function_type,
      $.tuple_type,
    ),

    scoped_type_identifier: $ => seq(
      $.identifier,
      repeat1(seq('::', $.identifier)),
    ),

    generic_type: $ => prec(1, seq(
      field('type', choice($._type_identifier, $.scoped_type_identifier)),
      field('type_arguments', $.type_arguments),
    )),

    type_arguments: $ => seq('<', commaSep1($._type), '>'),

    reference_type: $ => prec.right(seq('&', optional('var'), field('type', $._type))),

    own_type: $ => seq('own', '<', $._type, '>'),

    array_type: $ => seq('[', field('element', $._type), ';', field('length', choice($.integer, $.identifier)), ']'),

    slice_type: $ => seq('[', field('element', $._type), ']'),

    dyn_type: $ => prec.right(seq('dyn', $._type_identifier, optional($.type_arguments))),

    function_type: $ => prec.right(seq(
      '(',
      commaSep($.parameter),
      ')',
      '=>',
      field('return_type', $._type),
    )),

    tuple_type: $ => seq('(', $._type, repeat1(seq(',', $._type)), optional(','), ')'),

    // ---- Statements --------------------------------------------------

    block: $ => seq(
      '{',
      optional($._separators),
      repeat(seq($._statement, optional($._separators))),
      '}',
    ),

    _statement: $ => choice(
      $.val_statement,
      $.guard_statement,
      $.defer_statement,
      $.return_statement,
      $.yield_statement,
      $.while_statement,
      $.for_statement,
      $.break_statement,
      $.continue_statement,
      $.expression_statement,
    ),

    val_statement: $ => seq(
      choice('val', 'var'),
      field('name', $.identifier),
      optional(seq(':', field('type', $._type))),
      '=',
      field('value', $._expression),
    ),

    // A pattern after `val` or `var` starts with `.` or a path, or is a
    // tuple or a slice, or is alternatives: a bare name is a `val`
    // (grammar §2). One that can fail has an `else`.
    guard_statement: $ => prec(1, seq(
      choice('val', 'var'),
      field('pattern', choice($.tuple_pattern, $.variant_pattern, $.slice_pattern, $.or_pattern)),
      '=',
      field('value', $._expression),
      optional(seq('else', field('else', $.block))),
    )),

    defer_statement: $ => seq('defer', $._expression),

    return_statement: $ => prec.right(seq('return', optional($._expression))),

    // `yield value`: to the list being built, or to
    // whoever asks a generator.
    yield_statement: $ => seq('yield', $._expression),

    label: $ => seq($.identifier, ':'),

    while_statement: $ => seq(
      optional($.label),
      'while',
      field('condition', $._expression),
      field('body', $.block),
    ),

    // Where a statement stands, a `for` is one; where a value stands, it
    // is a generator.
    for_statement: $ => prec(1, seq(
      optional($.label),
      'for',
      field('pattern', $._pattern),
      'in',
      field('value', $._expression),
      optional(seq(choice('..', '..='), field('end', $._expression))),
      field('body', $.block),
    )),

    break_statement: $ => prec.right(seq('break', optional($.identifier))),

    continue_statement: $ => prec.right(seq('continue', optional($.identifier))),

    expression_statement: $ => $._expression,

    // ---- Expressions -------------------------------------------------

    _expression: $ => choice(
      $.identifier,
      $.self,
      $.integer,
      $.float,
      $.string,
      $.char,
      $.byte,
      $.boolean,
      $.null,
      $.scoped_identifier,
      $.generic_expression,
      $.variant_expression,
      $.array_expression,
      $.tuple_expression,
      $.parenthesized_expression,
      $.lambda_expression,
      $.call_expression,
      $.field_expression,
      $.index_expression,
      $.try_expression,
      $.unary_expression,
      $.binary_expression,
      $.cast_expression,
      $.is_expression,
      $.assignment_expression,
      $.compound_assignment_expression,
      $.lend_expression,
      $.if_expression,
      $.match_expression,
      $.assert_expression,
      $.block,
      // A loop where a value stands: a generator.
      $.for_element,
    ),

    scoped_identifier: $ => prec(PREC.path, seq(
      $.identifier,
      repeat1(seq('::', choice($.identifier, $.generic_segment))),
    )),

    generic_segment: $ => seq($.identifier, $.type_arguments),

    // `f<i64>`, `Vec<i64>::new` (grammar.md R7).
    generic_expression: $ => prec.dynamic(-1, seq(
      field('name', $.identifier),
      field('type_arguments', $.type_arguments),
      repeat(seq('::', $.identifier)),
    )),

    // `.Some`, the expected enum's variant.
    variant_expression: $ => seq('.', field('name', $.identifier)),

    array_expression: $ => seq(
      '[',
      optional(choice(
        seq($._expression, ';', field('count', $._expression)),
        // A `for` among them is a loop that yields.
        commaSep1($._expression),
      )),
      ']',
    ),

    // `for x in xs { … }` where a value stands: among a list literal's
    // elements, or a generator.
    for_element: $ => seq(
      'for',
      field('pattern', $._pattern),
      'in',
      field('value', $._expression),
      optional(seq(choice('..', '..='), field('end', $._expression))),
      field('body', $.block),
    ),

    tuple_expression: $ => seq('(', $._expression, repeat1(seq(',', $._expression)), optional(','), ')'),

    parenthesized_expression: $ => seq('(', $._expression, ')'),

    lambda_expression: $ => prec.right(PREC.assign, seq(
      '(',
      commaSep($.lambda_parameter),
      ')',
      optional(seq(':', field('return_type', $._type))),
      '=>',
      field('body', $._expression),
    )),

    lambda_parameter: $ => seq($.identifier, optional(seq(':', $._type))),

    call_expression: $ => prec(PREC.postfix, seq(
      field('function', $._expression),
      field('arguments', $.arguments),
    )),

    // A call whose callee names a struct builds it, `..base` last giving
    // the fields it does not name.
    arguments: $ => seq(
      '(',
      commaSep(choice($._expression, $.named_argument, $.rest_argument)),
      ')',
    ),

    rest_argument: $ => seq('..', $._expression),

    named_argument: $ => seq(field('name', $.identifier), ':', field('value', $._expression)),

    field_expression: $ => prec(PREC.postfix, seq(
      field('value', $._expression),
      '.',
      field('field', choice($._field_identifier, $.integer)),
    )),

    index_expression: $ => prec(PREC.postfix, seq(
      $._expression,
      '[',
      choice(
        $._expression,
        seq(optional($._expression), '..', optional($._expression)),
        // `xs[lo..=hi]` takes `hi` too, so it needs one.
        seq(optional($._expression), '..=', $._expression),
      ),
      ']',
    )),

    try_expression: $ => prec(PREC.postfix, seq($._expression, '?')),

    // `own for x in xs { … }` is `own` before a loop that yields: a list
    // of what it yields.
    unary_expression: $ => prec(PREC.unary, seq(
      field('operator', choice('-', '!', seq('&', optional('var')), 'move', 'own')),
      field('operand', $._expression),
    )),

    binary_expression: $ => {
      const table = [
        [PREC.or, '||'],
        [PREC.and, '&&'],
        [PREC.equality, choice('==', '!=')],
        [PREC.comparison, choice('<', '<=', '>', '>=')],
        [PREC.bit_or, '|'],
        [PREC.bit_xor, '^'],
        [PREC.bit_and, '&'],
        // One token here, though Wip's lexer has none: Tree-sitter lexes
        // by what may come next, so `own<own<i64>>` still ends in two `>`.
        [PREC.shift, choice('<<', '>>')],
        [PREC.additive, choice('+', '-', '+%', '-%')],
        [PREC.multiplicative, choice('*', '/', '%', '*%')],
      ];
      return choice(...table.map(([p, op]) => prec.left(p, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._expression),
      ))));
    },

    cast_expression: $ => prec.left(PREC.cast, seq(
      field('value', $._expression),
      'as',
      field('type', $._type),
    )),

    // `value is pattern`, and `value !is pattern`, written together.
    is_expression: $ => prec.left(PREC.comparison, seq(
      field('value', $._expression),
      choice('is', '!is'),
      field('pattern', $._pattern),
    )),

    assignment_expression: $ => prec.right(PREC.assign, seq(
      field('left', $._expression),
      '=',
      field('right', $._expression),
    )),

    compound_assignment_expression: $ => prec.right(PREC.assign, seq(
      field('left', $._expression),
      field('operator', choice(
        '+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '<<=', '>>=',
        '+%=', '-%=', '*%=',
      )),
      field('right', $._expression),
    )),

    lend_expression: $ => prec.right(PREC.lend, seq('lend', $._expression)),

    // `if c { … } else { … }`, or `if c then a else b`, whose branches
    // are one expression or a jump each.
    if_expression: $ => prec.right(choice(
      seq(
        'if',
        field('condition', $._expression),
        field('consequence', $.block),
        optional(seq('else', field('alternative', choice($.block, $.if_expression)))),
      ),
      seq(
        'if',
        field('condition', $._expression),
        'then',
        field('consequence', $._then_branch),
        optional(seq('else', field('alternative', $._then_branch))),
      ),
    )),

    _then_branch: $ => choice(
      $._expression,
      $.return_statement,
      $.break_statement,
      $.continue_statement,
    ),

    match_expression: $ => seq(
      'match',
      field('value', $._expression),
      field('body', $.match_block),
    ),

    match_block: $ => seq(
      '{',
      optional($._list_separators),
      repeat(seq($.match_arm, optional($._list_separators))),
      '}',
    ),

    // An arm's value may be a jump, which ends the function or the loop.
    match_arm: $ => prec.right(seq(
      field('pattern', $._pattern),
      optional(seq('if', field('guard', $._expression))),
      '=>',
      field('value', choice(
        $._expression,
        $.return_statement,
        $.break_statement,
        $.continue_statement,
      )),
    )),

    assert_expression: $ => seq(
      'assert',
      '(',
      $._expression,
      optional(seq(',', $._expression)),
      optional(','),
      ')',
    ),

    // ---- Patterns ----------------------------------------------------

    _pattern: $ => choice(
      $.wildcard_pattern,
      $.identifier_pattern,
      $.literal_pattern,
      $.range_pattern,
      $.tuple_pattern,
      $.slice_pattern,
      $.variant_pattern,
      $.or_pattern,
    ),

    wildcard_pattern: _ => '_',

    identifier_pattern: $ => $.identifier,

    literal_pattern: $ => choice(
      seq(optional('-'), $.integer),
      $.boolean,
      $.string,
      $.char,
      $.byte,
    ),

    // `'a'..='z'`, `0..10`, `..0`, `10..`: the values between two ends,
    // each a literal or a constant. What can end one after
    // `..` does, as the compiler's parser reads it.
    range_pattern: $ => prec.right(choice(
      seq(
        field('start', $._range_bound),
        choice('..', '..='),
        optional(field('end', $._range_bound)),
      ),
      seq(choice('..', '..='), field('end', $._range_bound)),
    )),

    // A literal or a name before `..` is a range's start, as the
    // compiler's parser reads it, not a pattern followed by a range
    // expression.
    _range_bound: $ => choice(
      prec(1, seq(optional('-'), $.integer)),
      prec(1, $.char),
      prec(1, $.byte),
      prec(1, $.identifier),
      prec(1, $.scoped_identifier),
    ),

    tuple_pattern: $ => seq('(', $._pattern, repeat1(seq(',', $._pattern)), optional(','), ')'),

    // `[first, ..rest]`, `[.., last]`, `[]`: an array or a slice by its
    // elements. `..rest` reads as a range to the name, which the checker
    // tells apart.
    slice_pattern: $ => seq('[', commaSep(choice($._pattern, $.rest_pattern)), ']'),

    // `.Some(x)`, `Shape::Circle`, `Point(x, y)`. A bare name is an
    // identifier pattern: whether it names a variant or binds one is the
    // checker's to say.
    variant_pattern: $ => choice(
      seq('.', field('name', $.identifier), optional($.binders)),
      seq(field('name', $.scoped_identifier), optional($.binders)),
      seq(field('name', $.identifier), $.binders),
    ),

    binders: $ => seq('(', commaSep(choice($.binder, $.rest_pattern)), ')'),

    rest_pattern: _ => '..',

    binder: $ => choice(
      seq(field('field', $.identifier), optional(seq(':', field('pattern', $._pattern)))),
      $._pattern,
    ),

    // After `is`, a `|` is the pattern's, not an integer's or,
    // as the compiler's parser reads it.
    or_pattern: $ => prec.left(PREC.comparison + 1, seq($._pattern, '|', $._pattern)),

    // ---- Tokens ------------------------------------------------------

    identifier: _ => /[A-Za-z_][A-Za-z0-9_]*/,

    // A name where a type or a field is meant: an identifier, called what
    // it is for the queries.
    _type_identifier: $ => alias($.identifier, $.type_identifier),

    _field_identifier: $ => alias($.identifier, $.field_identifier),

    self: _ => 'self',

    boolean: _ => choice('true', 'false'),

    null: _ => 'null',

    integer: _ => token(choice(
      /0x[0-9a-fA-F](_?[0-9a-fA-F])*/,
      /0o[0-7](_?[0-7])*/,
      /0b[01](_?[01])*/,
      /[0-9](_?[0-9])*/,
    )),

    float: _ => token(choice(
      /[0-9](_?[0-9])*\.[0-9](_?[0-9])*([eE][+-]?[0-9](_?[0-9])*)?/,
      /[0-9](_?[0-9])*[eE][+-]?[0-9](_?[0-9])*/,
    )),

    string: $ => choice(
      seq(
        '"',
        repeat(choice(
          $.string_content,
          $.escape_sequence,
          $.interpolation,
        )),
        token.immediate('"'),
      ),
      // A text block: the lines between two `"""`s.
      seq(
        '"""',
        repeat(choice(
          $.text_block_content,
          $.escape_sequence,
          $.interpolation,
        )),
        token.immediate('"""'),
      ),
    ),

    string_content: _ => token.immediate(prec(1, /[^"\\\n]+/)),

    escape_sequence: _ => token.immediate(seq(
      '\\',
      choice(/[ntr0\\"']/, /u\{[0-9a-fA-F]{1,6}\}/),
    )),

    // `\(value)` in a string, and `\(value, width: 4, fill: '0')`.
    interpolation: $ => seq(
      token.immediate('\\('),
      $._expression,
      repeat(seq(',', field('option', $.identifier), ':', $._expression)),
      ')',
    ),

    char: $ => seq(
      "'",
      choice(token.immediate(/[^'\\\n]/), $.escape_sequence),
      token.immediate("'"),
    ),

    // `b'x'`: the byte of an ASCII character.
    byte: $ => seq(
      "b'",
      choice(token.immediate(/[^'\\\n]/), $.escape_sequence),
      token.immediate("'"),
    ),

    comment: _ => token(seq('//', /[^\n]*/)),
  },
});
