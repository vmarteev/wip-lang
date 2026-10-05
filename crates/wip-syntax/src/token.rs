use std::fmt::Write;

use crate::{LineIndex, Span, Symbol};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    /// The token is the first on its line (the end of file counts as one).
    /// Where line breaks are significant, this is what ends a statement
    /// (grammar §1.1).
    pub line_start: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// An identifier, including built-in type names such as `i64`.
    Ident(Symbol),
    /// An integer literal. Its value is read from the source text later, so
    /// range errors are reported by the phase that knows the expected type.
    Int,
    /// A float literal, read from the source text later like [`TokenKind::Int`].
    Float,
    /// A string literal. The symbol holds its contents with escapes applied.
    Str(Symbol),
    /// A character literal, `'x'`: the Unicode scalar value it holds.
    Char(u32),
    /// A byte literal, `b'x'`: the byte of one ASCII character.
    Byte(u8),
    /// The text of an interpolated literal before its first `\(`.
    /// The expression follows as ordinary tokens.
    StrStart(Symbol),
    /// The text between one `)` and the next `\(`.
    StrMid(Symbol),
    /// The text from the last `)` to the closing quote.
    StrEnd(Symbol),

    // Keywords.
    As,
    /// `assert(cond)` and `assert(cond, "note")`. The symbol
    /// is the message a failure prints, which the lexer makes from the
    /// condition's own text, since the parser has no interner; it is `None`
    /// where what follows is not an assert's arguments, which the parser
    /// reports.
    Assert(Option<Symbol>),
    Defer,
    Else,
    /// `then`: where an `if`'s condition ends, before a branch of one
    /// expression.
    Then,
    Enum,
    Extern,
    False,
    Fn,
    If,
    /// Reserved: reported with a fix to `val`.
    Let,
    Match,
    Move,
    Own,
    Import,
    Pub,
    /// `self`, a method's receiver, and the module itself in an import
    /// list.
    SelfKw,
    /// `static fn`: a function of a type with no receiver.
    Static,
    /// `type name` in an extern block: an opaque C type.
    Type,
    /// `null`: a C pointer that points at nothing.
    Null,
    /// `impl`, which Wip writes `extend`: a keyword so that writing it is
    /// reported rather than read as a name.
    Impl,
    /// `interface Name { … }`.
    Interface,
    /// `&dyn Name`: a reference to a value of some type that implements the
    /// interface.
    Dyn,
    Return,
    /// `lend place`: what a projection ends with.
    Lend,
    /// `yield value`: hands a value to the list being built, or to whoever
    /// asks a generator.
    Yield,
    Struct,
    True,
    Val,
    Var,
    While,
    For,
    In,
    Break,
    Continue,
    Is,

    // Punctuation.
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    ColonColon,
    Semi,
    Dot,
    DotDot,
    DotDotDot,
    /// `..=`: a range that takes its end, in a pattern.
    DotDotEq,
    Arrow,
    FatArrow,
    Underscore,
    Eq,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Amp,
    AmpAmp,
    PipePipe,
    Pipe,
    Caret,
    /// `expr?`: the value, or an early return of the other variant.
    Question,
    /// `@name`: an annotation before a declaration.
    At,
    /// `+=` and the other compound assignments but `<<=` and `>>=`, which
    /// are two tokens each.
    PlusEq,
    /// `+%`, `-%` and `*%`: arithmetic that wraps rather than panicking.
    PlusPercent,
    MinusPercent,
    StarPercent,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,

    /// End of input. Always the last token, with an empty span at the end of
    /// the file.
    Eof,
}

impl TokenKind {
    pub fn keyword(text: &str) -> Option<TokenKind> {
        use TokenKind::*;
        Some(match text {
            "as" => As,
            "assert" => Assert(None),
            "defer" => Defer,
            "else" => Else,
            "then" => Then,
            "enum" => Enum,
            "extern" => Extern,
            "false" => False,
            "fn" => Fn,
            "if" => If,
            "import" => Import,
            "let" => Let,
            "match" => Match,
            "move" => Move,
            "own" => Own,
            "pub" => Pub,
            "return" => Return,
            "struct" => Struct,
            "true" => True,
            "val" => Val,
            "var" => Var,
            "while" => While,
            "for" => For,
            "in" => In,
            "break" => Break,
            "continue" => Continue,
            "is" => Is,
            "lend" => Lend,
            "yield" => Yield,
            "self" => SelfKw,
            "static" => Static,
            "type" => Type,
            "null" => Null,
            "impl" => Impl,
            "interface" => Interface,
            "dyn" => Dyn,
            _ => return None,
        })
    }

    /// The exact text of a keyword or punctuation token; `None` for tokens
    /// whose text varies (identifiers, literals) and for end of file.
    pub fn text(self) -> Option<&'static str> {
        use TokenKind::*;
        Some(match self {
            Ident(_) | Int | Float | Str(_) | Char(_) | Byte(_) | StrStart(_) | StrMid(_)
            | StrEnd(_) | Eof => {
                return None;
            }
            As => "as",
            Assert(_) => "assert",
            Defer => "defer",
            Else => "else",
            Then => "then",
            Enum => "enum",
            Extern => "extern",
            False => "false",
            Fn => "fn",
            If => "if",
            Let => "let",
            Match => "match",
            Move => "move",
            Own => "own",
            Import => "import",
            Pub => "pub",
            SelfKw => "self",
            Static => "static",
            Type => "type",
            Null => "null",
            Impl => "impl",
            Interface => "interface",
            Dyn => "dyn",
            Return => "return",
            Lend => "lend",
            Yield => "yield",
            Struct => "struct",
            True => "true",
            Val => "val",
            Var => "var",
            While => "while",
            For => "for",
            In => "in",
            Break => "break",
            Continue => "continue",
            Is => "is",
            LBrace => "{",
            RBrace => "}",
            LParen => "(",
            RParen => ")",
            LBracket => "[",
            RBracket => "]",
            Comma => ",",
            Colon => ":",
            ColonColon => "::",
            Semi => ";",
            Dot => ".",
            DotDot => "..",
            DotDotDot => "...",
            DotDotEq => "..=",
            Arrow => "->",
            FatArrow => "=>",
            Underscore => "_",
            Eq => "=",
            EqEq => "==",
            BangEq => "!=",
            Lt => "<",
            LtEq => "<=",
            Gt => ">",
            GtEq => ">=",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Percent => "%",
            Bang => "!",
            Amp => "&",
            AmpAmp => "&&",
            PipePipe => "||",
            Pipe => "|",
            Caret => "^",
            Question => "?",
            At => "@",
            PlusEq => "+=",
            PlusPercent => "+%",
            MinusPercent => "-%",
            StarPercent => "*%",
            MinusEq => "-=",
            StarEq => "*=",
            SlashEq => "/=",
            PercentEq => "%=",
            AmpEq => "&=",
            PipeEq => "|=",
            CaretEq => "^=",
        })
    }

    /// How the token is named in diagnostics: `` `fn` ``, `identifier`,
    /// `end of file`.
    pub fn describe(self) -> String {
        use TokenKind::*;
        match self.text() {
            Some(text) => format!("`{text}`"),
            None => match self {
                Ident(_) => "identifier",
                Int => "integer literal",
                Float => "float literal",
                Str(_) | StrStart(_) | StrMid(_) | StrEnd(_) => "string literal",
                Char(_) => "character literal",
                Byte(_) => "byte literal",
                _ => "end of file",
            }
            .to_string(),
        }
    }

    /// The short class name used in token dumps for tokens without fixed text.
    fn class(self) -> &'static str {
        use TokenKind::*;
        match self {
            Ident(_) => "ident",
            Int => "int",
            Float => "float",
            Str(_) => "str",
            Char(_) => "char",
            Byte(_) => "byte",
            StrStart(_) => "str-start",
            StrMid(_) => "str-mid",
            StrEnd(_) => "str-end",
            _ => "eof",
        }
    }
}

/// Renders tokens one per line as `line:col  kind  text`, for `wip lex` and
/// snapshot tests. Keywords and punctuation show their text as the kind;
/// identifiers and literals show their source text.
pub fn dump(src: &str, tokens: &[Token]) -> String {
    let index = LineIndex::new(src);
    let mut out = String::new();
    for token in tokens {
        let (line, col) = index.line_col(src, token.span.lo);
        let pos = format!("{line}:{col}");
        let line = match token.kind.text() {
            Some(text) => format!("{pos:<8}{text}"),
            None => format!(
                "{pos:<8}{:<8}{}",
                token.kind.class(),
                &src[token.span.range()]
            ),
        };
        writeln!(out, "{}", line.trim_end()).unwrap();
    }
    out
}
