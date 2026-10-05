//! Building: checking a loaded program, generating its code, compiling
//! the C it carries and linking the executable or library.

use super::*;

/// Where to look for the C a program binds: the directories a command line
/// added with `-I` and `-L`. What a module needs is in the
/// module; where it is on this machine is not.
#[derive(Debug, Default, Clone)]
pub struct Search {
    pub includes: Vec<PathBuf>,
    pub library_paths: Vec<PathBuf>,
}

/// What `wip build` writes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Emit {
    /// An executable, with the C entry point.
    #[default]
    Program,
    /// An archive C links against: the objects, the runtime, and the C
    /// files the modules carry.
    Static,
    /// A shared library: `.dylib` on macOS, `.so` elsewhere.
    Dynamic,
}

impl Emit {
    /// Whether what is written needs a `main` — a library does not.
    pub fn is_program(self) -> bool {
        self == Emit::Program
    }

    /// What the flag's value is written as, for a message.
    pub fn text(self) -> &'static str {
        match self {
            Emit::Program => "program",
            Emit::Static => "static",
            Emit::Dynamic => "dynamic",
        }
    }
}

/// Whether a build is for debugging or for use: the
/// program does the same either way, and a release build is faster.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Not optimised, with the move checks; the default.
    #[default]
    Debug,
    /// Optimised, without the move checks, and with the debug information
    /// in a file beside the program.
    Release,
}

impl Profile {
    pub fn optimizes(self) -> bool {
        self == Profile::Release
    }
}

/// How a program is built, beyond what and where: where the C it binds
/// is on this machine, what is written, and whether for debugging or for
/// use. The default is a debug build of an executable, looking for C where
/// the compiler looks by itself.
#[derive(Debug, Default, Clone)]
pub struct BuildOptions {
    /// The directories `-I` and `-L` added.
    pub search: Search,
    /// A program, or a library C links against.
    pub emit: Emit,
    /// Where to write the C declarations of what a library exports, if
    /// anywhere.
    pub header: Option<PathBuf>,
    /// Debug or release.
    pub profile: Profile,
    /// The processor the program is built for.
    pub cpu: wip_codegen::Cpu,
    /// What `--backend` said; the profile's otherwise.
    pub backend: Option<Backend>,
}

/// Compiles a loaded program as `options` say, to `output`, and answers
/// the warnings.
pub fn build_loaded(
    loaded: &mut Loaded,
    output: &Path,
    options: &BuildOptions,
    timings: &mut Timings,
) -> Result<Vec<Diagnostic>, BuildError> {
    let mut checked = check_loaded(loaded, timings);
    if checked.diagnostics.iter().any(Diagnostic::is_error) {
        return Err(BuildError::Diagnostics(checked.diagnostics));
    }
    checked.program.check_moves = options.profile == Profile::Debug;
    // A program has an entry point; a library is called from one.
    let main = match options.emit.is_program() {
        true => Some(wip_codegen::Entry::Main(
            wip_hir::check_main(&checked.program, &loaded.interner)
                .map_err(|d| BuildError::Diagnostics(vec![*d]))?
                .main,
        )),
        false => None,
    };
    write_binary(loaded, &checked, main, output, options, timings)?;
    Ok(checked.diagnostics)
}

/// Compiles a checked program that starts at `entry` and links it, writing
/// the declarations of what a library exports where one was asked for.
fn write_binary(
    loaded: &Loaded,
    checked: &Checked,
    entry: Option<wip_codegen::Entry<'_>>,
    output: &Path,
    options: &BuildOptions,
    timings: &mut Timings,
) -> Result<(), BuildError> {
    let BuildOptions {
        search,
        emit,
        header,
        profile,
        cpu,
        backend,
    } = options;
    let (emit, header) = (*emit, header.as_deref());
    let (backend, _) = choose_backend(*profile, *backend)?;
    let llvm = backend == Backend::Llvm;
    // What C cannot be given to call, whichever backend builds it.
    let refused = wip_mir::c_abi::refused_exports(
        &checked.program,
        &loaded.interner,
        wip_mir::c_abi::Arch::host(),
    );
    if !refused.is_empty() {
        return Err(BuildError::Diagnostics(refused));
    }
    let object = timings.time("codegen", || {
        if llvm {
            let locate = |span| loaded.sources.locate(span);
            let entry = entry.map(|entry| match entry {
                wip_codegen::Entry::Main(main) => wip_llvm::Entry::Main(main),
                wip_codegen::Entry::Tests(tests) => wip_llvm::Entry::Tests(tests),
            });
            let ir = wip_llvm::compile(&checked.program, &loaded.interner, entry, &locate);
            return crate::backend::llvm_object(&ir, *cpu).map(|object| vec![object]);
        }
        let locate = |span| loaded.sources.locate(span);
        wip_codegen::compile(
            &checked.program,
            &loaded.interner,
            entry,
            &locate,
            wip_codegen::Settings {
                optimize: profile.optimizes(),
                cpu: *cpu,
            },
        )
        .map_err(BuildError::Diagnostics)
    })?;
    timings.time("link", || {
        let shims = match llvm {
            true => {
                let shimmed = wip_llvm::shimmed(&checked.program);
                wip_mir::write_shims(&checked.program, &loaded.interner, |id| {
                    shimmed.contains(&id)
                })
            }
            false => shims(&checked.program, &loaded.interner),
        };
        let dirs: Vec<PathBuf> = loaded.modules.iter().map(|m| m.dir.clone()).collect();
        let include_dirs = c_include_dirs(&checked.program, &dirs, &loaded.include_dirs)?;
        // The C a module names with `@source`, and the header it puts
        // before its own C with `@prefix`, and what it
        // defines for it with `@define`. All are known
        // only once the program is checked, which is why they join the
        // files here rather than where the modules were read.
        let c_files = c_sources(&checked.program, &dirs, &loaded.c_files)?;
        link(
            &object,
            &Linking {
                c_files: &c_files,
                shims: shims.as_deref(),
                include_dirs: &include_dirs,
                search,
                libraries: &checked.program.libraries,
                frameworks: &checked.program.frameworks,
                emit,
                profile: *profile,
                cpu: *cpu,
            },
            output,
        )
    })?;
    if let Some(path) = header {
        let guard = header_guard(path);
        let text = wip_mir::write_header(&checked.program, &loaded.interner, &guard);
        std::fs::write(path, text)
            .map_err(|err| BuildError::Link(format!("cannot write `{}`: {err}", path.display())))?;
    }
    Ok(())
}

/// Compiles a loaded program's tests to a runner at `output`: it calls each
/// test the program declares, in declaration order, and `filter` keeps the
/// ones whose name, as the runner prints it — `board::placesABall` — holds
/// it. The count is how many tests the runner holds. What
/// `options` say of what is written and where its declarations go is not
/// asked: tests are a program of their own, with no header to write.
pub fn build_tests(
    loaded: &mut Loaded,
    output: &Path,
    options: &BuildOptions,
    filter: Option<&str>,
    timings: &mut Timings,
) -> Result<(Vec<Diagnostic>, usize), BuildError> {
    let profile = options.profile;
    let mut checked = check_loaded(loaded, timings);
    if checked.diagnostics.iter().any(Diagnostic::is_error) {
        return Err(BuildError::Diagnostics(checked.diagnostics));
    }
    checked.program.check_moves = profile == Profile::Debug;
    let tests: Vec<wip_hir::FnId> = checked
        .program
        .fns
        .iter()
        .filter(|(_, def)| def.is_test)
        .filter(|&(id, _)| {
            filter.is_none_or(|text| {
                checked
                    .program
                    .test_name(id, &loaded.interner)
                    .contains(text)
            })
        })
        .map(|(id, _)| id)
        .collect();
    let options = BuildOptions {
        emit: Emit::Program,
        header: None,
        ..options.clone()
    };
    let entry = Some(wip_codegen::Entry::Tests(&tests));
    write_binary(loaded, &checked, entry, output, &options, timings)?;
    Ok((checked.diagnostics, tests.len()))
}

/// `BOARD_H` for `board.h`: what keeps a header from being read twice.
fn header_guard(path: &Path) -> String {
    let stem: String = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "wip".to_string())
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_uppercase(),
            false => '_',
        })
        .collect();
    format!("WIP_{stem}")
}

pub enum BuildError {
    /// The program has errors, or uses something the code generator does not
    /// support yet.
    Diagnostics(Vec<Diagnostic>),
    /// Writing temporary files or running the C compiler failed.
    Link(String),
}

/// Compiles `file` to a native executable at `output`, and returns the
/// warnings. The file is a program of its own, with the prelude and the
/// modules it imports, as `wip build` would compile it: every program
/// needs the prelude, which is where a panic is written.
pub fn build(file: &SourceFile, output: &Path) -> Result<Vec<Diagnostic>, BuildError> {
    let mut timings = Timings::default();
    let mut loaded = load_source(file, &mut timings).map_err(BuildError::Link)?;
    build_loaded(&mut loaded, output, &BuildOptions::default(), &mut timings)
}

/// Every C file compiled with the program: the ones found beside a
/// module's Wip files, and the ones a module names with `@source`, each
/// carrying the `@prefix` header of the module it belongs to.
fn c_sources(
    program: &wip_hir::Program,
    dirs: &[PathBuf],
    found: &[PathBuf],
) -> Result<Vec<c_build::CFile>, BuildError> {
    // A module's prefix header, as a path, by the module it is in.
    let prefix_of = |module: usize| -> Option<PathBuf> {
        program
            .c_prefix
            .iter()
            .find(|(owner, _)| *owner == module)
            .map(|(_, relative)| dirs[module].join(relative))
    };
    // What a module's C is compiled with defined.
    let defines_of = |module: usize| -> Vec<String> {
        program
            .c_defines
            .iter()
            .filter(|(owner, _)| *owner == module)
            .map(|(_, define)| define.clone())
            .collect()
    };
    // Which module a file found on disk belongs to: the deepest module
    // directory it is under.
    let owner_of = |file: &Path| -> Option<usize> {
        dirs.iter()
            .enumerate()
            .filter(|(_, dir)| !dir.as_os_str().is_empty() && file.starts_with(dir))
            .max_by_key(|(_, dir)| dir.components().count())
            .map(|(module, _)| module)
            // A module in the current directory: `wip run main.wip`.
            .or_else(|| dirs.iter().position(|dir| dir.as_os_str().is_empty()))
    };
    let mut files: Vec<c_build::CFile> = Vec::new();
    for file in found {
        let mut c = c_build::CFile::of(file.clone());
        let owner = owner_of(file);
        c.prefix = owner.and_then(prefix_of);
        c.defines = owner.map(defines_of).unwrap_or_default();
        files.push(c);
    }
    for (module, relative, language) in &program.c_sources {
        let path = dirs[*module].join(relative);
        if !path.is_file() {
            let name = match program.modules[*module].is_empty() {
                true => "the root module".to_string(),
                false => format!("module `{}`", program.modules[*module]),
            };
            return Err(BuildError::Link(format!(
                "`@source(\"{relative}\")` in {name}: `{}` is not a file",
                path.display()
            )));
        }
        let mut c = c_build::CFile::of(path);
        // What `@source` said it is written in.
        match language {
            Some(wip_hir::CLanguage::C) => c.language = c_build::Language::C,
            Some(wip_hir::CLanguage::ObjectiveC) => c.language = c_build::Language::ObjectiveC,
            None => {}
        }
        // Objective-C is Apple's, as it is for a file found beside the
        // module's own.
        if c.language == c_build::Language::ObjectiveC && targets::Target::host().vendor != "apple"
        {
            continue;
        }
        c.prefix = prefix_of(*module);
        c.defines = defines_of(*module);
        files.push(c);
    }
    Ok(files)
}

/// Where the program's C looks for headers: the modules' directories, and
/// then the directories they name with `@include`, each inside its module.
/// One that is not there is an error, since a header the C
/// cannot find would be reported by the C compiler, far from the cause.
fn c_include_dirs(
    program: &wip_hir::Program,
    module_dirs: &[PathBuf],
    modules: &[PathBuf],
) -> Result<Vec<PathBuf>, BuildError> {
    let mut dirs = modules.to_vec();
    for (module, relative) in &program.c_includes {
        let path = &program.modules[*module];
        let dir = module_dirs[*module].join(relative);
        if !dir.is_dir() {
            let module = match path.is_empty() {
                true => "the root module".to_string(),
                false => format!("module `{path}`"),
            };
            return Err(BuildError::Link(format!(
                "`@include(\"{relative}\")` in {module}: `{}` is not a directory",
                dir.display()
            )));
        }
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    Ok(dirs)
}

/// What the linker is given besides the objects: where to look for libraries,
/// the libraries the extern blocks name, and on an Apple target the frameworks.
fn link_args(
    library_paths: &[PathBuf],
    libraries: &[String],
    frameworks: &[String],
) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    for dir in library_paths {
        let mut arg = OsString::from("-L");
        arg.push(dir);
        args.push(arg);
    }
    args.extend(
        libraries
            .iter()
            .map(|name| OsString::from(format!("-l{name}"))),
    );
    // The runtime starts threads: a Linux whose C library
    // does not have them in it has them in a library of their own. A Mac
    // has them in libSystem.
    if cfg!(target_os = "linux") {
        args.push("-lpthread".into());
    }
    // No other system has frameworks, so elsewhere they are not asked for.
    if cfg!(target_vendor = "apple") {
        for name in frameworks {
            args.push("-framework".into());
            args.push(name.into());
        }
    }
    args
}

/// Everything the C toolchain is given besides the object files: what to
/// compile beside them, where to look, and what to write.
struct Linking<'a> {
    /// The modules' own C: the files found beside their Wip ones and
    /// those they name with `@source`, each with the `@prefix` header of
    /// its module.
    c_files: &'a [c_build::CFile],
    shims: Option<&'a str>,
    /// Where the program's C looks for headers: the modules' directories,
    /// and those they name with `@include`.
    include_dirs: &'a [PathBuf],
    search: &'a Search,
    libraries: &'a [String],
    frameworks: &'a [String],
    emit: Emit,
    profile: Profile,
    cpu: wip_codegen::Cpu,
}

/// The C the compiler writes for the calls it cannot make itself: the
/// ones `wip_mir::c_abi` leaves to a wrapper on this target.
fn shims(program: &Program, interner: &Interner) -> Option<String> {
    let shimmed = wip_mir::c_abi::shimmed(program, wip_mir::c_abi::Arch::host());
    wip_mir::write_shims(program, interner, |id| shimmed.contains(&id))
}

/// Compiles the program's C, each file once, and links it
/// with the modules' object files, using `$CC` or `cc`.
fn link(objects: &[Vec<u8>], linking: &Linking<'_>, output: &Path) -> Result<(), BuildError> {
    let Linking {
        c_files,
        shims,
        include_dirs,
        search,
        libraries,
        frameworks,
        emit,
        profile,
        cpu,
    } = *linking;
    let dir = TempDir::new()
        .map_err(|err| BuildError::Link(format!("cannot create a temporary directory: {err}")))?;
    let object_paths: Vec<PathBuf> = (0..objects.len())
        .map(|i| dir.path().join(format!("module{i}.o")))
        .collect();
    objects
        .iter()
        .zip(&object_paths)
        .try_for_each(|(object, path)| std::fs::write(path, object))
        .map_err(|err| BuildError::Link(format!("cannot write temporary files: {err}")))?;

    // The C the compiler wrote for the calls C has to make is kept where its
    // path depends only on what it says, so it compiles once. A person can read
    // it there if a call ever goes wrong.
    // The runtime is Wip, in the prelude: a program that
    // brings no C and needs no wrapper compiles no C.
    let cache = c_build::cache_dir();
    let mut files: Vec<c_build::CFile> = c_files.to_vec();
    if let Some(source) = shims {
        let generated =
            c_build::generated(&cache, &[("wip_shims.c", source)]).map_err(BuildError::Link)?;
        files.push(c_build::CFile::c(generated.join("wip_shims.c")));
    }

    // The modules' directories first, then the command line's, so a
    // module's header wins over one of the same name elsewhere.
    let mut includes: Vec<PathBuf> = Vec::new();
    // Each as the file system names it, so that the same directory is the
    // same flag wherever the build was started.
    // An empty path is the current directory, as a bare file name's parent
    // is: written as it is, it would be a `-I` that takes the next argument.
    for dir in include_dirs.iter().chain(&search.includes) {
        let dir = match dir.as_os_str().is_empty() {
            true => Path::new("."),
            false => dir.as_path(),
        };
        let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        if !includes.contains(&dir) {
            includes.push(dir);
        }
    }
    // Code in a shared library is placed wherever the library is loaded;
    // Apple's compilers make all code so.
    let position_independent = emit == Emit::Dynamic && !cfg!(target_vendor = "apple");
    let compiled = c_build::compile(
        &files,
        &includes,
        &cache,
        position_independent,
        profile.optimizes(),
        cpu,
    )
    .map_err(BuildError::Link)?;
    let mut members = object_paths;
    members.extend(compiled.objects);

    // An archive is made of objects, and `ar` collects them.
    if emit == Emit::Static {
        let mut args: Vec<OsString> = vec!["rcs".into(), output.into()];
        args.extend(members.iter().map(OsString::from));
        let ar = std::env::var_os("AR").unwrap_or_else(|| OsString::from("ar"));
        return run(&ar, &args);
    }
    let mut args: Vec<OsString> = vec!["-o".into(), output.into()];
    args.extend(members.iter().map(OsString::from));
    args.extend(link_args(&search.library_paths, libraries, frameworks));
    // A shared library has no entry point, and each platform spells it
    // differently.
    if emit == Emit::Dynamic {
        let flag = match std::env::consts::OS {
            "macos" | "ios" => "-dynamiclib",
            _ => "-shared",
        };
        args.insert(0, flag.into());
    }
    run(&c_build::compiler(), &args)?;
    // Apple's linker leaves the debug information in the objects and names
    // them in the program; the program's objects go when this returns, so
    // it is gathered beside it first, as `clang -g` does.
    // What the linker left in the program, naming the objects, is then of
    // no use, and is taken out, as Xcode does for a release: the debugger
    // finds the `.dSYM` by the program's UUID.
    if cfg!(target_vendor = "apple") {
        let args: Vec<OsString> = vec![output.into(), "-o".into(), beside(output, ".dSYM")];
        run(&OsString::from("dsymutil"), &args)?;
        return run(&OsString::from("strip"), &["-S".into(), output.into()]);
    }
    // Elsewhere the linker puts it in the program. A release build moves
    // it into a file beside the program, which the program names, as
    // Linux distributions ship theirs: the program stays small, and gdb
    // and profilers find the file where it is kept.
    if profile == Profile::Release {
        let objcopy = std::env::var_os("OBJCOPY").unwrap_or_else(|| OsString::from("objcopy"));
        let debug = beside(output, ".debug");
        run(
            &objcopy,
            &["--only-keep-debug".into(), output.into(), debug.clone()],
        )?;
        let mut link = OsString::from("--add-gnu-debuglink=");
        link.push(&debug);
        run(&objcopy, &["--strip-debug".into(), link, output.into()])?;
    }
    Ok(())
}

/// A file beside the program, named after it: `game.dSYM`, `game.debug`.
fn beside(output: &Path, suffix: &str) -> OsString {
    let mut path = output.as_os_str().to_owned();
    path.push(suffix);
    path
}

/// Runs a tool of the toolchain, saying what it said when it fails.
pub(crate) fn run(tool: &OsString, args: &[OsString]) -> Result<(), BuildError> {
    let result = Command::new(tool).args(args).output().map_err(|err| {
        BuildError::Link(format!("cannot run `{}`: {err}", tool.to_string_lossy()))
    })?;
    if !result.status.success() {
        return Err(BuildError::Link(format!(
            "`{}` failed:\n{}",
            tool.to_string_lossy(),
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the linker is told after the objects, in order: where to look
    /// for libraries, the libraries, the frameworks on an Apple target, and
    /// the maths library.
    #[test]
    fn link_arguments() {
        let args = link_args(
            &[PathBuf::from("/opt/lib")],
            &["sqlite3".to_string(), "z".to_string()],
            &["Cocoa".to_string()],
        );
        let args: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let mut expected = vec!["-L/opt/lib", "-lsqlite3", "-lz"];
        if cfg!(target_os = "linux") {
            expected.push("-lpthread");
        }
        if cfg!(target_vendor = "apple") {
            expected.extend(["-framework", "Cocoa"]);
        }
        // The maths are not added here: the prelude asks for them where it
        // needs them.
        assert_eq!(args, expected);
    }
}
