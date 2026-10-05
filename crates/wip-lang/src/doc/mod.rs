//! `wip doc`: a program's documentation, from what its source says of
//! itself. Each file is read on its own — its opening `//`
//! paragraph, and its `pub` items with the `///` lines above them — into
//! the model here, which `html` writes as pages and `text` prints one item
//! of. Nothing is type-checked: a declaration is shown as it was written.

mod html;
mod text;

pub use html::write_site;
pub use text::print_item;

use std::path::Path;

use wip_syntax::ast::{self, Ast, Item as AstItem};
use wip_syntax::{Interner, Span};

use crate::lsp::navigate::documentation;
use crate::{Loaded, Timings};

/// Every module documented, the program's first, then its dependencies',
/// then the standard library's.
pub struct Docs {
    pub modules: Vec<Module>,
}

/// Whose a module is, which says where the index lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Program,
    Dependency,
    Std,
}

/// One module: its path, `std::collections`, empty for a program's root,
/// and its files in order.
pub struct Module {
    pub path: String,
    pub group: Group,
    pub files: Vec<File>,
}

/// One file of a module: its name, the `//` paragraph it begins with, and
/// its `pub` items in order.
pub struct File {
    pub name: String,
    pub about: String,
    pub items: Vec<Item>,
    /// What the file imports by name, and from which module: how a name in
    /// a declaration is found, for its link.
    pub imports: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Struct,
    Enum,
    Interface,
    Function,
    Constant,
    Alias,
    /// A type the language has, whose methods the prelude writes:
    /// `str`, `i64`, `[T]`.
    BuiltIn,
    /// A C type, function or variable a module binds.
    C,
}

/// A `pub` item: its declaration as written, its `///` lines, and, for a
/// type, its fields or variants, its methods and what it implements.
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub decl: String,
    pub doc: String,
    pub members: Vec<Member>,
    pub methods: Vec<Methods>,
    pub implements: Vec<String>,
}

/// A field, a variant, a method or an interface's method.
pub struct Member {
    pub name: String,
    pub decl: String,
    pub doc: String,
}

/// Methods from one place: the type's own body, or an `extend` block,
/// whose header is kept where it says more than the type does —
/// `extend Vec<T: Eq>`, methods only where `T` has `==`.
pub struct Methods {
    pub header: Option<String>,
    pub methods: Vec<Member>,
}

impl Item {
    /// Whether the item has a page of its own: a type with methods.
    pub fn has_page(&self) -> bool {
        matches!(self.kind, Kind::Struct | Kind::Enum | Kind::BuiltIn)
            && self.methods.iter().any(|m| !m.methods.is_empty())
    }
}

impl Module {
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.files.iter().flat_map(|f| &f.items)
    }

    /// How the module is named on its page: its path, or the program's
    /// root module.
    pub fn title(&self) -> &str {
        if self.path.is_empty() {
            "the program"
        } else {
            &self.path
        }
    }
}

/// The documentation of the program at `entry`, with its dependencies and
/// the standard library; or of the standard library alone, where `entry`
/// is `None`.
pub fn read(entry: Option<&Path>) -> Result<Docs, String> {
    let mut modules = Vec::new();
    if let Some(entry) = entry {
        let loaded = crate::load_mode(entry, crate::Mode::Program, &mut Timings::default())?;
        modules.extend(program_modules(&loaded));
    }
    for path in crate::modules() {
        let files = crate::std_library::std_files(path, crate::targets::Target::host());
        let files: Vec<(String, String)> = files
            .into_iter()
            .filter(|(name, _)| !name.ends_with(".test.wip"))
            .collect();
        modules.push(read_module(path, Group::Std, &files));
    }
    Ok(Docs { modules })
}

/// The program's modules and its dependencies', from what was loaded: the
/// standard library's are read whole instead, below.
fn program_modules(loaded: &Loaded) -> Vec<Module> {
    let mut out = Vec::new();
    for module in &loaded.modules {
        if module.path == "std" || module.path.starts_with("std::") {
            continue;
        }
        // Each file's name and text, by where its items' spans lie.
        let files: Vec<(String, String)> = module
            .files
            .iter()
            .filter_map(|parsed| {
                let span = item_span(parsed.ast.items.first()?);
                let (name, text, _) = loaded.sources.of(span)?;
                Some((name.to_string(), text.to_string()))
            })
            .collect();
        // A dependency's modules are written from its name; the program's
        // own from nothing.
        let group = match module.prefix.is_empty() {
            true => Group::Program,
            false => Group::Dependency,
        };
        out.push(read_module(&module.path, group, &files));
    }
    out
}

/// A module's files read, and the methods of each type gathered from every
/// file onto the type.
fn read_module(path: &str, group: Group, files: &[(String, String)]) -> Module {
    let mut read: Vec<(File, Vec<Extension>)> = files
        .iter()
        .map(|(name, text)| read_file(name, text))
        .collect();
    // An `extend` block's methods go to the type wherever it is declared
    // in the module; one for a type the module does not declare is a
    // built-in type's, which the prelude extends, and has an item made
    // for it in the file of its first block.
    let extensions: Vec<(usize, Extension)> = read
        .iter_mut()
        .enumerate()
        .flat_map(|(i, (_, ext))| std::mem::take(ext).into_iter().map(move |e| (i, e)))
        .collect();
    let mut files: Vec<File> = read.into_iter().map(|(file, _)| file).collect();
    for (at, extension) in extensions {
        let found = files
            .iter_mut()
            .flat_map(|f| f.items.iter_mut())
            .find(|item| item.name == extension.target && is_type(item.kind));
        let item = match found {
            Some(item) => item,
            None if extension.declared_here => continue,
            None => {
                if extension.interface.is_some() && extension.methods.is_empty() {
                    // An interface implemented for a type of another
                    // module's, as a built-in one may be:
                    // listed with the built-in type in the prelude, not
                    // here.
                    if path != "std::prelude" {
                        continue;
                    }
                }
                files[at].items.push(Item {
                    kind: Kind::BuiltIn,
                    name: extension.target.clone(),
                    decl: format!("built-in type {}", extension.target),
                    doc: String::new(),
                    members: Vec::new(),
                    methods: Vec::new(),
                    implements: Vec::new(),
                });
                files[at].items.last_mut().expect("just pushed")
            }
        };
        if let Some(interface) = extension.interface {
            if !item.implements.contains(&interface) {
                item.implements.push(interface);
            }
            continue;
        }
        if extension.methods.is_empty() {
            continue;
        }
        item.methods.push(Methods {
            header: extension.header,
            methods: extension.methods,
        });
    }
    // A type's own methods first, then those of blocks that ask more of
    // its parameters, each kind in the order the files have them.
    // Blocks with the same header are one group, wherever they are.
    for item in files.iter_mut().flat_map(|f| f.items.iter_mut()) {
        let mut merged: Vec<Methods> = Vec::new();
        for group in std::mem::take(&mut item.methods) {
            match merged.iter_mut().find(|m| m.header == group.header) {
                Some(same) => same.methods.extend(group.methods),
                None => merged.push(group),
            }
        }
        merged.sort_by_key(|group| group.header.is_some());
        item.methods = merged;
    }
    Module {
        path: path.to_string(),
        group,
        files,
    }
}

fn is_type(kind: Kind) -> bool {
    matches!(kind, Kind::Struct | Kind::Enum | Kind::BuiltIn | Kind::C)
}

/// What an `extend` block adds to a type: the type's name as an item is
/// named, the interface it implements if it does, its `pub` methods, and
/// its header where it constrains the type's parameters.
struct Extension {
    target: String,
    interface: Option<String>,
    methods: Vec<Member>,
    header: Option<String>,
    /// Whether the module declares a type of that name, so that a block
    /// for a type not found is not a built-in's.
    declared_here: bool,
}

/// One file read on its own: its paragraph, its items, and its `extend`
/// blocks, which the module gives to their types.
fn read_file(name: &str, text: &str) -> (File, Vec<Extension>) {
    let mut interner = Interner::new();
    let lexed = wip_syntax::lex(text, &mut interner);
    let ast = wip_syntax::parse(text, &lexed).ast;
    let reader = Reader {
        text,
        ast: &ast,
        interner: &interner,
    };
    let mut items = Vec::new();
    let mut extensions = Vec::new();
    for item in &ast.items {
        match item {
            AstItem::Extend(block) => extensions.push(reader.extension(block)),
            _ => items.extend(reader.item(item)),
        }
    }
    let declared: Vec<String> = items.iter().map(|i: &Item| i.name.clone()).collect();
    for extension in &mut extensions {
        extension.declared_here = declared.contains(&extension.target);
    }
    let imports = ast
        .items
        .iter()
        .filter_map(|item| match item {
            AstItem::Import(import) => Some(reader.imports(import)),
            _ => None,
        })
        .flatten()
        .collect();
    let file = File {
        name: name.to_string(),
        about: file_paragraph(text),
        items,
        imports,
    };
    (file, extensions)
}

/// The `//` lines a file begins with, which say what it is for: the
/// paragraph that opens every file of the standard library. A blank
/// comment line parts paragraphs, as a blank `///` line does.
pub fn file_paragraph(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("//") else {
            break;
        };
        if rest.starts_with('/') {
            break;
        }
        lines.push(rest.strip_prefix(' ').unwrap_or(rest));
    }
    lines.join("\n")
}

struct Reader<'a> {
    text: &'a str,
    ast: &'a Ast,
    interner: &'a Interner,
}

impl Reader<'_> {
    fn name(&self, name: ast::Name) -> String {
        self.interner.resolve(name.sym).to_string()
    }

    fn source(&self, lo: u32, hi: u32) -> &str {
        &self.text[lo as usize..hi as usize]
    }

    /// The `///` lines above a declaration that starts at `at`.
    fn doc(&self, at: u32) -> String {
        documentation(self.text, at as usize)
    }

    /// Where a declaration begins: its first annotation, or itself.
    fn start(annotations: &[ast::Annotation], span: Span) -> u32 {
        annotations.first().map_or(span.lo, |a| a.span.lo)
    }

    /// Text as a declaration line: whitespace run together, `pub` left
    /// out, since everything shown is.
    fn line(text: &str) -> String {
        let joined: Vec<&str> = text.split_whitespace().collect();
        let line = joined.join(" ");
        // A list written a line an element ends with a comma, which a line
        // of its own has no use for.
        let line = line
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(",)", ")")
            .replace(", >", ">")
            .replace(",>", ">");
        line.strip_prefix("pub ")
            .map_or(line.clone(), str::to_string)
    }

    fn item(&self, item: &AstItem) -> Vec<Item> {
        match item {
            AstItem::Fn(f) if f.is_pub => vec![Item {
                kind: Kind::Function,
                name: self.name(f.sig.name),
                decl: self.fn_decl(f),
                doc: self.doc(f.span.lo),
                members: Vec::new(),
                methods: Vec::new(),
                implements: Vec::new(),
            }],
            AstItem::Struct(s) if s.is_pub => vec![self.struct_item(s, Kind::Struct)],
            AstItem::Enum(e) if e.is_pub => vec![self.enum_item(e)],
            AstItem::Interface(i) if i.is_pub => vec![self.interface_item(i)],
            AstItem::Val(v) if v.is_pub => {
                let decl = match v.ty {
                    Some(ty) => format!(
                        "val {}: {}",
                        self.name(v.name),
                        self.source(self.ast.types[ty].span.lo, self.ast.types[ty].span.hi)
                    ),
                    None => format!("val {}", self.name(v.name)),
                };
                vec![Item {
                    kind: Kind::Constant,
                    name: self.name(v.name),
                    decl,
                    doc: self.doc(Self::start(&v.annotations, v.span)),
                    members: Vec::new(),
                    methods: Vec::new(),
                    implements: Vec::new(),
                }]
            }
            AstItem::Type(t) if t.is_pub => vec![Item {
                kind: Kind::Alias,
                name: self.name(t.name),
                decl: Self::line(self.source(t.span.lo, t.span.hi)),
                doc: self.doc(t.span.lo),
                members: Vec::new(),
                methods: Vec::new(),
                implements: Vec::new(),
            }],
            AstItem::Extern(block) => self.extern_items(block),
            _ => Vec::new(),
        }
    }

    /// A function's declaration: from its annotations or its first word to
    /// the end of its signature, `= …` and the body left out.
    fn fn_decl(&self, f: &ast::FnDecl) -> String {
        let annotations = self.annotations(&f.annotations);
        let head = Self::line(self.source(f.span.lo, f.sig.span.hi));
        let head = head
            .strip_prefix(annotations.trim())
            .unwrap_or(&head)
            .trim()
            .to_string();
        match annotations.is_empty() {
            true => head,
            false => format!("{annotations} {head}"),
        }
    }

    /// The annotations a declaration carries, as written, but for the
    /// compiler's own: `@intrinsic` says how the compiler writes the body,
    /// which a reader of the documentation has no use for.
    fn annotations(&self, annotations: &[ast::Annotation]) -> String {
        let shown: Vec<String> = annotations
            .iter()
            .filter(|a| self.name(a.name) != "intrinsic")
            .map(|a| Self::line(self.source(a.span.lo, a.span.hi)))
            .collect();
        shown.join(" ")
    }

    fn methods(&self, methods: &[ast::FnDecl]) -> Vec<Member> {
        methods
            .iter()
            .filter(|m| m.is_pub)
            .map(|m| Member {
                name: self.name(m.sig.name),
                decl: self.fn_decl(m),
                doc: self.doc(m.span.lo),
            })
            .collect()
    }

    /// A type's header: `struct Pair<A, B>`, its annotations before it.
    fn type_header(
        &self,
        annotations: &[ast::Annotation],
        lo: u32,
        name_hi: u32,
        generics: &[ast::GenericParam],
    ) -> String {
        let hi = generics.last().map_or(name_hi, |g| {
            // The `>` after the last parameter.
            let after = &self.text[g.span.hi as usize..];
            g.span.hi + after.find('>').map_or(0, |i| i as u32 + 1)
        });
        let head = Self::line(self.source(lo, hi));
        let annotations = self.annotations(annotations);
        let head = head
            .strip_prefix(annotations.trim())
            .unwrap_or(&head)
            .trim()
            .to_string();
        match annotations.is_empty() {
            true => head,
            false => format!("{annotations} {head}"),
        }
    }

    fn struct_item(&self, s: &ast::StructDecl, kind: Kind) -> Item {
        let lo = Self::start(&s.annotations, s.span);
        let members = s
            .fields
            .iter()
            .filter(|f| f.is_pub || s.is_extern)
            .map(|f| Member {
                name: self.name(f.name),
                decl: Self::line(self.source(f.span.lo, f.span.hi)),
                doc: self.doc(f.span.lo),
            })
            .collect();
        let methods = self.methods(&s.methods);
        Item {
            kind,
            name: self.name(s.name),
            decl: self.type_header(&s.annotations, s.span.lo, s.name.span.hi, &s.generics),
            doc: self.doc(lo),
            members,
            methods: own_methods(methods),
            implements: self.derived(&s.annotations),
        }
    }

    fn enum_item(&self, e: &ast::EnumDecl) -> Item {
        let lo = Self::start(&e.annotations, e.span);
        let members = e
            .variants
            .iter()
            .map(|v| Member {
                name: self.name(v.name),
                decl: Self::line(self.source(v.span.lo, v.span.hi)),
                doc: self.doc(v.span.lo),
            })
            .collect();
        let methods = self.methods(&e.methods);
        Item {
            kind: Kind::Enum,
            name: self.name(e.name),
            decl: self.type_header(&e.annotations, e.span.lo, e.name.span.hi, &e.generics),
            doc: self.doc(lo),
            members,
            methods: own_methods(methods),
            implements: self.derived(&e.annotations),
        }
    }

    /// The interfaces `@derive` writes for a type.
    fn derived(&self, annotations: &[ast::Annotation]) -> Vec<String> {
        annotations
            .iter()
            .filter(|a| self.name(a.name) == "derive")
            .flat_map(|a| &a.args)
            .map(|arg| Self::line(self.source(arg.span.lo, arg.span.hi)))
            .collect()
    }

    fn interface_item(&self, i: &ast::InterfaceDecl) -> Item {
        let lo = Self::start(&i.annotations, i.span);
        let members = i
            .methods
            .iter()
            .map(|m| Member {
                name: self.name(m.sig.name),
                decl: Self::line(self.source(m.span.lo, m.sig.span.hi)),
                doc: self.doc(m.span.lo),
            })
            .collect();
        Item {
            kind: Kind::Interface,
            name: self.name(i.name),
            decl: self.type_header(&i.annotations, i.span.lo, i.name.span.hi, &i.generics),
            doc: self.doc(lo),
            members,
            methods: Vec::new(),
            implements: Vec::new(),
        }
    }

    /// A C block's `pub` types, functions, variables and structs.
    fn extern_items(&self, block: &ast::ExternBlock) -> Vec<Item> {
        let c = |name: String, decl: String, doc: String| Item {
            kind: Kind::C,
            name,
            decl,
            doc,
            members: Vec::new(),
            methods: Vec::new(),
            implements: Vec::new(),
        };
        let mut items = Vec::new();
        for t in block.types.iter().filter(|t| t.is_pub) {
            let lo = Self::start(&t.annotations, t.name.span);
            items.push(c(
                self.name(t.name),
                format!("type {}", self.name(t.name)),
                self.doc(lo),
            ));
        }
        for s in block.structs.iter().filter(|s| s.is_pub) {
            items.push(self.struct_item(s, Kind::C));
        }
        for f in block.fns.iter().filter(|f| f.is_pub) {
            let lo = Self::start(&f.annotations, f.sig.span);
            items.push(c(
                self.name(f.sig.name),
                Self::line(self.source(f.sig.span.lo, f.sig.span.hi)),
                self.doc(lo),
            ));
        }
        for g in block.globals.iter().filter(|g| g.is_pub) {
            let lo = Self::start(&g.annotations, g.span);
            items.push(c(
                self.name(g.name),
                Self::line(self.source(g.span.lo, g.span.hi)),
                self.doc(lo),
            ));
        }
        items
    }

    /// The names an import brings in, each with its module:
    /// `import std::collections::{Map, Set}`.
    fn imports(&self, import: &ast::ImportDecl) -> Vec<(String, String)> {
        let path: Vec<String> = import.path.iter().map(|n| self.name(*n)).collect();
        let module = path.join("::");
        match &import.items {
            Some(items) => items
                .iter()
                .filter_map(|item| item.name.map(|n| (self.name(n), module.clone())))
                .collect(),
            None => Vec::new(),
        }
    }

    fn extension(&self, block: &ast::ExtendBlock) -> Extension {
        let target = match (block.slice_of, block.path.last()) {
            (Some(elem), _) => format!("[{}]", self.name(elem)),
            (None, Some(name)) => self.name(*name),
            (None, None) => String::new(),
        };
        let interface = block.interface.map(|i| {
            let args = block
                .interface_args
                .as_ref()
                .map(|a| Self::line(self.source(a.span.lo, a.span.hi)))
                .unwrap_or_default();
            format!("{}{args}", self.name(i))
        });
        // The header says more than the type where it constrains a
        // parameter: `extend Vec<T: Eq>`.
        let header = block
            .generics
            .iter()
            .any(|g| !g.bounds.is_empty())
            .then(|| {
                let lo = block.span.lo;
                let open = self.text[lo as usize..]
                    .find('{')
                    .map_or(block.span.hi, |i| lo + i as u32);
                Self::line(self.source(lo, open))
            });
        Extension {
            target,
            interface,
            methods: if block.interface.is_some() {
                Vec::new()
            } else {
                self.methods(&block.methods)
            },
            header,
            declared_here: false,
        }
    }
}

/// The methods a type's own body declares, as the one group they are.
fn own_methods(methods: Vec<Member>) -> Vec<Methods> {
    match methods.is_empty() {
        true => Vec::new(),
        false => vec![Methods {
            header: None,
            methods,
        }],
    }
}

/// Where an item's text begins, to find the file it was read from.
fn item_span(item: &AstItem) -> Span {
    match item {
        AstItem::Import(i) => i.span,
        AstItem::Val(v) => v.span,
        AstItem::Struct(s) => s.span,
        AstItem::Enum(e) => e.span,
        AstItem::Fn(f) => f.span,
        AstItem::Extern(e) => e.span,
        AstItem::Extend(e) => e.span,
        AstItem::Interface(i) => i.span,
        AstItem::Type(t) => t.span,
        AstItem::Assert(a) => a.span,
    }
}

/// The program at `entry`, read for one item: what a program at the
/// current directory would be, or the standard library alone where there is
/// none.
pub fn read_for_lookup(entry: &Path) -> Docs {
    match read(Some(entry)) {
        Ok(docs) => docs,
        Err(_) => read(None).expect("the standard library is in the compiler"),
    }
}

#[cfg(test)]
mod tests;
