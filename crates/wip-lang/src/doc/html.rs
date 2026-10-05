//! The documentation as pages: an index, a page per
//! module, and a page per type with methods, in plain HTML and one
//! stylesheet, readable from the disk.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::{Docs, Group, Item, Kind, Member, Module};

/// Writes every page of `docs` under `dir`, and answers how many pages
/// were written.
pub fn write_site(docs: &Docs, dir: &Path) -> Result<usize, String> {
    let site = Site { docs };
    let mut pages = vec![(PathBuf::from("index.html"), site.index())];
    for module in &docs.modules {
        pages.push((module_file(module), site.module_page(module)));
        for item in module.items().filter(|item| item.has_page()) {
            pages.push((item_file(module, item), site.item_page(module, item)));
        }
    }
    pages.push((PathBuf::from("style.css"), STYLE.to_string()));
    for (path, text) in &pages {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    }
    Ok(pages.len() - 1)
}

/// A module's page: `std/collections.html`, and `program.html` for a
/// program's root.
fn module_file(module: &Module) -> PathBuf {
    match module.path.is_empty() {
        true => PathBuf::from("program.html"),
        false => PathBuf::from(format!("{}.html", module.path.replace("::", "/"))),
    }
}

/// A type's page, in its module's directory: `std/collections/Map.html`.
fn item_file(module: &Module, item: &Item) -> PathBuf {
    let dir = match module.path.is_empty() {
        true => "program".to_string(),
        false => module.path.replace("::", "/"),
    };
    PathBuf::from(format!("{dir}/{}.html", file_name(&item.name)))
}

/// A name as a file's: `[T]` is `slice`.
fn file_name(name: &str) -> String {
    if name.starts_with('[') {
        return "slice".to_string();
    }
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The way from a page back to the site's root: `../` for each directory
/// it is in.
fn root_from(page: &Path) -> String {
    "../".repeat(page.components().count().saturating_sub(1))
}

struct Site<'a> {
    docs: &'a Docs,
}

impl Site<'_> {
    fn page(&self, file: &Path, title: &str, trail: &str, body: &str) -> String {
        let root = root_from(file);
        format!(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <title>{title}</title>\n<link rel=\"stylesheet\" href=\"{root}style.css\">\n\
             </head>\n<body>\n<nav><a href=\"{root}index.html\">index</a>{trail}</nav>\n\
             <main>\n{body}</main>\n</body>\n</html>\n",
            title = escape(title)
        )
    }

    fn index(&self) -> String {
        let file = PathBuf::from("index.html");
        let mut body = String::from("<h1>Documentation</h1>\n");
        for (group, heading) in [
            (Group::Program, "The program"),
            (Group::Dependency, "Packages it depends on"),
            (Group::Std, "The standard library"),
        ] {
            let modules: Vec<&Module> = self
                .docs
                .modules
                .iter()
                .filter(|m| m.group == group)
                .collect();
            if modules.is_empty() {
                continue;
            }
            let _ = writeln!(body, "<h2>{heading}</h2>\n<ul class=\"modules\">");
            for module in modules {
                let about = module
                    .files
                    .iter()
                    .map(|f| first_sentence(&f.about))
                    .find(|s| !s.is_empty())
                    .unwrap_or_default();
                let _ = writeln!(
                    body,
                    "<li><a href=\"{}\">{}</a> <span class=\"about\">{}</span></li>",
                    module_file(module).display(),
                    escape(module.title()),
                    inline(&about)
                );
            }
            body.push_str("</ul>\n");
        }
        // Every item by name, which a browser's find searches.
        let mut all: Vec<(String, String, String)> = Vec::new();
        for module in &self.docs.modules {
            for item in module.items() {
                let href = match item.has_page() {
                    true => item_file(module, item).display().to_string(),
                    false => format!("{}#{}", module_file(module).display(), anchor(&item.name)),
                };
                all.push((item.name.clone(), module.title().to_string(), href));
            }
        }
        all.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.1.cmp(&b.1))
        });
        body.push_str("<h2>Every item</h2>\n<ul class=\"every\">\n");
        for (name, module, href) in all {
            let _ = writeln!(
                body,
                "<li><a href=\"{href}\"><code>{}</code></a> <span class=\"about\">{}</span></li>",
                escape(&name),
                escape(&module)
            );
        }
        body.push_str("</ul>\n");
        self.page(&file, "Documentation", "", &body)
    }

    fn module_page(&self, module: &Module) -> String {
        let file = module_file(module);
        let root = root_from(&file);
        let mut body = format!("<h1>{}</h1>\n", escape(module.title()));
        for f in &module.files {
            if f.items.is_empty() && f.about.is_empty() {
                continue;
            }
            let _ = writeln!(
                body,
                "<section class=\"file\">\n<h2 class=\"file\">{}</h2>",
                escape(&f.name)
            );
            body.push_str(&prose(&f.about));
            for item in &f.items {
                body.push_str(&self.item_summary(module, item, &root, f));
            }
            body.push_str("</section>\n");
        }
        let trail = format!(" › {}", escape(module.title()));
        self.page(&file, module.title(), &trail, &body)
    }

    /// An item on its module's page: whole, or, for a type with a page of
    /// its own, its declaration and first paragraph, linked.
    fn item_summary(&self, module: &Module, item: &Item, root: &str, file: &super::File) -> String {
        let mut out = format!("<article id=\"{}\">\n", anchor(&item.name));
        let decl = self.decl(module, file, &item.decl, root);
        // A type with a page of its own: its name in the declaration links
        // there, as any type's name does.
        if item.has_page() {
            let _ = writeln!(out, "<pre class=\"decl\">{decl}</pre>");
            out.push_str(&prose(&first_paragraph(&item.doc)));
        } else {
            out.push_str(&self.item_body(module, item, root, file));
        }
        out.push_str("</article>\n");
        out
    }

    fn item_page(&self, module: &Module, item: &Item) -> String {
        let file = item_file(module, item);
        let root = root_from(&file);
        let home = module
            .files
            .iter()
            .find(|f| f.items.iter().any(|i| std::ptr::eq(i, item)))
            .expect("an item is in a file of its module");
        let mut body = format!("<h1>{}</h1>\n", escape(&item.name));
        body.push_str(&self.item_body(module, item, &root, home));
        let trail = format!(
            " › <a href=\"{root}{}\">{}</a> › {}",
            module_file(module).display(),
            escape(module.title()),
            escape(&item.name)
        );
        self.page(&file, &item.name, &trail, &body)
    }

    /// An item whole: its declaration, its documentation, its fields or
    /// variants, what it implements, and its methods.
    fn item_body(&self, module: &Module, item: &Item, root: &str, file: &super::File) -> String {
        let mut out = format!(
            "<pre class=\"decl\">{}</pre>\n",
            self.decl(module, file, &item.decl, root)
        );
        out.push_str(&prose(&item.doc));
        if !item.members.is_empty() {
            let heading = match item.kind {
                Kind::Enum => "Variants",
                Kind::Interface => "Methods",
                _ => "Fields",
            };
            let _ = writeln!(out, "<h3>{heading}</h3>");
            out.push_str(&self.members(module, file, &item.members, root));
        }
        if !item.implements.is_empty() {
            let names: Vec<String> = item
                .implements
                .iter()
                .map(|i| format!("<code>{}</code>", self.decl(module, file, i, root)))
                .collect();
            let _ = writeln!(
                out,
                "<p class=\"implements\">Implements {}.</p>",
                names.join(", ")
            );
        }
        for group in &item.methods {
            if group.methods.is_empty() {
                continue;
            }
            match &group.header {
                Some(header) => {
                    let _ = writeln!(
                        out,
                        "<h3>Methods <code>{}</code></h3>",
                        self.decl(module, file, header, root)
                    );
                }
                None => out.push_str("<h3>Methods</h3>\n"),
            }
            out.push_str(&self.members(module, file, &group.methods, root));
        }
        out
    }

    fn members(
        &self,
        module: &Module,
        file: &super::File,
        members: &[Member],
        root: &str,
    ) -> String {
        let mut out = String::from("<dl class=\"members\">\n");
        for m in members {
            let _ = writeln!(
                out,
                "<dt id=\"{}\"><code>{}</code></dt>\n<dd>{}</dd>",
                anchor(&m.name),
                self.decl(module, file, &m.decl, root),
                prose(&m.doc)
            );
        }
        out.push_str("</dl>\n");
        out
    }

    /// A declaration, escaped, with each type's name that is documented
    /// linked to it.
    fn decl(&self, module: &Module, file: &super::File, text: &str, root: &str) -> String {
        let mut out = String::new();
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String| {
            if word.is_empty() {
                return;
            }
            match self.find(module, file, word) {
                Some(href) => {
                    let _ = write!(out, "<a href=\"{root}{href}\">{}</a>", escape(word));
                }
                None => out.push_str(&escape(word)),
            }
            word.clear();
        };
        for c in text.chars() {
            if c.is_alphanumeric() || c == '_' {
                word.push(c);
            } else {
                flush(&mut word, &mut out);
                out.push_str(&escape(&c.to_string()));
            }
        }
        flush(&mut word, &mut out);
        out
    }

    /// Where a type of this name is documented, found as the checker finds
    /// a type's name: the module's own, what the file imports, then the
    /// prelude's.
    fn find(&self, module: &Module, file: &super::File, name: &str) -> Option<String> {
        let typed = |item: &Item| {
            item.name == name && item.kind != Kind::Function && item.kind != Kind::Constant
        };
        let link = |m: &Module, item: &Item| match item.has_page() {
            true => item_file(m, item).display().to_string(),
            false => format!("{}#{}", module_file(m).display(), anchor(&item.name)),
        };
        if let Some(item) = module.items().find(|i| typed(i)) {
            return Some(link(module, item));
        }
        if let Some((_, from)) = file.imports.iter().find(|(n, _)| n == name)
            && let Some(m) = self.docs.modules.iter().find(|m| &m.path == from)
            && let Some(item) = m.items().find(|i| typed(i))
        {
            return Some(link(m, item));
        }
        let prelude = self
            .docs
            .modules
            .iter()
            .find(|m| m.path == "std::prelude")?;
        let item = prelude.items().find(|i| typed(i))?;
        Some(link(prelude, item))
    }
}

/// An item's anchor: its name, as an HTML id can hold it.
fn anchor(name: &str) -> String {
    file_name(name)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A comment's text as HTML: paragraphs parted by a blank line, a line
/// indented four spaces a code example, and `code` code. Nothing else is
/// markup.
pub(super) fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut code: Vec<&str> = Vec::new();
    let flush_paragraph = |paragraph: &mut Vec<&str>, out: &mut String| {
        if !paragraph.is_empty() {
            let _ = writeln!(out, "<p>{}</p>", inline(&paragraph.join(" ")));
            paragraph.clear();
        }
    };
    let flush_code = |code: &mut Vec<&str>, out: &mut String| {
        if !code.is_empty() {
            let _ = writeln!(
                out,
                "<pre class=\"example\">{}</pre>",
                escape(&code.join("\n"))
            );
            code.clear();
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("    ") {
            flush_paragraph(&mut paragraph, &mut out);
            code.push(rest);
        } else if line.trim().is_empty() {
            flush_paragraph(&mut paragraph, &mut out);
            flush_code(&mut code, &mut out);
        } else {
            flush_code(&mut code, &mut out);
            paragraph.push(line.trim());
        }
    }
    flush_paragraph(&mut paragraph, &mut out);
    flush_code(&mut code, &mut out);
    out
}

/// One line of prose: escaped, with what is between backticks as code.
fn inline(text: &str) -> String {
    let mut out = String::new();
    for (i, piece) in text.split('`').enumerate() {
        match i % 2 {
            1 => {
                let _ = write!(out, "<code>{}</code>", escape(piece));
            }
            _ => out.push_str(&escape(piece)),
        }
    }
    out
}

fn first_paragraph(text: &str) -> String {
    text.split("\n\n").next().unwrap_or_default().to_string()
}

/// What a module's index line says of it: its first file's first
/// sentence.
fn first_sentence(text: &str) -> String {
    let joined = text
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match joined.find(". ") {
        Some(end) => joined[..=end].to_string(),
        None => joined,
    }
}

const STYLE: &str = r#":root {
  --bg: #fdfdfc; --fg: #1d1d1b; --muted: #6b6b66; --line: #e4e4df;
  --code-bg: #f3f3ef; --link: #2b5fb4; --accent: #8a5a00;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #17181a; --fg: #e6e6e3; --muted: #9a9a94; --line: #2c2d30;
    --code-bg: #212226; --link: #7aa7f0; --accent: #e0b25c;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--bg); color: var(--fg);
  font: 16px/1.55 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
nav {
  padding: 10px 24px; border-bottom: 1px solid var(--line);
  color: var(--muted); font-size: 14px;
}
main { max-width: 860px; margin: 0 auto; padding: 8px 24px 64px; }
a { color: var(--link); text-decoration: none; }
a:hover { text-decoration: underline; }
h1 { font-size: 28px; margin: 24px 0 8px; }
h2 { font-size: 20px; margin: 32px 0 8px; }
h2.file { color: var(--muted); font-size: 14px; font-weight: 600; letter-spacing: 0.02em; }
h3 { font-size: 16px; margin: 20px 0 6px; }
code, pre { font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; }
code { background: var(--code-bg); padding: 1px 4px; border-radius: 4px; }
pre { background: var(--code-bg); padding: 10px 12px; border-radius: 6px; overflow-x: auto; }
pre.decl { border-left: 3px solid var(--accent); }
pre code, dt code { background: none; padding: 0; }
article { margin: 18px 0 26px; }
section.file { border-top: 1px solid var(--line); margin-top: 28px; }
dl.members dt { margin-top: 12px; }
dl.members dd { margin: 4px 0 0 20px; color: var(--fg); }
dl.members dd p { margin: 4px 0; }
.about, .implements { color: var(--muted); }
ul.modules, ul.every { list-style: none; padding: 0; }
ul.modules li, ul.every li { margin: 4px 0; }
"#;
