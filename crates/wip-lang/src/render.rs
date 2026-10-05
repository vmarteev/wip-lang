//! Turns structured diagnostics into text. Nothing else in the compiler decides
//! how a diagnostic looks.

use std::ops::Range;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind};
use wip_syntax::{Diagnostic, Severity, Span};

use crate::Sources;

/// Where a span falls: the file it is in, and its offsets within that file.
type Located = (String, Range<usize>);

pub fn render(diagnostic: &Diagnostic, file_name: &str, src: &str, color: bool) -> String {
    let sources = Sources::single(file_name.to_string(), src.to_string());
    render_one(diagnostic, &sources, color)
}

pub fn render_all(diagnostics: &[Diagnostic], file_name: &str, src: &str, color: bool) -> String {
    let sources = Sources::single(file_name.to_string(), src.to_string());
    render_program(diagnostics, &sources, color)
}

/// Renders every diagnostic against the file it belongs to.
/// One that points into no file, such as a missing `main` in a program with
/// no files, is shown without an excerpt rather than dropped.
pub fn render_program(diagnostics: &[Diagnostic], sources: &Sources, color: bool) -> String {
    diagnostics
        .iter()
        .map(|d| render_one(d, sources, color))
        .collect()
}

/// One diagnostic. Its labels may lie in several files, since a name declared
/// twice in one module is declared in two of them.
fn render_one(diagnostic: &Diagnostic, sources: &Sources, color: bool) -> String {
    let locate = |span: Span| -> Option<Located> {
        let (name, _, base) = sources.of(span)?;
        Some((
            name.to_string(),
            (span.lo - base) as usize..(span.hi - base) as usize,
        ))
    };
    let Some(primary) = locate(diagnostic.primary.span) else {
        return render_bare(diagnostic);
    };
    let kind = match diagnostic.severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
    };
    let config = Config::default()
        .with_index_type(IndexType::Byte)
        .with_color(color);
    let mut report = Report::build(kind, primary)
        .with_config(config)
        .with_code(diagnostic.code.as_str())
        .with_message(&diagnostic.message);
    // ariadne starts a new source section whenever a label goes backwards, so
    // hand it the labels in source order. Files lie end to end, so that order
    // takes the files in order too.
    let mut labels: Vec<(&wip_syntax::Label, Color, i32)> =
        std::iter::once((&diagnostic.primary, Color::Red, 1))
            .chain(
                diagnostic
                    .secondary
                    .iter()
                    .map(|label| (label, Color::Blue, 0)),
            )
            .collect();
    labels.sort_by_key(|(label, ..)| (label.span.lo, label.span.hi));
    for (label, color, priority) in labels {
        let Some(at) = locate(label.span) else {
            continue;
        };
        report = report.with_label(
            Label::new(at)
                .with_message(&label.message)
                .with_color(color)
                .with_priority(priority),
        );
    }
    for note in &diagnostic.notes {
        report = report.with_note(note);
    }
    // The fix is the primary suggestion, so it comes before any other help.
    if let Some(fix) = &diagnostic.fix {
        report = report.with_help(&fix.message);
    }
    for help in &diagnostic.help {
        report = report.with_help(help);
    }

    let mut out = Vec::new();
    let cache = ariadne::sources(
        sources
            .entries()
            .map(|(name, text, _)| (name.to_string(), text)),
    );
    report
        .finish()
        .write(cache, &mut out)
        .expect("writing to a Vec cannot fail");
    String::from_utf8(out).expect("ariadne writes UTF-8")
}

/// A diagnostic with no source to show: its code, message, notes and help,
/// laid out as the full rendering lays them out.
fn render_bare(diagnostic: &Diagnostic) -> String {
    let kind = match diagnostic.severity {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
    };
    let mut out = format!(
        "[{}] {kind}: {}\n",
        diagnostic.code.as_str(),
        diagnostic.message
    );
    for note in &diagnostic.notes {
        out.push_str(&format!("    Note: {note}\n"));
    }
    let fix = diagnostic.fix.iter().map(|fix| &fix.message);
    for help in fix.chain(&diagnostic.help) {
        out.push_str(&format!("    Help: {help}\n"));
    }
    out
}
