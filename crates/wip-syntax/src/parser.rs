//! Recursive-descent parser for the grammar in `docs/grammar.md` §2.
//!
//! # Line breaks
//!
//! Every token records whether it starts a line, and the parser tracks
//! whether line breaks are significant where it is (grammar §1.1): at the top
//! level and inside `{ }` they are, inside `( )`, `[ ]` and conditions they
//! are not. Where they are significant, a line break separates statements and
//! list items, and a complete expression does not continue across one.
//!
//! # Error recovery
//!
//! A parse error never stops the parse. Every production returns a node even
//! when its input is malformed — an `Error` node if nothing better fits — and
//! after an error the parser resynchronizes: at the next separator or closing
//! delimiter inside a list, at the next line break, `;` or statement keyword
//! inside a block, and at the next item keyword at the top level. An item
//! keyword inside a block means the block was never closed.
//!
//! One mistake should produce one error. A diagnostic is therefore dropped
//! when another was already reported at the same position, or when the lexer
//! skipped text earlier in the same statement (see [`Lexed::gaps`]).

use std::cell::Cell;

use crate::ast::*;
use crate::diagnostic::{Diagnostic, Edit};
use crate::lexer::Lexed;
use crate::token::{Token, TokenKind as T};
use crate::{Code, MAX_TUPLE, MIN_TUPLE, Span, Symbol};

mod exprs;
mod items;
mod lists;
mod patterns;
mod recovery;
mod stmts;
#[cfg(test)]
mod tests;
mod types;

use crate::codes;

pub struct Parsed {
    pub ast: Ast,
    pub diagnostics: Vec<Diagnostic>,
}

/// A `package.wip`: the package's name and what it says of itself.
pub struct ParsedPackage {
    /// `@version(…)`, `@depends(…)` and whatever else was written, checked
    /// by the loader, which knows what each means.
    pub annotations: Vec<Annotation>,
    /// The name after `package`, where one was written.
    pub name: Option<Name>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses a `package.wip`, whose spans start at `base`: annotations, then
/// `package NAME`, and nothing else. `package` is a word
/// only here, so a program may still write `package::VERSION`.
pub fn parse_package_at(src: &str, base: u32, lexed: &Lexed) -> ParsedPackage {
    let mut parser = Parser {
        src,
        base,
        tokens: &lexed.tokens,
        gaps: &lexed.gaps,
        pos: 0,
        ast: Ast::default(),
        diagnostics: Vec::new(),
        region_start: 0,
        newlines: true,
        no_struct: false,
        brace_pairs: Vec::new(),
        variadic: None,
        last_error_at: None,
        fuel: Cell::new(0),
        type_args: 0,
        half_eq: None,
    };
    let (annotations, name) = parser.package_file();
    ParsedPackage {
        annotations,
        name,
        diagnostics: parser.diagnostics,
    }
}

pub fn parse(src: &str, lexed: &Lexed) -> Parsed {
    parse_at(src, 0, lexed)
}

/// Parses a file whose spans start at `base`.
pub fn parse_at(src: &str, base: u32, lexed: &Lexed) -> Parsed {
    let mut parser = Parser {
        src,
        base,
        tokens: &lexed.tokens,
        gaps: &lexed.gaps,
        pos: 0,
        ast: Ast::default(),
        diagnostics: Vec::new(),
        region_start: 0,
        newlines: true,
        no_struct: false,
        brace_pairs: Vec::new(),
        variadic: None,
        last_error_at: None,
        fuel: Cell::new(0),
        type_args: 0,
        half_eq: None,
    };
    parser.program();
    Parsed {
        ast: parser.ast,
        diagnostics: parser.diagnostics,
    }
}

/// Binding powers for [`Parser::expr_bp`], as (left, right). Left below right
/// makes an operator left-associative; `=` is the only right-associative one.
/// Infix `&` sits where Rust puts bitwise-and; it is parsed only to report it.
fn infix_binding_power(kind: T) -> Option<(u8, u8)> {
    Some(match kind {
        T::Eq => ASSIGN_BINDING_POWER,
        T::PipePipe => (3, 4),
        T::AmpAmp => (5, 6),
        T::EqEq | T::BangEq => (7, 8),
        T::Lt | T::LtEq | T::Gt | T::GtEq => (9, 10),
        // Bitwise operators bind tighter than comparisons, as in Rust and
        // unlike C.
        T::Pipe => (11, 12),
        T::Caret => (13, 14),
        T::Amp => (15, 16),
        // `<<` and `>>` are two tokens; see `Parser::shift`.
        T::Plus | T::Minus | T::PlusPercent | T::MinusPercent => (19, 20),
        T::Star | T::Slash | T::Percent | T::StarPercent => (21, 22),
        _ => return None,
    })
}

/// `=` and the compound assignments: the loosest, and
/// right-associative.
const ASSIGN_BINDING_POWER: (u8, u8) = (2, 1);
/// `<<` and `>>`: tighter than `&`, looser than `+`.
const SHIFT_BINDING_POWER: (u8, u8) = (17, 18);
/// `is` binds as a comparison does, looser than `|` and tighter than `==`.
const IS_BINDING_POWER: u8 = 9;
/// `as` binds tighter than `*` and looser than the prefix operators.
const CAST_BINDING_POWER: u8 = 22;
const PREFIX_BINDING_POWER: u8 = 23;

fn binary_op(kind: T) -> BinaryOp {
    match kind {
        T::PipePipe => BinaryOp::Or,
        T::AmpAmp => BinaryOp::And,
        T::EqEq => BinaryOp::Eq,
        T::BangEq => BinaryOp::Ne,
        T::Lt => BinaryOp::Lt,
        T::LtEq => BinaryOp::Le,
        T::Gt => BinaryOp::Gt,
        T::GtEq => BinaryOp::Ge,
        // `+%`, `-%` and `*%` are the same operators, wrapping.
        T::Plus | T::PlusPercent => BinaryOp::Add,
        T::Minus | T::MinusPercent => BinaryOp::Sub,
        T::Star | T::StarPercent => BinaryOp::Mul,
        T::Slash => BinaryOp::Div,
        T::Percent => BinaryOp::Rem,
        T::Amp => BinaryOp::BitAnd,
        T::Pipe => BinaryOp::BitOr,
        T::Caret => BinaryOp::BitXor,
        _ => unreachable!("not a binary operator: {kind:?}"),
    }
}

fn is_item_start(kind: T) -> bool {
    if matches!(kind, T::Import | T::Pub) {
        return true;
    }
    matches!(
        kind,
        T::Fn | T::Struct | T::Enum | T::Extern | T::Impl | T::Interface
    )
}

fn is_stmt_keyword(kind: T) -> bool {
    matches!(
        kind,
        T::Let
            | T::Val
            | T::Var
            | T::Defer
            | T::Return
            | T::While
            | T::For
            | T::Break
            | T::Continue
    )
}

/// Operators that can only continue an expression, never begin one. A line
/// that starts with one is E0113.
fn is_binary_only(kind: T) -> bool {
    matches!(
        kind,
        T::Plus
            | T::Star
            | T::Slash
            | T::Percent
            | T::EqEq
            | T::BangEq
            | T::Lt
            | T::LtEq
            | T::Gt
            | T::GtEq
            | T::AmpAmp
            | T::PipePipe
            | T::Pipe
            | T::Caret
            | T::Eq
            | T::PlusEq
            | T::MinusEq
            | T::StarEq
            | T::SlashEq
            | T::PercentEq
            | T::AmpEq
            | T::PipeEq
            | T::CaretEq
            | T::As
    )
}

fn is_ident(kind: T) -> bool {
    matches!(kind, T::Ident(_))
}

fn can_start_expr(kind: T) -> bool {
    matches!(
        kind,
        T::Int
            | T::Float
            | T::Str(_)
            | T::Char(_)
            | T::Byte(_)
            // The first piece of an interpolated literal.
            | T::StrStart(_)
            | T::True
            | T::False
            | T::Null
            | T::Ident(_)
            | T::LBracket
            | T::LBrace
            | T::If
            | T::Assert(_)
            | T::Match
            | T::LParen
            | T::Minus
            | T::Bang
            | T::Amp
            | T::AmpAmp
            | T::Move
            | T::Own
            | T::SelfKw
            | T::Lend
            | T::Yield
            // `.Variant`, whose enum comes from the expected type.
            | T::Dot
    )
}

fn can_start_pattern(kind: T) -> bool {
    matches!(
        kind,
        T::Ident(_)
            | T::Underscore
            | T::Dot
            | T::LParen
            // The elements of an array or a slice.
            | T::LBracket
            // A value the scrutinee must equal.
            | T::Int
            | T::Float
            | T::Minus
            | T::True
            | T::False
            | T::Str(_)
            | T::Char(_)
            | T::Byte(_)
            // A range with no lower end.
            | T::DotDot
            | T::DotDotEq
    )
}

/// What a bound of a range pattern may start with: a number, a
/// character, a byte, or a constant's name.
fn can_start_bound(kind: T) -> bool {
    matches!(
        kind,
        T::Int | T::Minus | T::Char(_) | T::Byte(_) | T::Ident(_)
    )
}

fn can_start_type(kind: T) -> bool {
    matches!(
        kind,
        T::Ident(_) | T::Own | T::Amp | T::AmpAmp | T::LBracket | T::LParen | T::Fn
    )
}

fn can_start_param(kind: T) -> bool {
    matches!(kind, T::Ident(_) | T::Underscore | T::DotDotDot)
}

/// Tokens a parenthesized list never skips past while recovering: they end
/// the statement or item the list is part of.
fn ends_list(kind: T) -> bool {
    matches!(kind, T::Semi | T::LBrace | T::Eof) || is_item_start(kind) || is_stmt_keyword(kind)
}

/// How a brace list separates its items (grammar §1.1).
struct BraceList<'w> {
    /// `,` for lists, `;` for extern blocks.
    sep: T,
    /// What one item is called: "a field".
    what: &'w str,
    /// What the items are called together, for E0112/E0104: "fields".
    items: &'w str,
    code: Code,
}

struct Parser<'a> {
    src: &'a str,
    /// Where this file's spans start.
    base: u32,
    tokens: &'a [Token],
    gaps: &'a [Span],
    pos: usize,
    ast: Ast,
    diagnostics: Vec<Diagnostic>,
    /// Where the statement or item being parsed begins, counting the
    /// whitespace before it. See [`Parser::report`].
    region_start: u32,
    /// Line breaks separate statements and end expressions here (grammar
    /// §1.1): true at the top level and inside `{ }`.
    newlines: bool,
    /// Set while parsing a condition, where struct literals are not allowed
    /// (grammar R1). Parentheses, brackets, braces and argument lists clear it.
    no_struct: bool,
    /// Every matched `{` … `}` so far, for guessing which brace is missing.
    brace_pairs: Vec<(Span, Span)>,
    last_error_at: Option<u32>,
    /// `...` in the parameter list being read, where it was written.
    variadic: Option<Span>,
    /// Lookahead calls since the parser last consumed a token. A parser bug
    /// that loops without consuming trips the assertion in `nth`.
    fuel: Cell<u32>,
    /// Nonzero inside a list of type arguments or parameters, where a `>=`
    /// closes the list with its first character: `val x: Option<i64>= y`.
    /// `>` and `>=` are one token each, as `>>` is two.
    type_args: u32,
    /// The `=` left over where a `>=` closed a list of type arguments.
    half_eq: Option<Span>,
}

impl<'a> Parser<'a> {
    // ---- tokens ----

    fn nth(&self, n: usize) -> T {
        let fuel = self.fuel.get() + 1;
        assert!(
            fuel < 50_000,
            "parser made no progress at token {}",
            self.pos
        );
        self.fuel.set(fuel);
        let kind = self.tokens.get(self.pos + n).map_or(T::Eof, |t| t.kind);
        if n > 0 {
            return kind;
        }
        // What is left of a `>=` that closed a list of type arguments, and
        // the `>` of one that is about to.
        match kind {
            _ if self.half_eq.is_some() => T::Eq,
            T::GtEq if self.type_args > 0 => T::Gt,
            kind => kind,
        }
    }

    fn peek(&self) -> T {
        self.nth(0)
    }

    /// The text of the token `n` ahead, for the words that are keywords
    /// only where they are written: `union` after `extern`.
    fn nth_text(&self, n: usize) -> &'a str {
        match self.tokens.get(self.pos + n) {
            Some(token) => self.text(token.span),
            None => "",
        }
    }

    fn at(&self, kind: T) -> bool {
        self.peek() == kind
    }

    fn span(&self) -> Span {
        match self.half_eq {
            Some(span) => span,
            None => self.tokens[self.pos].span,
        }
    }

    fn prev_span(&self) -> Span {
        if self.pos == 0 {
            Span::at(self.base)
        } else {
            self.tokens[self.pos - 1].span
        }
    }

    fn bump(&mut self) -> Span {
        // The `=` of a `>=` whose `>` closed a list of type arguments.
        if let Some(span) = self.half_eq.take() {
            self.pos += 1;
            self.fuel.set(0);
            return span;
        }
        let token = self.tokens[self.pos];
        if token.kind == T::GtEq && self.type_args > 0 {
            self.half_eq = Some(Span::new(token.span.lo + 1, token.span.hi));
            self.fuel.set(0);
            return Span::new(token.span.lo, token.span.lo + 1);
        }
        let span = self.span();
        if token.kind != T::Eof {
            self.pos += 1;
            self.fuel.set(0);
        }
        span
    }

    /// Parses a list of type arguments or parameters, where a `>=` closes the
    /// list with its first character.
    fn in_type_args<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.type_args += 1;
        let result = f(self);
        self.type_args -= 1;
        result
    }

    fn eat(&mut self, kind: T) -> bool {
        let found = self.at(kind);
        if found {
            self.bump();
        }
        found
    }

    /// Whether a significant line break comes before the token `n` ahead.
    fn line_break_at(&self, n: usize) -> bool {
        self.newlines && self.tokens.get(self.pos + n).is_none_or(|t| t.line_start)
    }

    /// Whether a significant line break comes before the current token.
    fn line_break(&self) -> bool {
        self.line_break_at(0)
    }

    /// A span's offset within this file's own text.
    fn local(&self, pos: u32) -> usize {
        (pos - self.base) as usize
    }

    fn text(&self, span: Span) -> &'a str {
        &self.src[self.local(span.lo)..self.local(span.hi)]
    }

    fn found(&self) -> String {
        match self.peek() {
            T::Eof => "end of file".to_string(),
            _ => format!("`{}`", self.text(self.span())),
        }
    }

    /// Whether the current token starts on a later line than the previous
    /// token ends (or is the end of file).
    fn on_later_line(&self) -> bool {
        let between = self.local(self.prev_span().hi)..self.local(self.span().lo);
        self.at(T::Eof) || self.src[between].contains('\n')
    }

    // ---- node construction ----

    fn alloc_expr(&mut self, kind: ExprKind, span: Span) -> ExprId {
        self.ast.exprs.alloc(Expr { kind, span })
    }

    fn alloc_stmt(&mut self, kind: StmtKind, span: Span) -> StmtId {
        self.ast.stmts.alloc(Stmt { kind, span })
    }

    fn alloc_type(&mut self, kind: TypeKind, span: Span) -> TypeId {
        self.ast.types.alloc(Type { kind, span })
    }

    /// An `Error` expression where something was expected but nothing was
    /// consumed.
    fn error_expr(&mut self) -> ExprId {
        let span = Span::at(self.prev_span().hi);
        self.alloc_expr(ExprKind::Error, span)
    }

    fn expr_span(&self, id: ExprId) -> Span {
        self.ast.exprs[id].span
    }

    /// Runs `f` with line breaks significant or not, and struct literals
    /// allowed or not, restoring both afterwards.
    fn in_context<R>(
        &mut self,
        newlines: bool,
        no_struct: bool,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let saved = (self.newlines, self.no_struct);
        (self.newlines, self.no_struct) = (newlines, no_struct);
        let result = f(self);
        (self.newlines, self.no_struct) = saved;
        result
    }

    /// Inside `( )` and `[ ]`: line breaks are whitespace, struct literals are
    /// allowed.
    fn in_parens<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.in_context(false, false, f)
    }

    /// Inside `{ }`: line breaks are significant, struct literals are allowed.
    fn in_braces<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.in_context(true, false, f)
    }

    /// Parses a condition: line breaks are whitespace up to its `{`, and a
    /// struct literal may not appear at its top level.
    fn cond(&mut self) -> ExprId {
        self.in_context(false, true, |p| p.expr())
    }

    /// A parameter's name, or `_` for one that is not used.
    fn param_name(&mut self, what: &str) -> Option<Name> {
        if self.at(T::Underscore) {
            let span = self.bump();
            return Some(Name {
                sym: Symbol::ignored(),
                span,
            });
        }
        self.name(what)
    }

    fn name(&mut self, what: &str) -> Option<Name> {
        if let T::Ident(sym) = self.peek() {
            let span = self.bump();
            return Some(Name { sym, span });
        }
        let mut diagnostic = self.expected(what);
        if let Some(keyword) = self
            .peek()
            .text()
            .filter(|t| t.chars().all(|c| c.is_ascii_alphabetic()))
        {
            diagnostic = diagnostic.with_help(format!(
                "`{keyword}` is a keyword and cannot be used as a name"
            ));
        }
        self.report(diagnostic);
        None
    }
}
