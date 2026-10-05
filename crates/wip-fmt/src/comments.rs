//! Comments, which the lexer passes over: they are found again in the text
//! between the tokens, and handed to the printer in order, so that each is
//! written where it was — on a line of its own before what followed it, or
//! at the end of the line it ended.

use wip_syntax::Token;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    /// Where it starts in the file.
    pub at: u32,
    /// `// …`, without what trails it.
    pub text: String,
    /// It has a line of its own, rather than ending a line of code.
    pub own_line: bool,
    /// A blank line comes before it.
    pub blank_before: bool,
}

/// Every `//` comment of a file, in order. Only the text between tokens is
/// looked at, so a `//` inside a string is not one.
pub fn comments(src: &str, tokens: &[Token]) -> Vec<Comment> {
    let mut found = Vec::new();
    let mut from = 0usize;
    for token in tokens {
        let to = token.span.lo as usize;
        if to > from {
            gap(src, from, to, &mut found);
        }
        from = from.max(token.span.hi as usize);
    }
    if from < src.len() {
        gap(src, from, src.len(), &mut found);
    }
    found
}

/// The comments in `src[from..to]`, which holds nothing but space and
/// comments.
fn gap(src: &str, from: usize, to: usize, found: &mut Vec<Comment>) {
    let text = &src[from..to];
    let mut offset = 0;
    while let Some(start) = text[offset..].find("//") {
        let at = offset + start;
        let end = text[at..].find('\n').map_or(text.len(), |n| at + n);
        let before = &src[..from + at];
        let line_start = before.rfind('\n').map_or(0, |n| n + 1);
        let own_line = before[line_start..]
            .trim_start_matches('\u{feff}')
            .trim()
            .is_empty();
        // A blank line before it: two line breaks with nothing but space
        // between them and the comment.
        let blank_before = own_line && {
            let upto = before[..line_start].trim_end_matches(['\n', '\r']);
            let newlines = before[upto.len()..].matches('\n').count();
            newlines >= 2 && !upto.is_empty()
        };
        found.push(Comment {
            at: (from + at) as u32,
            text: text[at..end].trim_end().to_string(),
            own_line,
            blank_before,
        });
        offset = end;
    }
}

/// Whether `src[from..to]` holds a blank line: two line breaks with only
/// space between them. Comments count as something, so a blank line after
/// a comment is found where it is.
pub fn blank_between(src: &str, from: usize, to: usize) -> bool {
    let text = &src[from.min(to)..to];
    let mut newlines = 0;
    let mut in_comment = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                in_comment = false;
                newlines += 1;
                if newlines >= 2 {
                    return true;
                }
            }
            '/' if !in_comment && chars.peek() == Some(&'/') => {
                in_comment = true;
                newlines = 0;
            }
            ' ' | '\t' | '\r' => {}
            _ if in_comment => {}
            _ => newlines = 0,
        }
    }
    false
}
