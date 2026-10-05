//! Diagnostics as structured data.
//!
//! Every phase reports problems as [`Diagnostic`] values; only the driver
//! (`wip-lang`) turns them into text. Every code is declared in
//! [`crate::codes`], grouped by phase, and never reused:
//!
//! | Range   | Phase                      |
//! |---------|----------------------------|
//! | `E00xx` | lexer                      |
//! | `E01xx` | parser                     |
//! | `E02xx` | name resolution            |
//! | `E03xx` | types                      |
//! | `E04xx` | ownership and references   |
//! | `E05xx` | `defer` and drop order     |
//! | `E09xx` | code generation            |

use crate::{Code, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// One text replacement. An empty span inserts; an empty replacement deletes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub span: Span,
    pub replacement: String,
}

impl Edit {
    pub fn insert(pos: u32, text: impl Into<String>) -> Edit {
        Edit {
            span: Span::at(pos),
            replacement: text.into(),
        }
    }

    pub fn replace(span: Span, text: impl Into<String>) -> Edit {
        Edit {
            span,
            replacement: text.into(),
        }
    }
}

/// A machine-applicable suggestion: applying every edit fixes the problem the
/// diagnostic describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub message: String,
    pub edits: Vec<Edit>,
}

impl Fix {
    /// Returns `src` with the edits applied. Edits must not overlap.
    pub fn apply(&self, src: &str) -> String {
        let mut edits: Vec<&Edit> = self.edits.iter().collect();
        edits.sort_by_key(|e| (e.span.lo, e.span.hi));
        let mut out = String::with_capacity(src.len());
        let mut pos = 0;
        for edit in edits {
            assert!(edit.span.lo as usize >= pos, "overlapping edits in fix");
            out.push_str(&src[pos..edit.span.lo as usize]);
            out.push_str(&edit.replacement);
            pos = edit.span.hi as usize;
        }
        out.push_str(&src[pos..]);
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Code,
    pub message: String,
    /// What the diagnostic is about. There is always exactly one.
    pub primary: Label,
    /// Related locations, such as where an unclosed delimiter was opened.
    pub secondary: Vec<Label>,
    /// Background that explains the rule.
    pub notes: Vec<String>,
    /// Advice that is not a mechanical edit.
    pub help: Vec<String>,
    pub fix: Option<Fix>,
}

impl Diagnostic {
    pub fn error(
        code: Code,
        message: impl Into<String>,
        span: Span,
        label: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code,
            message: message.into(),
            primary: Label {
                span,
                message: label.into(),
            },
            secondary: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
            fix: None,
        }
    }

    pub fn warning(
        code: Code,
        message: impl Into<String>,
        span: Span,
        label: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(code, message, span, label)
        }
    }

    pub fn with_secondary(mut self, span: Span, label: impl Into<String>) -> Diagnostic {
        self.secondary.push(Label {
            span,
            message: label.into(),
        });
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help.push(help.into());
        self
    }

    pub fn with_fix(
        mut self,
        message: impl Into<String>,
        edits: impl IntoIterator<Item = Edit>,
    ) -> Diagnostic {
        self.fix = Some(Fix {
            message: message.into(),
            edits: edits.into_iter().collect(),
        });
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
