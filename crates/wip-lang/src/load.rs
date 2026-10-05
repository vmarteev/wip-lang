//! Reading a program: its modules' files, from disk, from an editor's
//! unsaved buffers, or from memory, following its imports.

use super::*;

/// Every file of a program, laid out end to end so that one `Span` names a
/// place in exactly one of them.
#[derive(Default)]
pub struct Sources {
    /// Name, text, and the offset where the file's spans begin.
    files: Vec<(String, String, u32)>,
    /// Each file's line starts, found the first time a place in it is
    /// asked for: debug information asks for thousands.
    lines: Vec<std::sync::OnceLock<wip_syntax::LineIndex>>,
}

impl Sources {
    /// Adds a file and returns the offset its spans start at.
    pub fn add(&mut self, name: String, text: String) -> u32 {
        let start = self
            .files
            .last()
            .map_or(0, |(_, text, start)| start + text.len() as u32 + 1);
        self.files.push((name, text, start));
        self.lines.push(std::sync::OnceLock::new());
        start
    }

    pub fn single(name: String, text: String) -> Sources {
        let mut sources = Sources::default();
        sources.add(name, text);
        sources
    }

    /// Where a span was written, for a panic's message: the
    /// file as diagnostics name it, and the line and column, from one.
    pub fn locate(&self, span: Span) -> (String, u32, u32) {
        let Some(file) = self
            .files
            .iter()
            .rposition(|(_, _, start)| *start <= span.lo)
        else {
            return ("<unknown>".to_string(), 0, 0);
        };
        let (name, text, base) = &self.files[file];
        let index = self.lines[file].get_or_init(|| wip_syntax::LineIndex::new(text));
        let (line, column) = index.line_col(text, span.lo - base);
        (name.to_string(), line, column)
    }

    /// Every file, in the order their spans lie in: name, text, and the
    /// offset where its spans begin.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str, u32)> {
        self.files
            .iter()
            .map(|(name, text, base)| (name.as_str(), text.as_str(), *base))
    }

    /// The file a span falls in: its name, its text, and its base offset.
    pub fn of(&self, span: Span) -> Option<(&str, &str, u32)> {
        self.files
            .iter()
            .rev()
            .find(|(_, _, start)| *start <= span.lo)
            .map(|(name, text, start)| (name.as_str(), text.as_str(), *start))
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// One module's parsed files.
pub struct LoadedModule {
    /// `a::b::c`, empty for the root module; a dependency's modules begin
    /// with its name, `engine::render`.
    pub path: String,
    pub files: Vec<Parsed>,
    /// What `@derive` writes for its types: syntax the checker reads as
    /// one more of its files, placed in the files above.
    pub derived: Ast,
    /// Where its files are. Empty for the standard library, which is in
    /// the compiler.
    pub dir: PathBuf,
    /// What its paths are written from, and the packages they may begin
    /// with: its package's.
    pub prefix: String,
    pub depends: Vec<String>,
    /// Its package's root, which a file it embeds may not leave.
    /// `None` for the standard library.
    pub root: Option<PathBuf>,
}

impl LoadedModule {
    /// The module as the checker reads it: its files, and then what
    /// `@derive` wrote for them.
    pub fn syntax(&self) -> wip_hir::ModuleAst<'_> {
        wip_hir::ModuleAst {
            path: self.path.clone(),
            files: self
                .files
                .iter()
                .map(|f| &f.ast)
                .chain((!self.derived.items.is_empty()).then_some(&self.derived))
                .collect(),
            prefix: self.prefix.clone(),
            depends: self.depends.clone(),
            // A module of the program has a package; the standard
            // library's have none, and no directory. An empty directory is
            // the current one, as a bare file name's parent is.
            dir: self
                .root
                .as_ref()
                .map(|_| match self.dir.as_os_str().is_empty() {
                    true => PathBuf::from("."),
                    false => self.dir.clone(),
                }),
            root: self
                .root
                .as_ref()
                .map(|root| match root.as_os_str().is_empty() {
                    true => PathBuf::from("."),
                    false => root.clone(),
                }),
        }
    }
}

/// A program: every module it reaches, and every file's text.
pub struct Loaded {
    pub sources: Sources,
    pub interner: Interner,
    pub modules: Vec<LoadedModule>,
    /// The `.c` files of the modules, compiled and linked with the program.
    /// A module's directory is also a header search path, so its C files can
    /// include their own headers.
    pub c_files: Vec<PathBuf>,
    /// Every module's directory, searched for the headers a declaration
    /// names with `@header`.
    pub include_dirs: Vec<PathBuf>,
    /// What lexing, parsing and module discovery reported.
    pub diagnostics: Vec<Diagnostic>,
    /// The entry's directory, where the modules are found.
    pub root: PathBuf,
    /// The program's package and the ones it depends on.
    pub packages: Vec<Package>,
}

/// What is compiled: a program; a program and every module of its
/// package, imported or not, which is what `wip check` checks; or those
/// together with the tests the modules carry in their `.test.wip` files.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Program,
    Package,
    Tests,
}

impl Mode {
    /// Whether a file belongs to what is being compiled: a `.test.wip` file
    /// is compiled by `wip test` and by nothing else.
    fn takes(self, name: &str) -> bool {
        self == Mode::Tests || !name.ends_with(".test.wip")
    }

    /// Whether every module of the program's package is loaded, so that
    /// one nothing imports yet is still checked, and its tests run.
    fn whole_package(self) -> bool {
        matches!(self, Mode::Package | Mode::Tests)
    }
}

/// Reads the program that starts at `entry`, following its imports. The
/// entry file's directory is the root module.
pub fn load(entry: &Path) -> Result<Loaded, String> {
    load_mode(entry, Mode::Program, &mut Timings::default())
}

/// Like [`load`], compiling what `mode` says, and
/// recording how long reading, lexing and parsing take. The entry must be
/// a `.wip` file, or the directory of a package, on disk: what a command
/// line names.
///
/// Modules are loaded in waves: the root, then the modules it imports,
/// then the ones those import. The files of a wave are lexed and parsed in
/// parallel.
pub fn load_mode(entry: &Path, mode: Mode, timings: &mut Timings) -> Result<Loaded, String> {
    load_for(entry, mode, targets::Target::host(), timings)
}

/// Like [`load_mode`], for `target` rather than this machine: the items
/// and files that target takes, and its `TARGET_OS`, `TARGET_ARCH` and
/// `TARGET_VENDOR`. What is loaded can be checked, and is
/// built only where `target` is this machine.
pub fn load_for(
    entry: &Path,
    mode: Mode,
    target: targets::Target,
    timings: &mut Timings,
) -> Result<Loaded, String> {
    // The entry's whole directory is compiled, so a mistyped entry must not
    // quietly compile whatever else is there.
    let metadata = std::fs::metadata(entry)
        .map_err(|err| format!("cannot read `{}`: {err}", entry.display()))?;
    // A package is built by naming its directory, or its `package.wip`.
    if metadata.is_dir() {
        if !entry.join("package.wip").is_file() {
            return Err(format!(
                "`{}` is a directory with no `package.wip`: name the `.wip` file the program starts in",
                entry.display()
            ));
        }
        return load_with(entry, None, mode, target, &Overlay::default(), timings);
    }
    if !metadata.is_file() || entry.extension().is_none_or(|e| e != "wip") {
        return Err(format!("`{}` is not a `.wip` file", entry.display()));
    }
    // A bare file name's parent is the empty path, which `module_files`
    // reads as the current directory.
    let root = entry.parent().unwrap_or_else(|| Path::new(""));
    load_with(root, None, mode, target, &Overlay::default(), timings)
}

/// Texts that stand in for files on disk: what an editor has not saved.
/// A file is known by its canonical path, so two paths to
/// it are one file, and one not yet on disk is read as if it were.
#[derive(Default, Clone)]
pub struct Overlay {
    files: HashMap<PathBuf, String>,
}

impl Overlay {
    /// `text` is `path`'s, whatever the disk says.
    pub fn set(&mut self, path: &Path, text: String) {
        self.files.insert(canonical_file(path), text);
    }

    /// `path` is read from the disk again.
    pub fn remove(&mut self, path: &Path) {
        self.files.remove(&canonical_file(path));
    }

    /// `path`'s text: the overlay's, or the disk's.
    pub(crate) fn read(&self, path: &Path) -> std::io::Result<String> {
        if !self.files.is_empty()
            && let Some(text) = self.files.get(&canonical_file(path))
        {
            return Ok(text.clone());
        }
        std::fs::read_to_string(path)
    }

    /// The files the overlay holds in `dir` that are not on disk: named
    /// as `dir` names them.
    fn unsaved_in(&self, dir: &Path) -> Vec<PathBuf> {
        if self.files.is_empty() {
            return Vec::new();
        }
        let canonical = identity(dir);
        self.files
            .keys()
            .filter(|file| file.parent() == Some(canonical.as_path()) && !file.exists())
            .filter_map(|file| Some(dir.join(file.file_name()?)))
            .collect()
    }
}

/// A file's canonical path, which it has even before it is on disk: its
/// directory's, and its name.
pub(crate) fn canonical_file(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    match path.file_name() {
        Some(name) => identity(dir).join(name),
        None => identity(path),
    }
}

/// Like [`load_mode`], with what `overlay` holds read in place of the
/// files on disk: the program an editor shows. `entry` is
/// the root the editor worked out, and is not checked on disk as a command
/// line's is, since what the editor holds may not be saved yet.
pub fn load_overlaid(entry: &Path, mode: Mode, overlay: &Overlay) -> Result<Loaded, String> {
    let mut timings = Timings::default();
    if entry.is_dir() {
        return load_with(
            entry,
            None,
            mode,
            targets::Target::host(),
            overlay,
            &mut timings,
        );
    }
    let root = entry.parent().unwrap_or_else(|| Path::new(""));
    load_with(
        root,
        None,
        mode,
        targets::Target::host(),
        overlay,
        &mut timings,
    )
}

/// A program whose root module is one file held in memory, with the
/// prelude and whatever it imports read as for any program. The file's
/// directory is where its imports are looked for, and nothing else in it
/// is compiled.
pub fn load_source(file: &SourceFile, timings: &mut Timings) -> Result<Loaded, String> {
    let root = Path::new(&file.name)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let given = vec![(file.name.clone(), file.text.clone())];
    load_with(
        root,
        Some(given),
        Mode::Program,
        targets::Target::host(),
        &Overlay::default(),
        timings,
    )
}

/// The loader behind both: `given` is the root module's files, where
/// they are not read from `root`, and `target` what they are loaded for.
fn load_with(
    root: &Path,
    given: Option<Vec<(String, String)>>,
    mode: Mode,
    target: targets::Target,
    overlay: &Overlay,
    timings: &mut Timings,
) -> Result<Loaded, String> {
    let mut loaded = Loaded {
        sources: Sources::default(),
        interner: Interner::new(),
        modules: Vec::new(),
        c_files: Vec::new(),
        include_dirs: Vec::new(),
        diagnostics: Vec::new(),
        root: root.to_path_buf(),
        packages: Vec::new(),
    };
    // The program's package and every one it depends on, from their
    // `package.wip` files, before any module is read.
    loaded.packages = timings.time("read", || packages(root, &mut loaded, overlay));
    // `package::NAME` and `package::VERSION`: a module of two constants
    // for each package that has a name, written here, as the prelude's
    // `target.wip` is.
    let generated: Vec<(String, String, String)> = loaded
        .packages
        .iter()
        .filter_map(|package| {
            let name = package.name.as_ref()?;
            let path = match package.prefix.is_empty() {
                true => "package".to_string(),
                false => format!("{}::package", package.prefix),
            };
            let mut text = format!("pub val NAME: str = \"{name}\"\n");
            if let Some(version) = &package.version {
                text.push_str(&format!("pub val VERSION: str = \"{version}\"\n"));
            }
            Some((path, format!("<package {name}>"), text))
        })
        .collect();
    let threads = parallel::threads();
    // The root module, the prelude, which every program has without an
    // import, and each package's own.
    let mut wave = vec![String::new(), wip_hir::PRELUDE.to_string()];
    wave.extend(generated.iter().map(|(path, ..)| path.clone()));
    // `wip check` and `wip test` of a package see all of it: each module,
    // imported or not, is loaded as though the root imported it, and is
    // checked, and its tests run, as an imported one's are.
    if mode.whole_package()
        && given.is_none()
        && let Some(package) = loaded.packages.first()
        && package.dir.join("package.wip").is_file()
    {
        let mut modules = Vec::new();
        package_modules(&package.dir, "", &mut modules);
        for module in modules {
            if !wave.contains(&module) {
                wave.push(module);
            }
        }
    }
    let mut seen: Vec<String> = wave.clone();
    // Which module imports which, and where, so a cycle can be reported.
    let mut edges: Vec<(String, String, Span)> = Vec::new();
    while !wave.is_empty() {
        // The wave's files follow the earlier waves' in `loaded.sources`;
        // `owners` says which of the wave's modules each belongs to.
        let first = loaded.sources.files.len();
        let mut owners = Vec::new();
        let mut places: Vec<(PathBuf, Option<usize>)> = Vec::new();
        for (module, path) in wave.iter().enumerate() {
            let made = generated.iter().find(|(p, ..)| p == path);
            let located = locate(&loaded.packages, path);
            let files = match (made, &given, path.is_empty(), &located) {
                (Some((_, name, text)), ..) => vec![(name.clone(), text.clone())],
                (None, Some(given), true, _) => given.clone(),
                (None, _, _, None) => std_files(path, target),
                (None, _, _, Some((package, local))) => {
                    let dir = &loaded.packages[*package].dir;
                    // `wip test game` runs `game`'s tests; a dependency's
                    // are run by testing it.
                    let mode = match package {
                        0 => mode,
                        _ => Mode::Program,
                    };
                    timings.time("read", || module_files(dir, local, mode, target, overlay))?
                }
            };
            for (name, text) in files {
                loaded.sources.add(name, text);
                owners.push(module);
            }
            // Its C files are compiled with the program, and its directory
            // is searched for the headers it names. A root
            // given in memory has none, and neither has a module the
            // compiler wrote.
            let place = match (&located, made) {
                (Some((package, local)), None) => {
                    let dir = module_dir(&loaded.packages[*package].dir, local);
                    if given.is_none() || !path.is_empty() {
                        loaded
                            .c_files
                            .extend(timings.time("read", || module_c_files(&dir, target)));
                    }
                    if !loaded.include_dirs.contains(&dir) {
                        loaded.include_dirs.push(dir.clone());
                    }
                    (dir, Some(*package))
                }
                _ => (PathBuf::new(), located.map(|(package, _)| package)),
            };
            places.push(place);
        }
        let sources = &loaded.sources.files[first..];
        let sizes: Vec<usize> = sources.iter().map(|(_, text, _)| text.len()).collect();
        let ranges = parallel::split(&sizes, threads);
        // Each range of files is lexed with an interner of its own. Absorbed
        // in order, they number the symbols as one interner would have.
        let lexed = timings.time("lex", || {
            parallel::run(ranges.clone(), |range| {
                let mut interner = Interner::new();
                let lexed: Vec<Lexed> = sources[range]
                    .iter()
                    .map(|(_, text, base)| lex_at(text, *base, &mut interner))
                    .collect();
                (interner, lexed)
            })
        });
        let renamed: Vec<(Range<usize>, Vec<Symbol>, Vec<Lexed>)> = timings.time("lex", || {
            ranges
                .into_iter()
                .zip(lexed)
                .map(|(range, (interner, lexed))| (range, loaded.interner.absorb(&interner), lexed))
                .collect()
        });
        let parsed = timings.time("parse", || {
            parallel::run(renamed, |(range, map, lexed)| {
                sources[range]
                    .iter()
                    .zip(lexed)
                    .map(|((_, text, base), mut lexed)| {
                        lexed.rename_symbols(&map);
                        let parsed = parse_at(text, *base, &lexed);
                        (lexed.diagnostics, parsed)
                    })
                    .collect::<Vec<_>>()
            })
        });

        // In file order: diagnostics, imports, and the next wave.
        let mut files: Vec<Vec<Parsed>> = wave.iter().map(|_| Vec::new()).collect();
        let mut next = Vec::new();
        for ((lex_diagnostics, mut parsed), &module) in parsed.into_iter().flatten().zip(&owners) {
            loaded.diagnostics.extend(lex_diagnostics);
            loaded
                .diagnostics
                .extend(parsed.diagnostics.iter().cloned());
            // What this target does not take is dropped before anything
            // reads the file: an import it held is not followed, and a
            // name it declared is free.
            loaded
                .diagnostics
                .extend(targets::keep(&mut parsed.ast, &loaded.interner, target));
            for item in &parsed.ast.items {
                let Item::Import(decl) = item else { continue };
                let written: Vec<&str> = decl
                    .path
                    .iter()
                    .map(|n| loaded.interner.resolve(n.sym))
                    .collect();
                let written = written.join("::");
                // What the path means from this module's package, which the
                // checker works out the same way.
                let owner = places[module].1;
                let imported = meant(&loaded.packages, owner, &written);
                if let Some(diagnostic) =
                    import_across_packages(&loaded.packages, owner, &written, &imported, decl.span)
                {
                    loaded.diagnostics.push(diagnostic);
                    continue;
                }
                edges.push((wave[module].clone(), imported.clone(), decl.span));
                if !seen.contains(&imported) {
                    seen.push(imported.clone());
                    next.push(imported);
                }
            }
            files[module].push(parsed);
        }
        for ((path, files), (dir, package)) in wave.into_iter().zip(files).zip(places) {
            // A directory with no files is no module: the import that named
            // it reports it, with the span of the path it wrote.
            if files.is_empty() && !path.is_empty() {
                continue;
            }
            // What `@derive` writes, as syntax of the module's own, placed
            // in its files.
            let written: Vec<&Ast> = files.iter().map(|f| &f.ast).collect();
            let derived = wip_syntax::derive::expand(&written, &mut loaded.interner);
            let (prefix, depends, root) = match package {
                Some(package) => {
                    let package = &loaded.packages[package];
                    let depends = package.depends.iter().map(|(name, ..)| name.clone());
                    (
                        package.prefix.clone(),
                        depends.collect(),
                        Some(package.dir.clone()),
                    )
                }
                None => (String::new(), Vec::new(), None),
            };
            loaded.modules.push(LoadedModule {
                path,
                files,
                derived,
                dir,
                prefix,
                depends,
                root,
            });
        }
        wave = next;
    }
    // The root module was the first wave, so it is index 0, which is where
    // `main` must be.
    if let Some(diagnostic) = import_cycle(&edges) {
        loaded.diagnostics.push(diagnostic);
    }
    loaded.diagnostics.sort_by_key(|d| d.primary.span.lo);
    Ok(loaded)
}

/// The `.wip` files of one module of a package whose root is `root`, in
/// name order. `package.wip` is the package's, not a module's.
fn module_files(
    root: &Path,
    path: &str,
    mode: Mode,
    target: targets::Target,
    overlay: &Overlay,
) -> Result<Vec<(String, String)>, String> {
    let dir = module_dir(root, path);
    // An empty path is the current directory: `wip run main.wip`. Reading it
    // as a path would find nothing.
    let listing = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir.as_path()
    };
    let Ok(entries) = std::fs::read_dir(listing) else {
        // The import reports it, with the span of the path that named it.
        return Ok(Vec::new());
    };
    // Named as the user wrote the path: `main.wip`, not `./main.wip`.
    let mut found: Vec<PathBuf> = entries.flatten().map(|e| dir.join(e.file_name())).collect();
    found.extend(
        overlay
            .unsaved_in(listing)
            .into_iter()
            .map(|f| match f.file_name() {
                Some(name) => dir.join(name),
                None => f,
            }),
    );
    let mut files = Vec::new();
    for file in found {
        if file.extension().is_none_or(|e| e != "wip") {
            continue;
        }
        // A test file is compiled by `wip test` and by nothing else,
        // and a file may say which target it is for.
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name == "package.wip" || !mode.takes(&name) || !targets::takes_file(&name, target) {
            continue;
        }
        let text = overlay
            .read(&file)
            .map_err(|err| format!("cannot read `{}`: {err}", file.display()))?;
        if u32::try_from(text.len()).is_err() {
            return Err(format!("`{}` is larger than 4 GiB", file.display()));
        }
        files.push((file.display().to_string(), text));
    }
    files.sort();
    Ok(files)
}

/// Where a module's own files are: the root, and then its path.
/// The modules of a package below `dir`, whose path is `prefix`: every
/// directory under it that holds a `.wip` file and is named as a module
/// may be, but not a hidden one, `target`, or one with a `package.wip` of
/// its own, which is another package. In name order, so a
/// run loads them the same way each time.
fn package_modules(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .filter(|(name, path)| {
            is_module_name(name) && name != "target" && !path.join("package.wip").exists()
        })
        .collect();
    dirs.sort();
    for (name, path) in dirs {
        let module = match prefix {
            "" => name,
            _ => format!("{prefix}::{name}"),
        };
        let holds_wip = std::fs::read_dir(&path).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry.path().extension().is_some_and(|e| e == "wip")
                    && entry.file_type().is_ok_and(|t| t.is_file())
            })
        });
        if holds_wip {
            out.push(module.clone());
        }
        package_modules(&path, &module, out);
    }
}

/// Whether a directory's name could be a module's in an import: a letter
/// or `_`, then letters, digits and `_`. A hidden directory is not one.
fn is_module_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn module_dir(root: &Path, path: &str) -> PathBuf {
    let mut dir = root.to_path_buf();
    for segment in path.split("::").filter(|s| !s.is_empty()) {
        dir.push(segment);
    }
    dir
}
