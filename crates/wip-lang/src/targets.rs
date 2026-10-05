//! What a program says about the machine it is compiled for.
//!
//! `@target(os = "linux")` on an item compiles it only where the condition
//! holds. The item is dropped here, before anything is declared, so a name
//! another target's item takes is free, and nothing that is not compiled
//! is checked — as with Rust's `cfg`, an item for another target is not
//! checked unless that target is checked for.
//!
//! A program is built for the machine the compiler runs on, and may be
//! checked for any target the standard library is written for: `wip check
//! --target linux` reads the items and files Linux takes.

use wip_syntax::ast::{Annotation, AnnotationValue, Ast, Item};
use wip_syntax::codes;
use wip_syntax::{Diagnostic, Interner, Span, Symbol};

/// The machine being compiled for: what `@target`'s keys are answered
/// with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub os: &'static str,
    pub arch: &'static str,
    /// `apple` for macOS and its siblings, and nothing elsewhere: what
    /// `@framework` and Objective-C belong to.
    pub vendor: &'static str,
}

impl Target {
    /// The machine this compiler runs on.
    pub fn host() -> Target {
        Target {
            os: match std::env::consts::OS {
                "macos" | "ios" => "macos",
                "windows" => "windows",
                _ => "linux",
            },
            arch: match std::env::consts::ARCH {
                "aarch64" => "arm64",
                arch => arch,
            },
            vendor: match std::env::consts::OS {
                "macos" | "ios" => "apple",
                _ => "",
            },
        }
    }

    /// The targets the standard library is written for: what `--target
    /// all` checks.
    pub fn all() -> [Target; 4] {
        let target = |os, arch| Target {
            os,
            arch,
            vendor: if os == "macos" { "apple" } else { "" },
        };
        [
            target("macos", "arm64"),
            target("macos", "x86_64"),
            target("linux", "arm64"),
            target("linux", "x86_64"),
        ]
    }

    /// What `--target` names: one of [`Target::all`] as `os-arch`, a
    /// system alone on this machine's processor, or `all`.
    pub fn parse(name: &str) -> Result<Vec<Target>, String> {
        if name == "all" {
            return Ok(Target::all().to_vec());
        }
        if name == "windows" || name.starts_with("windows-") {
            return Err(format!(
                "`{name}` is not a target to check: the standard library is not written for Windows"
            ));
        }
        let (os, arch) = match name.split_once('-') {
            Some((os, arch)) => (os, arch),
            None => (name, Target::host().arch),
        };
        Target::all()
            .into_iter()
            .find(|target| target.os == os && target.arch == arch)
            .map(|target| vec![target])
            .ok_or_else(|| {
                let names: Vec<String> =
                    Target::all().iter().map(|target| format!("`{}`", target.name())).collect();
                format!(
                    "`{name}` is not a target: there are {}, `macos` and `linux` for this machine's processor, and `all`",
                    names.join(", ")
                )
            })
    }

    /// Its name, as `--target` takes it: `linux-x86_64`.
    pub fn name(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }

    fn answer(&self, key: &str) -> &str {
        match key {
            "os" => self.os,
            "arch" => self.arch,
            _ => self.vendor,
        }
    }
}

/// The keys `@target` takes, and what each one may be. The lists are
/// closed: a value the compiler does not know is a mistake rather than a
/// condition that is quietly false.
const KEYS: [(&str, &[&str]); 3] = [
    ("os", &["macos", "linux", "windows"]),
    ("arch", &["arm64", "x86_64"]),
    ("vendor", &["apple"]),
];

/// Drops the items `target` does not take, and reports every `@target`
/// that cannot be read — including on the items that are dropped, so that
/// a condition is checked wherever it is compiled.
pub fn keep(ast: &mut Ast, interner: &Interner, target: Target) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut kept = Vec::with_capacity(ast.items.len());
    for mut item in std::mem::take(&mut ast.items) {
        if !wanted(annotations_of(&item), interner, target, &mut diagnostics) {
            continue;
        }
        // An `extern` block's own declarations may each say what they are
        // for: a library spells a function one way here and another there.
        if let Item::Extern(block) = &mut item {
            block
                .fns
                .retain(|f| wanted(&f.annotations, interner, target, &mut diagnostics));
            block
                .globals
                .retain(|g| wanted(&g.annotations, interner, target, &mut diagnostics));
            block
                .types
                .retain(|t| wanted(&t.annotations, interner, target, &mut diagnostics));
        }
        kept.push(item);
    }
    ast.items = kept;
    diagnostics
}

/// The annotations written before an item. An import and a type alias
/// take none, and the parser refuses one written before either.
fn annotations_of(item: &Item) -> &[Annotation] {
    match item {
        Item::Import(_) | Item::Type(_) => &[],
        Item::Val(v) => &v.annotations,
        Item::Assert(a) => &a.annotations,
        Item::Struct(s) => &s.annotations,
        Item::Enum(e) => &e.annotations,
        Item::Fn(f) => &f.annotations,
        Item::Extern(e) => &e.annotations,
        Item::Extend(e) => &e.annotations,
        Item::Interface(i) => &i.annotations,
    }
}

/// Whether a declaration carrying `annotations` is compiled for `target`:
/// one that says nothing is for every target, one that says something is
/// for the targets any of its `@target`s names.
fn wanted(
    annotations: &[Annotation],
    interner: &Interner,
    target: Target,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let mut said = false;
    let mut matched = false;
    for annotation in annotations {
        if interner.resolve(annotation.name.sym) != "target" {
            continue;
        }
        said = true;
        matched |= condition(annotation, interner, target, diagnostics);
    }
    !said || matched
}

/// Whether one `@target(…)` holds: every key it names must answer.
fn condition(
    annotation: &Annotation,
    interner: &Interner,
    target: Target,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    if annotation.args.is_empty() {
        diagnostics.push(
            Diagnostic::error(
                codes::ANNOTATION,
                "`@target` takes what it is for",
                annotation.span,
                "nothing is named",
            )
            .with_help(format!("the keys are {}", key_names()))
            .with_note("`@target(os = \"linux\")` compiles an item for one target"),
        );
        return false;
    }
    let mut holds = true;
    for arg in &annotation.args {
        let Some(name) = arg.name else {
            diagnostics.push(
                Diagnostic::error(
                    codes::ANNOTATION,
                    "`@target` takes `key = \"value\"`",
                    arg.span,
                    "no key",
                )
                .with_help(format!("the keys are {}", key_names())),
            );
            holds = false;
            continue;
        };
        let key = interner.resolve(name.sym).to_string();
        let Some((_, values)) = KEYS.iter().find(|(known, _)| *known == key) else {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                format!("`@target` knows no key `{key}`"),
                name.span,
                "not a key",
            )
            .with_help(format!("the keys are {}", key_names()));
            diagnostics.push(diagnostic);
            holds = false;
            continue;
        };
        let AnnotationValue::Str(value) = arg.value else {
            diagnostics.push(
                Diagnostic::error(
                    codes::ANNOTATION,
                    format!("`{key}` is a name in quotes"),
                    arg.span,
                    "not a string",
                )
                .with_help(format!("`{key}` is one of {}", quoted(values))),
            );
            holds = false;
            continue;
        };
        holds &= value_holds(
            key.as_str(),
            value,
            values,
            arg.span,
            interner,
            target,
            diagnostics,
        );
    }
    holds
}

/// Whether one `key = "value"` holds, where the value is one the compiler
/// knows.
fn value_holds(
    key: &str,
    value: Symbol,
    values: &[&str],
    span: Span,
    interner: &Interner,
    target: Target,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let text = interner.resolve(value);
    if !values.contains(&text) {
        diagnostics.push(
            Diagnostic::error(
                codes::ANNOTATION,
                format!("the compiler knows no `{key}` called `{text}`"),
                span,
                "not a target",
            )
            .with_help(format!("`{key}` is one of {}", quoted(values))),
        );
        return false;
    }
    target.answer(key) == text
}

fn key_names() -> String {
    quoted(&KEYS.iter().map(|(name, _)| *name).collect::<Vec<_>>())
}

fn quoted(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether a file belongs to `target`, by its name: `window.linux.wip` is
/// Linux's and `rglfw.macos.c` is a Mac's, as `.test.wip` is a test's.
/// A name whose last piece is not a target the compiler knows says nothing, and
/// belongs to every target.
pub fn takes_file(name: &str, target: Target) -> bool {
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) => stem,
        None => name,
    };
    // `board.linux.test.wip` is a test for Linux: the test suffix is read
    // first, and what is left says the target.
    let stem = stem.strip_suffix(".test").unwrap_or(stem);
    let Some((_, last)) = stem.rsplit_once('.') else {
        return true;
    };
    for (key, values) in KEYS {
        if values.contains(&last) {
            return target.answer(key) == last;
        }
    }
    true
}

/// The prelude file that says what the target is. It is
/// written here because the compiler is the only one that knows, and it
/// is the prelude so that every file can ask without an import.
pub fn prelude(target: Target) -> String {
    format!(
        "// Written by the compiler: the machine this program is compiled\n\
         // for. It is here, in the prelude, for the places\n\
         // where splitting an item with `@target` is more than the\n\
         // difference is worth; the branch that cannot be taken costs\n\
         // nothing, since the comparison is of constants.\n\
         \n\
         pub val TARGET_OS: str = \"{}\"\n\
         pub val TARGET_ARCH: str = \"{}\"\n\
         /// `apple` on a Mac, and empty elsewhere.\n\
         pub val TARGET_VENDOR: str = \"{}\"\n",
        target.os, target.arch, target.vendor
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: Target = Target {
        os: "macos",
        arch: "arm64",
        vendor: "apple",
    };

    #[test]
    fn a_target_is_named() {
        let linux = Target::parse("linux-x86_64").unwrap();
        assert_eq!(linux.len(), 1);
        assert_eq!(
            (linux[0].os, linux[0].arch, linux[0].vendor),
            ("linux", "x86_64", "")
        );
        let mac = Target::parse("macos-arm64").unwrap();
        assert_eq!(
            (mac[0].os, mac[0].arch, mac[0].vendor),
            ("macos", "arm64", "apple")
        );
        // A system alone is on this machine's processor.
        assert_eq!(Target::parse("linux").unwrap()[0].arch, Target::host().arch);
        assert_eq!(Target::parse("all").unwrap().len(), 4);
        assert_eq!(
            Target::parse("linux-arm64").unwrap()[0].name(),
            "linux-arm64"
        );
        assert!(
            Target::parse("windows")
                .unwrap_err()
                .contains("not written for Windows")
        );
        assert!(
            Target::parse("linux-riscv64")
                .unwrap_err()
                .contains("`linux-riscv64` is not a target")
        );
        assert!(Target::parse("freebsd").is_err());
    }

    #[test]
    fn a_file_says_what_it_is_for() {
        assert!(takes_file("window.wip", MAC));
        assert!(takes_file("window.macos.wip", MAC));
        assert!(!takes_file("window.linux.wip", MAC));
        assert!(takes_file("window.apple.wip", MAC));
        assert!(takes_file("window.arm64.wip", MAC));
        assert!(!takes_file("window.x86_64.wip", MAC));
        // A test for one target, and a name that merely holds a dot.
        assert!(takes_file("board.macos.test.wip", MAC));
        assert!(!takes_file("board.linux.test.wip", MAC));
        assert!(takes_file("v1.2.wip", MAC));
        // C is read the same way.
        assert!(takes_file("rglfw.macos.c", MAC));
        assert!(!takes_file("rglfw.linux.c", MAC));
    }
}
