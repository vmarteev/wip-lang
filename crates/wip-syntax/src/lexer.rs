//! Source text to tokens.
//!
//! Lexing never stops at an error. Every malformed construct is reported and
//! still produces the token the parser most likely expects, so one run reports
//! every lexical error in the file and parsing can continue past them. The
//! rules are in `docs/grammar.md` §1.

use crate::diagnostic::{Diagnostic, Edit};
use crate::token::{Token, TokenKind};
use crate::{Interner, Span, Symbol};

use crate::codes;

pub struct Lexed {
    /// Ends with exactly one [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
    /// Where the lexer dropped text (unexpected characters) or cut a token
    /// short (unterminated strings), in source order. The parser stays quiet
    /// about errors next to these, since they are almost always a consequence
    /// of the lexical error rather than a separate mistake.
    pub gaps: Vec<Span>,
}

impl Lexed {
    /// Renames every symbol through `map`, once the interner these tokens
    /// were lexed with has been absorbed into another
    /// ([`Interner::absorb`]).
    pub fn rename_symbols(&mut self, map: &[Symbol]) {
        for token in &mut self.tokens {
            if let TokenKind::Assert(Some(sym)) = &mut token.kind {
                *sym = map[sym.index()];
            } else if let TokenKind::Ident(sym)
            | TokenKind::Str(sym)
            | TokenKind::StrStart(sym)
            | TokenKind::StrMid(sym)
            | TokenKind::StrEnd(sym) = &mut token.kind
            {
                *sym = map[sym.index()];
            }
        }
    }
}

/// Lexes a whole source file.
///
/// # Panics
///
/// If `src` is longer than `u32::MAX` bytes. The driver rejects such files
/// before lexing.
pub fn lex(src: &str, interner: &mut Interner) -> Lexed {
    lex_at(src, 0, interner)
}

/// Lexes a file whose spans start at `base`, so that the files of one program
/// lie end to end and a `Span` names a place in exactly one of them.
pub fn lex_at(src: &str, base: u32, interner: &mut Interner) -> Lexed {
    assert!(
        u32::try_from(src.len()).is_ok(),
        "source file larger than 4 GiB"
    );
    let mut lexer = Lexer {
        src,
        base: base as usize,
        // A UTF-8 byte order mark is not part of the program.
        pos: if src.starts_with('\u{feff}') { 3 } else { 0 },
        interner,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
        gaps: Vec::new(),
        buf: String::new(),
        line_start: true,
        interp: Vec::new(),
    };
    lexer.run();
    lexer.intern_assert_messages();
    Lexed {
        tokens: lexer.tokens,
        diagnostics: lexer.diagnostics,
        gaps: lexer.gaps,
    }
}

struct Lexer<'src, 'i> {
    src: &'src str,
    /// Where this file's spans start.
    base: usize,
    pos: usize,
    interner: &'i mut Interner,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    gaps: Vec<Span>,
    /// Scratch space for the contents of the current string literal.
    buf: String,
    /// No token has been pushed since the last line break.
    line_start: bool,
    /// One entry per interpolation being lexed, holding how many `(` are
    /// open inside it — the `)` that closes it is the one found at zero
    /// — and the text block the literal is, if it is one.
    interp: Vec<(u32, Option<Block>)>,
}

/// A text block being lexed: the line its closing `"""`
/// is on begins at `line`, and the `"""` at `close`. What is before it
/// on that line is the indentation taken off every line of the text.
#[derive(Clone, Copy)]
struct Block {
    line: usize,
    close: usize,
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether `c` can begin a token or trivia. Anything else is an unexpected
/// character.
fn starts_token(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_\"/{}()[],:;.-=!<>&|+*%".contains(c) || is_space(c)
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

impl Lexer<'_, '_> {
    fn run(&mut self) {
        loop {
            self.skip_trivia();
            let start = self.pos;
            let Some(c) = self.peek_char() else { break };
            // `b'x'`: a byte. An identifier is never
            // followed directly by a `'`, so the two cannot be confused.
            if c == 'b' && self.byte(1) == Some(b'\'') {
                self.byte_literal(start);
            } else if c.is_ascii_alphabetic() || c == '_' {
                self.ident(start);
            } else if c.is_ascii_digit() {
                self.number(start);
            } else if c == '"' && self.src[self.pos..].starts_with("\"\"\"") {
                self.text_block(start);
            } else if c == '"' {
                self.string(start);
            } else if c == '\'' {
                let value = self.quoted(start, false);
                self.push(TokenKind::Char(value), start, self.pos);
            } else if c == ')' && self.interp.last().is_some_and(|&(depth, _)| depth == 0) {
                // The `)` that closes `\(`: what follows is the rest of the
                // literal, not a token of its own.
                let (_, block) = self.interp.pop().expect("just checked");
                self.pos += 1;
                self.string_piece(start, false, block);
            } else if !self.punct(start) {
                self.unexpected(start);
            } else if let Some((depth, _)) = self.interp.last_mut() {
                // Parentheses inside an interpolation, so that the one that
                // ends it is known.
                match self.tokens.last().map(|token| token.kind) {
                    Some(TokenKind::LParen) => *depth += 1,
                    Some(TokenKind::RParen) => *depth -= 1,
                    _ => {}
                }
            }
        }
        let end = (self.base + self.src.len()) as u32;
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::at(end),
            line_start: true,
        });
    }

    fn byte(&self, ahead: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + ahead).copied()
    }

    fn peek_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn eat_while(&mut self, pred: impl Fn(u8) -> bool) {
        while self.byte(0).is_some_and(&pred) {
            self.pos += 1;
        }
    }

    /// The message each `assert` in this file prints when it fails.
    /// It is made here because the lexer is what holds the
    /// interner: the parser, which reads these tokens, cannot intern the
    /// text it sees.
    fn intern_assert_messages(&mut self) {
        for index in 0..self.tokens.len() {
            if self.tokens[index].kind != TokenKind::Assert(None) {
                continue;
            }
            let Some(message) = self.assert_message(index) else {
                continue;
            };
            let message = self.interner.intern(&message);
            self.tokens[index].kind = TokenKind::Assert(Some(message));
        }
    }

    /// The message of the `assert` whose keyword is the token at `at`:
    /// `"<note>: <condition>"` where the call gives a note written as one
    /// string, `": <condition>"` after a note built when it fails, and
    /// `"assertion failed: <condition>"` where it gives none. `None` where what
    /// follows is not `(` … `)`, which the parser reports.
    fn assert_message(&self, at: usize) -> Option<String> {
        if self.tokens.get(at + 1)?.kind != TokenKind::LParen {
            return None;
        }
        let open = self.tokens[at + 1].span.hi;
        // The condition runs to the closing parenthesis, or to the comma
        // before the note. Nesting is counted so that a call's own commas
        // and parentheses belong to the condition.
        let (mut depth, mut comma, mut close) = (0usize, None, None);
        for (index, token) in self.tokens.iter().enumerate().skip(at + 2) {
            match token.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen if depth == 0 => {
                    close = Some(index);
                    break;
                }
                // A closer with no opener is a file with a mistake in it,
                // which the parser reports; the message stops at it.
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Comma if depth == 0 && comma.is_none() => comma = Some(index),
                TokenKind::Eof => break,
                _ => {}
            }
        }
        let close = close?;
        let condition = self.source(open, self.tokens[comma.unwrap_or(close)].span.lo);
        // A note is a string written at the call, as a panic's message is;
        // anything else is reported by the checker, and
        // the message is the condition alone.
        let note = match comma {
            Some(comma) if comma + 2 == close => match self.tokens[comma + 1].kind {
                TokenKind::Str(note) => Some(self.interner.resolve(note)),
                _ => None,
            },
            _ => None,
        };
        // A note built when the assert fails comes first, and this after it.
        Some(match (note, comma) {
            (Some(note), _) => format!("{note}: {condition}"),
            (None, Some(_)) => format!(": {condition}"),
            (None, None) => format!("assertion failed: {condition}"),
        })
    }

    /// The source text from `lo` to `hi`, on one line: a condition written
    /// over several lines reads as one in the message.
    fn source(&self, lo: u32, hi: u32) -> String {
        let lo = (lo as usize).saturating_sub(self.base);
        let hi = (hi as usize).saturating_sub(self.base);
        let text = self.src.get(lo..hi).unwrap_or_default();
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn span(&self, lo: usize, hi: usize) -> Span {
        Span::new((self.base + lo) as u32, (self.base + hi) as u32)
    }

    fn push(&mut self, kind: TokenKind, lo: usize, hi: usize) {
        let span = self.span(lo, hi);
        let line_start = std::mem::replace(&mut self.line_start, false);
        self.tokens.push(Token {
            kind,
            span,
            line_start,
        });
    }

    fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.byte(0) {
                Some(b'\n') => {
                    self.pos += 1;
                    self.line_start = true;
                }
                Some(b' ' | b'\t' | b'\r') => self.pos += 1,
                Some(b'/') if self.byte(1) == Some(b'/') => {
                    self.pos = match self.src[self.pos..].find('\n') {
                        Some(offset) => self.pos + offset,
                        None => self.src.len(),
                    };
                }
                _ => break,
            }
        }
    }

    fn ident(&mut self, start: usize) {
        self.eat_while(is_ident_continue);
        let text = &self.src[start..self.pos];
        let kind = if text == "_" {
            TokenKind::Underscore
        } else if let Some(keyword) = TokenKind::keyword(text) {
            keyword
        } else {
            TokenKind::Ident(self.interner.intern(text))
        };
        self.push(kind, start, self.pos);
    }

    fn number(&mut self, start: usize) {
        // `0x`, `0o` and `0b`.
        if self.byte(0) == Some(b'0') {
            let base = match self.byte(1) {
                Some(b'x') => Some(16),
                Some(b'o') => Some(8),
                Some(b'b') => Some(2),
                _ => None,
            };
            if let Some(base) = base {
                self.pos += 2;
                self.based_number(start, base);
                return;
            }
        }
        self.digits(|b| b.is_ascii_digit());
        let int_end = self.pos;
        let int_text = &self.src[start..int_end];
        if int_text.len() > 1 && int_text.starts_with('0') {
            // Keep at least one digit: `00` becomes `0`, `007` becomes `7`,
            // and `0_1` becomes `1`.
            let zeros = int_text[..int_text.len() - 1]
                .bytes()
                .take_while(|&b| b == b'0' || b == b'_')
                .count();
            let fixed = &self.src[start + zeros..int_end];
            let diagnostic = Diagnostic::error(
                codes::LEADING_ZERO,
                "number literal has a leading zero",
                self.span(start, int_end),
                "leading zero",
            )
            .with_note("in C a leading zero makes a literal octal; Wip rejects it so the literal means the same thing to every reader, and writes octal `0o755`")
            .with_fix(
                format!("remove the leading zeros: `{fixed}`"),
                [Edit::replace(self.span(start, start + zeros), "")],
            );
            self.report(diagnostic);
        }

        let mut is_float = false;
        if self.byte(0) == Some(b'.') {
            match self.byte(1) {
                Some(b) if b.is_ascii_digit() => {
                    self.pos += 1;
                    self.digits(|b| b.is_ascii_digit());
                    is_float = true;
                }
                // `1.foo` is an integer followed by a field access.
                Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
                // `1..2` is an integer followed by `..`.
                Some(b'.') => {}
                _ => {
                    self.pos += 1;
                    is_float = true;
                    let digits = &self.src[start..self.pos];
                    let diagnostic = Diagnostic::error(
                        codes::FLOAT_MISSING_FRACTION,
                        "expected a digit after the decimal point",
                        self.span(start, self.pos),
                        "float literal needs a fractional part",
                    )
                    .with_fix(
                        format!("add a zero: `{digits}0`"),
                        [Edit::insert(self.pos as u32, "0")],
                    );
                    self.report(diagnostic);
                }
            }
        }

        if matches!(self.byte(0), Some(b'e' | b'E')) {
            let first_digit = if matches!(self.byte(1), Some(b'+' | b'-')) {
                2
            } else {
                1
            };
            if self.byte(first_digit).is_some_and(|b| b.is_ascii_digit()) {
                self.pos += first_digit;
                self.digits(|b| b.is_ascii_digit());
                is_float = true;
            }
        }

        let end = self.pos;
        self.suffix(start, end, true);
        let kind = if is_float {
            TokenKind::Float
        } else {
            TokenKind::Int
        };
        self.push(kind, start, end);
    }

    /// The digits of a number after `0x`, `0o` or `0b`.
    fn based_number(&mut self, start: usize, base: u32) {
        let digits_start = self.pos;
        self.digits(|b| char::from(b).is_digit(base));
        let prefix = &self.src[start..digits_start];
        if base < 10 && self.byte(0).is_some_and(|b| b.is_ascii_digit()) {
            // A decimal digit the base does not allow: `0b102`, `0o9`.
            let bad = self.pos;
            self.eat_while(|b| b.is_ascii_digit() || b == b'_');
            let (name, digits) = if base == 2 {
                ("a binary", "binary digits are 0 and 1")
            } else {
                ("an octal", "octal digits are 0 to 7")
            };
            let diagnostic = Diagnostic::error(
                codes::NUMBER_BASE,
                format!("invalid digit in {name} literal"),
                self.span(bad, self.pos),
                format!("not {name} digit"),
            )
            .with_note(digits);
            self.report(diagnostic);
        } else if self.pos == digits_start {
            self.eat_while(is_ident_continue);
            let diagnostic = Diagnostic::error(
                codes::NUMBER_BASE,
                format!("`{prefix}` needs digits after it"),
                self.span(start, self.pos),
                "no digits",
            )
            .with_help(format!("for example `{prefix}0`"));
            self.report(diagnostic);
        }
        let end = self.pos;
        self.suffix(start, end, false);
        self.push(TokenKind::Int, start, end);
    }

    /// Digits that satisfy `is_digit`, with single `_` separators between
    /// them. A run of several `_` is reported, with a fix
    /// that keeps one. A `_` with no digit after it is not part of the
    /// number; [`Lexer::suffix`] reports it.
    fn digits(&mut self, is_digit: impl Fn(u8) -> bool) {
        loop {
            match self.byte(0) {
                Some(b) if is_digit(b) => self.pos += 1,
                Some(b'_') if self.pos > 0 && is_digit(self.src.as_bytes()[self.pos - 1]) => {
                    let run = self.src.as_bytes()[self.pos..]
                        .iter()
                        .take_while(|&&b| b == b'_')
                        .count();
                    if !self.byte(run).is_some_and(&is_digit) {
                        return;
                    }
                    if run > 1 {
                        let span = self.span(self.pos, self.pos + run);
                        let diagnostic = Diagnostic::error(
                            codes::DIGIT_SEPARATOR,
                            "digits are separated by one `_`",
                            span,
                            "more than one `_`",
                        )
                        .with_fix("keep one `_`", [Edit::replace(span, "_")]);
                        self.report(diagnostic);
                    }
                    self.pos += run;
                }
                _ => return,
            }
        }
    }

    /// Reports identifier characters that run into the number ending at
    /// `end`: `12abc`, `3e`, `1_`.
    fn suffix(&mut self, start: usize, end: usize, decimal: bool) {
        if !self.byte(0).is_some_and(is_ident_continue) {
            return;
        }
        self.eat_while(is_ident_continue);
        let suffix = &self.src[end..self.pos];
        if suffix.bytes().all(|b| b == b'_') {
            let span = self.span(end, self.pos);
            let diagnostic = Diagnostic::error(
                codes::DIGIT_SEPARATOR,
                "a number cannot end with `_`",
                span,
                "not between two digits",
            )
            .with_fix("remove it", [Edit::replace(span, "")]);
            self.report(diagnostic);
            return;
        }
        let mut diagnostic = Diagnostic::error(
            codes::NUMBER_SUFFIX,
            format!("invalid suffix `{suffix}` on number literal"),
            self.span(end, self.pos),
            "not part of the number",
        );
        diagnostic = if decimal && suffix.eq_ignore_ascii_case("e") {
            let digits = &self.src[start..end];
            diagnostic.with_fix(
                format!("an exponent needs digits: `{digits}{suffix}0`"),
                [Edit::insert(self.pos as u32, "0")],
            )
        } else {
            diagnostic.with_help("a number cannot run into a name; separate them with an operator")
        };
        self.report(diagnostic);
    }

    fn string(&mut self, start: usize) {
        self.pos += 1;
        self.string_piece(start, true, None);
    }

    /// A text block: `"""` at the end of a line, the lines
    /// of the text, and `"""` first on a line of its own. The closing
    /// `"""`'s indentation is taken off every line, and the line breaks
    /// after the opening and before the closing are not part of the text.
    fn text_block(&mut self, start: usize) {
        self.pos += 3;
        let line_end = self.src[self.pos..]
            .find('\n')
            .map_or(self.src.len(), |at| self.pos + at);
        let after = &self.src[self.pos..line_end];
        if !after.trim_matches([' ', '\t', '\r']).is_empty() {
            let diagnostic = Diagnostic::error(
                codes::TEXT_BLOCK,
                "a text block begins on the line after its `\"\"\"`",
                self.span(self.pos, line_end),
                "text on the opening line",
            )
            .with_note("a text block is the lines between its `\"\"\"`s; one line of text is a string, `\"…\"`")
            .with_fix(
                "start the text on the next line",
                [Edit::insert(self.pos as u32, "\n")],
            );
            self.report(diagnostic);
        }
        // The closing `"""` is the first that begins a line, after the
        // line's indentation.
        let mut line = line_end + 1;
        let mut found = None;
        while line <= self.src.len() {
            let rest = &self.src[line..];
            let indent = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            if rest[indent..].starts_with("\"\"\"") {
                found = Some(Block {
                    line,
                    close: line + indent,
                });
                break;
            }
            match rest.find('\n') {
                Some(at) => line += at + 1,
                None => break,
            }
        }
        let Some(block) = found else {
            self.gaps.push(Span::at(self.src.len() as u32));
            let diagnostic = Diagnostic::error(
                codes::UNTERMINATED_STRING,
                "unterminated text block",
                self.span(start, start + 3),
                "this text block is never closed",
            )
            .with_help("end it with `\"\"\"` at the start of a line of its own, after the line's indentation");
            self.report(diagnostic);
            self.pos = self.src.len();
            let symbol = self.interner.intern("");
            self.push(TokenKind::Str(symbol), start, self.pos);
            return;
        };
        if line_end + 1 == block.line {
            // Nothing between the two: the empty text.
            self.pos = block.close + 3;
            let symbol = self.interner.intern("");
            self.push(TokenKind::Str(symbol), start, self.pos);
            return;
        }
        self.pos = line_end + 1;
        self.buf.clear();
        self.block_indentation(block);
        self.block_piece(start, true, block);
    }

    /// At the start of a line of a text block: steps over the indentation
    /// the closing `"""` has, which is not part of the text. A line of
    /// nothing but space is an empty one; any other must begin with that
    /// indentation.
    fn block_indentation(&mut self, block: Block) {
        let indent = &self.src[block.line..block.close];
        let rest = &self.src[self.pos..];
        if rest.starts_with(indent) {
            self.pos += indent.len();
            return;
        }
        let space = rest.len() - rest.trim_start_matches([' ', '\t', '\r']).len();
        let blank = matches!(rest[space..].chars().next(), None | Some('\n'));
        if !blank {
            let diagnostic = Diagnostic::error(
                codes::TEXT_BLOCK,
                "a line of a text block is indented less than its closing `\"\"\"`",
                self.span(self.pos, self.pos + space),
                "less than the closing `\"\"\"`",
            )
            .with_note("the closing `\"\"\"`'s indentation is taken off every line of the text, so each line begins with it");
            self.report(diagnostic);
        }
        self.pos += space;
    }

    /// One piece of a text block, as `string_piece` is of a string: up to
    /// `\(`, or to the closing `"""`. Space at the end of a line is not
    /// part of the text; an escape, `\t` or `\u{20}`, keeps it.
    fn block_piece(&mut self, start: usize, first: bool, block: Block) {
        // Where the text of this line begins in `buf`, or ends what an
        // escape wrote: only the space after it is taken off.
        let mut kept = self.buf.len();
        let mut interpolates = false;
        loop {
            if self.pos >= block.close {
                // An interpolation ran past the end of the block.
                self.unterminated(start);
                break;
            }
            match self.peek_char() {
                Some('\n') => {
                    while self.buf.len() > kept && self.buf.ends_with([' ', '\t', '\r']) {
                        self.buf.pop();
                    }
                    if self.pos + 1 == block.line {
                        self.pos = block.close + 3;
                        break;
                    }
                    self.buf.push('\n');
                    self.pos += 1;
                    self.block_indentation(block);
                    kept = self.buf.len();
                }
                Some('"') if self.src[self.pos..].starts_with("\"\"\"") => {
                    let diagnostic = Diagnostic::error(
                        codes::TEXT_BLOCK,
                        "a text block cannot hold `\"\"\"`",
                        self.span(self.pos, self.pos + 3),
                        "three quotes",
                    )
                    .with_fix(
                        "escape the first quote: `\\\"\"\"`",
                        [Edit::insert(self.pos as u32, "\\")],
                    );
                    self.report(diagnostic);
                    self.buf.push_str("\"\"\"");
                    self.pos += 3;
                }
                Some('\\') if self.byte(1) == Some(b'(') => {
                    self.pos += 2;
                    interpolates = true;
                    break;
                }
                Some('\\') if matches!(self.byte(1), Some(b'\n' | b'\r')) => {
                    let diagnostic = Diagnostic::error(
                        codes::TEXT_BLOCK,
                        "a backslash ends this line of a text block",
                        self.span(self.pos, self.pos + 1),
                        "escapes nothing",
                    )
                    .with_help("the line break is part of the text; write `\\\\` for a backslash");
                    self.report(diagnostic);
                    self.pos += 1;
                }
                Some('\\') => {
                    self.escape();
                    kept = self.buf.len();
                }
                Some(c) => {
                    self.buf.push(c);
                    self.pos += c.len_utf8();
                }
                None => {
                    self.unterminated(start);
                    break;
                }
            }
        }
        let symbol = self.interner.intern(&self.buf);
        self.buf.clear();
        let kind = match (first, interpolates) {
            (true, false) => TokenKind::Str(symbol),
            (true, true) => TokenKind::StrStart(symbol),
            (false, true) => TokenKind::StrMid(symbol),
            (false, false) => TokenKind::StrEnd(symbol),
        };
        self.push(kind, start, self.pos);
        if interpolates {
            self.interp.push((0, Some(block)));
        }
    }

    /// One piece of a literal: the text up to `\(`, which begins an
    /// interpolation, or up to the closing quote, which ends the literal.
    /// `first` says whether the piece begins the literal,
    /// which is what tells a plain literal from an interpolated one; a
    /// piece of a text block is the block's own.
    fn string_piece(&mut self, start: usize, first: bool, block: Option<Block>) {
        self.buf.clear();
        if let Some(block) = block {
            self.block_piece(start, first, block);
            return;
        }
        let mut interpolates = false;
        loop {
            match self.peek_char() {
                Some('"') => {
                    self.pos += 1;
                    break;
                }
                None | Some('\n') => {
                    self.unterminated(start);
                    break;
                }
                // `\(`: the text so far ends here, and an expression
                // follows.
                Some('\\') if self.byte(1) == Some(b'(') => {
                    self.pos += 2;
                    interpolates = true;
                    break;
                }
                Some('\\') => self.escape(),
                Some(c) => {
                    self.buf.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        let symbol = self.interner.intern(&self.buf);
        let kind = match (first, interpolates) {
            (true, false) => TokenKind::Str(symbol),
            (true, true) => TokenKind::StrStart(symbol),
            (false, true) => TokenKind::StrMid(symbol),
            (false, false) => TokenKind::StrEnd(symbol),
        };
        self.push(kind, start, self.pos);
        if interpolates {
            self.interp.push((0, None));
        }
    }

    fn escape(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let value = match self.peek_char() {
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some('0') => '\0',
            Some('\\') => '\\',
            Some('"') => '"',
            Some('\'') => '\'',
            // `\u{1F600}`: a character by its number.
            Some('u') if self.byte(1) == Some(b'{') => {
                self.unicode_escape(start);
                return;
            }
            // A backslash at the end of the line or file: the string loop
            // reports the literal as unterminated.
            None | Some('\n') => return,
            Some(other) => {
                self.pos += other.len_utf8();
                let diagnostic = Diagnostic::error(
                    codes::UNKNOWN_ESCAPE,
                    format!("unknown escape sequence `\\{other}`"),
                    self.span(start, self.pos),
                    "unknown escape",
                )
                .with_help(
                    r#"the escapes are `\n`, `\t`, `\r`, `\0`, `\\`, `\"`, `\'` and `\u{…}`"#,
                )
                .with_fix(
                    format!("to write a backslash, escape it: `\\\\{other}`"),
                    [Edit::insert(start as u32, "\\")],
                );
                self.report(diagnostic);
                self.buf.push('\\');
                self.buf.push(other);
                return;
            }
        };
        self.pos += 1;
        self.buf.push(value);
    }

    /// `\u{1F600}`, with `self.pos` at the `u`: one to six hex digits,
    /// naming a Unicode scalar value.
    fn unicode_escape(&mut self, start: usize) {
        self.pos += 2;
        let digits_start = self.pos;
        while self.byte(0).is_some_and(|b| b.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        let digits = &self.src[digits_start..self.pos];
        let closed = self.byte(0) == Some(b'}');
        if closed {
            self.pos += 1;
        }
        let value = u32::from_str_radix(digits, 16).ok();
        let scalar = value
            .filter(|_| closed && digits.len() <= 6)
            .and_then(char::from_u32);
        match scalar {
            Some(c) => self.buf.push(c),
            None => {
                let diagnostic = Diagnostic::error(
                    codes::UNKNOWN_ESCAPE,
                    "`\\u{…}` needs a Unicode scalar value",
                    self.span(start, self.pos),
                    if !closed || digits.is_empty() {
                        "one to six hex digits, then `}`"
                    } else {
                        "not a character: past 10FFFF, or a surrogate"
                    },
                )
                .with_help("write the character's number in hex: `\\u{e9}` is `é`");
                self.report(diagnostic);
                self.buf.push(char::REPLACEMENT_CHARACTER);
            }
        }
    }

    /// `b'x'`: the byte of one ASCII character, read as a character is.
    /// One past `7F` is an error that says how a byte
    /// that is not a character is written.
    fn byte_literal(&mut self, start: usize) {
        self.pos += 1;
        let value = self.quoted(start + 1, true);
        let byte = match u8::try_from(value) {
            Ok(byte) if byte.is_ascii() => byte,
            _ => {
                let escaped = self.src[start..self.pos].contains('\\');
                let mut diagnostic = Diagnostic::error(
                    codes::BYTE_LITERAL,
                    "a byte literal holds one ASCII character",
                    self.span(start, self.pos),
                    "not ASCII",
                );
                diagnostic = match char::from_u32(value) {
                    // `b'é'`: a character, whose UTF-8 is more than one
                    // byte.
                    Some(c) if !escaped => diagnostic.with_help(format!(
                        "`'{c}'` is a character, a `char`; in UTF-8 it is {} bytes",
                        c.len_utf8()
                    )),
                    _ if value <= 0xFF => diagnostic.with_help(format!(
                        "a byte past 127 is written as its number: `0x{value:02X}`"
                    )),
                    _ => diagnostic.with_help(
                        "a byte is a number up to 255, and past 127 it is written as one: `0xE9`",
                    ),
                };
                self.report(diagnostic.with_note(
                    "`b'x'` is the byte of an ASCII character, which is what every byte below 128 is in UTF-8",
                ));
                0
            }
        };
        self.push(TokenKind::Byte(byte), start, self.pos);
    }

    /// `'x'`: one character, with `quote` at the `'`, or
    /// the `'x'` of a byte literal, `b'x'`, which is written the same way.
    /// An escape is read as in a string. What holds none,
    /// or more than one, is an error that says which, and lexing continues
    /// with what was there.
    fn quoted(&mut self, quote: usize, byte: bool) -> u32 {
        let start = quote;
        // The whole literal, `b` included, and what it is called.
        let (whole, what) = match byte {
            true => (quote - 1, "a byte literal"),
            false => (quote, "a character literal"),
        };
        self.pos += 1;
        self.buf.clear();
        let mut closed = false;
        loop {
            match self.peek_char() {
                Some('\'') => {
                    self.pos += 1;
                    closed = true;
                    break;
                }
                None | Some('\n') => {
                    let diagnostic = Diagnostic::error(
                        codes::SINGLE_QUOTES,
                        format!("unterminated {}", &what[2..]),
                        self.span(whole, start + 1),
                        "this character is never closed",
                    )
                    .with_help("add the closing `'`; text of more than one character is written in double quotes");
                    self.report(diagnostic);
                    break;
                }
                Some('\\') => self.escape(),
                Some(c) => {
                    self.buf.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        let mut chars = self.buf.chars();
        match (chars.next(), chars.next()) {
            // Reported as unterminated already.
            (first, _) if !closed => first.map_or(0, |c| c as u32),
            (Some(c), None) => c as u32,
            (None, _) => {
                let diagnostic = Diagnostic::error(
                    codes::SINGLE_QUOTES,
                    format!("{what} holds one character"),
                    self.span(whole, self.pos),
                    "holds none",
                )
                .with_help(match byte {
                    true => "`b' '` is a space",
                    false => "`' '` is a space, and `\"\"` the empty text",
                });
                self.report(diagnostic);
                0
            }
            (Some(first), Some(_)) => {
                let contents = &self.src[start + 1..self.pos.saturating_sub(1)];
                let mut diagnostic = Diagnostic::error(
                    codes::SINGLE_QUOTES,
                    format!("{what} holds one character"),
                    self.span(whole, self.pos),
                    "holds more than one",
                )
                .with_note(match byte {
                    true => "a byte literal is the byte of one ASCII character; text, which is bytes, is written in double quotes",
                    false => "a character is one Unicode scalar value; a letter written with a mark after it may be two",
                });
                if !contents.contains('"') && self.src[..self.pos].ends_with('\'') {
                    diagnostic = diagnostic.with_fix(
                        format!("write text in double quotes: `\"{contents}\"`"),
                        [
                            Edit::replace(self.span(whole, start + 1), "\""),
                            Edit::replace(self.span(self.pos - 1, self.pos), "\""),
                        ],
                    );
                }
                self.report(diagnostic);
                first as u32
            }
        }
    }

    fn unterminated(&mut self, start: usize) {
        self.gaps.push(Span::at(self.pos as u32));
        let at_eof = self.pos == self.src.len();
        let diagnostic = Diagnostic::error(
            codes::UNTERMINATED_STRING,
            "unterminated string literal",
            self.span(start, start + 1),
            "this string is never closed",
        )
        .with_help(if at_eof {
            "add the closing `\"`"
        } else {
            "add the closing `\"`; a string cannot span lines, so write `\\n` for a newline"
        });
        self.report(diagnostic);
    }

    /// Lexes a punctuation token at `start`. Returns false if the character
    /// there is not punctuation.
    fn punct(&mut self, start: usize) -> bool {
        use TokenKind::*;
        let Some(b) = self.byte(0) else { return false };
        let (kind, len) = match (b, self.byte(1)) {
            (b'{', _) => (LBrace, 1),
            (b'}', _) => (RBrace, 1),
            (b'(', _) => (LParen, 1),
            (b')', _) => (RParen, 1),
            (b'[', _) => (LBracket, 1),
            (b']', _) => (RBracket, 1),
            (b',', _) => (Comma, 1),
            (b';', _) => (Semi, 1),
            (b'.', Some(b'.')) if self.byte(2) == Some(b'.') => (DotDotDot, 3),
            (b'.', Some(b'.')) if self.byte(2) == Some(b'=') => (DotDotEq, 3),
            (b'.', Some(b'.')) => (DotDot, 2),
            (b'.', _) => (Dot, 1),
            (b'+', Some(b'=')) => (PlusEq, 2),
            // `+%`, `-%` and `*%` wrap rather than panic.
            (b'+', Some(b'%')) => (PlusPercent, 2),
            (b'+', _) => (Plus, 1),
            (b'*', Some(b'%')) => (StarPercent, 2),
            (b'*', Some(b'=')) => (StarEq, 2),
            (b'*', _) => (Star, 1),
            (b'/', Some(b'=')) => (SlashEq, 2),
            (b'/', _) => (Slash, 1),
            (b'%', Some(b'=')) => (PercentEq, 2),
            (b'%', _) => (Percent, 1),
            (b':', Some(b':')) => (ColonColon, 2),
            (b':', _) => (Colon, 1),
            (b'-', Some(b'>')) => (Arrow, 2),
            (b'-', Some(b'=')) => (MinusEq, 2),
            (b'-', Some(b'%')) => (MinusPercent, 2),
            (b'-', _) => (Minus, 1),
            (b'=', Some(b'=')) => (EqEq, 2),
            (b'=', Some(b'>')) => (FatArrow, 2),
            (b'=', _) => (Eq, 1),
            (b'!', Some(b'=')) => (BangEq, 2),
            (b'!', _) => (Bang, 1),
            (b'<', Some(b'=')) => (LtEq, 2),
            (b'<', _) => (Lt, 1),
            (b'>', Some(b'=')) => (GtEq, 2),
            (b'>', _) => (Gt, 1),
            (b'&', Some(b'&')) => (AmpAmp, 2),
            (b'&', Some(b'=')) => (AmpEq, 2),
            (b'&', _) => (Amp, 1),
            (b'|', Some(b'|')) => (PipePipe, 2),
            (b'|', Some(b'=')) => (PipeEq, 2),
            (b'|', _) => (Pipe, 1),
            (b'?', _) => (Question, 1),
            (b'@', _) => (At, 1),
            (b'^', Some(b'=')) => (CaretEq, 2),
            (b'^', _) => (Caret, 1),
            (b'~', _) => {
                let diagnostic = Diagnostic::error(
                    codes::TILDE,
                    "`~` is not an operator",
                    self.span(start, start + 1),
                    "not an operator",
                )
                .with_note("`!` flips every bit of an integer, as it negates a `bool`")
                .with_fix(
                    "write `!`",
                    [Edit::replace(self.span(start, start + 1), "!")],
                );
                self.report(diagnostic);
                (Bang, 1)
            }
            _ => return false,
        };
        self.pos += len;
        self.push(kind, start, self.pos);
        true
    }

    /// Reports a run of characters that cannot start a token, as one
    /// diagnostic, and skips it.
    fn unexpected(&mut self, start: usize) {
        while let Some(c) = self.peek_char() {
            if self.pos > start && starts_token(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        let text = &self.src[start..self.pos];
        let span = self.span(start, self.pos);
        self.gaps.push(span);
        let shown: String = text
            .chars()
            .map(|c| {
                if c.is_control() || c.is_whitespace() {
                    format!("\\u{{{:04x}}}", c as u32)
                } else {
                    c.to_string()
                }
            })
            .collect();
        let mut chars = text.chars();
        let first = chars.next().expect("unexpected() called at end of input");
        let single = chars.next().is_none();
        let message = if single {
            format!("unexpected character `{shown}`")
        } else {
            format!("unexpected characters `{shown}`")
        };
        let mut diagnostic = Diagnostic::error(
            codes::UNEXPECTED_CHAR,
            message,
            span,
            "not part of the language",
        );
        if first.is_whitespace() && single {
            diagnostic = diagnostic.with_fix(
                "replace it with an ordinary space",
                [Edit::replace(span, " ")],
            );
        } else if first.is_alphabetic() {
            diagnostic =
                diagnostic.with_note("identifiers may contain only ASCII letters, digits and `_`");
        }
        self.report(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::dump;

    fn lex_str(src: &str) -> (Vec<TokenKind>, Vec<&'static str>) {
        let mut interner = Interner::new();
        let lexed = lex(src, &mut interner);
        let kinds = lexed.tokens.iter().map(|t| t.kind).collect();
        let codes = lexed.diagnostics.iter().map(|d| d.code.as_str()).collect();
        (kinds, codes)
    }

    fn codes(src: &str) -> Vec<&'static str> {
        lex_str(src).1
    }

    #[test]
    fn every_keyword_and_punctuation() {
        let src = "as defer else enum extern false fn if let match move own return struct true val var while\n\
                   { } ( ) [ ] , : :: ; . -> => _\n\
                   = == != < <= > >= + - * / % ! & && ||\n";
        let mut interner = Interner::new();
        let lexed = lex(src, &mut interner);
        assert!(lexed.diagnostics.is_empty());
        insta::assert_snapshot!(dump(src, &lexed.tokens));
    }

    #[test]
    fn literals_and_identifiers() {
        let src = "x _x x_1 i64 0 42 1.5 0.25e-3 2e10 1E+2 \"a\\tb\" \"\" \"日本\"";
        let mut interner = Interner::new();
        let lexed = lex(src, &mut interner);
        assert!(lexed.diagnostics.is_empty());
        insta::assert_snapshot!(dump(src, &lexed.tokens));
    }

    /// Compound assignments are one token each, except `<<=`
    /// and `>>=`, which are two, as `<<` and `>>` are.
    #[test]
    fn compound_assignments() {
        let (kinds, codes) = lex_str("+= -= *= /= %= &= |= ^= <<= >>= -> && ||");
        assert!(codes.is_empty(), "{codes:?}");
        use TokenKind::*;
        assert_eq!(
            kinds,
            [
                PlusEq, MinusEq, StarEq, SlashEq, PercentEq, AmpEq, PipeEq, CaretEq, Lt, LtEq, Gt,
                GtEq, Arrow, AmpAmp, PipePipe, Eof
            ]
        );
    }

    /// `|` and `^`, and literals in other bases.
    #[test]
    fn bitwise_operators_and_number_bases() {
        let (kinds, codes) = lex_str("| ^ 0xff 0o17 0b1010_1010 1_000_000 2.5_0 0xAbC");
        assert!(codes.is_empty(), "{codes:?}");
        use TokenKind::*;
        assert_eq!(kinds, [Pipe, Caret, Int, Int, Int, Int, Float, Int, Eof]);
    }

    #[test]
    fn number_and_operator_errors() {
        let cases: &[(&str, &[&str])] = &[
            ("~x", &["E0009"]),
            ("1__000", &["E0010"]),
            ("1_", &["E0010"]),
            ("0x", &["E0011"]),
            ("0xg", &["E0011"]),
            ("0b102", &["E0011"]),
            ("0o9", &["E0011"]),
            ("0xfg", &["E0007"]),
            ("0755", &["E0005"]),
        ];
        for (src, expected) in cases {
            assert_eq!(codes(src), *expected, "{src:?}");
        }
    }

    #[test]
    fn number_and_operator_fixes_apply_cleanly() {
        for src in ["~x", "1__000", "1_", "0_1"] {
            let mut interner = Interner::new();
            let lexed = lex(src, &mut interner);
            assert_eq!(lexed.diagnostics.len(), 1, "{src:?}");
            let fix = lexed.diagnostics[0].fix.as_ref().expect("a fix");
            let fixed = fix.apply(src);
            assert!(codes(&fixed).is_empty(), "{src:?} fixed to {fixed:?}");
        }
    }

    #[test]
    fn nested_generics_lex_as_separate_gt() {
        let (kinds, codes) = lex_str(">>");
        assert!(codes.is_empty());
        assert_eq!(kinds, [TokenKind::Gt, TokenKind::Gt, TokenKind::Eof]);
    }

    #[test]
    fn string_contents_are_unescaped() {
        let mut interner = Interner::new();
        let lexed = lex(r#""tab\t nl\n cr\r nul\0 bs\\ q\" é""#, &mut interner);
        assert!(lexed.diagnostics.is_empty());
        let TokenKind::Str(sym) = lexed.tokens[0].kind else {
            panic!("not a string")
        };
        assert_eq!(interner.resolve(sym), "tab\t nl\n cr\r nul\0 bs\\ q\" é");
    }

    /// A text block: the closing `"""`'s indentation off
    /// every line, a blank line empty, space at the end of a line dropped
    /// unless an escape wrote it, and no line break after the opening or
    /// before the closing.
    #[test]
    fn text_blocks_lose_their_indentation() {
        let mut interner = Interner::new();
        let src = "\"\"\"  \n\t\tfirst \"x\" \n\n\t\t  second\\t\n\t\t\"\"\"";
        let lexed = lex(src, &mut interner);
        assert!(lexed.diagnostics.is_empty(), "{:?}", lexed.diagnostics);
        let TokenKind::Str(sym) = lexed.tokens[0].kind else {
            panic!("not a string")
        };
        assert_eq!(interner.resolve(sym), "first \"x\"\n\n  second\t");
        assert!(matches!(lexed.tokens[1].kind, TokenKind::Eof));
    }

    #[test]
    fn integer_then_field_access() {
        use TokenKind::*;
        let (kinds, codes) = lex_str("1.x");
        assert!(codes.is_empty());
        assert!(matches!(kinds[..], [Int, Dot, Ident(_), Eof]));
    }

    #[test]
    fn comments_and_trivia() {
        use TokenKind::*;
        let (kinds, codes) =
            lex_str("\u{feff}// only\n  a // trailing\r\n//// more\nb// no newline");
        assert!(codes.is_empty());
        assert!(matches!(kinds[..], [Ident(_), Ident(_), Eof]));
    }

    #[test]
    fn eof_is_last_and_empty() {
        let mut interner = Interner::new();
        for src in ["", "   ", "x", "\"open", "@"] {
            let lexed = lex(src, &mut interner);
            let last = lexed.tokens.last().unwrap();
            assert_eq!(last.kind, TokenKind::Eof, "{src:?}");
            assert_eq!(last.span, Span::at(src.len() as u32), "{src:?}");
            assert_eq!(
                lexed
                    .tokens
                    .iter()
                    .filter(|t| t.kind == TokenKind::Eof)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn error_codes() {
        assert_eq!(codes("$"), ["E0001"]);
        assert_eq!(
            codes("a $# b"),
            ["E0001"],
            "a run of bad characters is one error"
        );
        assert_eq!(codes("$ #"), ["E0001", "E0001"]);
        assert_eq!(codes("\"abc\nx"), ["E0002"]);
        assert_eq!(codes("\"abc"), ["E0002"]);
        assert_eq!(codes("\"abc\\"), ["E0002"]);
        assert_eq!(codes(r#""\q\w""#), ["E0003", "E0003"]);
        assert_eq!(codes("1."), ["E0004"]);
        assert_eq!(codes("1.;"), ["E0004"]);
        assert_eq!(codes("007"), ["E0005"]);
        assert_eq!(codes("00.5"), ["E0005"]);
        assert_eq!(codes("a | b ^ c"), [] as [&str; 0], "bitwise operators");
        assert_eq!(codes("~x"), ["E0009"]);
        assert_eq!(codes("12abc"), ["E0007"]);
        assert_eq!(codes("3e"), ["E0007"]);
        assert_eq!(codes("3e+"), ["E0007"]);
        // A character literal holds one character.
        assert_eq!(codes("'x' '\\n' '\\'' '\\u{1F600}' 'é'"), [] as [&str; 0]);
        assert_eq!(codes("''"), ["E0008"], "an empty character is one error");
        assert_eq!(codes("'it\"s'"), ["E0008"], "more than one character");
        assert_eq!(
            codes("'x\n'"),
            ["E0008", "E0008"],
            "a character does not run past its line"
        );
        assert_eq!(
            codes("'\\u{D800}'"),
            ["E0003"],
            "a surrogate is no character"
        );
        // A byte literal holds one ASCII character.
        assert_eq!(codes("b'x' b'\\n' b'\\'' b'\\u{7F}'"), [] as [&str; 0]);
        assert_eq!(codes("b'é'"), ["E0012"], "not ASCII");
        assert_eq!(codes("b'\\u{80}'"), ["E0012"], "past 127");
        assert_eq!(codes("b''"), ["E0008"], "an empty byte is one error");
        assert_eq!(codes("b'xy'"), ["E0008"], "more than one character");
        assert_eq!(codes("0 0.0 10 1e5 1.5E-5"), [] as [&str; 0]);
    }

    #[test]
    fn errors_still_produce_tokens() {
        use TokenKind::*;
        assert!(matches!(lex_str("\"abc\nx").0[..], [Str(_), Ident(_), Eof]));
        assert!(matches!(lex_str("1.;").0[..], [Float, Semi, Eof]));
        assert!(matches!(lex_str("007").0[..], [Int, Eof]));
        assert!(matches!(
            lex_str("a | b").0[..],
            [Ident(_), Pipe, Ident(_), Eof]
        ));
        assert!(matches!(lex_str("~x").0[..], [Bang, Ident(_), Eof]));
        assert!(matches!(lex_str("12abc;").0[..], [Int, Semi, Eof]));
        assert!(matches!(lex_str("a $ b").0[..], [Ident(_), Ident(_), Eof]));
        assert!(matches!(
            lex_str("f('xy')").0[..],
            [Ident(_), LParen, Char(120), RParen, Eof]
        ));
        // `b'` begins a byte; `b` before anything else is a name.
        assert!(matches!(
            lex_str("b'/' b ab'x'").0[..],
            [Byte(47), Ident(_), Ident(_), Char(120), Eof]
        ));
    }

    /// Every fix the lexer suggests must produce source that lexes cleanly.
    #[test]
    fn fixes_apply_cleanly() {
        let cases = [
            "let x = 007;",
            "let x = 1.;",
            "let x = 3e;",
            r#"let p = "C:\Users";"#,
            "let x = ~y;",
            "let x = 1__000;",
            "let c = 'xy';",
            "let c = b'xy';",
            "let x =\u{a0}1;",
        ];
        for src in cases {
            let mut interner = Interner::new();
            let mut fixed = src.to_string();
            // Fixes from one run are independent, so apply them back to front.
            let lexed = lex(src, &mut interner);
            assert!(!lexed.diagnostics.is_empty(), "{src:?} should have errors");
            let mut fixes: Vec<_> = lexed
                .diagnostics
                .iter()
                .filter_map(|d| d.fix.clone())
                .collect();
            assert_eq!(
                fixes.len(),
                lexed.diagnostics.len(),
                "{src:?}: every error has a fix"
            );
            fixes.sort_by_key(|f| std::cmp::Reverse(f.edits[0].span.lo));
            for fix in fixes {
                fixed = fix.apply(&fixed);
            }
            let relexed = lex(&fixed, &mut interner);
            assert!(
                relexed.diagnostics.is_empty(),
                "{src:?} fixed to {fixed:?} still has errors"
            );
        }
    }
}
