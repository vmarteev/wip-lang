//! One item of the documentation, printed: what
//! `wip doc std::collections::Map` or `wip doc str::findLast` shows at a
//! terminal.

use std::fmt::Write as _;

use super::{Docs, Item, Kind, Member, Module};

/// The item a path names, as text: `std::collections::Map`, a method
/// `std::collections::Map::get`, or a type found by name alone, `str` and
/// `str::findLast`. Where the path names nothing, what it might have meant.
pub fn print_item(docs: &Docs, path: &str) -> Result<String, String> {
    let segments: Vec<&str> = path.split("::").collect();
    // The longest start of the path that is a module, and what follows.
    let (module, rest) = (0..=segments.len())
        .rev()
        .find_map(|n| {
            let prefix = segments[..n].join("::");
            docs.modules
                .iter()
                .find(|m| m.path == prefix && (n > 0 || !m.path.is_empty()))
                .map(|m| (Some(m), &segments[n..]))
        })
        .unwrap_or((None, &segments[..]));
    let Some((&name, member)) = rest.split_first() else {
        // A module alone: its items, each a line.
        let module = module.expect("a path names a module where nothing follows it");
        return Ok(module_text(module));
    };
    let found: Vec<(&Module, &Item)> = match module {
        Some(m) => m
            .items()
            .filter(|i| i.name == name)
            .map(|i| (m, i))
            .collect(),
        None => docs
            .modules
            .iter()
            .flat_map(|m| m.items().filter(|i| i.name == name).map(move |i| (m, i)))
            .collect(),
    };
    let (module, item) = match found.as_slice() {
        [] => return Err(format!("nothing is documented as `{path}`")),
        [one] => *one,
        many => {
            // A type and a function may share a name across modules; the
            // program's own comes first, as the index lists it.
            let choices: Vec<String> = many
                .iter()
                .map(|(m, i)| format!("  {}::{}", m.path, i.name))
                .collect();
            return Err(format!(
                "`{path}` names more than one item; say which:\n{}",
                choices.join("\n")
            ));
        }
    };
    match member {
        [] => Ok(item_text(module, item)),
        [member] => {
            let members = item
                .members
                .iter()
                .chain(item.methods.iter().flat_map(|g| &g.methods));
            let found: Vec<&Member> = members.filter(|m| m.name == *member).collect();
            match found.as_slice() {
                [] => Err(format!(
                    "`{}` has nothing documented as `{member}`",
                    item.name
                )),
                [m] => Ok(member_text(module, item, m)),
                many => Ok(many
                    .iter()
                    .map(|m| member_text(module, item, m))
                    .collect::<Vec<_>>()
                    .join("\n")),
            }
        }
        _ => Err(format!("nothing is documented as `{path}`")),
    }
}

fn module_text(module: &Module) -> String {
    let mut out = format!("{}\n", module.title());
    for file in &module.files {
        for item in &file.items {
            let _ = writeln!(out, "\n{}", item.decl);
            if let Some(first) = item.doc.split("\n\n").next().filter(|s| !s.is_empty()) {
                let _ = writeln!(out, "{}", indent(first, 4));
            }
        }
    }
    out
}

fn item_text(module: &Module, item: &Item) -> String {
    let mut out = format!("{}::{}\n\n{}\n", module.path, item.name, item.decl);
    if !item.doc.is_empty() {
        let _ = writeln!(out, "\n{}", item.doc);
    }
    if !item.members.is_empty() {
        let heading = match item.kind {
            Kind::Enum => "Variants",
            Kind::Interface => "Methods",
            _ => "Fields",
        };
        let _ = writeln!(out, "\n{heading}:");
        for m in &item.members {
            out.push_str(&member_lines(m));
        }
    }
    if !item.implements.is_empty() {
        let _ = writeln!(out, "\nImplements {}.", item.implements.join(", "));
    }
    for group in &item.methods {
        if group.methods.is_empty() {
            continue;
        }
        match &group.header {
            Some(header) => {
                let _ = writeln!(out, "\nMethods, {header}:");
            }
            None => out.push_str("\nMethods:\n"),
        }
        for m in &group.methods {
            out.push_str(&member_lines(m));
        }
    }
    out
}

fn member_text(module: &Module, item: &Item, member: &Member) -> String {
    let mut out = format!(
        "{}::{}::{}\n\n{}\n",
        module.path, item.name, member.name, member.decl
    );
    if !member.doc.is_empty() {
        let _ = writeln!(out, "\n{}", member.doc);
    }
    out
}

fn member_lines(member: &Member) -> String {
    let mut out = format!("  {}\n", member.decl);
    if !member.doc.is_empty() {
        out.push_str(&indent(&member.doc, 6));
        out.push('\n');
    }
    out
}

fn indent(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    text.lines()
        .map(|l| match l.is_empty() {
            true => String::new(),
            false => format!("{pad}{l}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
