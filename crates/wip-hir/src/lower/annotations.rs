//! Annotations: `@name` and `@name(arguments)` before a declaration.
//!
//! The set is closed and written here. An annotation the compiler does not
//! know is an error, and so is one on a declaration that does not take it,
//! one given the wrong arguments, and one written twice: an annotation that
//! quietly does nothing is worse than none, because a reader believes it.

use super::*;

/// What a declaration is, for the table below.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Fn,
    Struct,
    Enum,
    Interface,
    Extend,
    Extern,
    /// A declaration inside an extern block: a function, a variable or a
    /// type C owns.
    ExternFn,
    ExternVar,
    ExternType,
    Val,
    Assert,
}

impl Target {
    fn text(self) -> &'static str {
        match self {
            Target::Fn => "a function",
            Target::Struct => "a struct",
            Target::Enum => "an enum",
            Target::Interface => "an interface",
            Target::Extend => "an `extend` block",
            Target::Extern => "an `extern` block",
            Target::ExternFn => "a C function's declaration",
            Target::ExternVar => "a C variable's declaration",
            Target::ExternType => "a C type's declaration",
            Target::Val => "a constant",
            Target::Assert => "a top-level `assert`",
        }
    }
}

/// An interface the compiler can write from a type's fields, which is what
/// `@derive` takes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Derived {
    /// `equals`, field by field, written as code.
    Eq,
    /// `compare`, field by field, written as code.
    Ord,
    /// `hashInto`, part by part.
    Hash,
    /// `appendTo`, as the literal that would make the value.
    Text,
    /// `clone`, field by field, written as code.
    Clone,
}

impl Derived {
    pub(super) const ALL: [(Derived, &'static str); 5] = [
        (Derived::Eq, "Eq"),
        (Derived::Ord, "Ord"),
        (Derived::Hash, "Hash"),
        (Derived::Text, "Text"),
        (Derived::Clone, "Clone"),
    ];

    pub(super) fn name(self) -> &'static str {
        Derived::ALL
            .iter()
            .find(|&&(d, _)| d == self)
            .map(|&(_, n)| n)
            .expect("every one is listed")
    }
}

/// What each annotation that wrote a body was called before `@derive`:
/// reported with a fix to the form that replaced it.
const DERIVED_BEFORE: [(&str, &str); 4] = [
    ("eq", "Eq"),
    ("ord", "Ord"),
    ("hash", "Hash"),
    ("text", "Text"),
];

/// One annotation the compiler knows: its name, where it may be written, and
/// whether it takes arguments.
struct Known {
    name: &'static str,
    targets: &'static [Target],
    /// What it takes, for the message when it is given something else. No
    /// annotation takes arguments yet.
    arguments: Option<&'static str>,
}

const KNOWN: &[Known] = &[
    Known {
        name: "tailrec",
        targets: &[Target::Fn],
        arguments: None,
    },
    Known {
        name: "inline",
        targets: &[Target::Fn],
        arguments: None,
    },
    Known {
        name: "intrinsic",
        targets: &[Target::Fn, Target::Struct],
        arguments: None,
    },
    Known {
        name: "target",
        // Every kind of declaration that carries annotations: what a
        // target has and has not is not one kind of thing.
        targets: &[
            Target::Fn,
            Target::Struct,
            Target::Enum,
            Target::Interface,
            Target::Extern,
            Target::ExternFn,
            Target::ExternVar,
            Target::ExternType,
            Target::Extend,
            Target::Val,
            Target::Assert,
        ],
        arguments: Some("what it is for, as in `@target(os = \"linux\")`"),
    },
    Known {
        name: "test",
        targets: &[Target::Fn],
        arguments: None,
    },
    Known {
        name: "export",
        targets: &[Target::Fn],
        arguments: Some("one ABI name, as in `@export(\"C\")`"),
    },
    Known {
        name: "link",
        targets: &[Target::Extern],
        arguments: Some("library names, as in `@link(\"sqlite3\")`"),
    },
    Known {
        name: "include",
        targets: &[Target::Extern],
        arguments: Some("directories in the module, as in `@include(\"src\")`"),
    },
    Known {
        name: "source",
        targets: &[Target::Extern],
        arguments: Some(
            "C files in the module, as in `@source(\"src/rcore.c\")`, and what they are written in, `language = \"objective-c\"`",
        ),
    },
    Known {
        name: "define",
        targets: &[Target::Extern],
        arguments: Some(
            "settings for the module's C, as in `@define(\"PLATFORM_DESKTOP\", \"LEVEL=2\")`",
        ),
    },
    Known {
        name: "prefix",
        targets: &[Target::Extern],
        arguments: Some("one header, as in `@prefix(\"build.h\")`"),
    },
    Known {
        name: "framework",
        targets: &[Target::Extern],
        arguments: Some("framework names, as in `@framework(\"Cocoa\")`"),
    },
    Known {
        name: "symbol",
        targets: &[Target::ExternFn, Target::ExternVar],
        arguments: Some("one C name, as in `@symbol(\"sqlite3_open\")`"),
    },
    Known {
        name: "derive",
        targets: &[Target::Struct, Target::Enum],
        arguments: Some("the interfaces it writes, as in `@derive(Eq, Hash)`"),
    },
    Known {
        name: "oneOf",
        targets: &[Target::Interface],
        arguments: Some(
            "the methods an implementation writes one of, as in `@oneOf(toString, appendTo)`",
        ),
    },
    Known {
        name: "opaque",
        targets: &[Target::Struct],
        arguments: None,
    },
    Known {
        name: "comptime",
        targets: &[Target::Val],
        arguments: None,
    },
    Known {
        name: "header",
        targets: &[
            Target::Extern,
            Target::Struct,
            Target::ExternFn,
            Target::ExternVar,
        ],
        arguments: Some("one header to include, as in `@header(\"<math.h>\")`"),
    },
];

/// What the annotations of a declaration say about it.
#[derive(Default, Clone)]
pub(super) struct Annotations {
    /// `@tailrec`.
    pub tailrec: bool,
    /// `@intrinsic`: the compiler writes this body.
    pub intrinsic: bool,
    /// `@inline`: every call to it is spliced, or the program does not
    /// compile.
    pub inline: Option<Span>,
    /// `@test`: `wip test` runs it.
    pub test: Option<Span>,
    /// `@link("name", …)`: the libraries the declarations, or the C the
    /// module builds, come from.
    pub link: Vec<(Symbol, Span)>,
    /// `@export("C")`: the function gets a plain C symbol, so that C can
    /// call it by name.
    pub export: Option<(Symbol, Span)>,
    /// `@symbol("sqlite3_open")`: what C calls it, when Wip calls it
    /// something else.
    pub symbol: Option<(Symbol, Span)>,
    /// `@derive(Eq, Ord, Hash, Text)`: the interfaces the compiler writes
    /// from the fields, and where each was named.
    pub derive: Vec<(Derived, Span)>,
    /// `@opaque`: C knows this struct's layout and Wip does not, so its
    /// fields are reached through C.
    pub opaque: Option<Span>,
    /// `@comptime`: the constant is worked out by running code while the
    /// program is compiled.
    pub comptime: Option<Span>,
    /// `@oneOf(a, b)`, as many as are written: methods of the interface of
    /// which an implementation writes one at least.
    pub one_of: Vec<OneOf>,
    /// `@header("<math.h>")`: the declarations are what this header says
    /// they are, so each call goes through C that includes it.
    pub header: Option<(Symbol, Span)>,
    /// `@include("src", …)`: directories in the module where its C looks
    /// for headers.
    pub include: Vec<(Symbol, Span)>,
    /// `@framework("Cocoa", …)`: what an Apple target links besides the
    /// libraries.
    pub framework: Vec<(Symbol, Span)>,
    /// `@source("src/rcore.c", …)`: C files of the module, below its own
    /// directory, each compiled with the program, and
    /// `language = "…"`, what they are written in where their extension
    /// does not say.
    pub source: Vec<(Symbol, Span)>,
    pub source_language: Option<CLanguage>,
    /// `@prefix("build.h")`: a header put before every C file of the
    /// module, which is how a vendored library is configured.
    pub prefix: Option<(Symbol, Span)>,
    /// `@define("PLATFORM_DESKTOP", "LEVEL=2")`: what every C file of the
    /// module is compiled with defined, as `-D` defines it.
    pub define: Vec<(Symbol, Span)>,
}

impl Lowerer<'_> {
    /// Checks the annotations written on a declaration, and returns what they
    /// say.
    pub(super) fn annotations(
        &mut self,
        written: &[ast::Annotation],
        target: Target,
    ) -> Annotations {
        let mut annotations = Annotations::default();
        let mut seen: Vec<(Symbol, Span)> = Vec::new();
        for annotation in written {
            let name = self.text(annotation.name.sym).to_string();
            // `@eq` and its three siblings are written `@derive(Eq)` now.
            if let Some(&(_, interface)) = DERIVED_BEFORE.iter().find(|&&(old, _)| old == name) {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    format!("`@{name}` is written `@derive({interface})`"),
                    annotation.span,
                    "the old spelling",
                )
                .with_fix(
                    format!("write `@derive({interface})`"),
                    [Edit::replace(annotation.span, format!("@derive({interface})"))],
                )
                .with_note(
                    "the annotations that write an interface's body are one, and it names the interface: `@derive(Eq, Ord, Hash, Text)`",
                );
                self.report(diagnostic);
                continue;
            }
            let Some(known) = KNOWN.iter().find(|known| known.name == name) else {
                let mut diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    format!("there is no annotation `@{name}`"),
                    annotation.name.span,
                    "not an annotation",
                )
                .with_note(
                    "the compiler knows every annotation there is; a name it does not know is a mistake, not a hint",
                );
                if let Some(similar) = wording::suggest(&name, KNOWN.iter().map(|k| k.name)) {
                    diagnostic = diagnostic.with_fix(
                        format!("did you mean `@{similar}`?"),
                        [Edit::replace(annotation.name.span, similar)],
                    );
                }
                self.report(diagnostic);
                continue;
            };
            // `@derive` may be written more than once, and what the lines
            // name is one list: it is what fixing `@eq @hash` one at a
            // time leaves. `@target` may too, and the
            // lines are the targets it is for, any of them;
            // and `@oneOf`, each line a group of its own.
            if !matches!(known.name, "derive" | "target" | "oneOf")
                && let Some(&(_, first)) = seen.iter().find(|&&(sym, _)| sym == annotation.name.sym)
            {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    format!("`@{name}` is written twice"),
                    annotation.span,
                    "written again",
                )
                .with_secondary(first, "written here")
                .with_help("remove one of them");
                self.report(diagnostic);
                continue;
            }
            seen.push((annotation.name.sym, annotation.span));
            if !known.targets.contains(&target) {
                let places: Vec<&str> = known.targets.iter().map(|t| t.text()).collect();
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    format!("`@{name}` cannot be written on {}", target.text()),
                    annotation.span,
                    format!("on {}", target.text()),
                )
                .with_help(format!("`@{name}` belongs on {}", one_of(&places)));
                self.report(diagnostic);
                continue;
            }
            let arguments_fit = match (known.arguments, annotation.parens) {
                (None, Some(parens)) => {
                    let diagnostic = Diagnostic::error(
                        codes::ANNOTATION,
                        format!("`@{name}` takes no arguments"),
                        parens,
                        "arguments",
                    )
                    .with_fix("remove them", [Edit::replace(parens, "")]);
                    self.report(diagnostic);
                    false
                }
                (Some(takes), None) => {
                    let diagnostic = Diagnostic::error(
                        codes::ANNOTATION,
                        format!("`@{name}` takes {takes}"),
                        annotation.span,
                        "no arguments",
                    );
                    self.report(diagnostic);
                    false
                }
                _ => true,
            };
            if !arguments_fit {
                continue;
            }
            match known.name {
                "tailrec" => annotations.tailrec = true,
                "intrinsic" => annotations.intrinsic = true,
                "inline" => annotations.inline = Some(annotation.span),
                "test" => annotations.test = Some(annotation.span),
                // The driver has already read it and dropped what this
                // target does not take; here it is known
                // so that it is not reported as an annotation that is
                // not.
                "target" => {}
                "link" => annotations.link = self.strings(annotation),
                "header" => annotations.header = self.one_string(annotation),
                "include" => annotations.include = self.strings(annotation),
                "framework" => annotations.framework = self.strings(annotation),
                "source" => {
                    let (files, language) = self.sources(annotation);
                    annotations.source = files;
                    annotations.source_language = language;
                }
                "prefix" => annotations.prefix = self.one_string(annotation),
                "define" => annotations.define = self.strings(annotation),
                "symbol" => annotations.symbol = self.one_string(annotation),
                "opaque" => annotations.opaque = Some(annotation.span),
                "comptime" => annotations.comptime = Some(annotation.span),
                "derive" => {
                    let more = self.derived(annotation, &annotations.derive);
                    annotations.derive.extend(more);
                }
                "export" => annotations.export = self.one_string(annotation),
                "oneOf" => annotations.one_of.extend(self.one_of(annotation)),
                name => unreachable!("`@{name}` is known but does nothing"),
            }
        }
        annotations
    }

    /// The interfaces `@derive` names, each one the compiler can write,
    /// and each once.
    pub(super) fn derived(
        &mut self,
        annotation: &ast::Annotation,
        earlier: &[(Derived, Span)],
    ) -> Vec<(Derived, Span)> {
        let mut derived: Vec<(Derived, Span)> = Vec::new();
        if annotation.args.is_empty() {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                "`@derive` names what it writes",
                annotation.span,
                "nothing named",
            )
            .with_help("write the interfaces, as in `@derive(Eq, Hash)`");
            self.report(diagnostic);
        }
        for arg in &annotation.args {
            let ast::AnnotationValue::Name(sym) = arg.value else {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    "`@derive` takes the names of interfaces",
                    arg.span,
                    "not a name",
                );
                self.report(diagnostic);
                continue;
            };
            if let Some(written) = arg.name {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    "`@derive`'s arguments have no names",
                    written.span,
                    "a named argument",
                );
                self.report(diagnostic);
                continue;
            }
            let text = self.text(sym).to_string();
            let Some(&(which, _)) = Derived::ALL.iter().find(|&&(_, n)| n == text) else {
                let names: Vec<String> = Derived::ALL
                    .iter()
                    .map(|&(_, n)| format!("`{n}`"))
                    .collect();
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    format!("the compiler cannot write `{text}`"),
                    arg.span,
                    "not an interface it writes",
                )
                .with_note(format!(
                    "`@derive` writes {} from the fields; any other interface is written by hand in an `extend` block",
                    names.join(", ")
                ));
                self.report(diagnostic);
                continue;
            };
            if let Some(&(_, first)) = earlier.iter().chain(&derived).find(|&&(d, _)| d == which) {
                let diagnostic = Diagnostic::error(
                    codes::ANNOTATION,
                    format!("`{text}` is named twice"),
                    arg.span,
                    "named again",
                )
                .with_secondary(first, "named here");
                self.report(diagnostic);
                continue;
            }
            derived.push((which, arg.span));
        }
        derived
    }

    /// The methods `@oneOf` names, as names; whether the interface has
    /// them is asked once its methods are known.
    fn one_of(&mut self, annotation: &ast::Annotation) -> Option<OneOf> {
        let mut names = Vec::new();
        for arg in &annotation.args {
            match (arg.name, &arg.value) {
                (None, &ast::AnnotationValue::Name(sym)) => names.push((sym, arg.span)),
                _ => {
                    let diagnostic = Diagnostic::error(
                        codes::ANNOTATION,
                        "`@oneOf` takes the names of the interface's methods",
                        arg.span,
                        "not a method's name",
                    )
                    .with_help(
                        "write them as they are declared, as in `@oneOf(toString, appendTo)`",
                    );
                    self.report(diagnostic);
                    return None;
                }
            }
        }
        Some(OneOf {
            names,
            span: annotation.span,
        })
    }

    /// The one string an annotation was given, where it takes one: reports
    /// anything else, since an annotation that is read wrongly is worse than
    /// one that is not read at all.
    fn one_string(&mut self, annotation: &ast::Annotation) -> Option<(Symbol, Span)> {
        let name = self.text(annotation.name.sym).to_string();
        let [arg] = &annotation.args[..] else {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                format!("`@{name}` takes one argument"),
                annotation.span,
                format!(
                    "{} given",
                    plural(annotation.args.len(), "argument", "arguments")
                ),
            );
            self.report(diagnostic);
            return None;
        };
        self.annotation_string(&name, arg)
    }

    /// `@source("sokol.h", language = "objective-c")`: the files, each
    /// checked as `strings` checks its strings, and the language they are
    /// written in, if it is said.
    fn sources(
        &mut self,
        annotation: &ast::Annotation,
    ) -> (Vec<(Symbol, Span)>, Option<CLanguage>) {
        let mut language = None;
        let mut files = Vec::new();
        let mut wrong = false;
        for arg in &annotation.args {
            let Some(name) = arg.name else {
                files.extend(self.annotation_string("source", arg));
                continue;
            };
            let said = match arg.value {
                ast::AnnotationValue::Str(text) if self.text(name.sym) == "language" => {
                    match self.text(text) {
                        "c" => Some(CLanguage::C),
                        "objective-c" => Some(CLanguage::ObjectiveC),
                        _ => None,
                    }
                }
                _ => None,
            };
            match said {
                Some(said) if language.is_none() => language = Some(said),
                _ => {
                    let diagnostic = Diagnostic::error(
                        codes::ANNOTATION,
                        "`@source` takes files, and `language = \"c\"` or `language = \"objective-c\"` once",
                        arg.span,
                        "not a file, nor a language it knows",
                    )
                    .with_note("a file is compiled as its extension says — `.c` as C, `.m` as Objective-C — unless `language` says otherwise, as a header holding a library's implementation needs");
                    self.report(diagnostic);
                    wrong = true;
                }
            }
        }
        if files.is_empty() && !wrong {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                "`@source` takes one file or more",
                annotation.span,
                "no files given",
            );
            self.report(diagnostic);
        }
        (files, language)
    }

    /// `@include("src", "include")`: one string or more, each checked as
    /// `one_string` checks its one.
    fn strings(&mut self, annotation: &ast::Annotation) -> Vec<(Symbol, Span)> {
        let name = self.text(annotation.name.sym).to_string();
        if annotation.args.is_empty() {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                format!("`@{name}` takes one string or more"),
                annotation.span,
                "no arguments given",
            );
            self.report(diagnostic);
            return Vec::new();
        }
        annotation
            .args
            .iter()
            .filter_map(|arg| self.annotation_string(&name, arg))
            .collect()
    }

    /// One argument of `@name(…)` that must be a string, not named and not
    /// empty.
    fn annotation_string(
        &mut self,
        name: &str,
        arg: &ast::AnnotationArg,
    ) -> Option<(Symbol, Span)> {
        if let Some(written) = arg.name {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                format!("`@{name}`'s argument has no name"),
                written.span,
                "a named argument",
            );
            self.report(diagnostic);
            return None;
        }
        let ast::AnnotationValue::Str(text) = arg.value else {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                format!("`@{name}` takes a string"),
                arg.span,
                "not a string",
            );
            self.report(diagnostic);
            return None;
        };
        if self.text(text).is_empty() {
            let diagnostic = Diagnostic::error(
                codes::ANNOTATION,
                format!("`@{name}` needs a name"),
                arg.span,
                "an empty string",
            );
            self.report(diagnostic);
            return None;
        }
        Some((text, arg.span))
    }
}

/// `a`, `a or b`, `a, b or c`: the places an annotation may be written.
fn one_of(places: &[&str]) -> String {
    match places {
        [] => String::new(),
        [only] => only.to_string(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}
