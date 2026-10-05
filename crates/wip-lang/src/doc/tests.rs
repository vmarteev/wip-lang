use super::*;

/// A module of two files: a type declared in one and extended in the
/// other, with what is and is not `pub`, a derive, an interface, a
/// constrained block, a constant and an alias.
fn shapes() -> Module {
    let first = "\
// Shapes, and what they cover.
//
// A second paragraph.

import std::collections::{Map}

/// A point on the plane.
@derive(Eq, Text)
pub struct Point {
    /// Across.
    pub x: f64
    hidden: f64
}

/// What a shape is.
pub enum Shape {
    /// A circle, by its radius.
    Round(radius: f64)
    Dot
}

fn private(): i64 = 0

/// The largest there is.
pub val LIMIT: i64 = 10

/// Another name.
pub type Points = Map<str, Point>
";
    let second = "\
// Methods of a point.

extend Point {
    /// How far from the origin.
    ///
    ///     val d = p.length()
    pub fn length(): f64 = self.x

    fn helper(): f64 = 0.0
}

extend Point: Clone {
    fn clone(): Point = Point(x: self.x, hidden: 0.0)
}

extend Pair<T: Eq> {
    pub fn same(): bool = true
}

/// Two of a kind.
pub struct Pair<T> {
    pub a: T
}
";
    read_module(
        "shapes",
        Group::Program,
        &[
            ("shapes/first.wip".to_string(), first.to_string()),
            ("shapes/second.wip".to_string(), second.to_string()),
        ],
    )
}

fn item<'a>(module: &'a Module, name: &str) -> &'a Item {
    module
        .items()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("`{name}` is documented"))
}

#[test]
fn what_is_pub_is_documented_with_its_comment() {
    let module = shapes();
    let names: Vec<&str> = module.items().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["Point", "Shape", "LIMIT", "Points", "Pair"]);
    let point = item(&module, "Point");
    assert_eq!(point.kind, Kind::Struct);
    assert_eq!(point.decl, "@derive(Eq, Text) struct Point");
    assert_eq!(point.doc, "A point on the plane.");
    // Its `pub` field alone, with the field's own comment.
    let fields: Vec<(&str, &str)> = point
        .members
        .iter()
        .map(|m| (m.decl.as_str(), m.doc.as_str()))
        .collect();
    assert_eq!(fields, [("x: f64", "Across.")]);
    let shape = item(&module, "Shape");
    assert_eq!(shape.members.len(), 2);
    assert_eq!(shape.members[0].decl, "Round(radius: f64)");
    assert_eq!(item(&module, "LIMIT").decl, "val LIMIT: i64");
    assert_eq!(
        item(&module, "Points").decl,
        "type Points = Map<str, Point>"
    );
}

#[test]
fn a_types_methods_and_interfaces_are_gathered_from_every_file() {
    let module = shapes();
    let point = item(&module, "Point");
    assert_eq!(point.implements, ["Eq", "Text", "Clone"]);
    let methods: Vec<&str> = point
        .methods
        .iter()
        .flat_map(|g| &g.methods)
        .map(|m| m.decl.as_str())
        .collect();
    assert_eq!(methods, ["fn length(): f64"], "`helper` is not `pub`");
    let length = &point.methods[0].methods[0];
    assert_eq!(
        length.doc,
        "How far from the origin.\n\n    val d = p.length()"
    );
    // A block that asks more of the type's parameters keeps its header.
    let pair = item(&module, "Pair");
    assert_eq!(
        pair.methods[0].header.as_deref(),
        Some("extend Pair<T: Eq>")
    );
    assert!(pair.has_page() && point.has_page() && !item(&module, "Shape").has_page());
}

#[test]
fn a_file_says_what_it_is_for_in_its_opening_paragraph() {
    let module = shapes();
    assert_eq!(
        module.files[0].about,
        "Shapes, and what they cover.\n\nA second paragraph."
    );
    assert_eq!(module.files[1].about, "Methods of a point.");
    assert_eq!(file_paragraph("/// An item's.\nfn a() = {}\n"), "");
    assert_eq!(
        module.files[0].imports,
        [("Map".to_string(), "std::collections".to_string())]
    );
}

#[test]
fn a_built_in_types_methods_are_its_own_item() {
    let module = read_module(
        "std::prelude",
        Group::Std,
        &[(
            "std/prelude/text.wip".to_string(),
            "extend str {\n    /// Twice.\n    pub fn twice(): String = String()\n}\n".to_string(),
        )],
    );
    let text = item(&module, "str");
    assert_eq!(text.kind, Kind::BuiltIn);
    assert_eq!(text.methods[0].methods[0].doc, "Twice.");
}

#[test]
fn an_item_is_found_by_its_path_or_its_name() {
    let docs = Docs {
        modules: vec![shapes()],
    };
    let point = print_item(&docs, "shapes::Point").expect("found");
    assert!(point.starts_with("shapes::Point\n\n@derive(Eq, Text) struct Point\n"));
    assert!(point.contains("Implements Eq, Text, Clone."));
    let by_name = print_item(&docs, "Point").expect("found by its name alone");
    assert_eq!(by_name, point);
    let method = print_item(&docs, "Point::length").expect("a method");
    assert!(method.starts_with("shapes::Point::length\n\nfn length(): f64\n"));
    let module = print_item(&docs, "shapes").expect("a module alone");
    assert!(module.starts_with("shapes\n") && module.contains("struct Point"));
    assert!(print_item(&docs, "shapes::Nothing").is_err());
    assert!(print_item(&docs, "Point::nothing").is_err());
}

#[test]
fn a_comment_is_paragraphs_code_and_examples() {
    let html = html::prose("One `a<b>` line\nand more.\n\n    let x = 1\n\nTwo.");
    assert_eq!(
        html,
        "<p>One <code>a&lt;b&gt;</code> line and more.</p>\n\
         <pre class=\"example\">let x = 1</pre>\n<p>Two.</p>\n"
    );
}

/// The whole standard library is written, and every link on its pages
/// is to a page that was written and to an anchor that page has: what
/// the gate asks of `wip doc`.
#[test]
fn the_standard_librarys_pages_link_only_to_what_they_wrote() {
    let docs = read(None).expect("the standard library is in the compiler");
    let dir = crate::temp_dir::TempDir::new().expect("a temporary directory");
    let pages = write_site(&docs, dir.path()).expect("written");
    assert!(pages > 50, "{pages} pages");
    let mut ids = std::collections::HashMap::new();
    let mut files = Vec::new();
    let mut stack = vec![dir.path().to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).expect("a directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "html") {
                let text = std::fs::read_to_string(&path).expect("a page");
                let found: Vec<String> = text
                    .split("id=\"")
                    .skip(1)
                    .filter_map(|s| s.split('"').next().map(str::to_string))
                    .collect();
                ids.insert(path.clone(), found);
                files.push((path, text));
            }
        }
    }
    for (path, text) in &files {
        for href in text
            .split("href=\"")
            .skip(1)
            .filter_map(|s| s.split('"').next())
        {
            let (target, anchor) = href.split_once('#').unwrap_or((href, ""));
            let file = match target.is_empty() {
                true => path.clone(),
                false => path.parent().expect("a page's directory").join(target),
            };
            let file = normalize(&file);
            assert!(
                file.exists(),
                "{} links to {href}, which is not there",
                path.display()
            );
            if !anchor.is_empty()
                && let Some(ids) = ids.get(&file)
            {
                assert!(
                    ids.iter().any(|id| id == anchor),
                    "{} links to {href}, which has no such anchor",
                    path.display()
                );
            }
        }
    }
}

/// A path with its `..` steps taken.
fn normalize(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
