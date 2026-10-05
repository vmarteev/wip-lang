//! The Wip front end: lexer, parser and syntax tree.
//!
//! Also home of the types every later phase shares: [`Span`], [`Symbol`] and
//! [`Diagnostic`], and the [`parallel`] helpers every phase splits work
//! with.

pub mod ast;
pub mod codes;
pub mod derive;
pub mod diagnostic;
pub mod lexer;
pub mod parallel;
pub mod parser;
mod span;
mod symbol;
pub mod token;

pub use codes::Code;
pub use diagnostic::{Diagnostic, Edit, Fix, Label, Severity};
pub use lexer::{Lexed, lex, lex_at};
pub use parser::{Parsed, ParsedPackage, parse, parse_at, parse_package_at};
pub use span::{LineIndex, Span};
pub use symbol::{Interner, MAX_TUPLE, MIN_TUPLE, Symbol};
pub use token::{Token, TokenKind};
