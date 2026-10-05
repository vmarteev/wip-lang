# Wip — Grammar

This file is the syntax specification. The parser and this file must agree.
What the constructs *mean* is [the language reference](language/README.md);
this file is their shape. Any construct the parser accepts that is not
described here is a bug in one of the two — fix whichever is wrong, never
leave them disagreeing. Every new construct gets a production here and an
example program in `tests/cases/` before any parser code is written.

Notation: `"x"` is a literal token, `UPPER` is a token class from §1, `?` is
optional, `*` is zero or more, `+` is one or more, `|` separates
alternatives, `( )` groups, `(* … *)` is a comment.

---

## 1. Lexical grammar

Source files are UTF-8. Non-ASCII characters may appear only inside string
literals and comments.

```ebnf
whitespace  = " " | "\t" | "\n" | "\r" ;
comment     = "//" (* any characters up to the end of the line *) ;

IDENT       = ( letter | "_" ) ( letter | digit | "_" )* ;   (* except keywords and "_" alone *)
letter      = "a" … "z" | "A" … "Z" ;
digit       = "0" … "9" ;

INT         = decimal | "0x" hexDigits | "0o" octDigits | "0b" binDigits ;
decimal     = "0" | ( "1" … "9" ) ( "_"? digit )* ;
hexDigits   = hexDigit ( "_"? hexDigit )* ;
hexDigit    = digit | "a" … "f" | "A" … "F" ;
octDigits   = ( "0" … "7" ) ( "_"? ( "0" … "7" ) )* ;
binDigits   = ( "0" | "1" ) ( "_"? ( "0" | "1" ) )* ;
FLOAT       = decimal "." digits exponent?
            | decimal exponent ;
digits      = digit ( "_"? digit )* ;
exponent    = ( "e" | "E" ) ( "+" | "-" )? digits ;

STRING      = '"' ( strChar | escape )* '"' ;
strChar     = (* any character except '"', "\" and newline *) ;
escape      = "\" ( "n" | "t" | "r" | "0" | "\" | '"' | "'" )
            | "\u{" hexDigit hexDigit? hexDigit? hexDigit? hexDigit? hexDigit? "}" ;
                                         (* a Unicode scalar value *)

(* One character. *)
CHAR        = "'" ( charChar | escape ) "'" ;
charChar    = (* any character except "'", "\" and newline *) ;

(* The byte of one ASCII character, a u8. *)
BYTE        = "b'" ( charChar | escape ) "'" ;

(* A literal with "\(" in it is interpolated, and its value is a String
   rather than a str. *)
INTERP      = '"' piece ( "\(" expr ( "," IDENT ":" expr )* ")" piece )+ '"' ;
                                         (* after the value, `width`, `fill`
                                            and `align` *)
piece       = ( strChar | escape )* ;

(* A text block: nothing but space after the opening '"""',
   and the closing '"""' first on its line after its indentation, which is
   taken off every line. It may hold "\(" expr ")" as a string may. *)
TEXTBLOCK   = '"""' space* newline ( line newline )* space* '"""' ;
line        = ( blockChar | escape )* ;
blockChar   = (* any character except "\" and newline, and '"""' *) ;
```

Rules the EBNF does not express:

- **A `.` that starts a line begins a variant**, since a line break has
  already ended the expression above it. Where the line above could have
  gone on, the parser warns (E0126): a variant gets a fix that writes
  `return` before it, and a name in lower case — a method or a field — one
  that moves the `.` to the end of the line above, where it continues it.

- **Longest match**, with one exception: there is no `>>` or `<<` token.
  `>>` always lexes as two `>`, so `own<own<i64>>` needs no special case. In
  an expression, two `<` or `>` with nothing between them are a shift.
- **`_` separates digits**: one `_`, between two digits.
  `1__000` and `1_` are errors (E0010). A base prefix needs digits its base
  allows: `0x`, `0b102` and `0o9` are errors (E0011).
- **`~` is not an operator** (E0009), with a fix to `!`.
- **No leading zeros.** `007` is an error (E0005).
- **A number may not be followed directly by an identifier character.**
  `12abc` and `3e` are errors (E0007).
- **`1.` is an error** (E0004) unless the `.` is followed by an identifier.
- **Strings cannot span lines** (E0002); a text block can.
  The closing `"""`'s indentation is taken off each line, and a line
  indented less is an error (E0013), as are text after the opening `"""`,
  a `"""` inside, and a `\` ending a line. Space at the end of a line is
  not part of the text; an escape keeps it. The line breaks after the
  opening and before the closing are not part of the text either.
- **A character literal holds one character**: `''` and
  `'ab'` are errors (E0008), the second with a fix to `"ab"`, and so is a
  `'` that is not closed on its line. `\u{…}` names a Unicode scalar value:
  past `10FFFF` or a surrogate is an error (E0003).
- **A byte literal holds one ASCII character**: `b'é'`
  and `b'\u{80}'` are errors (E0012), and `b''` and `b'ab'` are as a
  character's are (E0008). `b` directly before `'` begins one; a name is
  never followed by a `'`.
- **Integer range** is checked by the type checker, which knows whether the
  literal is an `i64` or an `i32`.
- There are no block comments. A `///` comment is a comment like any
  other, which `wip doc` and the language server read as documentation.
- Each token records whether it is the **first on its line**. §1.1 says when
  that matters.

### Keywords

```
as  assert  break  continue  defer  dyn  else  enum  extern  false  fn  for
if  import  in  interface  is  lend  match  move  null  own  pub  return
self  static  struct  then  true  type  val  var  while  yield
```

`let` and `impl` are **reserved**: neither is part of the language, and
the parser reports each with a fix — `let` to `val` and
`impl` to `extend`. `yield` where a value is expected is
reported with a fix to `lend`; as a statement it hands a
value to a list being built or to whoever asks a generator. `extend` is
not a keyword: it is a word only where an item begins, so a program may
still name something `extend`.

The built-in type names are **not** keywords: `i8` `i16` `i32` `i64` `i128`
`isize` `u8` `u16` `u32` `u64` `u128` `usize` `f32` `f64` `bool` `str`
`cstring` `void` `never`, and C's own names `c_char` `c_schar` `c_uchar`
`c_short` `c_ushort` `c_int` `c_uint` `c_long` `c_ulong` `c_longlong`
`c_ulonglong` `c_float` `c_double` `size_t`, which stand for the integer
types C uses on the target. They are identifiers that name built-in types;
declaring a type with one of those names is an error.

### Punctuation

```
{  }  (  )  [  ]  ,  :  ::  ;  .  ..  ..=  ...  =>  _  ?  @
=  ==  !=  <  <=  >  >=  +  -  *  /  %  !  &  &&  |  ||  ^
+=  -=  *=  /=  %=  &=  |=  ^=
+%  -%  *%
```

`->` is lexed only so the parser can report it with a fix to `:`, and `~`
only to report it with a fix to `!` (E0009). There is no `<<` or `>>`
token (§1), and so no `<<=` or `>>=` token: they are `<` and `<=`, or `>`
and `>=`, with nothing between them. `+%=`, `-%=` and `*%=` are likewise
`+%`, `-%` or `*%` and `=`, and `!is` is `!` and `is`.

### 1.1 Line breaks

Semicolons and commas are optional at the end of a line. A
line break takes their place where the grammar below writes `sep` or `lsep`:

```ebnf
sep         = ( ";" | NL )+ ;            (* between statements *)
lsep        = NL* "," NL* | NL+ ;        (* between the items of a brace list *)
```

`NL` is not a token. It is the line break before a token that is the first
on its line, in a place where line breaks are **significant**:

- **Line breaks are whitespace** inside `( )` and `[ ]`, and in the
  condition of `if`, `while` and `match`, and in the head of `for`, up to
  the `{` that begins its body.
- **Line breaks are significant** at the top level and directly inside
  `{ }`: in blocks, match arms, struct and enum declarations and extern
  blocks. The innermost bracket decides, so a block inside an
  argument list is significant again.

Where line breaks are significant:

1. **They separate things.** Statements, match arms, fields, variants and
   extern declarations are separated by a line break. Two of them on one
   line need a `;` (statements) or `,` (everything else) between them
   (E0112).
2. **They end a complete expression.** An expression does not continue
   across a line break once it is complete. A binary operator, `(` or `[` at
   the start of a line begins something new; it does not continue the line
   above. An incomplete expression — a line ending in an operator, `.`, `=`,
   `=>`, `then`, `,` or an open bracket — continues onto the next line.
3. **One token continues the line above:** `else`, which cannot begin a
   statement. A `.` does not: at the start of a line it begins a variant.

**A long expression wraps after an operator or a `.`** that ends its line,
or inside parentheses, where line breaks are whitespace:

```wip
val best = some.
    map((v: i64) => v * 2).
    unwrapOr(0)

val total = price +
    tax -
    discount

val also = (price
    + tax
    - discount)
```

Outside brackets, the operator or the `.` goes at the end of the line it
continues from, never at the start of the next one.

A line that starts with a binary operator other than `-` and `&` is an
error (E0113), with a fix that joins it to the line above. A line starting
with `-` is a new expression, so `total = price` followed by `- discount` on
the next line is two statements; the type checker warns that the second one
has no effect (E0320).

Line breaks right after `{` and right before `}` are always allowed. A
leading `,` in a brace list is not. Items at the top level need no
separator: each begins with a keyword.

---

## 2. Syntax

```ebnf
program     = item* EOF ;                (* separated by line breaks or ";" *)
packageFile = annotation* "package" IDENT ;
                                         (* a `package.wip`: its annotations,
                                            and its name *)
item        = annotation* ( importDecl
            | "pub"? ( structDecl | enumDecl | fnDecl | interfaceDecl | valDecl
                     | typeDecl )
            | extendBlock
            | externBlock
            | assertExpr ) ;   (* `pub` exports from the module;
                                  annotations come first; an
                                  assert at the top level is checked while
                                  the program is compiled *)
typeDecl    = "type" IDENT generics? "=" type ;
                                        (* another name for a type; inside an extern block `type`
                                           declares a C type *)
valDecl     = "val" IDENT ( ":" type )? "=" expr ;
                                        (* a constant: its value is known where
                                           it is written, or,
                                           marked `@comptime`, worked out by
                                           running code while the program is
                                           compiled *)
annotation  = "@" IDENT ( "(" ( annArg ( "," annArg )* ","? )? ")" )? ;
annArg      = ( IDENT "=" )? ( INT | STR | "true" | "false" | IDENT ) ;
                                        (* a name is what `@derive(Eq, Hash)`
                                           takes *)
                                        (* the set of names is closed and the
                                           checker knows it *)
importDecl  = "import" path ( "as" IDENT
                           | "::" "{" importItem ( lsep importItem )* lsep? "}" )? ;
importItem  = ( IDENT | "self" ) ( "as" IDENT )? ;
                                        (* "self" names the module *)
path        = IDENT ( "::" IDENT )* ;

(* ---- declarations ---- *)

structDecl  = ( "extern" | "view" )? "struct" IDENT generics? "{" ( member ( lsep member )* lsep? )? "}" ;
                                         (* `view struct`: it may borrow. `view` is a word
                                            only here. *)
                            (* `extern struct` is C's layout, and takes no
                               generics *)
                                         (* a member is a field or a method *)
field       = ( "pub" "var"? )? IDENT ":" type ( "=" expr )? ;
                                         (* `pub` exports reading the field and
                                            `pub var` writing it too; without
                                            either it is the module's *)
                                         (* `= value` is what a literal that
                                            leaves the field out puts there: a
                                            constant *)

enumDecl    = "view"? "enum" IDENT generics? "{" ( member ( lsep member )* lsep? )? "}" ;
                                         (* a member is a variant or a method *)
                                         (* `view enum`: its variants may borrow. `view` is a word
                                            only here. *)
variant     = IDENT ( "(" params ")" )? ;
                                         (* a field's `= value` is its default,
                                            as a parameter's *)

extendBlock = "extend" extendType ( ":" IDENT typeArgs? )?
              "{" ( method ( lsep method )* lsep? )? "}" ;
extendType  = path generics?              (* a type of this module *)
            | "[" IDENT bounds? "]" ;     (* a slice, whose element is named
                                             here; the prelude's *)
            (* a type's methods, outside its declaration, and after ":" the
               methods of the one interface it implements; the type arguments
               name the type's own parameters *)

interfaceDecl = "interface" IDENT generics?
                "{" ( ifaceMethod ( lsep ifaceMethod )* lsep? )? "}" ;
ifaceMethod = ( "var" | "move" | "static" )? "fn" IDENT generics?
              "(" params? ")" ( ":" type )? ( "=" expr )? ;
            (* `= expr` is a default body *)

member      = field | variant | method ;   (* in any order *)
method      = annotation* "pub"? ( "var" | "move" | "static" | "lend" )? fnDecl ;
                                         (* the word before `fn` says what the
                                            method does with its receiver;
                                            `lend` both ways *)

fnDecl      = "fn" IDENT generics? "(" params? ")" ( ":" type )? "=" expr ;
generics    = "<" typeParam ( "," typeParam )* ","? ">" ;
typeParam   = IDENT ( ":" constraint ( "+" constraint )* )? ( "=" type )? ;
                                         (* a default: a struct's or an
                                            enum's, last *)
constraint  = IDENT typeArgs? ;          (* `copy`, or an interface with the
                                            types it takes *)
params      = param ( "," param )* ","? ;
param       = ( IDENT | "_" ) ":" type ( "=" expr )? ;
                                          (* a default: a constant;
                                             `_`: not used *)

(* `extern union Name { … }`: C's union. `union` is a word
   only here; elsewhere it is an ordinary identifier. *)
externUnion = "extern" "union" IDENT "{" lsep? ( field ( lsep field )* lsep? )? "}" ;

externBlock = "extern" STRING "{" sep? ( externItem ( sep externItem )* sep? )? "}" ;
                                          (* the STRING must be "C" *)
externItem  = annotation* "pub"? ( externFn | externType | externVar
                                 | structDecl ) ;
                                          (* `@symbol("name")`: what C calls
                                             it; a `struct` or
                                             `union` in a block is C's and
                                             takes the block's `@header` *)
externVar   = ( "val" | "var" ) IDENT ":" type ;
                                          (* a variable C owns; `var` may be
                                             written *)
externFn    = "fn" IDENT generics? "(" ( params ( "," "..." )? )? ")" ( ":" type )? ;
                                          (* generics are reported: C has none.
                                             `...`: it takes more than it
                                             declares *)
externType  = "type" IDENT ;              (* a C type whose contents are not
                                             known *)

(* ---- types ---- *)

type        = "own" "<" type ">"           (* `own<(…) => R>` is an owned
                                              closure *)
            | "ptr" "<" type ">"           (* a C pointer; `ptr<void>` is
                                              `void *` *)
            | "&" "var"? type             (* parameters only;
                                              `&(…) => R` is a closure, lent for
                                              one call *)
            | "[" type ";" ( INT | IDENT ) "]"
                                           (* a length is a literal or a
                                              constant *)
            | "[" type "]"                 (* a slice: only behind "&" *)
            | "dyn" IDENT                  (* some type that implements the interface:
                                              only behind "&" *)
            | "(" ( param ( "," param )* ","? )? ")" "=>" type
                                           (* a function type, `=> void` for none;
                                              names are required *)
            | "(" type ( "," type )+ ","? ")"
                                           (* a tuple, two elements to four:
                                              the prelude's `TupleN` *)
            | path typeArgs? ;             (* a module's type is `a::b::Point` *)
typeArgs    = "<" type ( "," type )* ","? ">" ;

(* ---- blocks and statements ---- *)

block       = "{" sep? ( stmt ( sep stmt )* sep? )? "}" ;
stmt        = bindStmt | patternStmt | deferStmt | returnStmt
            | yieldStmt | whileStmt | forStmt | breakStmt | continueStmt | expr ;
bindStmt    = ( "val" | "var" ) IDENT ( ":" type )? "=" expr ;
patternStmt = ( "val" | "var" ) pattern "=" expr ( "else" block )? ;
                                         (* a pattern that can fail needs `else`,
                                            whose block must leave; after `var`,
                                            each name is a variable of its own *)
deferStmt   = "defer" expr ;             (* runs when the block exits *)
returnStmt  = "return" expr? ;           (* a line break after `return` means no value *)
yieldStmt   = "yield" expr ;             (* to the list being built,
                                            or to whoever asks the generator it is
                                            in *)
whileStmt   = ( IDENT ":" )? "while" cond block ;
                                         (* a name, which `break name` leaves *)
forStmt     = ( IDENT ":" )? "for" pattern "in" cond ( ( ".." | "..=" ) cond )? block ;
                                         (* a name, `_`, or a struct taken
                                            apart *)
breakStmt   = "break" IDENT? ;           (* a loop's name *)
continueStmt = "continue" IDENT? ;

(* ---- expressions ---- *)

cond        = expr ;                     (* its `{` begins the block, see R1 *)

expr        = "lend" expr             (* a projection lends a place *)
            | assign ;
assign      = logicOr ( assignOp assign )? ;
assignOp    = "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^="
            | "<<=" | ">>=" | "+%=" | "-%=" | "*%=" ;
                                         (* the last five, two adjacent tokens *)
logicOr     = logicAnd ( "||" logicAnd )* ;
logicAnd    = equality ( "&&" equality )* ;
equality    = comparison ( ( "==" | "!=" ) comparison )* ;
comparison  = bitOr ( ( "<" | "<=" | ">" | ">=" ) bitOr | ( "is" | "!is" ) pattern )* ;
                                         (* a pattern that can fail; `!is`
                                            binds nothing *)
bitOr       = bitXor ( "|" bitXor )* ;
bitXor      = bitAnd ( "^" bitAnd )* ;
bitAnd      = shift ( "&" shift )* ;
shift       = term ( ( "<<" | ">>" ) term )* ;   (* two adjacent "<" or ">" *)
term        = factor ( ( "+" | "-" | "+%" | "-%" ) factor )* ;
factor      = cast ( ( "*" | "/" | "%" | "*%" ) cast )* ;
                                         (* `+%`, `-%` and `*%` wrap *)
cast        = unary ( "as" type )* ;     (* numeric conversions;
                                           the type takes no typeArgs, see R7 *)
unary       = ( "-" | "!" | "&" "var"? | "move" | "own" ) unary
            | postfix ;
            (* `own` before a lambda makes an owned closure, which holds what
               it captured on the heap *)
postfix     = primary ( "." IDENT | "." INT | "(" args? ")" | "[" index "]" | "?" )* ;
            (* `pair.0` is a tuple's element *)
            (* `expr?` is the value, or an early return of the other variant
               of an `Option` or a `Result` *)
            (* `x.name(…)` calls a method of `x`'s type, or a field of
               function type; `Type::name(…)` a static function *)
index       = expr | expr? ".." expr? | expr? "..=" expr ;  (* a range must be borrowed: &xs[lo..hi];
                                           on a type of one's own, `x[i]` calls the
                                           `at` of `Index` *)
primary     = lambda
            | INT | FLOAT | STRING | CHAR | BYTE | INTERP | "true" | "false"
            | pathExpr
            | IDENT
            | "self"                     (* a method's receiver *)
            | arrayLit
            | block
            | ifExpr
            | matchExpr
            | assertExpr
            | forElement                 (* a loop where a value stands: a
                                            generator *)
            | "(" expr ")"
            | "(" expr ( "," expr )+ ","? ")" ;
                                         (* a tuple, two elements to four, which
                                            is `TupleN { _0: …, _1: … }`;
                                            `pair.0` reads one *)

lambda      = "(" ( lambdaParam ( "," lambdaParam )* ","? )? ")"
              ( ":" type )? "=>" expr ;
lambdaParam = ( IDENT | "_" ) ( ":" type )? ;  (* the type comes from what is
                                             expected, where it is left out *)

assertExpr  = "assert" "(" expr ( "," STRING )? ")" ;
                                         (* the condition holds, or the program
                                            prints it and ends; the note stands
                                            before it *)

ifExpr      = "if" cond block ( "else" ( block | ifExpr ) )?
            | "if" cond "then" branch ( "else" branch )? ;
branch      = expr | jump ;          (* one expression, no braces; an `else
                                        if` chain takes one form throughout *)
matchExpr   = "match" cond "{" ( arm ( lsep arm )* lsep? )? "}" ;
arm         = pattern ( "if" cond )? "=>" ( expr | jump ) ;
                                         (* a guard: the arm matches only where
                                            it is true *)
jump        = "return" expr? | "break" IDENT? | "continue" IDENT? ;
                                         (* the arm ends the function or the
                                            loop *)

pathExpr    = "." IDENT | exprPath ;    (* "." takes the expected enum;
                                           a longer path names a module *)
exprPath    = IDENT typeArgs? ( "::" IDENT typeArgs? )* ;
            (* at most one typeArgs, after the generic item's name, and only
               where R7 allows: f<i64>(x), Pair<A, B> { … }, Option<i64>::None *)

arrayLit    = "[" ( element ( ";" expr | ( "," element )* ","? ) )? "]" ;
element     = expr | forElement ;
forElement  = "for" pattern "in" expr ( ( ".." | "..=" ) expr )? block ;
                            (* an element that yields — a `for`, or an `if`,
                               `match` or block with `yield` inside — makes
                               the length known only as the list is built:
                               only `own [ … ]` may hold one, and `own
                               forElement` is the list of what one loop
                               yields; anywhere else a value
                               stands, a `for` is a generator;
                               where a statement stands, it is a forStmt *)
            (* a count that is not an INT literal needs `own`: own<[T]> *)

pattern     = onePattern ( "|" onePattern )* ;
                                         (* any one of them *)
onePattern  = "_"
            | IDENT
            | "-"? INT | "true" | "false" | STRING | CHAR | BYTE
                                         (* a value the scrutinee must equal;
                                            a float is reported *)
            | bound ( ".." | "..=" ) bound? | ( ".." | "..=" ) bound
                                         (* the integers or characters between
                                            two ends, `..=` taking the last;
                                            a bound after the dots is read as
                                            the end *)
            | "(" pattern ( "," pattern )+ ","? ")"
                                         (* a tuple taken apart *)
            | "[" ( sliceElem ( "," sliceElem )* ","? )? "]"
                                         (* an array or a slice, by its
                                            elements; `..` once, for those
                                            between *)
            | "."? path ( "(" binders? ")" )? ;
                                         (* a path with no leading "." and
                                            fields after it is a struct taken
                                            apart *)
sliceElem   = pattern | ".." IDENT? ;      (* `..rest` binds the elements
                                            between, as a slice *)
bound       = "-"? INT | CHAR | BYTE | path ;
                                         (* a literal, or a constant's name *)
binders     = ( binder ( "," binder )* ( "," ".." )? | ".." ) ","? ;
binder      = IDENT ( ":" pattern )? | pattern ;
                                         (* a field, or field: pattern — a name renames
                                            it, and a pattern takes the field apart or
                                            tests it; a pattern alone is the one field
                                            the variant has *)

args        = arg ( "," arg )* ( "," ".." expr )? ","?
            | ".." expr ","? ;
arg         = ( IDENT ":" )? expr ;     (* a named argument; positional ones come first *)
            (* a call whose callee names a struct builds it, its arguments its
               fields; `..base` gives the fields it does not name, last and
               once *)
```

**The value of a block** is the value of its last statement, if that
statement is an expression; otherwise it is nothing, `void`, which is
written `{}`. A `;` after the last expression changes nothing: semicolons
are separators, never meaningful.

`primary` alternatives that start with `IDENT` are distinguished by the next
token: `::` → `pathExpr`, anything else → `IDENT`. `IDENT {` is not a
struct: it is refused as the way one used to be written (E0133), with a fix
that writes the call. A `{` where an expression is expected begins a block.
A `.` where an expression or a pattern is expected begins a variant of the
expected enum; it is never field access, which only follows a primary. Apart
from R7, no production needs more than one token of lookahead; §1.1 needs
one token of line information.

---

## 3. Operator precedence

From tightest to loosest. All binary operators are left-associative except
`=`.

| Level | Operators | Associativity |
|---|---|---|
| 1 | postfix `.f`  `f(…)`  `a[i]` | left |
| 2 | prefix `-`  `!`  `&`  `move`  `own` | right (prefix) |
| 3 | `as` | left |
| 4 | `*`  `/`  `%`  `*%` | left |
| 5 | `+`  `-`  `+%`  `-%` | left |
| 6 | `<<`  `>>` | left |
| 7 | `&` | left |
| 8 | `^` | left |
| 9 | `\|` | left |
| 10 | `<`  `<=`  `>`  `>=`  `is`  `!is` | left |
| 11 | `==`  `!=` | left |
| 12 | `&&` | left |
| 13 | `\|\|` | left |
| 14 | `=` | right |

`if`, `match` and blocks are primaries, so `if a { 1 } else { 2 } + 3` adds
3 to the result of the `if`.

- `own f(x)` is `own (f(x))`.
- `own Point(x: 1).x` is `own (Point(x: 1).x)`.
- `-v.x` is `-(v.x)`; `&a[i]` is `&(a[i])`; `move p.next` is `move
  (p.next)`.
- `-x as i64` is `(-x) as i64`; `a * b as f64` is `a * (b as f64)`.
- `x & MASK == 0` is `(x & MASK) == 0`, unlike in C.

---

## 4. Resolved ambiguities

### R1 — A condition's `{` begins its block

A struct is built with a call, so a `{` after a `cond` —
the condition of `if` and `while`, what a `for` walks or its bounds, and
the scrutinee of `match` — always begins the block. `IDENT {` followed by
`IDENT :` there cannot begin a block, and is refused as a struct written
with braces (E0133).

### R2 — `&` is positional

Prefix `&` borrows; infix `&` is bitwise and. `&&` in borrow position is
E0109 with a fix to `& &`.

### R3 — `own` as type constructor versus expression prefix

In type position `own` must be followed by `<`; in expression position it is
a prefix operator.

### R4 — Separators in brace lists

Match arms, fields, variants and struct-literal fields are separated by line
breaks, or by `,` when two share a line. A missing `,` between two arms on
one line is E0104.

### R5 — Trailing separators

A trailing `,` is permitted in every list, and a trailing `;` in every
block.

### R6 — Line breaks

§1.1. The one-line form of every construct is the multi-line form with `;`
or `,` where the line breaks were.

### R7 — `<` after a name

In a type, `<` after a type's name starts its type arguments: `Pair<i64,
bool>`. In an expression, `<` after a name starts type arguments when:

- the tokens up to its matching `>` form types: each token can follow the
  one before it in a type, so `xs[i]` does not, and a `(` counts only when
  its `)` is followed by `=>`, so `(b)` does not;
- the token after that `>` is `(`, `::`, a `{` outside a condition, which is
  refused as a struct written with braces (E0133), or one that cannot start
  an operand: `)`, `,`, `]`, `}`, `;`, or the end of the line. The last kind
  is for a generic function named as a value, as in `sort(&var xs,
  less<i64>)`;
- and the `<` is not half of `<<`.

Otherwise `<` is a comparison. The parser looks ahead no further than the
matching `>`.

No correct program reads differently: `bool` values cannot be ordered, so `a
< b > (c)` is a type error as a comparison; in `f(a < b, c > (d))` the
comparisons are errors too unless `a` is a generic function and `b` and `c`
are types, which cannot be compared; and `a < b >` followed by a token that
ends an expression has no right operand. `f(a < xs[i], c > (d))` stays two
comparisons, since `xs[i]` is not a type. When a comparison does read as
type arguments, the type checker reports E0211 with a fix that adds
parentheses. Rust writes `f::<i64>(x)` to avoid the question; Wip reports
`::<` (E0120), with a fix that removes the `::`.

The type after `as` takes no type arguments, so `x as i64 < y` compares.

---

## 5. Rules the parser enforces beyond the EBNF

Each has its own diagnostic rather than a generic "unexpected token"
(E0101).

- The ABI string of an `extern` block must be `"C"` (E0105).
- `...` only in an `extern` declaration, only last, and only after a
  parameter (E0428); a call that passes more names none of
  its arguments (E0429).
- A `fnDecl` needs `= body`; an `externFn` has none (E0106).
- An unclosed `{` (E0108), pointing at the likely culprit by indentation.
- `&&` where a borrow or reference type is expected (E0109).
- An integer literal above `2⁶⁴−1` (E0111).
- Two statements, arms or list items on one line without `;` or `,` (E0112).
- A line that begins with a binary operator (E0113), with a fix that joins
  it to the line above.
- `let` (E0114, fix: `val`) and `->` in a return type (E0115, fix: `:`).
- `::<` before type arguments in an expression (E0120), with a fix that
  removes the `::`.
- A function type written `fn(A, B): R`, or with a parameter that has no
  name (E0121).
- A positional argument after a named one (E0122).
- A pattern that can fail, after `val` or `var`, without `else` (E0123).
- The R4 diagnostic (E0104), and a struct written with braces (E0133).

Left to later phases: whether an assignment target is a place, whether
`move` is applied to a place, integer literal range, and every rule about
where `&T` may appear.

---

## 6. Example

```wip
import std::io

extern "C" {
    fn getpid(): i32
}

struct Point {
    x: i64
    y: i64
}

enum Shape {
    Circle(radius: f64)
    Rect(w: f64, h: f64)
}

fn area(s: &Shape): f64 = match s {
    .Circle(radius) => 3.14159 * radius * radius
    .Rect(w, h) => w * h
}

fn square(x: i64): i64 = {
    val square = x * x
    return square
}

fn abs(x: i64): i64 = if x < 0 then -x else x

fn pid(): i32 = getpid()

fn main() = {
    var total = 0
    var i = 0
    while i < 10 {
        total = total + square(i)
        i = i + 1
    }
    io::printInt(total)
    io::printInt(pid() as i64)
    val p = Point(x: 1, y: 2); io::printInt(p.x + p.y)
}
```
