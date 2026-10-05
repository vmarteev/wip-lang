//! `wip fmt`: a file printed again from its syntax tree,
//! in one layout, to a width — as Prettier does.
//!
//! A file is lexed and parsed, its comments are found between the tokens,
//! and the tree is turned into a document of groups that print flat where
//! they fit and broken where they do not. Before the result is handed
//! back it is parsed again: it must be the same program, with the same
//! comments, and formatting it again must change nothing.

mod comments;
mod doc;
mod print;

pub use doc::Style;
use wip_syntax::{Diagnostic, Interner, lex, parse, parse_package_at};

/// Why a file was not formatted.
#[derive(Debug)]
pub enum Error {
    /// It does not parse; the printer does not guess.
    Parse(Vec<Diagnostic>),
    /// What would have been written is not the same program, or not the
    /// same comments, or not stable: a bug in `wip fmt`, which is reported
    /// rather than written.
    Bug(String),
}

/// The file's text, laid out. `package` says the file is a `package.wip`.
pub fn format(src: &str, style: Style, package: bool) -> Result<String, Error> {
    let first = layout(src, style, package)?;
    let second = layout(&first, style, package).map_err(|err| match err {
        Error::Parse(diagnostics) => Error::Bug(format!(
            "what it wrote does not parse: {}",
            diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; ")
        )),
        bug => bug,
    })?;
    if package {
        if first != second {
            return Err(Error::Bug("formatting it again changes it".into()));
        }
        return Ok(first);
    }
    let (before, after) = (shape(src), shape(&first));
    if before != after {
        return Err(Error::Bug(format!(
            "what it wrote is a different program ({})",
            first_difference(&before, &after)
        )));
    }
    let (before, after) = (comment_texts(src), comment_texts(&first));
    if before != after {
        return Err(Error::Bug(format!(
            "what it wrote has different comments ({})",
            first_difference(&before, &after)
        )));
    }
    if first != second {
        let a: Vec<String> = first.lines().map(str::to_string).collect();
        let b: Vec<String> = second.lines().map(str::to_string).collect();
        return Err(Error::Bug(format!(
            "formatting it again changes it ({})",
            first_difference(&a, &b)
        )));
    }
    Ok(first)
}

/// Where two lists first differ, for a report of a bug.
fn first_difference(before: &[String], after: &[String]) -> String {
    let at = before
        .iter()
        .zip(after)
        .position(|(a, b)| a != b)
        .unwrap_or(before.len().min(after.len()));
    let show = |lines: &[String]| {
        lines
            .get(at)
            .map_or("nothing".to_string(), |l| format!("`{}`", l.trim()))
    };
    format!("{} became {}", show(before), show(after))
}

/// One pass: parse, and print.
fn layout(src: &str, style: Style, package: bool) -> Result<String, Error> {
    let mut interner = Interner::new();
    let lexed = lex(src, &mut interner);
    let found = comments::comments(src, &lexed.tokens);
    if package {
        let parsed = parse_package_at(src, 0, &lexed);
        let errors = errors(lexed.diagnostics.iter().chain(&parsed.diagnostics));
        if !errors.is_empty() {
            return Err(Error::Parse(errors));
        }
        let ast = wip_syntax::ast::Ast::default();
        let mut printer = print::Printer::new(src, &ast, &interner, found);
        let doc = printer.package(&parsed.annotations, parsed.name);
        return Ok(doc::print(&doc, style));
    }
    let parsed = parse(src, &lexed);
    let errors = errors(lexed.diagnostics.iter().chain(&parsed.diagnostics));
    if !errors.is_empty() {
        return Err(Error::Parse(errors));
    }
    let mut printer = print::Printer::new(src, &parsed.ast, &interner, found);
    let doc = printer.file();
    Ok(doc::print(&doc, style))
}

fn errors<'d>(diagnostics: impl Iterator<Item = &'d Diagnostic>) -> Vec<Diagnostic> {
    diagnostics.filter(|d| d.is_error()).cloned().collect()
}

/// The syntax tree as `wip parse` shows it, without where each part is:
/// what must not change. An `assert`'s message is the condition's text as
/// it was written, so it is left out too — laying the condition out is the
/// point.
fn shape(src: &str) -> Vec<String> {
    let mut interner = Interner::new();
    let lexed = lex(src, &mut interner);
    let parsed = parse(src, &lexed);
    wip_syntax::ast::dump(&parsed.ast, src, &interner)
        .lines()
        .map(|line| {
            let line = match line.rfind("  @") {
                Some(at) => &line[..at],
                None => line,
            };
            match line.find("assert \"") {
                Some(at) => line[..at + "assert".len()].to_string(),
                None => line.to_string(),
            }
        })
        .collect()
}

/// A file's comments, in order.
fn comment_texts(src: &str) -> Vec<String> {
    let mut interner = Interner::new();
    let lexed = lex(src, &mut interner);
    comments::comments(src, &lexed.tokens)
        .into_iter()
        .map(|c| c.text)
        .collect()
}
