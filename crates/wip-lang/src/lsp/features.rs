//! What the server answers with: a program's diagnostics and their fixes,
//! a file laid out by `wip fmt`, and a file's outline.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use wip_syntax::ast::{FnDecl, Item, Name};
use wip_syntax::{Diagnostic, Interner, Severity, Span};

use super::navigate;
use super::protocol::{Lines, uri_of};
use crate::{Loaded, Mode, Overlay, SourceFile, Sources, Timings, canonical_file};
use wip_hir::Program;

/// A diagnostic's fix, as the editor offers it: a title, the diagnostic,
/// and the edits, by file.
pub struct Fix {
    pub title: String,
    pub diagnostic: Value,
    pub changes: Value,
}

/// What is wrong, by file, and what can be fixed.
#[derive(Default)]
pub struct Found {
    pub diagnostics: HashMap<String, Vec<Value>>,
    pub fixes: HashMap<String, Vec<Fix>>,
    /// Each program checked, to answer what its names mean.
    pub programs: Vec<(Loaded, Program)>,
}

/// Checks the programs of the open files, which are `(uri, path, text)`.
/// A file no program reaches — one of the standard library's, or a module
/// nothing imports — is only parsed.
pub fn diagnostics(
    open: &[(&str, &Path, &str)],
    overlay: &Overlay,
    known: &mut HashMap<PathBuf, PathBuf>,
) -> Found {
    let mut found = Found::default();
    let mut places = Places::new(open);
    let mut roots: Vec<PathBuf> = open
        .iter()
        .filter(|(_, path, _)| !in_std(path))
        .map(|(_, path, _)| program_root(path, overlay, known))
        .collect();
    roots.sort();
    roots.dedup();
    let mut reached: HashSet<PathBuf> = HashSet::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for root in roots {
        let Ok(mut loaded) = crate::load_overlaid(&root, Mode::Tests, overlay) else {
            continue;
        };
        let checked = crate::check_loaded(&mut loaded, &mut Timings::default());
        for (name, ..) in loaded.sources.entries() {
            if Path::new(name).is_absolute() {
                reached.insert(canonical_file(Path::new(name)));
            }
        }
        for diagnostic in &checked.diagnostics {
            // A file two programs share is reported once.
            if let Some((uri, range)) = places.place(&loaded.sources, diagnostic.primary.span)
                && seen.insert((
                    uri,
                    format!("{range}{}{}", diagnostic.code.as_str(), diagnostic.message),
                ))
            {
                found.add(diagnostic, &loaded.sources, &mut places);
            }
        }
        found.programs.push((loaded, checked.program));
    }
    for &(_, path, text) in open {
        if reached.contains(&canonical_file(path)) {
            continue;
        }
        let name = path.display().to_string();
        let parsed = crate::parse_file(&SourceFile {
            name: name.clone(),
            text: text.to_string(),
        });
        let sources = Sources::single(name, text.to_string());
        for diagnostic in &parsed.diagnostics {
            found.add(diagnostic, &sources, &mut places);
        }
    }
    found
}

impl Found {
    fn add(&mut self, diagnostic: &Diagnostic, sources: &Sources, places: &mut Places) {
        let Some((uri, range)) = places.place(sources, diagnostic.primary.span) else {
            return;
        };
        let mut message = diagnostic.message.clone();
        let label = &diagnostic.primary.message;
        if !label.is_empty() && label != &message {
            message.push_str(&format!(": {label}"));
        }
        for help in &diagnostic.help {
            message.push_str(&format!("\nhelp: {help}"));
        }
        for note in &diagnostic.notes {
            message.push_str(&format!("\nnote: {note}"));
        }
        let related: Vec<Value> = diagnostic
            .secondary
            .iter()
            .filter_map(|label| {
                let (uri, range) = places.place(sources, label.span)?;
                Some(json!({
                    "location": { "uri": uri, "range": range },
                    "message": label.message,
                }))
            })
            .collect();
        let value = json!({
            "range": range,
            "severity": match diagnostic.severity {
                Severity::Error => 1,
                Severity::Warning => 2,
            },
            "code": diagnostic.code.as_str(),
            "source": "wip",
            "message": message,
            "relatedInformation": related,
        });
        if let Some(fix) = &diagnostic.fix {
            let mut changes: HashMap<String, Vec<Value>> = HashMap::new();
            for edit in &fix.edits {
                if let Some((uri, range)) = places.place(sources, edit.span) {
                    changes
                        .entry(uri)
                        .or_default()
                        .push(json!({ "range": range, "newText": edit.replacement }));
                }
            }
            self.fixes.entry(uri.clone()).or_default().push(Fix {
                title: fix.message.clone(),
                diagnostic: value.clone(),
                changes: json!(changes),
            });
        }
        self.diagnostics.entry(uri).or_default().push(value);
    }
}

/// Turns spans into URIs and ranges, remembering each file's lines.
#[derive(Default)]
struct Places {
    /// The URI of each open file, by its canonical path.
    uris: HashMap<PathBuf, String>,
    /// A file's name in the sources, and its URI.
    named: HashMap<String, Option<String>>,
}

impl Places {
    /// An open file is named by the URI the editor gave it, however else
    /// its path is written.
    fn new(open: &[(&str, &Path, &str)]) -> Places {
        let mut places = Places::default();
        for &(uri, path, _) in open {
            places.uris.insert(canonical_file(path), uri.to_string());
        }
        places
    }

    /// Like [`Places::place`], and a span in the standard library is in
    /// the compiler's sources, where they are on disk.
    fn locate(&mut self, sources: &Sources, span: Span) -> Option<(String, Value)> {
        let (name, text, base) = sources.of(span)?;
        if Path::new(name).is_absolute() {
            return self.place(sources, span);
        }
        let file = crate::std_on_disk()?.parent()?.join(name);
        if !file.is_file() {
            return None;
        }
        let lines = Lines::new(text);
        let lo = span.lo.saturating_sub(base) as usize;
        let hi = span.hi.saturating_sub(base) as usize;
        Some((uri_of(&file), lines.range(lo, hi.max(lo))))
    }

    /// The file a span is in and its range there; `None` for the standard
    /// library's files, which are in the compiler.
    fn place(&mut self, sources: &Sources, span: Span) -> Option<(String, Value)> {
        let (name, text, base) = sources.of(span)?;
        let uris = &self.uris;
        let uri = self
            .named
            .entry(name.to_string())
            .or_insert_with(|| {
                let path = Path::new(name);
                // The standard library is named from the compiler's
                // sources, not from where anything is on disk.
                if !path.is_absolute() {
                    return None;
                }
                let canonical = canonical_file(path);
                Some(
                    uris.get(&canonical)
                        .cloned()
                        .unwrap_or_else(|| uri_of(&canonical)),
                )
            })
            .clone()?;
        let lines = Lines::new(text);
        let lo = span.lo.saturating_sub(base) as usize;
        let hi = span.hi.saturating_sub(base) as usize;
        Some((uri, lines.range(lo, hi.max(lo))))
    }
}

/// The directory of the program a file belongs to: the
/// nearest above it with a `package.wip`; or else, of the directories
/// above it, one inside the next, that hold `.wip` files, the highest whose
/// program reaches the file — where a program's `main.wip` is, above its
/// modules. Each directory's answer is kept in `known`.
pub fn program_root(
    file: &Path,
    overlay: &Overlay,
    known: &mut HashMap<PathBuf, PathBuf>,
) -> PathBuf {
    let dir = file.parent().unwrap_or_else(|| Path::new("/"));
    if let Some(root) = known.get(dir) {
        return root.clone();
    }
    let root = find_root(file, dir, overlay);
    known.insert(dir.to_path_buf(), root.clone());
    root
}

fn find_root(file: &Path, dir: &Path, overlay: &Overlay) -> PathBuf {
    if let Some(package) = dir.ancestors().find(|a| a.join("package.wip").is_file()) {
        return package.to_path_buf();
    }
    let holds_wip = |dir: &Path| {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.path().extension().is_some_and(|x| x == "wip"))
        })
    };
    let mut chain = vec![dir];
    for above in dir.ancestors().skip(1) {
        if !holds_wip(above) {
            break;
        }
        chain.push(above);
    }
    let file = canonical_file(file);
    let reaches = |root: &Path| {
        crate::load_overlaid(root, Mode::Tests, overlay).is_ok_and(|loaded| {
            loaded.sources.entries().any(|(name, ..)| {
                Path::new(name).is_absolute() && canonical_file(Path::new(name)) == file
            })
        })
    };
    // Its own directory reaches it, being its module; a directory above
    // does only if it imports it.
    let highest = chain
        .iter()
        .rev()
        .find(|root| chain.len() == 1 || reaches(root));
    highest.unwrap_or(&dir).to_path_buf()
}

/// Whether a file is one of the standard library's, which is in the
/// compiler and no program of its own: the copy an editor was sent to, or
/// the checkout this compiler was built from, where someone changing the
/// library opens it.
pub fn in_std(path: &Path) -> bool {
    let file = canonical_file(path);
    std_dirs().iter().any(|std| file.starts_with(std))
}

/// Where the standard library is on disk: the copy of what the compiler
/// carries, and the checkout it was built from, while that is there.
fn std_dirs() -> Vec<PathBuf> {
    let copy = crate::std_on_disk().and_then(|dir| std::fs::canonicalize(dir).ok());
    let checkout = std::fs::canonicalize(concat!(env!("CARGO_MANIFEST_DIR"), "/../../std")).ok();
    copy.into_iter().chain(checkout).collect()
}

/// The program that holds `path`, and where the file's spans begin in it.
fn holding<'a>(
    programs: &'a [(Loaded, Program)],
    path: &Path,
) -> Option<(&'a Loaded, &'a Program, u32)> {
    let file = canonical_file(path);
    programs.iter().find_map(|(loaded, program)| {
        let (_, _, base) = loaded.sources.entries().find(|(name, ..)| {
            Path::new(name).is_absolute() && canonical_file(Path::new(name)) == file
        })?;
        Some((loaded, program, base))
    })
}

/// What is at `position` in the open file `path`, whose text is `text`,
/// in the program that holds it.
fn hit_at<'a>(
    programs: &'a [(Loaded, Program)],
    path: &Path,
    text: &str,
    position: &Value,
) -> Option<(&'a Loaded, navigate::Hit)> {
    let (loaded, program, base) = holding(programs, path)?;
    let offset = u32::try_from(Lines::new(text).offset(position)).ok()?;
    let hit = navigate::at(program, &loaded.interner, base + offset)?;
    Some((loaded, hit))
}

/// A request answered at a position: the programs, the open files, the
/// file asked about and its text, and the position.
pub type AtPosition =
    fn(&[(Loaded, Program)], &[(&str, &Path, &str)], (&Path, &str), &Value) -> Value;

/// What a name is, as Wip would write it, and what its declaration's
/// `///` comment says.
pub fn hover(
    programs: &[(Loaded, Program)],
    open: &[(&str, &Path, &str)],
    (path, text): (&Path, &str),
    position: &Value,
) -> Value {
    let Some((loaded, hit)) = hit_at(programs, path, text, position) else {
        return Value::Null;
    };
    let mut shown = format!("```wip\n{}\n```", hit.shown);
    if let Some(declared) = hit.declared
        && let Some((_, text, base)) = loaded.sources.of(declared)
    {
        let docs = navigate::documentation(text, declared.lo.saturating_sub(base) as usize);
        if !docs.is_empty() {
            shown.push_str(&format!("\n\n{docs}"));
        }
    }
    let range = Places::new(open)
        .place(&loaded.sources, hit.span)
        .map(|(_, range)| range);
    json!({ "contents": { "kind": "markdown", "value": shown }, "range": range })
}

/// Where a name was declared.
pub fn definition(
    programs: &[(Loaded, Program)],
    open: &[(&str, &Path, &str)],
    (path, text): (&Path, &str),
    position: &Value,
) -> Value {
    let Some((loaded, hit)) = hit_at(programs, path, text, position) else {
        return Value::Null;
    };
    let located = hit
        .declared
        .and_then(|declared| Places::new(open).locate(&loaded.sources, declared));
    match located {
        Some((uri, range)) => json!({ "uri": uri, "range": range }),
        None => Value::Null,
    }
}

/// A file as every program names it: its canonical path, or, for one of
/// the standard library's, the name the compiler gives it.
fn identity(name: &str) -> String {
    let path = Path::new(name);
    match path.is_absolute() {
        true => canonical_file(path).display().to_string(),
        false => name.to_string(),
    }
}

/// Every place that names what is at `position`, in every program checked,
/// as `(uri, range)`; with the declaration's own name, where `declaration`
/// says so. `None` where nothing with a declaration is there.
fn occurrences(
    programs: &[(Loaded, Program)],
    open: &[(&str, &Path, &str)],
    (path, text): (&Path, &str),
    position: &Value,
    declaration: bool,
) -> Option<Vec<(String, Value)>> {
    let (first, hit) = hit_at(programs, path, text, position)?;
    let declared = hit.declared?;
    let (name, _, base) = first.sources.of(declared)?;
    // Where it was declared, as every program can find it: each numbers
    // its files, and its declarations, its own way.
    let file = identity(name);
    let (lo, hi) = (declared.lo - base, declared.hi - base);
    let program = &programs.iter().find(|(l, _)| std::ptr::eq(l, first))?.1;
    let written = navigate::name_of(program, &first.interner, hit.target)?;
    let importable = navigate::importable(program, &first.interner, hit.target)
        .map(|(_, module)| program.modules[module as usize].clone());
    let mut places = Places::new(open);
    let mut found: Vec<(String, Value)> = Vec::new();
    for (loaded, program) in programs {
        let Some(base) = loaded
            .sources
            .entries()
            .find(|(name, ..)| identity(name) == file)
            .map(|(_, _, base)| base)
        else {
            continue;
        };
        let declared = Span::new(base + lo, base + hi);
        let text_of = |s: Span| loaded.sources.of(s).map(|(_, text, base)| (text, base));
        let mut spans =
            navigate::references(program, &loaded.interner, (&written, declared), text_of);
        if !declaration
            && let Some((text, base)) = text_of(declared)
            && let Some(own) = navigate::name_in(text, base, declared, &written, &[])
        {
            spans.retain(|&s| s != own);
        }
        // `import shapes::{area}` names it too.
        if let Some(module) = &importable {
            spans.extend(imported_as(loaded, module, &written));
        }
        for span in spans {
            if let Some(place) = places.locate(&loaded.sources, span)
                && !found.contains(&place)
            {
                found.push(place);
            }
        }
    }
    Some(found)
}

/// Where the imports of `loaded`'s modules name the item `name` of the
/// module `module` in braces.
fn imported_as(loaded: &Loaded, module: &str, name: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for importer in &loaded.modules {
        for file in &importer.files {
            for item in &file.ast.items {
                let Item::Import(decl) = item else { continue };
                let Some(items) = &decl.items else { continue };
                let written: Vec<&str> = decl
                    .path
                    .iter()
                    .map(|n| loaded.interner.resolve(n.sym))
                    .collect();
                let written = written.join("::");
                let meant = written == module
                    || (!importer.prefix.is_empty()
                        && format!("{}::{written}", importer.prefix) == module);
                if !meant {
                    continue;
                }
                for item in items {
                    if let Some(n) = item.name
                        && loaded.interner.resolve(n.sym) == name
                    {
                        spans.push(n.span);
                    }
                }
            }
        }
    }
    spans
}

/// Every place that names what is at a position.
pub fn references(
    programs: &[(Loaded, Program)],
    open: &[(&str, &Path, &str)],
    file: (&Path, &str),
    position: &Value,
    declaration: bool,
) -> Value {
    match occurrences(programs, open, file, position, declaration) {
        Some(found) => json!(
            found
                .into_iter()
                .map(|(uri, range)| json!({ "uri": uri, "range": range }))
                .collect::<Vec<_>>()
        ),
        None => Value::Null,
    }
}

/// Every place that names what is at a position, renamed `new`: the
/// edits, or why it cannot be.
pub fn rename(
    programs: &[(Loaded, Program)],
    open: &[(&str, &Path, &str)],
    file: (&Path, &str),
    position: &Value,
    new: &str,
) -> Result<Value, String> {
    let mut chars = new.chars();
    let is_name = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_');
    if !is_name || super::complete::is_keyword(new) {
        return Err(format!("`{new}` is not a name"));
    }
    let found = occurrences(programs, open, file, position, true)
        .ok_or_else(|| "nothing here can be renamed".to_string())?;
    for std in std_dirs() {
        let std = uri_of(&std);
        if found.iter().any(|(uri, _)| uri.starts_with(&std)) {
            return Err("it is declared in the standard library".to_string());
        }
    }
    let mut changes: HashMap<String, Vec<Value>> = HashMap::new();
    for (uri, range) in found {
        changes
            .entry(uri)
            .or_default()
            .push(json!({ "range": range, "newText": new }));
    }
    Ok(json!({ "changes": changes }))
}

/// The file laid out by `wip fmt`, as one edit; `null` when it cannot be,
/// so the editor keeps what it has, and no edits for a file that asks to
/// be left as written.
pub fn format(path: &Path, text: &str) -> Value {
    if wip_fmt::switched_off(text) {
        return json!([]);
    }
    let Ok(style) = crate::format_style(path) else {
        return Value::Null;
    };
    let package = path.file_name().is_some_and(|n| n == "package.wip");
    match wip_fmt::format(text, style, package) {
        Ok(formatted) if formatted == text => json!([]),
        Ok(formatted) => json!([{ "range": Lines::new(text).whole(), "newText": formatted }]),
        Err(_) => Value::Null,
    }
}

// The protocol's numbers for kinds of symbol.
const NAMESPACE: u32 = 3;
const METHOD: u32 = 6;
const FIELD: u32 = 8;
const ENUM: u32 = 10;
const INTERFACE: u32 = 11;
const FUNCTION: u32 = 12;
const VARIABLE: u32 = 13;
const CONSTANT: u32 = 14;
const OBJECT: u32 = 19;
const ENUM_MEMBER: u32 = 22;
const STRUCT: u32 = 23;
const TYPE_PARAMETER: u32 = 26;

/// The file's declarations, and what each holds: its outline.
pub fn symbols(path: &Path, text: &str) -> Value {
    let parsed = crate::parse_file(&SourceFile {
        name: path.display().to_string(),
        text: text.to_string(),
    });
    let outline = Outline {
        lines: Lines::new(text),
        interner: &parsed.interner,
        text,
    };
    let symbols: Vec<Value> = parsed
        .ast
        .items
        .iter()
        .filter_map(|item| outline.item(item))
        .collect();
    json!(symbols)
}

struct Outline<'a> {
    lines: Lines<'a>,
    interner: &'a Interner,
    text: &'a str,
}

impl Outline<'_> {
    fn name(&self, name: &Name) -> String {
        self.interner.resolve(name.sym).to_string()
    }

    fn symbol(&self, name: String, kind: u32, span: Span, at: Span, children: Vec<Value>) -> Value {
        // What is selected must be inside what is shown.
        let lo = span.lo.min(at.lo) as usize;
        let hi = span.hi.max(at.hi) as usize;
        json!({
            "name": name,
            "kind": kind,
            "range": self.lines.range(lo, hi),
            "selectionRange": self.lines.range(at.lo as usize, at.hi as usize),
            "children": children,
        })
    }

    fn function(&self, f: &FnDecl) -> Value {
        let kind = if f.receiver.is_some() {
            METHOD
        } else {
            FUNCTION
        };
        self.symbol(
            self.name(&f.sig.name),
            kind,
            f.span,
            f.sig.name.span,
            Vec::new(),
        )
    }

    fn item(&self, item: &Item) -> Option<Value> {
        Some(match item {
            // An assert has no name to list.
            Item::Import(_) | Item::Assert(_) => return None,
            Item::Fn(f) => self.function(f),
            Item::Val(v) => self.symbol(
                self.name(&v.name),
                CONSTANT,
                v.span,
                v.name.span,
                Vec::new(),
            ),
            Item::Type(t) => self.symbol(
                self.name(&t.name),
                TYPE_PARAMETER,
                t.span,
                t.name.span,
                Vec::new(),
            ),
            Item::Struct(s) => {
                let mut children: Vec<Value> = s
                    .fields
                    .iter()
                    .map(|f| {
                        self.symbol(self.name(&f.name), FIELD, f.span, f.name.span, Vec::new())
                    })
                    .collect();
                children.extend(s.methods.iter().map(|m| self.function(m)));
                self.symbol(self.name(&s.name), STRUCT, s.span, s.name.span, children)
            }
            Item::Enum(e) => {
                let mut children: Vec<Value> = e
                    .variants
                    .iter()
                    .map(|v| {
                        self.symbol(
                            self.name(&v.name),
                            ENUM_MEMBER,
                            v.span,
                            v.name.span,
                            Vec::new(),
                        )
                    })
                    .collect();
                children.extend(e.methods.iter().map(|m| self.function(m)));
                self.symbol(self.name(&e.name), ENUM, e.span, e.name.span, children)
            }
            Item::Interface(i) => {
                let children = i
                    .methods
                    .iter()
                    .map(|m| {
                        self.symbol(
                            self.name(&m.sig.name),
                            METHOD,
                            m.span,
                            m.sig.name.span,
                            Vec::new(),
                        )
                    })
                    .collect();
                self.symbol(self.name(&i.name), INTERFACE, i.span, i.name.span, children)
            }
            Item::Extend(x) => {
                let path: Vec<String> = x.path.iter().map(|n| self.name(n)).collect();
                let mut name = match &x.slice_of {
                    Some(element) => format!("extend [{}]", self.name(element)),
                    None => format!("extend {}", path.join("::")),
                };
                if let Some(interface) = &x.interface {
                    name.push_str(&format!(": {}", self.name(interface)));
                }
                let at = x.path.first().map_or(x.span, |first| first.span);
                let children = x.methods.iter().map(|m| self.function(m)).collect();
                self.symbol(name, OBJECT, x.span, at, children)
            }
            Item::Extern(e) => {
                let mut children: Vec<Value> = Vec::new();
                for f in &e.fns {
                    let at = f.sig.name.span;
                    children.push(self.symbol(
                        self.name(&f.sig.name),
                        FUNCTION,
                        at,
                        at,
                        Vec::new(),
                    ));
                }
                for t in &e.types {
                    let at = t.name.span;
                    children.push(self.symbol(
                        self.name(&t.name),
                        TYPE_PARAMETER,
                        at,
                        at,
                        Vec::new(),
                    ));
                }
                for g in &e.globals {
                    children.push(self.symbol(
                        self.name(&g.name),
                        VARIABLE,
                        g.span,
                        g.name.span,
                        Vec::new(),
                    ));
                }
                for s in &e.structs {
                    if let Some(symbol) = self.item(&Item::Struct(s.clone())) {
                        children.push(symbol);
                    }
                }
                let abi = self
                    .text
                    .get(e.abi.lo as usize..e.abi.hi as usize)
                    .unwrap_or("");
                self.symbol(format!("extern {abi}"), NAMESPACE, e.span, e.abi, children)
            }
        })
    }
}
