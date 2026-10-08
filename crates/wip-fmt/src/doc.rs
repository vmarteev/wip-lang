//! The document a file is printed from, and the printer that fits it to a
//! width. It is Wadler's "prettier printer" as Prettier
//! uses it: text, groups, indentation, and line breaks that are spaces
//! while the group they are in fits.

/// A document.
#[derive(Clone, Debug)]
pub enum Doc {
    /// Text with no line break in it.
    Text(String),
    /// A space while its group fits, and a line break where it does not.
    Line,
    /// Nothing while its group fits, and a line break where it does not.
    Soft,
    /// A line break always; the groups around it cannot be flat.
    Hard,
    /// A line with nothing on it: a blank line between two things.
    Blank,
    Concat(Vec<Doc>),
    /// Laid out flat if it fits in what is left of the line, and broken
    /// otherwise.
    Group(Box<Doc>),
    /// One indentation deeper, from the next line break on.
    Indent(Box<Doc>),
    /// The first where its group is broken, the second where it is flat.
    IfBreak(Box<Doc>, Box<Doc>),
    /// Text put at the end of the line it is on, before its line break: a
    /// comment that ended a line. Its group cannot be flat, or the comment
    /// would swallow what follows it.
    LineEnd(String),
    /// Words with a space between them, as many on a line as fit where its
    /// group is broken: a table of numbers, filled as a paragraph is.
    Fill(Vec<String>),
    /// Always flat, and where it ends past the width, the `Choice` it is
    /// in takes its second layout: an `if`'s condition and its `then`.
    Rigid(Box<Doc>),
    /// Where a `Rigid` ends: the printer checks it ended within the width.
    RigidEnd,
    /// The first layout where its rigid parts each fit on their line, and
    /// the second where one does not.
    Choice(Box<Doc>, Box<Doc>),
}

pub fn text(s: impl Into<String>) -> Doc {
    Doc::Text(s.into())
}

pub fn concat(parts: Vec<Doc>) -> Doc {
    Doc::Concat(parts)
}

pub fn group(doc: Doc) -> Doc {
    Doc::Group(Box::new(doc))
}

pub fn indent(doc: Doc) -> Doc {
    Doc::Indent(Box::new(doc))
}

pub fn if_break(broken: Doc, flat: Doc) -> Doc {
    Doc::IfBreak(Box::new(broken), Box::new(flat))
}

pub fn rigid(doc: Doc) -> Doc {
    Doc::Rigid(Box::new(doc))
}

pub fn choice(first: Doc, second: Doc) -> Doc {
    Doc::Choice(Box::new(first), Box::new(second))
}

/// The document laid out on one line, where it can be: nothing in it
/// breaks a line whatever the width, a comment that ends one included.
pub fn flat(doc: &Doc) -> Option<String> {
    let mut out = String::new();
    let mut stack = vec![doc];
    while let Some(doc) = stack.pop() {
        match doc {
            Doc::Text(s) => out.push_str(s),
            Doc::Line => out.push(' '),
            Doc::Soft | Doc::RigidEnd => {}
            Doc::Hard | Doc::Blank | Doc::LineEnd(_) => return None,
            Doc::Concat(parts) => stack.extend(parts.iter().rev()),
            Doc::Group(inner) | Doc::Indent(inner) | Doc::Rigid(inner) => stack.push(inner),
            Doc::IfBreak(_, flat) => stack.push(flat),
            Doc::Choice(first, _) => stack.push(first),
            Doc::Fill(words) => out.push_str(&words.join(" ")),
        }
    }
    Some(out)
}

/// `parts` with `sep` between them.
pub fn join(parts: Vec<Doc>, sep: Doc) -> Doc {
    let mut out = Vec::with_capacity(parts.len() * 2);
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            out.push(sep.clone());
        }
        out.push(part);
    }
    Doc::Concat(out)
}

/// How a line is indented, and how wide it may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub width: usize,
    pub tabs: bool,
    /// Spaces per indentation, or the columns a tab counts as.
    pub size: usize,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            width: 100,
            tabs: true,
            size: 4,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

/// Whether a document holds something that no group around it can be
/// flat with: a line break that always breaks, or a comment that ends a
/// line.
fn forces_break(doc: &Doc) -> bool {
    match doc {
        Doc::Hard | Doc::Blank | Doc::LineEnd(_) => true,
        Doc::Concat(parts) => parts.iter().any(forces_break),
        Doc::Group(inner) | Doc::Indent(inner) => forces_break(inner),
        Doc::IfBreak(broken, flat) => forces_break(broken) || forces_break(flat),
        Doc::Rigid(inner) => forces_break(inner),
        Doc::Choice(first, _) => forces_break(first),
        Doc::Text(_) | Doc::Line | Doc::Soft | Doc::Fill(_) | Doc::RigidEnd => false,
    }
}

/// Prints a document to `style`'s width.
pub fn print(doc: &Doc, style: Style) -> String {
    let mut layout = Layout {
        out: String::new(),
        column: 0,
        fresh: true,
        line_end: Vec::new(),
        overran: false,
        style,
    };
    let mut stack: Vec<(usize, Mode, &Doc)> = vec![(0, Mode::Break, doc)];
    layout.run(&mut stack, 0);
    let mut out = layout.out;
    for s in layout.line_end.drain(..) {
        out.push_str(&s);
    }
    trim_end(&mut out);
    out.push('\n');
    out
}

/// What is printed so far, and where.
struct Layout {
    out: String,
    column: usize,
    /// Nothing but indentation is on the current line yet.
    fresh: bool,
    /// What is waiting for the end of the line: comments that ended one.
    line_end: Vec<String>,
    /// Whether a rigid part ended past the width.
    overran: bool,
    style: Style,
}

impl Layout {
    /// Prints what is on `stack` above its first `bottom` entries, which
    /// are what follows, seen only to decide whether a group fits.
    fn run(&mut self, stack: &mut Vec<(usize, Mode, &Doc)>, bottom: usize) {
        let style = self.style;
        while stack.len() > bottom {
            let Some((level, mode, doc)) = stack.pop() else {
                break;
            };
            match doc {
                Doc::Text(s) => {
                    self.out.push_str(s);
                    self.column += s.chars().count();
                    self.fresh &= s.is_empty();
                }
                // Flat, all on this line; broken, a word goes on the next
                // line where it would not fit on this one.
                Doc::Fill(words) => {
                    for (i, word) in words.iter().enumerate() {
                        let width = word.chars().count();
                        if i > 0 {
                            if mode == Mode::Flat || self.column + 1 + width <= style.width {
                                self.out.push(' ');
                                self.column += 1;
                            } else {
                                trim_end(&mut self.out);
                                self.out.push('\n');
                                self.column = indentation(&mut self.out, level, style);
                            }
                        }
                        self.out.push_str(word);
                        self.column += width;
                        self.fresh = false;
                    }
                }
                Doc::Concat(parts) => {
                    for part in parts.iter().rev() {
                        stack.push((level, mode, part));
                    }
                }
                Doc::Indent(inner) => stack.push((level + 1, mode, inner)),
                // Inside what is flat, flat too: inside a group that fits,
                // it fits, and inside a rigid part it must.
                Doc::Group(inner) => {
                    let flat = mode == Mode::Flat
                        || !forces_break(inner)
                            && fits(
                                &[(level, Mode::Flat, inner.as_ref())],
                                stack,
                                style.width.saturating_sub(self.column),
                                style,
                            );
                    stack.push((level, if flat { Mode::Flat } else { Mode::Break }, inner));
                }
                Doc::IfBreak(broken, flat) => {
                    stack.push((level, mode, if mode == Mode::Break { broken } else { flat }))
                }
                // One with a line break that cannot be flat does not fit.
                Doc::Rigid(inner) => {
                    self.overran |= forces_break(inner);
                    stack.push((level, mode, &Doc::RigidEnd));
                    stack.push((level, Mode::Flat, inner));
                }
                Doc::RigidEnd => self.overran |= self.column > style.width,
                // Where the group around it is flat, it fits as the first
                // already. Otherwise the first is tried, on a copy of what
                // is printed, with what follows it to decide its groups.
                Doc::Choice(first, second) => {
                    if mode == Mode::Flat {
                        stack.push((level, mode, first));
                        continue;
                    }
                    let mut trial = Layout {
                        out: String::new(),
                        column: self.column,
                        fresh: self.fresh,
                        line_end: Vec::new(),
                        overran: false,
                        style,
                    };
                    let mut tried = stack.clone();
                    let below = tried.len();
                    tried.push((level, mode, first));
                    trial.run(&mut tried, below);
                    stack.push((level, mode, if trial.overran { second } else { first }));
                }
                // A comment that ended a line, met only once the next line
                // has begun: it goes back to the end of the line it ended.
                Doc::LineEnd(s) if self.fresh => {
                    let end = self.out.trim_end().len();
                    self.out.insert_str(end, s);
                }
                Doc::LineEnd(s) => self.line_end.push(s.clone()),
                Doc::Line | Doc::Soft if mode == Mode::Flat => {
                    if matches!(doc, Doc::Line) {
                        self.out.push(' ');
                        self.column += 1;
                        self.fresh = false;
                    }
                }
                Doc::Line | Doc::Soft | Doc::Hard | Doc::Blank => {
                    for s in self.line_end.drain(..) {
                        self.out.push_str(&s);
                    }
                    trim_end(&mut self.out);
                    self.out.push('\n');
                    if matches!(doc, Doc::Blank) {
                        self.out.push('\n');
                    }
                    self.column = indentation(&mut self.out, level, style);
                    self.fresh = true;
                }
            }
        }
    }
}

/// Takes the spaces and tabs off the end of what is written, so that no
/// line ends in them.
fn trim_end(out: &mut String) {
    while out.ends_with([' ', '\t']) {
        out.pop();
    }
}

/// Writes one line's indentation, and answers the column it reaches.
fn indentation(out: &mut String, level: usize, style: Style) -> usize {
    if style.tabs {
        out.extend(std::iter::repeat_n('\t', level));
    } else {
        out.extend(std::iter::repeat_n(' ', level * style.size));
    }
    level * style.size
}

/// Whether `first`, flat, and then what follows it up to its next line
/// break, fit in `width` columns.
fn fits(
    first: &[(usize, Mode, &Doc)],
    rest: &[(usize, Mode, &Doc)],
    width: usize,
    style: Style,
) -> bool {
    let mut left = width as isize;
    let mut stack: Vec<(Mode, &Doc)> = first.iter().map(|&(_, m, d)| (m, d)).collect();
    let mut rest = rest.iter().rev();
    let _ = style;
    loop {
        let Some((mode, doc)) = stack.pop().or_else(|| rest.next().map(|&(_, m, d)| (m, d))) else {
            return true;
        };
        match doc {
            Doc::Text(s) => {
                left -= s.chars().count() as isize;
                if left < 0 {
                    return false;
                }
            }
            // Flat, every word and the spaces; broken, the first word, and
            // then a line break.
            Doc::Fill(words) => {
                let flat = mode == Mode::Flat;
                for (i, word) in words.iter().enumerate() {
                    if i > 0 && !flat {
                        return true;
                    }
                    left -= word.chars().count() as isize + isize::from(i > 0);
                    if left < 0 {
                        return false;
                    }
                }
            }
            Doc::Concat(parts) => {
                for part in parts.iter().rev() {
                    stack.push((mode, part));
                }
            }
            Doc::Indent(inner) => stack.push((mode, inner)),
            Doc::Group(inner) => stack.push((mode, inner)),
            Doc::IfBreak(broken, flat) => {
                stack.push((mode, if mode == Mode::Break { broken } else { flat }))
            }
            Doc::Rigid(inner) => stack.push((Mode::Flat, inner)),
            Doc::Choice(first, _) => stack.push((mode, first)),
            Doc::LineEnd(_) | Doc::RigidEnd => {}
            Doc::Line | Doc::Soft if mode == Mode::Flat => {
                if matches!(doc, Doc::Line) {
                    left -= 1;
                }
            }
            // A line break of what follows, where it is broken: the line
            // ends there, and everything before it fitted.
            Doc::Line | Doc::Soft | Doc::Hard | Doc::Blank => return true,
        }
    }
}
