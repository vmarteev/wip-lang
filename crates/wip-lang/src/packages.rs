//! Packages: what a `package.wip` says, where a dependency is, and what a
//! module of one may import from another.

use super::*;

/// A package of the program: where its modules are, and what its
/// `package.wip` says. The program's own is first.
#[derive(Debug, Clone)]
pub struct Package {
    pub name: Option<String>,
    pub version: Option<String>,
    /// Its root, as the program was told where it is.
    pub dir: PathBuf,
    /// The same, resolved: two paths to one directory are one package.
    identity: PathBuf,
    /// Empty for the program's own package, and its name for a
    /// dependency: what begins its modules' paths.
    pub prefix: String,
    /// The packages it depends on: the name, which package, and where the
    /// `@depends` was written.
    pub depends: Vec<(String, usize, Span)>,
}

/// What a `package.wip` says.
struct Manifest {
    name: Option<(String, Span)>,
    version: Option<String>,
    /// The name each dependency is imported by, where its root is relative
    /// to this package's, and where the `@depends` was written.
    depends: Vec<(String, String, Span)>,
}

/// Reads the `package.wip` at `dir`, if there is one, reporting what is
/// wrong with it into `loaded`. Its text joins the sources, so that what is
/// reported can be shown.
fn manifest(dir: &Path, loaded: &mut Loaded, overlay: &Overlay) -> Option<Manifest> {
    let path = dir.join("package.wip");
    let text = overlay.read(&path).ok()?;
    let base = loaded.sources.add(path.display().to_string(), text.clone());
    let lexed = lex_at(&text, base, &mut loaded.interner);
    let parsed = wip_syntax::parse_package_at(&text, base, &lexed);
    loaded.diagnostics.extend(lexed.diagnostics.iter().cloned());
    loaded.diagnostics.extend(parsed.diagnostics);
    let text_of = |sym: Symbol, interner: &Interner| interner.resolve(sym).to_string();
    let mut manifest = Manifest {
        name: parsed
            .name
            .map(|name| (text_of(name.sym, &loaded.interner), name.span)),
        version: None,
        depends: Vec::new(),
    };
    let wrong = |message: String, span: Span, label: &str| {
        Diagnostic::error(wip_syntax::codes::PACKAGE, message, span, label.to_string())
    };
    for annotation in &parsed.annotations {
        let what = text_of(annotation.name.sym, &loaded.interner);
        let strings: Vec<(Option<String>, Option<String>, Span)> = annotation
            .args
            .iter()
            .map(|arg| {
                let name = arg.name.map(|n| text_of(n.sym, &loaded.interner));
                let value = match arg.value {
                    wip_syntax::ast::AnnotationValue::Str(sym) => {
                        Some(text_of(sym, &loaded.interner))
                    }
                    _ => None,
                };
                (name, value, arg.span)
            })
            .collect();
        match what.as_str() {
            "version" => {
                let version = match strings.as_slice() {
                    [(None, Some(version), _)] => version.clone(),
                    _ => {
                        loaded.diagnostics.push(
                            wrong(
                                "`@version` takes one version, as in `@version(\"1.4.0\")`".into(),
                                annotation.span,
                                "not a version",
                            )
                            .with_note("a package's version is a semantic version: `MAJOR.MINOR.PATCH`, with a `-pre` and a `+build` if written"),
                        );
                        continue;
                    }
                };
                if !is_semver(&version) {
                    loaded.diagnostics.push(
                        wrong(
                            format!("`{version}` is not a semantic version"),
                            annotation.span,
                            "not `MAJOR.MINOR.PATCH`",
                        )
                        .with_note("versions must compare once a dependency can name one, so a package's is `MAJOR.MINOR.PATCH`, with a `-pre` and a `+build` if written, as in `1.4.0` or `2.0.0-rc.1`"),
                    );
                } else if manifest.version.is_some() {
                    loaded.diagnostics.push(wrong(
                        "a package has one version".into(),
                        annotation.span,
                        "a second `@version`",
                    ));
                } else {
                    manifest.version = Some(version);
                }
            }
            "depends" => {
                let named = |key: &str| {
                    strings
                        .iter()
                        .find(|(name, ..)| name.as_deref() == Some(key))
                        .and_then(|(_, value, _)| value.clone())
                };
                let name = match strings.first() {
                    Some((None, Some(name), _)) => name.clone(),
                    _ => {
                        loaded.diagnostics.push(
                            wrong(
                                "`@depends` begins with the name the dependency is imported by".into(),
                                annotation.span,
                                "no name",
                            )
                            .with_help("write `@depends(\"engine\", path = \"../engine\")`"),
                        );
                        continue;
                    }
                };
                if strings.iter().any(|(key, ..)| matches!(key.as_deref(), Some("git" | "rev"))) {
                    loaded.diagnostics.push(
                        wrong(
                            format!("`{name}` is named by where it is fetched from, which is not built yet"),
                            annotation.span,
                            "a remote package",
                        )
                        .with_note("a remote source is reserved for when a program needs one, with a lock file; today a dependency is a directory, `path = \"…\"`"),
                    );
                    continue;
                }
                let unknown = strings.iter().skip(1).find(|(key, ..)| key.as_deref() != Some("path"));
                let Some(path) = named("path").filter(|_| unknown.is_none()) else {
                    loaded.diagnostics.push(
                        wrong(
                            format!("`@depends` says where `{name}` is with `path = \"…\"`, and nothing else"),
                            unknown.map_or(annotation.span, |(.., span)| *span),
                            "not what a dependency takes",
                        )
                        .with_help(format!("write `@depends(\"{name}\", path = \"../{name}\")`")),
                    );
                    continue;
                };
                manifest.depends.push((name, path, annotation.span));
            }
            // How `wip fmt` lays the package out, which
            // the formatter reads for itself.
            "format" => {
                if let Err(message) = format_settings(annotation, &loaded.interner) {
                    loaded.diagnostics.push(wrong(message, annotation.span, "not a layout `wip fmt` knows"));
                }
            }
            _ => loaded.diagnostics.push(
                wrong(
                    format!("`@{what}` is not something a package says"),
                    annotation.span,
                    "not a package's annotation",
                )
                .with_help("a `package.wip` takes `@version(\"1.4.0\")`, `@depends(\"engine\", path = \"../engine\")` and `@format(width = 120)`"),
            ),
        }
    }
    Some(manifest)
}

/// What a `@format(…)` says, over `style`: `width`,
/// `indent = "tabs"` or `"spaces"`, and `size`.
fn format_settings(
    annotation: &wip_syntax::ast::Annotation,
    interner: &Interner,
) -> Result<wip_fmt::Style, String> {
    let mut style = wip_fmt::Style::default();
    for arg in &annotation.args {
        let key = arg.name.map(|n| interner.resolve(n.sym));
        match (key, &arg.value) {
            (Some("width"), wip_syntax::ast::AnnotationValue::Int(n)) if (40..=400).contains(n) => {
                style.width = *n as usize;
            }
            (Some("size"), wip_syntax::ast::AnnotationValue::Int(n)) if (1..=16).contains(n) => {
                style.size = *n as usize;
            }
            (Some("indent"), wip_syntax::ast::AnnotationValue::Str(s)) => {
                match interner.resolve(*s) {
                    "tabs" => style.tabs = true,
                    "spaces" => style.tabs = false,
                    other => {
                        return Err(format!(
                            "`indent = \"{other}\"` is neither `\"tabs\"` nor `\"spaces\"`"
                        ));
                    }
                }
            }
            _ => {
                return Err(
                    "`@format` takes `width` (40 to 400), `indent = \"tabs\"` or `\"spaces\"`, and `size` (1 to 16)"
                        .to_string(),
                );
            }
        }
    }
    Ok(style)
}

/// How `wip fmt` lays out `file`: what the `@format` of the nearest
/// `package.wip` above it says, or the defaults.
pub fn format_style(file: &Path) -> Result<wip_fmt::Style, String> {
    let start = if file.is_dir() {
        file.to_path_buf()
    } else {
        file.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let start = identity(&start);
    for dir in start.ancestors() {
        let manifest = dir.join("package.wip");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let mut interner = Interner::new();
        let lexed = lex_at(&text, 0, &mut interner);
        let parsed = wip_syntax::parse_package_at(&text, 0, &lexed);
        let Some(annotation) = parsed
            .annotations
            .iter()
            .find(|a| interner.resolve(a.name.sym) == "format")
        else {
            return Ok(wip_fmt::Style::default());
        };
        return format_settings(annotation, &interner)
            .map_err(|message| format!("{}: {message}", manifest.display()));
    }
    Ok(wip_fmt::Style::default())
}

/// `MAJOR.MINOR.PATCH`, with a `-pre` and a `+build` if written: what
/// Semantic Versioning 2.0.0 says, which is what makes two versions
/// comparable.
fn is_semver(text: &str) -> bool {
    let (rest, build) = match text.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (text, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    let number = |part: &str| {
        !part.is_empty()
            && part.chars().all(|c| c.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let identifiers = |text: &str| {
        text.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| number(part))
        && pre.is_none_or(identifiers)
        && build.is_none_or(identifiers)
}

/// A path with `a/..` taken out, so that a dependency's files are named
/// `engine/render.wip` rather than `game/../engine/render.wip` in what is
/// reported. Nothing is resolved: that is [`identity`]'s job.
fn tidy(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A directory as the file system names it, which is what makes two paths
/// to one package the same package.
pub(crate) fn identity(dir: &Path) -> PathBuf {
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// The program's package, whose root is `root`, and every package it
/// depends on, however far down. A package is identified by
/// its directory, so two paths to it are one package; its name must be the
/// one it is depended on by; and a cycle is reported.
pub(crate) fn packages(root: &Path, loaded: &mut Loaded, overlay: &Overlay) -> Vec<Package> {
    let own = manifest(root, loaded, overlay);
    let mut packages = vec![Package {
        name: own
            .as_ref()
            .and_then(|m| m.name.as_ref().map(|(n, _)| n.clone())),
        version: own.as_ref().and_then(|m| m.version.clone()),
        dir: root.to_path_buf(),
        identity: identity(root),
        prefix: String::new(),
        depends: Vec::new(),
    }];
    let mut queue: std::collections::VecDeque<(usize, Manifest)> =
        own.into_iter().map(|m| (0, m)).collect();
    while let Some((owner, said)) = queue.pop_front() {
        for (name, path, span) in said.depends {
            let wrong = |message: String, label: &str| {
                Diagnostic::error(wip_syntax::codes::PACKAGE, message, span, label.to_string())
            };
            if name == "std" || name == "package" {
                loaded.diagnostics.push(
                    wrong(format!("`{name}` is a name every package has already"), "taken")
                        .with_note("`std` is the standard library, and `package::NAME` is a package's own name"),
                );
                continue;
            }
            let dir = tidy(&packages[owner].dir.join(&path));
            if !dir.join("package.wip").is_file() {
                loaded.diagnostics.push(
                    wrong(
                        format!("`{}` has no `package.wip`", dir.display()),
                        "not a package",
                    )
                    .with_note("a dependency is a directory whose `package.wip` names it; `path` is relative to this package's root"),
                );
                continue;
            }
            let id = identity(&dir);
            if let Some(found) = packages.iter().position(|p| p.identity == id) {
                // Reached again: a diamond, which is one package, or a
                // cycle, which is reported below.
                if packages[found].name.as_deref() != Some(name.as_str()) {
                    loaded.diagnostics.push(wrong(
                        format!(
                            "`{}` is package `{}`, not `{name}`",
                            dir.display(),
                            packages[found].name.as_deref().unwrap_or("without a name")
                        ),
                        "named differently here",
                    ));
                    continue;
                }
                packages[owner].depends.push((name, found, span));
                continue;
            }
            if packages
                .iter()
                .any(|p| p.name.as_deref() == Some(name.as_str()))
            {
                loaded.diagnostics.push(
                    wrong(
                        format!("two packages of this program are named `{name}`"),
                        "a second one",
                    )
                    .with_note("a package's name is the first segment of the paths that reach it, so one program has one package of each name"),
                );
                continue;
            }
            let Some(theirs) = manifest(&dir, loaded, overlay) else {
                continue;
            };
            match &theirs.name {
                Some((declared, _)) if *declared == name => {}
                Some((declared, declared_span)) => {
                    loaded.diagnostics.push(
                        wrong(
                            format!("`{}` is package `{declared}`, not `{name}`", dir.display()),
                            "depended on by this name",
                        )
                        .with_secondary(*declared_span, "it says this")
                        .with_note("a package is imported by the name it gives itself, so that a name is never a local alias of something else; `import engine as e` renames it where it is used"),
                    );
                    continue;
                }
                None => continue,
            }
            let index = packages.len();
            packages.push(Package {
                name: Some(name.clone()),
                version: theirs.version.clone(),
                dir,
                identity: id,
                prefix: name.clone(),
                depends: Vec::new(),
            });
            packages[owner].depends.push((name, index, span));
            queue.push_back((index, theirs));
        }
    }
    if let Some(diagnostic) = package_cycle(&packages) {
        loaded.diagnostics.push(diagnostic);
    }
    packages
}

/// A package that depends on itself, however far round: the first one
/// found, reported where the `@depends` that closes it was written.
fn package_cycle(packages: &[Package]) -> Option<Diagnostic> {
    fn walk(
        at: usize,
        packages: &[Package],
        path: &mut Vec<usize>,
        done: &mut Vec<usize>,
    ) -> Option<(usize, usize, Span)> {
        for &(_, to, span) in &packages[at].depends {
            if path.contains(&to) {
                return Some((at, to, span));
            }
            if done.contains(&to) {
                continue;
            }
            path.push(to);
            if let Some(found) = walk(to, packages, path, done) {
                return Some(found);
            }
            path.pop();
            done.push(to);
        }
        None
    }
    let (from, to, span) = walk(0, packages, &mut vec![0], &mut Vec::new())?;
    let name = |i: usize| {
        packages[i]
            .name
            .clone()
            .unwrap_or_else(|| "the program's".to_string())
    };
    Some(
        Diagnostic::error(
            wip_syntax::codes::PACKAGE,
            format!("packages `{}` and `{}` depend on each other", name(from), name(to)),
            span,
            "this closes the circle",
        )
        .with_note("packages form a tree without cycles, so that each can be compiled after what it uses; what both need belongs in a third"),
    )
}

/// Which package a module is in, and its path within it: `None` for the
/// standard library. A dependency's modules begin with its name.
pub(crate) fn locate(packages: &[Package], path: &str) -> Option<(usize, String)> {
    if path == "std" || path.starts_with("std::") {
        return None;
    }
    let (first, rest) = path.split_once("::").unwrap_or((path, ""));
    match packages.iter().skip(1).position(|p| p.prefix == first) {
        Some(index) => Some((index + 1, rest.to_string())),
        None => Some((0, path.to_string())),
    }
}

/// The module a path written in `owner`'s package means, as the checker
/// works it out: `std`'s and a dependency's as written, and
/// one of the package's own from its root.
pub(crate) fn meant(packages: &[Package], owner: Option<usize>, written: &str) -> String {
    let Some(owner) = owner else {
        return written.to_string();
    };
    let package = &packages[owner];
    let first = written.split("::").next().unwrap_or("");
    if package.prefix.is_empty()
        || first == "std"
        || package.depends.iter().any(|(name, ..)| name == first)
    {
        written.to_string()
    } else if written.is_empty() {
        package.prefix.clone()
    } else {
        format!("{}::{written}", package.prefix)
    }
}

/// What is wrong with an import that crosses from `owner`'s package into
/// another: a package it does not depend on, a module of
/// its own named like a dependency, or one under another package's
/// `internal/`.
pub(crate) fn import_across_packages(
    packages: &[Package],
    owner: Option<usize>,
    written: &str,
    imported: &str,
    span: Span,
) -> Option<Diagnostic> {
    let owner = owner?;
    let (target, local) = locate(packages, imported)?;
    let first = written.split("::").next().unwrap_or("");
    let depends = packages[owner]
        .depends
        .iter()
        .any(|(name, ..)| name == first);
    if depends && module_dir(&packages[owner].dir, first).is_dir() {
        return Some(
            Diagnostic::error(
                wip_syntax::codes::PACKAGE,
                format!("`{first}` is both a package this one depends on and a module of its own"),
                span,
                "which one is meant",
            )
            .with_help(format!(
                "rename the directory `{first}`, or depend on the package by another name"
            )),
        );
    }
    if target == owner {
        return None;
    }
    if !depends {
        let name = packages[target].name.clone().unwrap_or_default();
        return Some(
            Diagnostic::error(
                wip_syntax::codes::PACKAGE,
                format!("this package does not depend on `{name}`"),
                span,
                "not a dependency",
            )
            .with_help(format!(
                "say so in `package.wip`: `@depends(\"{name}\", path = \"…\")`"
            )),
        );
    }
    if local.split("::").any(|segment| segment == "internal") {
        let name = packages[target].name.clone().unwrap_or_default();
        return Some(
            Diagnostic::error(
                wip_syntax::codes::INTERNAL_MODULE,
                format!("`{written}` is internal to package `{name}`"),
                span,
                "not its interface",
            )
            .with_note("a module under a directory named `internal` can be imported by its own package's modules and by no other package's"),
        );
    }
    None
}

/// The `.c` files of one module for `target`, and on an Apple target its `.m`
/// files, in name order. The standard library is compiled in and has none; a
/// module that cannot be read has its missing import reported elsewhere.
pub(crate) fn module_c_files(dir: &Path, target: targets::Target) -> Vec<PathBuf> {
    let listing = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let Ok(entries) = std::fs::read_dir(listing) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| dir.join(entry.file_name()))
        // Objective-C is Apple's, so a `.m` file is glue for an Apple
        // target, and passed over elsewhere. A `.c` file
        // may say which target it is for in its name.
        .filter(|file| {
            let apple = target.vendor == "apple";
            let takes = file
                .extension()
                .is_some_and(|e| e == "c" || (e == "m" && apple));
            takes
                && file
                    .file_name()
                    .is_some_and(|name| targets::takes_file(&name.to_string_lossy(), target))
        })
        .collect();
    files.sort();
    files
}

/// The first import cycle, if the modules form one.
pub(crate) fn import_cycle(edges: &[(String, String, Span)]) -> Option<Diagnostic> {
    fn walk(
        at: &str,
        edges: &[(String, String, Span)],
        path: &mut Vec<String>,
        done: &mut Vec<String>,
    ) -> Option<((String, String), Span)> {
        for (from, to, span) in edges.iter().filter(|(from, ..)| from == at) {
            if path.contains(to) {
                return Some(((to.clone(), from.clone()), *span));
            }
            if done.contains(to) {
                continue;
            }
            path.push(to.clone());
            if let Some(found) = walk(to, edges, path, done) {
                return Some(found);
            }
            path.pop();
            done.push(to.clone());
        }
        None
    }
    let mut path = vec![String::new()];
    let mut done = Vec::new();
    let ((to, from), span) = walk("", edges, &mut path, &mut done)?;
    let diagnostic = Diagnostic::error(
        wip_syntax::codes::UNKNOWN_MODULE,
        format!("modules `{to}` and `{from}` import each other"),
        span,
        "this import closes the circle",
    )
    .with_note("modules form a directed acyclic graph, so each can be compiled on its own");
    Some(diagnostic)
}

#[cfg(test)]
mod package_tests {
    use super::{is_semver, tidy};
    use std::path::{Path, PathBuf};

    #[test]
    fn semantic_versions() {
        for good in [
            "0.0.0",
            "1.4.0",
            "10.20.30",
            "2.0.0-rc.1",
            "1.0.0+build.7",
            "1.0.0-alpha-2+sha.5114f85",
        ] {
            assert!(is_semver(good), "{good}");
        }
        for bad in [
            "1.4",
            "1.4.0.1",
            "01.4.0",
            "1.4.x",
            "v1.4.0",
            "1.4.0-",
            "1.4.0+",
            "1.4.0-a..b",
            "",
        ] {
            assert!(!is_semver(bad), "{bad}");
        }
    }

    #[test]
    fn a_dependency_s_path_is_tidied_not_resolved() {
        assert_eq!(tidy(Path::new("game/../engine")), PathBuf::from("engine"));
        assert_eq!(tidy(Path::new("../engine")), PathBuf::from("../engine"));
        assert_eq!(tidy(Path::new("./a/./b/../c")), PathBuf::from("a/c"));
    }
}
