use std::ffi::OsString;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use clap::{Parser, Subcommand, ValueEnum};
use wip_lang::{BuildError, SourceFile, TempDir, Timings, parse_file, render};
use wip_syntax::{Diagnostic, Interner, ast, lex, token};

// The system allocator on macOS slows down when the compiler's threads
// allocate at once, and mimalloc is faster on one thread too.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(
    name = "wip",
    version = wip_lang::VERSION,
    about = "Compiler prototype for the Wip language"
)]
struct Cli {
    /// When to color diagnostics.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, global = true)]
    color: ColorChoice,

    /// How many threads to compile with [default: one per core].
    #[arg(long, global = true)]
    threads: Option<usize>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
enum Command {
    /// Print the tokens of a source file, one per line.
    Lex { file: PathBuf },
    /// Print the syntax tree of a source file.
    Parse { file: PathBuf },
    /// Report every error in a source file.
    Check {
        file: PathBuf,
        /// Check as another target compiles the program: `macos-arm64`,
        /// `macos-x86_64`, `linux-arm64`, `linux-x86_64`, `macos` or
        /// `linux` on this machine's processor, or `all`.
        /// It may be given more than once.
        #[arg(long, value_name = "TARGET")]
        target: Vec<String>,
    },
    /// Print the mid-level IR of every function in a program.
    Mir { file: PathBuf },
    /// Compile a source file to a native executable.
    Build {
        /// Optimised, for use rather than debugging.
        #[arg(long)]
        release: bool,
        /// The processor to build for: `baseline`, every one of the
        /// machine's kind that its systems still run on [the default];
        /// `native`, this machine alone; or a level, as `x86-64-v3` or
        /// `armv8.1`.
        #[arg(long, value_name = "CPU", default_value = "baseline", value_parser = wip_codegen::Cpu::parse)]
        cpu: wip_codegen::Cpu,
        /// What compiles the program: Cranelift for a debug build, and for
        /// a release build LLVM where there is a clang 15 or newer to
        /// compile with, Cranelift otherwise. `llvm` asks
        /// for LLVM, and only of a release build.
        #[arg(long, value_name = "BACKEND")]
        backend: Option<BackendArg>,
        file: PathBuf,
        /// Where to write the executable [default: the file name without
        /// `.wip`, in the current directory]
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// A directory to look for C headers in, as `cc -I` does. May be
        /// written more than once.
        #[arg(short = 'I', long = "include", value_name = "DIR")]
        includes: Vec<PathBuf>,
        /// A directory to look for C libraries in, as `cc -L` does. May be
        /// written more than once.
        #[arg(short = 'L', long = "library-path", value_name = "DIR")]
        library_paths: Vec<PathBuf>,
        /// What to write: a program, or a library C links against.
        #[arg(long, value_name = "WHAT", default_value = "program")]
        emit: EmitArg,
        /// Where to write the C declarations for what the library exports.
        #[arg(long, value_name = "FILE")]
        header: Option<PathBuf>,
        /// Print how long each phase took.
        #[arg(long)]
        time: bool,
    },
    /// Compile a program with its tests and run them.
    Test {
        /// Optimised, for use rather than debugging.
        #[arg(long)]
        release: bool,
        /// The processor to build for: `baseline`, every one of the
        /// machine's kind that its systems still run on [the default];
        /// `native`, this machine alone; or a level, as `x86-64-v3` or
        /// `armv8.1`.
        #[arg(long, value_name = "CPU", default_value = "baseline", value_parser = wip_codegen::Cpu::parse)]
        cpu: wip_codegen::Cpu,
        /// What compiles the program: Cranelift for a debug build, and for
        /// a release build LLVM where there is a clang 15 or newer to
        /// compile with, Cranelift otherwise. `llvm` asks
        /// for LLVM, and only of a release build.
        #[arg(long, value_name = "BACKEND")]
        backend: Option<BackendArg>,
        file: PathBuf,
        /// Run only the tests whose name holds this text.
        filter: Option<String>,
        /// A directory to look for C headers in, as `cc -I` does.
        #[arg(short = 'I', long = "include", value_name = "DIR")]
        includes: Vec<PathBuf>,
        /// A directory to look for C libraries in, as `cc -L` does.
        #[arg(short = 'L', long = "library-path", value_name = "DIR")]
        library_paths: Vec<PathBuf>,
    },
    /// Write a program's documentation as pages, or print one item of it:
    /// each `pub` item's declaration as written, and the
    /// `///` lines above it, with the packages the program depends on and
    /// the standard library beside it.
    Doc {
        /// The program to document, a file or a package's directory [default:
        /// the current directory]; or an item to print, `std::collections::Map`
        /// or `str::findLast`.
        target: Option<String>,
        /// Where to write the pages [default: `doc`, in the current
        /// directory].
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Document the standard library alone.
        #[arg(long, conflicts_with = "target")]
        std: bool,
    },
    /// Lay `.wip` files out in one way, from their syntax tree.
    /// Files named, and every `.wip` file under a
    /// directory named; with none, the current directory. `-` formats
    /// standard input to standard output, as an editor wants. A file whose
    /// leading comments include `// wip fmt: off` is left as written.
    Fmt {
        paths: Vec<PathBuf>,
        /// With `-`: the file standard input stands for, whose package
        /// says how it is laid out and whose name errors give.
        #[arg(long, value_name = "FILE")]
        stdin_path: Option<PathBuf>,
        /// Write nothing: list the files that would change, and fail if
        /// there are any.
        #[arg(long)]
        check: bool,
        /// The column a line should not pass [default: the package's
        /// `@format`, or 100].
        #[arg(long)]
        width: Option<usize>,
        /// Indent with tabs [the default].
        #[arg(long, conflicts_with = "spaces")]
        tabs: bool,
        /// Indent with this many spaces.
        #[arg(long, value_name = "N")]
        spaces: Option<usize>,
    },
    /// Serve an editor: the Language Server Protocol on standard input
    /// and output.
    Lsp,
    /// Write a Wip module from a C header, with clang: `wip bindgen
    /// header.h -o bindings.wip`. `wip bindgen --help`
    /// says what else it takes.
    #[command(disable_help_flag = true)]
    Bindgen {
        /// What the tool is given: the header, and its options.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Start the debugger on a program, with the scripts that show Wip's
    /// values as they are meant: lldb on a Mac, gdb
    /// elsewhere. Build the program first, as a debug build.
    Debug {
        /// The debugger to start [default: lldb on a Mac, gdb elsewhere].
        #[arg(long, value_enum)]
        debugger: Option<DebuggerArg>,
        /// Write the scripts out, and print where each is, for an editor's
        /// debugger to load; start nothing.
        #[arg(long, conflicts_with = "program")]
        scripts: bool,
        /// The program, as `wip build` wrote it.
        #[arg(required_unless_present = "scripts")]
        program: Option<PathBuf>,
        /// Arguments for the program.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Compile a source file and run it, exiting with its exit code.
    Run {
        /// Optimised, for use rather than debugging.
        #[arg(long)]
        release: bool,
        /// The processor to build for: `baseline`, every one of the
        /// machine's kind that its systems still run on [the default];
        /// `native`, this machine alone; or a level, as `x86-64-v3` or
        /// `armv8.1`.
        #[arg(long, value_name = "CPU", default_value = "baseline", value_parser = wip_codegen::Cpu::parse)]
        cpu: wip_codegen::Cpu,
        /// What compiles the program: Cranelift for a debug build, and for
        /// a release build LLVM where there is a clang 15 or newer to
        /// compile with, Cranelift otherwise. `llvm` asks
        /// for LLVM, and only of a release build.
        #[arg(long, value_name = "BACKEND")]
        backend: Option<BackendArg>,
        file: PathBuf,
        /// A directory to look for C headers in, as `cc -I` does.
        #[arg(short = 'I', long = "include", value_name = "DIR")]
        includes: Vec<PathBuf>,
        /// A directory to look for C libraries in, as `cc -L` does.
        #[arg(short = 'L', long = "library-path", value_name = "DIR")]
        library_paths: Vec<PathBuf>,
        /// Arguments for the program, which it sees as `args[1]` and on.
        /// Options for `wip` go before the file.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
}

/// What `wip debug --debugger` takes.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum DebuggerArg {
    Lldb,
    Gdb,
}

/// What `--backend` takes.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum BackendArg {
    Cranelift,
    Llvm,
}

impl From<BackendArg> for wip_lang::Backend {
    fn from(arg: BackendArg) -> wip_lang::Backend {
        match arg {
            BackendArg::Cranelift => wip_lang::Backend::Cranelift,
            BackendArg::Llvm => wip_lang::Backend::Llvm,
        }
    }
}

/// What `wip build --emit` takes.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum EmitArg {
    /// An executable, which needs a `main`.
    Program,
    /// An archive C links against: `libname.a`.
    Static,
    /// A shared library: `.dylib` on macOS, `.so` elsewhere.
    Dynamic,
}

impl From<EmitArg> for wip_lang::Emit {
    fn from(arg: EmitArg) -> wip_lang::Emit {
        match arg {
            EmitArg::Program => wip_lang::Emit::Program,
            EmitArg::Static => wip_lang::Emit::Static,
            EmitArg::Dynamic => wip_lang::Emit::Dynamic,
        }
    }
}

impl Command {
    fn file(&self) -> &PathBuf {
        match self {
            Command::Lex { file }
            | Command::Parse { file }
            | Command::Check { file, .. }
            | Command::Mir { file }
            | Command::Build { file, .. }
            | Command::Test { file, .. }
            | Command::Run { file, .. } => file,
            Command::Fmt { .. } => unreachable!("`fmt` takes paths"),
            Command::Doc { .. } => unreachable!("`doc` takes a program or an item"),
            Command::Lsp => unreachable!("`lsp` takes no file"),
            Command::Bindgen { .. } => unreachable!("`bindgen` hands its arguments on"),
            Command::Debug { .. } => unreachable!("`debug` takes a program"),
        }
    }
}

/// A debug build, or a release one where `--release` says.
fn profile_of(release: bool) -> wip_lang::Profile {
    match release {
        true => wip_lang::Profile::Release,
        false => wip_lang::Profile::Debug,
    }
}

/// The backend a build uses, as `--release` and `--backend` say: with a
/// note where a release build falls back to Cranelift, and `None`, said
/// why, where the one asked for cannot build.
fn backend_of(release: bool, asked: Option<BackendArg>) -> Option<wip_lang::Backend> {
    match wip_lang::choose_backend(profile_of(release), asked.map(Into::into)) {
        Ok((backend, note)) => {
            if let Some(note) = note {
                eprintln!("note: {note}");
            }
            Some(backend)
        }
        Err(BuildError::Link(message)) => {
            eprintln!("error: {message}");
            None
        }
        Err(BuildError::Diagnostics(_)) => unreachable!("choosing a backend reads no program"),
    }
}

/// How `wip fmt` was asked to lay files out, over what their packages say.
struct Layout {
    width: Option<usize>,
    tabs: bool,
    spaces: Option<usize>,
}

impl Layout {
    /// The style for `file`: its package's `@format`, then the command line.
    fn style(&self, file: &Path) -> Result<wip_fmt::Style, String> {
        let mut style = wip_lang::format_style(file)?;
        if let Some(width) = self.width {
            style.width = width;
        }
        if self.tabs {
            style.tabs = true;
        }
        if let Some(n) = self.spaces {
            style.tabs = false;
            style.size = n;
        }
        Ok(style)
    }
}

/// `text`, of `file`, laid out; `None` when it cannot be, with why shown.
/// A file that does not parse has its errors shown; one the formatter gets
/// wrong is reported as its bug.
fn format_text(file: &Path, text: &str, layout: &Layout, color: bool) -> Option<String> {
    let style = match layout.style(file) {
        Ok(style) => style,
        Err(message) => {
            eprintln!("error: {message}");
            return None;
        }
    };
    let package = file.file_name().is_some_and(|n| n == "package.wip");
    match wip_fmt::format(text, style, package) {
        Ok(formatted) => Some(formatted),
        Err(wip_fmt::Error::Parse(diagnostics)) => {
            let name = file.display().to_string();
            eprint!("{}", render::render_all(&diagnostics, &name, text, color));
            None
        }
        Err(wip_fmt::Error::Bug(what)) => {
            eprintln!(
                "error: `{}` was left as it was, since `wip fmt` got it wrong: {what}. This is a bug in `wip fmt`.",
                file.display()
            );
            None
        }
    }
}

/// `wip fmt -`: standard input laid out to standard
/// output, as `stdin_path`'s package says. What does not parse writes
/// nothing, so an editor keeps its buffer as it was.
fn format_stdin(stdin_path: Option<&Path>, layout: &Layout, color: bool) -> ExitCode {
    let mut text = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut text) {
        eprintln!("error: cannot read standard input: {err}");
        return ExitCode::from(1);
    }
    // A file that asks to be left as written is given back as it came.
    if wip_fmt::switched_off(&text) {
        print!("{text}");
        return ExitCode::SUCCESS;
    }
    let file = stdin_path.map_or_else(|| PathBuf::from("<stdin>"), Path::to_path_buf);
    match format_text(&file, &text, layout, color) {
        Some(formatted) => {
            print!("{formatted}");
            ExitCode::SUCCESS
        }
        None => ExitCode::from(1),
    }
}

/// `wip fmt`: each file laid out as its package says, or
/// as the command line does. A file that cannot be is left alone, and so is
/// one that asks to be, which is said rather than passed over.
fn format_files(paths: &[PathBuf], check: bool, layout: &Layout, color: bool) -> ExitCode {
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    };
    let mut files = Vec::new();
    for root in &roots {
        if root.is_dir() {
            wip_files(root, &mut files);
        } else {
            files.push(root.clone());
        }
    }
    files.sort();
    let (mut changed, mut failed) = (0, 0);
    let mut left = Vec::new();
    for file in &files {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("error: cannot read `{}`: {err}", file.display());
                failed += 1;
                continue;
            }
        };
        if wip_fmt::switched_off(&text) {
            left.push(file);
            continue;
        }
        match format_text(file, &text, layout, color) {
            Some(formatted) if formatted == text => {}
            Some(formatted) => {
                changed += 1;
                if check {
                    println!("{}", file.display());
                } else if let Err(err) = std::fs::write(file, formatted) {
                    eprintln!("error: cannot write `{}`: {err}", file.display());
                    failed += 1;
                }
            }
            None => failed += 1,
        }
    }
    match left.as_slice() {
        [] => {}
        [one] => eprintln!(
            "`{}` is left as written: it says `{}`",
            one.display(),
            wip_fmt::OFF
        ),
        many => eprintln!(
            "{} files are left as written: each says `{}`",
            many.len(),
            wip_fmt::OFF
        ),
    }
    if failed > 0 || (check && changed > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Every `.wip` file under `dir`, but not what is hidden or built.
fn wip_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.is_dir() {
            wip_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "wip") {
            out.push(path);
        }
    }
}

/// What a library is called when `-o` does not say: `libboard.a`,
/// `libboard.dylib`.
fn default_output(stem: &str, emit: wip_lang::Emit) -> String {
    match emit {
        wip_lang::Emit::Program => stem.to_string(),
        wip_lang::Emit::Static => format!("lib{stem}.a"),
        wip_lang::Emit::Dynamic => match std::env::consts::OS {
            "macos" | "ios" => format!("lib{stem}.dylib"),
            _ => format!("lib{stem}.so"),
        },
    }
}

/// Everything after the file of `wip run`, or the program of `wip debug`,
/// is the program's, `--` too. clap takes a `--` written right after the
/// file as the end of its own options and drops it, so it is put back: the
/// arguments as typed end with the program's, after the file and a `--`.
fn keep_the_separator(cli: &mut Cli) {
    let typed: Vec<OsString> = std::env::args_os().collect();
    let (target, args) = match &mut cli.command {
        Command::Run { file, args, .. } => (file.as_os_str().to_os_string(), args),
        Command::Debug {
            program: Some(program),
            args,
            ..
        } => (program.as_os_str().to_os_string(), args),
        _ => return,
    };
    let count = args.len();
    if typed.len() >= count + 2
        && typed[typed.len() - count..] == args[..]
        && typed[typed.len() - count - 1] == "--"
        && typed[typed.len() - count - 2] == target
    {
        args.insert(0, OsString::from("--"));
    }
}

fn main() -> ExitCode {
    let mut cli = Cli::parse();
    keep_the_separator(&mut cli);
    if let Some(threads) = cli.threads {
        wip_syntax::parallel::set_threads(threads);
    }
    let color = match cli.color {
        ColorChoice::Auto => {
            std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none()
        }
        ColorChoice::Always => true,
        ColorChoice::Never => false,
    };
    if let Command::Lsp = &cli.command {
        return wip_lang::lsp::serve();
    }
    // The binding generator is a Wip program `wip` carries, built the first
    // time it is asked for and run with what it was given.
    if let Command::Bindgen { args } = &cli.command {
        let exe = match wip_lang::tools::bindgen(color) {
            Ok(exe) => exe,
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::FAILURE;
            }
        };
        return match Process::new(&exe).args(args).status() {
            Ok(status) => match status.code() {
                Some(code) => ExitCode::from(code as u8),
                None => {
                    eprintln!("error: `wip bindgen` was terminated: {status}");
                    ExitCode::FAILURE
                }
            },
            Err(err) => {
                eprintln!("error: cannot run `wip bindgen`: {err}");
                ExitCode::FAILURE
            }
        };
    }
    if let Command::Debug {
        debugger,
        scripts,
        program,
        args,
    } = &cli.command
    {
        return debug(*debugger, *scripts, program.as_deref(), args);
    }
    if let Command::Fmt {
        paths,
        stdin_path,
        check,
        width,
        tabs,
        spaces,
    } = &cli.command
    {
        let layout = Layout {
            width: *width,
            tabs: *tabs,
            spaces: *spaces,
        };
        if paths.iter().any(|p| p.as_os_str() == "-") {
            if paths.len() > 1 || *check {
                eprintln!(
                    "error: `wip fmt -` formats standard input alone, and has nothing to check"
                );
                return ExitCode::from(2);
            }
            return format_stdin(stdin_path.as_deref(), &layout, color);
        }
        if stdin_path.is_some() {
            eprintln!(
                "error: `--stdin-path` names the file standard input stands for, and needs `-`"
            );
            return ExitCode::from(2);
        }
        return format_files(paths, *check, &layout, color);
    }
    if let Command::Doc {
        target,
        output,
        std,
    } = &cli.command
    {
        return doc(target.as_deref(), output.as_deref(), *std);
    }
    let path = cli.command.file().clone();

    // `lex` and `parse` look at one file; the rest compile the program that
    // file starts, following its imports.
    if let Command::Lex { .. } | Command::Parse { .. } = &cli.command {
        let source = match SourceFile::read(&path) {
            Ok(source) => source,
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::from(2);
            }
        };
        let diagnostics = match &cli.command {
            Command::Lex { .. } => {
                let mut interner = Interner::new();
                let lexed = lex(&source.text, &mut interner);
                print!("{}", token::dump(&source.text, &lexed.tokens));
                lexed.diagnostics
            }
            _ => {
                let parsed = parse_file(&source);
                print!("{}", ast::dump(&parsed.ast, &source.text, &parsed.interner));
                parsed.diagnostics
            }
        };
        eprint!(
            "{}",
            render::render_all(&diagnostics, &source.name, &source.text, color)
        );
        return errors(&diagnostics);
    }

    // Checking for other targets loads the program once for each.
    if let Command::Check { target, .. } = &cli.command
        && !target.is_empty()
    {
        return check_for_targets(&path, target, color);
    }

    // `wip test` compiles the `.test.wip` files too.
    let mode = match &cli.command {
        Command::Test { .. } => wip_lang::Mode::Tests,
        // `wip check` of a package checks every module of it, imported
        // or not.
        Command::Check { .. } => wip_lang::Mode::Package,
        _ => wip_lang::Mode::Program,
    };
    let mut timings = Timings::default();
    let mut loaded = match wip_lang::load_mode(&path, mode, &mut timings) {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    let report = |diagnostics: &[Diagnostic], sources: &wip_lang::Sources| -> ExitCode {
        eprint!("{}", render::render_program(diagnostics, sources, color));
        errors(diagnostics)
    };

    match &cli.command {
        Command::Check { .. } => {
            let checked = wip_lang::check_loaded(&mut loaded, &mut Timings::default());
            report(&checked.diagnostics, &loaded.sources)
        }
        Command::Mir { .. } => {
            let checked = wip_lang::check_loaded(&mut loaded, &mut Timings::default());
            if checked.diagnostics.iter().any(Diagnostic::is_error) {
                return report(&checked.diagnostics, &loaded.sources);
            }
            let program = &checked.program;
            for (id, def) in program.fns.iter() {
                let Some(body) = &def.body else { continue };
                if !program.is_compiled(id) {
                    continue;
                }
                let name = program.fn_name(id, &loaded.interner);
                let mut mir = wip_mir::lower_fn(program, &loaded.interner, def, body);
                // What is compiled is what is dumped, `@inline` and all.
                wip_mir::inline(program, &loaded.interner, &mut mir);
                wip_mir::convert_c_bools(program, id, &mut mir);
                wip_mir::simplify(program, &mut mir);
                print!("{}", wip_mir::dump(program, &loaded.interner, &name, &mir));
            }
            report(&checked.diagnostics, &loaded.sources)
        }
        Command::Build {
            output,
            includes,
            library_paths,
            emit,
            header,
            time,
            release,
            cpu,
            backend,
            ..
        } => {
            let Some(backend) = backend_of(*release, *backend) else {
                return ExitCode::FAILURE;
            };
            let emit = wip_lang::Emit::from(*emit);
            let output = output.clone().unwrap_or_else(|| {
                // A package is built as what it calls itself, into its own
                // root, where a monorepo has no directory of that name in
                // the way; a program of files, as its entry file, where
                // the command is typed.
                let named = path.is_dir() || path.file_name().is_some_and(|n| n == "package.wip");
                match (named, &loaded.packages[0].name) {
                    (true, Some(name)) => loaded.packages[0].dir.join(default_output(name, emit)),
                    _ => {
                        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                        PathBuf::from(default_output(&stem, emit))
                    }
                }
            });
            let options = wip_lang::BuildOptions {
                search: wip_lang::Search {
                    includes: includes.clone(),
                    library_paths: library_paths.clone(),
                },
                emit,
                header: header.clone(),
                profile: profile_of(*release),
                cpu: *cpu,
                backend: Some(backend),
            };
            let result = wip_lang::build_loaded(&mut loaded, &output, &options, &mut timings);
            if *time {
                eprint!("{timings}");
            }
            match result {
                Ok(warnings) => report(&warnings, &loaded.sources),
                Err(BuildError::Diagnostics(diagnostics)) => report(&diagnostics, &loaded.sources),
                Err(BuildError::Link(message)) => {
                    eprintln!("error: {message}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Run {
            args,
            includes,
            library_paths,
            release,
            cpu,
            backend,
            ..
        } => {
            let Some(backend) = backend_of(*release, *backend) else {
                return ExitCode::FAILURE;
            };
            let dir = match TempDir::new() {
                Ok(dir) => dir,
                Err(err) => {
                    eprintln!("error: cannot create a temporary directory: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let exe = dir.path().join("program");
            let options = wip_lang::BuildOptions {
                search: wip_lang::Search {
                    includes: includes.clone(),
                    library_paths: library_paths.clone(),
                },
                profile: profile_of(*release),
                cpu: *cpu,
                backend: Some(backend),
                ..wip_lang::BuildOptions::default()
            };
            match wip_lang::build_loaded(&mut loaded, &exe, &options, &mut Timings::default()) {
                Ok(warnings) => {
                    if report(&warnings, &loaded.sources) != ExitCode::SUCCESS {
                        return ExitCode::FAILURE;
                    }
                }
                Err(BuildError::Diagnostics(diagnostics)) => {
                    return report(&diagnostics, &loaded.sources);
                }
                Err(BuildError::Link(message)) => {
                    eprintln!("error: {message}");
                    return ExitCode::FAILURE;
                }
            }
            match Process::new(&exe).args(args).status() {
                Ok(status) => match status.code() {
                    Some(code) => ExitCode::from(code as u8),
                    None => {
                        eprintln!("error: the program was terminated: {status}");
                        ExitCode::FAILURE
                    }
                },
                Err(err) => {
                    eprintln!("error: cannot run the program: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Test {
            filter,
            includes,
            library_paths,
            release,
            cpu,
            backend,
            ..
        } => {
            let Some(backend) = backend_of(*release, *backend) else {
                return ExitCode::FAILURE;
            };
            let dir = match TempDir::new() {
                Ok(dir) => dir,
                Err(err) => {
                    eprintln!("error: cannot create a temporary directory: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let exe = dir.path().join("tests");
            let options = wip_lang::BuildOptions {
                search: wip_lang::Search {
                    includes: includes.clone(),
                    library_paths: library_paths.clone(),
                },
                profile: profile_of(*release),
                cpu: *cpu,
                backend: Some(backend),
                ..wip_lang::BuildOptions::default()
            };
            let built = wip_lang::build_tests(
                &mut loaded,
                &exe,
                &options,
                filter.as_deref(),
                &mut Timings::default(),
            );
            match built {
                Ok((warnings, count)) => {
                    if report(&warnings, &loaded.sources) != ExitCode::SUCCESS {
                        return ExitCode::FAILURE;
                    }
                    if count == 0 {
                        // Nothing to run is not a failure: a program may
                        // have no tests yet.
                        println!("running 0 tests");
                        return ExitCode::SUCCESS;
                    }
                }
                Err(BuildError::Diagnostics(diagnostics)) => {
                    return report(&diagnostics, &loaded.sources);
                }
                Err(BuildError::Link(message)) => {
                    eprintln!("error: {message}");
                    return ExitCode::FAILURE;
                }
            }
            // A package's tests run in its root, so that the paths they
            // name are its own wherever the command is typed; a program
            // without a `package.wip` runs them where it is typed.
            let mut tests = Process::new(&exe);
            let root = &loaded.packages[0];
            if root.name.is_some() && !root.dir.as_os_str().is_empty() {
                tests.current_dir(&root.dir);
            }
            match tests.status() {
                Ok(status) => match status.code() {
                    Some(code) => ExitCode::from(code as u8),
                    None => {
                        eprintln!("error: the tests were terminated: {status}");
                        ExitCode::FAILURE
                    }
                },
                Err(err) => {
                    eprintln!("error: cannot run the tests: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Lex { .. }
        | Command::Parse { .. }
        | Command::Fmt { .. }
        | Command::Doc { .. }
        | Command::Lsp
        | Command::Bindgen { .. }
        | Command::Debug { .. } => {
            unreachable!("handled above")
        }
    }
}

/// `wip debug`: the debugger on `program`, with the scripts loaded, or,
/// with `--scripts`, where the scripts are.
fn debug(
    debugger: Option<DebuggerArg>,
    scripts: bool,
    program: Option<&Path>,
    args: &[OsString],
) -> ExitCode {
    use wip_lang::debugger::{self, Debugger};
    if scripts {
        return match debugger::scripts() {
            Ok(scripts) => {
                println!("lldb: {}", scripts.lldb.display());
                println!("gdb: {}", scripts.gdb.display());
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        };
    }
    let program = program.expect("clap asks for a program without `--scripts`");
    let debugger = match debugger {
        Some(DebuggerArg::Lldb) => Debugger::Lldb,
        Some(DebuggerArg::Gdb) => Debugger::Gdb,
        None => Debugger::native(),
    };
    let mut command = match debugger::command(debugger, program, args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    match command.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(err) => {
            let name = command.get_program().to_string_lossy().into_owned();
            eprintln!("error: cannot run `{name}`: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The exit code for a run that reported `diagnostics`, with a count.
/// `wip check --target`: the program checked as each target
/// compiles it. A diagnostic several targets report is shown once, and one
/// that not every target reports says which did.
fn check_for_targets(path: &Path, names: &[String], color: bool) -> ExitCode {
    let mut targets: Vec<wip_lang::targets::Target> = Vec::new();
    for name in names {
        match wip_lang::targets::Target::parse(name) {
            Ok(parsed) => {
                for target in parsed {
                    if !targets.contains(&target) {
                        targets.push(target);
                    }
                }
            }
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::from(2);
            }
        }
    }
    // Each diagnostic once, by what it says, with the first target that
    // reported it, for its sources, and every target that did.
    struct Found {
        said: String,
        diagnostic: Diagnostic,
        first: usize,
        targets: Vec<String>,
    }
    let mut programs = Vec::new();
    let mut found: Vec<Found> = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        let mut loaded = match wip_lang::load_for(
            path,
            wip_lang::Mode::Package,
            *target,
            &mut Timings::default(),
        ) {
            Ok(loaded) => loaded,
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::from(2);
            }
        };
        let checked = wip_lang::check_loaded(&mut loaded, &mut Timings::default());
        for diagnostic in checked.diagnostics {
            let said =
                render::render_program(std::slice::from_ref(&diagnostic), &loaded.sources, false);
            match found.iter_mut().find(|f| f.said == said) {
                Some(f) => f.targets.push(target.name()),
                None => found.push(Found {
                    said,
                    diagnostic,
                    first: index,
                    targets: vec![target.name()],
                }),
            }
        }
        programs.push(loaded);
    }
    let mut shown = Vec::new();
    for f in found {
        let mut diagnostic = f.diagnostic;
        if f.targets.len() < targets.len() {
            diagnostic
                .notes
                .push(format!("found when checking for {}", f.targets.join(", ")));
        }
        eprint!(
            "{}",
            render::render_program(
                std::slice::from_ref(&diagnostic),
                &programs[f.first].sources,
                color
            )
        );
        shown.push(diagnostic);
    }
    errors(&shown)
}

fn errors(diagnostics: &[Diagnostic]) -> ExitCode {
    match diagnostics.iter().filter(|d| d.is_error()).count() {
        0 => ExitCode::SUCCESS,
        1 => {
            eprintln!("1 error");
            ExitCode::FAILURE
        }
        n => {
            eprintln!("{n} errors");
            ExitCode::FAILURE
        }
    }
}

/// `wip doc`: an item printed, where the target is a path
/// of names rather than a file — looked up in the program here, or in the
/// standard library where there is none — and otherwise the pages written.
fn doc(target: Option<&str>, output: Option<&Path>, std: bool) -> ExitCode {
    let names_an_item = target.is_some_and(|t| t.contains("::") || !Path::new(t).exists());
    if let Some(item) = target.filter(|_| names_an_item) {
        let docs = wip_lang::doc::read_for_lookup(Path::new("."));
        return match wip_lang::doc::print_item(&docs, item) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::from(1)
            }
        };
    }
    let entry = PathBuf::from(target.unwrap_or("."));
    let docs = match std {
        true => wip_lang::doc::read(None),
        false => wip_lang::doc::read(Some(&entry)),
    };
    let docs = match docs {
        Ok(docs) => docs,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    let dir = output.map_or_else(|| PathBuf::from("doc"), Path::to_path_buf);
    match wip_lang::doc::write_site(&docs, &dir) {
        Ok(pages) => {
            println!("wrote {pages} pages: {}", dir.join("index.html").display());
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}
