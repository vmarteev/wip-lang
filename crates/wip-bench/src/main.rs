//! `wip-bench`: writes synthetic programs and times compiling them with `wip`
//! and with `clang -O0`. Run it from the repository
//! root with a release build:
//!
//! ```text
//! cargo run --release -p wip-bench -- measure
//! cargo run --release -p wip-bench -- modules
//! cargo run --release -p wip-bench -- speed
//! ```
//!
//! `speed` is the floor under the LLVM backend's speed: it
//! times what a release build makes against the times kept beside this
//! crate, and fails where one is slower than the noise explains.

use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use wip_lang::{SourceFile, Timings};

// The system allocator on macOS slows down when the compiler's threads
// allocate at once, and mimalloc is faster on one thread too.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(name = "wip-bench", about = "Compile-time measurements for question 4")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a program of about `lines` lines as `gen_<lines>.wip`, and the
    /// same program as `gen_<lines>.c`.
    Gen {
        #[arg(long, default_value_t = 10_000)]
        lines: usize,
        #[arg(long, default_value = "tests/bench")]
        out: PathBuf,
    },
    /// Generate a program of each size and time compiling it.
    Measure {
        /// Sizes, in lines of Wip.
        #[arg(long, value_delimiter = ',', default_value = "10000,100000,1000000")]
        sizes: Vec<usize>,
        /// Runs of each measurement; the median is reported.
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[arg(long, default_value = "tests/bench")]
        out: PathBuf,
        /// The C compiler to compare with.
        #[arg(long, default_value = "clang")]
        cc: String,
    },
    /// Build one program with wip and with clang, and time the programs
    /// themselves: how good the generated code is, rather than how fast it
    /// was generated.
    Runtime {
        #[arg(long, default_value_t = 2_000)]
        lines: usize,
        /// How many times the program repeats its whole computation.
        #[arg(long, default_value_t = 400)]
        repeat: usize,
        /// Runs of each program; the median is reported.
        #[arg(long, default_value_t = 5)]
        runs: usize,
        #[arg(long, default_value = "tests/bench")]
        out: PathBuf,
        #[arg(long, default_value = "clang")]
        cc: String,
    },
    /// Build clox and a generated program with LLVM, time them, and compare
    /// each time with the ones kept in `speed.txt` beside this crate, failing
    /// where one is slower by more than `margin` percent. Times kept on another
    /// machine are shown, not compared.
    Speed {
        /// The clox port, whose release build runs the book's benchmarks.
        #[arg(long, default_value = "../clox-wip")]
        clox: PathBuf,
        /// Crafting Interpreters' repository, which holds the benchmarks.
        #[arg(long, default_value = "../craftinginterpreters")]
        book: PathBuf,
        /// Runs of each program; the fastest is kept, as the least
        /// disturbed by what else the machine does.
        #[arg(long, default_value_t = 3)]
        runs: usize,
        /// How much slower than the kept time is not yet slower, in
        /// percent: what two runs of the same program differ by.
        #[arg(long, default_value_t = 10.0)]
        margin: f64,
        /// Keep the times found, in place of the ones kept.
        #[arg(long)]
        keep: bool,
        #[arg(long, default_value = "tests/bench")]
        out: PathBuf,
    },
    /// Spread one program over more and more modules, and time the code
    /// generation that compiles them in parallel.
    Modules {
        #[arg(long, default_value_t = 1_000_000)]
        lines: usize,
        /// How many modules to spread the program over.
        #[arg(long, value_delimiter = ',', default_value = "1,2,4,8,12")]
        counts: Vec<usize>,
        /// How many threads to compile with; 0 is one per core.
        #[arg(long, value_delimiter = ',', default_value = "0")]
        threads: Vec<usize>,
        /// Runs of each measurement; the median is reported.
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[arg(long, default_value = "tests/bench")]
        out: PathBuf,
    },
    /// Time code generation for functions whose deferred code is emitted at
    /// every exit, against the same code written as ordinary statements.
    Defers {
        #[arg(long, default_value_t = 2_000)]
        functions: usize,
        #[arg(long, default_value_t = 4)]
        defers: usize,
        #[arg(long, default_value_t = 4)]
        returns: usize,
        /// Runs of each measurement; the median is reported.
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[arg(long, default_value = "tests/bench")]
        out: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Gen { lines, out } => write(lines, &out).map(|written| {
            println!(
                "wrote {} ({} lines) and {} ({} lines)",
                written.wip.display(),
                written.wip_lines,
                written.c.display(),
                written.c_lines
            );
        }),
        Command::Measure {
            sizes,
            runs,
            out,
            cc,
        } => measure(&sizes, runs.max(1), &out, &cc),
        Command::Runtime {
            lines,
            repeat,
            runs,
            out,
            cc,
        } => measure_runtime(lines, repeat, runs.max(1), &out, &cc),
        Command::Modules {
            lines,
            counts,
            threads,
            runs,
            out,
        } => measure_modules(lines, &counts, &threads, runs.max(1), &out),
        Command::Defers {
            functions,
            defers,
            returns,
            runs,
            out,
        } => measure_defers(functions, defers, returns, runs.max(1), &out),
        Command::Speed {
            clox,
            book,
            runs,
            margin,
            keep,
            out,
        } => speed(&clox, &book, runs.max(1), margin, keep, &out),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

struct Written {
    wip: PathBuf,
    c: PathBuf,
    wip_lines: usize,
    c_lines: usize,
}

fn write(lines: usize, out: &Path) -> Result<Written, String> {
    std::fs::create_dir_all(out)
        .map_err(|err| format!("cannot create `{}`: {err}", out.display()))?;
    let program = wip_bench::generate(lines);
    let wip = out.join(format!("gen_{lines}.wip"));
    let c = out.join(format!("gen_{lines}.c"));
    for (path, text) in [(&wip, &program.wip), (&c, &program.c)] {
        std::fs::write(path, text)
            .map_err(|err| format!("cannot write `{}`: {err}", path.display()))?;
    }
    Ok(Written {
        wip,
        c,
        wip_lines: program.wip_lines,
        c_lines: program.c_lines,
    })
}

/// The measurements for one size.
struct Row {
    wip_lines: usize,
    c_lines: usize,
    /// The run of `wip` with the median total.
    wip: Timings,
    clang_front_end: Duration,
    clang_object: Duration,
}

/// The phases before code generation.
const FRONT_END: [&str; 5] = ["read", "lex", "parse", "type check", "move check"];

fn measure(sizes: &[usize], runs: usize, out: &Path, cc: &str) -> Result<(), String> {
    // Against clang, which compiles a file on one thread, wip uses one too
    // (answer 4); `modules` measures what more threads buy.
    wip_syntax::parallel::set_threads(1);
    let mut rows = Vec::new();
    for &size in sizes {
        eprintln!("measuring {size} lines");
        let written = write(size, out)?;
        let exe = out.join(format!("gen_{size}"));
        let mut wip_runs = (0..runs)
            .map(|_| time_wip(&written.wip, &exe))
            .collect::<Result<Vec<_>, _>>()?;
        wip_runs.sort_by_key(Timings::total);
        let wip = wip_runs.swap_remove(runs / 2);

        let object = out.join(format!("gen_{size}.o"));
        let clang_front_end = median(runs, || {
            time_command(Process::new(cc).arg("-fsyntax-only").arg(&written.c))
        })?;
        let clang_object = median(runs, || {
            time_command(
                Process::new(cc)
                    .args(["-O0", "-c", "-o"])
                    .arg(&object)
                    .arg(&written.c),
            )
        })?;
        rows.push(Row {
            wip_lines: written.wip_lines,
            c_lines: written.c_lines,
            wip,
            clang_front_end,
            clang_object,
        });
    }
    report(&rows);
    Ok(())
}

/// Builds one program three ways and times the programs themselves.
fn measure_runtime(
    lines: usize,
    repeat: usize,
    runs: usize,
    out: &Path,
    cc: &str,
) -> Result<(), String> {
    std::fs::create_dir_all(out)
        .map_err(|err| format!("cannot create `{}`: {err}", out.display()))?;
    let program = wip_bench::generate_repeated(lines, repeat);
    let wip_path = out.join("run.wip");
    let c_path = out.join("run.c");
    std::fs::write(&wip_path, &program.wip).map_err(|err| format!("cannot write: {err}"))?;
    std::fs::write(&c_path, &program.c).map_err(|err| format!("cannot write: {err}"))?;

    let wip_exe = out.join("run_wip");
    let source = SourceFile {
        name: wip_path.display().to_string(),
        text: program.wip.clone(),
    };
    wip_lang::build(&source, &wip_exe)
        .map_err(|_| format!("`{}` does not compile", wip_path.display()))?;

    let mut builds = vec![("wip (Cranelift)".to_string(), wip_exe)];
    for level in ["-O0", "-O2"] {
        let exe = out.join(format!("run_clang{level}"));
        time_command(
            Process::new(cc)
                .arg(level)
                .arg("-o")
                .arg(&exe)
                .arg(&c_path)
                .arg("-lm"),
        )?;
        builds.push((format!("clang {level}"), exe));
    }

    // Every build must print the same numbers; otherwise one of the code
    // generators is wrong and the timings mean nothing.
    let mut expected: Option<String> = None;
    let mut times = Vec::new();
    for (name, exe) in &builds {
        let output = Process::new(exe)
            .output()
            .map_err(|err| format!("cannot run `{}`: {err}", exe.display()))?;
        let printed = String::from_utf8_lossy(&output.stdout).to_string();
        match &expected {
            None => expected = Some(printed),
            Some(first) if *first != printed => {
                return Err(format!(
                    "`{name}` printed {printed:?}, but the first build printed {first:?}"
                ));
            }
            Some(_) => {}
        }
        let median = median(runs, || time_command(&mut Process::new(exe)))?;
        times.push((name.clone(), median));
    }

    println!(
        "{} lines of Wip, the whole program run {repeat} times; median of {runs}\n",
        program.wip_lines
    );
    println!("| build | run time (ms) | against wip |");
    println!("|---|---:|---:|");
    let base = times[0].1.as_secs_f64();
    for (name, time) in &times {
        println!(
            "| {name} | {} | {:.2}× |",
            ms(*time),
            time.as_secs_f64() / base
        );
    }
    Ok(())
}

/// The book's benchmarks that clox runs: `string_equality.lox` holds more
/// constants than a chunk does, in clox too.
const CLOX_BENCHMARKS: [&str; 9] = [
    "binary_trees",
    "equality",
    "fib",
    "instantiation",
    "invocation",
    "method_call",
    "properties",
    "trees",
    "zoo",
];

/// The floor under the LLVM backend's speed.
fn speed(
    clox: &Path,
    book: &Path,
    runs: usize,
    margin: f64,
    keep: bool,
    out: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(out)
        .map_err(|err| format!("cannot create `{}`: {err}", out.display()))?;
    let release = |entry: &Path, exe: &Path| -> Result<(), String> {
        let mut loaded = wip_lang::load(entry)?;
        let options = wip_lang::BuildOptions {
            profile: wip_lang::Profile::Release,
            backend: Some(wip_lang::Backend::Llvm),
            ..wip_lang::BuildOptions::default()
        };
        wip_lang::build_loaded(&mut loaded, exe, &options, &mut Timings::default())
            .map(|_| ())
            .map_err(|err| match err {
                wip_lang::BuildError::Link(message) => message,
                wip_lang::BuildError::Diagnostics(_) => {
                    format!("`{}` does not compile", entry.display())
                }
            })
    };
    let fastest = |command: &mut dyn FnMut() -> Process| -> Result<Duration, String> {
        (0..runs)
            .map(|_| time_command(&mut command()))
            .collect::<Result<Vec<_>, _>>()
            .map(|times| times.into_iter().min().expect("one run at least"))
    };

    let mut found: Vec<(String, Duration)> = Vec::new();
    // A program of this repository's own, so that there is a measure
    // without the ports beside it.
    // A program is its entry's directory: this one has a
    // directory to itself.
    let program = wip_bench::generate_repeated(2_000, 1_000_000);
    let dir = out.join("speed");
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("cannot create `{}`: {err}", dir.display()))?;
    let generated = dir.join("main.wip");
    std::fs::write(&generated, &program.wip).map_err(|err| format!("cannot write: {err}"))?;
    let exe = dir.join("speed");
    release(&generated, &exe)?;
    found.push((
        "generated".to_string(),
        fastest(&mut || Process::new(&exe))?,
    ));
    let benchmarks = book.join("test").join("benchmark");
    if clox.join("main.wip").is_file() && benchmarks.is_dir() {
        let exe = out.join("clox");
        release(&clox.join("main.wip"), &exe)?;
        for name in CLOX_BENCHMARKS {
            let script = benchmarks.join(format!("{name}.lox"));
            let time = fastest(&mut || {
                let mut command = Process::new(&exe);
                command.arg(&script);
                command
            })?;
            found.push((format!("clox/{name}"), time));
        }
    } else {
        println!(
            "clox is not measured: `{}` and `{}` are not there",
            clox.display(),
            benchmarks.display()
        );
    }

    let kept_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("speed.txt");
    let here = machine();
    if keep {
        let mut text = String::from(
            "# The LLVM backend's speed, kept by `wip-bench speed --keep`: the fastest\n\
             # of the runs, in seconds, on the machine named.\n",
        );
        text.push_str(&format!("machine {here}\n"));
        for (name, time) in &found {
            text.push_str(&format!("{name} {:.3}\n", time.as_secs_f64()));
        }
        std::fs::write(&kept_path, text)
            .map_err(|err| format!("cannot write `{}`: {err}", kept_path.display()))?;
        println!("kept in {}", kept_path.display());
    }
    let kept_text = std::fs::read_to_string(&kept_path).unwrap_or_default();
    let mut kept_machine = String::new();
    let mut kept: Vec<(String, f64)> = Vec::new();
    for line in kept_text.lines().filter(|line| !line.starts_with('#')) {
        if let Some(machine) = line.strip_prefix("machine ") {
            kept_machine = machine.to_string();
        } else if let Some((name, seconds)) = line.rsplit_once(' ')
            && let Ok(seconds) = seconds.parse()
        {
            kept.push((name.to_string(), seconds));
        }
    }
    let compared = kept_machine == here;
    println!("| program | kept (s) | now (s) | change |");
    println!("|---|---:|---:|---:|");
    let mut slower = Vec::new();
    for (name, time) in &found {
        let now = time.as_secs_f64();
        match kept.iter().find(|(kept_name, _)| kept_name == name) {
            Some(&(_, before)) => {
                let change = (now - before) / before * 100.0;
                println!("| {name} | {before:.3} | {now:.3} | {change:+.1}% |");
                if change > margin {
                    slower.push(name.clone());
                }
            }
            None => println!("| {name} | | {now:.3} | |"),
        }
    }
    if !compared {
        println!(
            "\nthe times kept are from `{kept_machine}`, not this machine (`{here}`): \
             `--keep` keeps this one's"
        );
        return Ok(());
    }
    match slower.is_empty() {
        true => Ok(()),
        false => Err(format!(
            "slower than kept by more than {margin}%: {}",
            slower.join(", ")
        )),
    }
}

/// What the machine is, as far as its times are comparable: its
/// processor, as the system names it.
fn machine() -> String {
    let named = match std::env::consts::OS {
        "macos" => Process::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string()),
        _ => std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|info| {
                info.lines()
                    .find(|line| line.starts_with("model name") || line.starts_with("CPU part"))
                    .and_then(|line| line.split(':').nth(1))
                    .map(|name| name.trim().to_string())
            }),
    };
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    format!(
        "{} ({}, {cores} cores)",
        named.unwrap_or_else(|| "unknown".to_string()),
        std::env::consts::ARCH
    )
}

/// Each phase of compiling one program spread over each number of modules,
/// on each number of threads.
fn measure_modules(
    lines: usize,
    counts: &[usize],
    threads: &[usize],
    runs: usize,
    out: &Path,
) -> Result<(), String> {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("{lines} lines of Wip, {cores} cores; median of {runs}\n");
    println!(
        "| modules | threads | read, lex, parse (ms) | type check (ms) | move check (ms) | codegen (ms) | to `.o` (ms) | link (ms) |"
    );
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|");
    for &count in counts {
        let root = out.join(format!("modules_{count}"));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in wip_bench::generate_modules(lines, count) {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().expect("a file has a directory"))
                .map_err(|err| format!("cannot create `{}`: {err}", root.display()))?;
            std::fs::write(&file, text)
                .map_err(|err| format!("cannot write `{}`: {err}", file.display()))?;
        }
        let exe = root.join("program");
        for &n in threads {
            wip_syntax::parallel::set_threads(n);
            let mut timings = Vec::new();
            for _ in 0..runs {
                let mut t = Timings::default();
                let entry = root.join("main.wip");
                let mut loaded = wip_lang::load_mode(&entry, wip_lang::Mode::Program, &mut t)?;
                wip_lang::build_loaded(
                    &mut loaded,
                    &exe,
                    &wip_lang::BuildOptions::default(),
                    &mut t,
                )
                .map_err(|_| format!("the {count}-module program does not compile"))?;
                timings.push(t);
            }
            timings.sort_by_key(|t| t.total() - t.get("link"));
            let t = &timings[runs / 2];
            let load: Duration = ["read", "lex", "parse"].iter().map(|p| t.get(p)).sum();
            println!(
                "| {count} | {} | {} | {} | {} | {} | {} | {} |",
                wip_syntax::parallel::threads(),
                ms(load),
                ms(t.get("type check")),
                ms(t.get("move check")),
                ms(t.get("codegen")),
                ms(t.total() - t.get("link")),
                ms(t.get("link"))
            );
        }
    }
    wip_syntax::parallel::set_threads(0);
    Ok(())
}

fn measure_defers(
    functions: usize,
    defers: usize,
    returns: usize,
    runs: usize,
    out: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(out)
        .map_err(|err| format!("cannot create `{}`: {err}", out.display()))?;
    println!("{functions} functions, each with {defers} assignments and {returns} early returns\n");
    println!("| assignments written as | codegen (ms) | executable (bytes) |");
    println!("|---|---:|---:|");
    for deferred in [false, true] {
        let name = if deferred { "defers" } else { "statements" };
        let path = out.join(format!("defers_{name}.wip"));
        let text = wip_bench::generate_defers(functions, defers, returns, deferred);
        std::fs::write(&path, text)
            .map_err(|err| format!("cannot write `{}`: {err}", path.display()))?;
        let exe = out.join(format!("defers_{name}"));
        let mut codegen = (0..runs)
            .map(|_| time_wip(&path, &exe).map(|timings| timings.get("codegen")))
            .collect::<Result<Vec<_>, _>>()?;
        codegen.sort();
        let size = std::fs::metadata(&exe)
            .map_err(|err| format!("cannot read `{}`: {err}", exe.display()))?
            .len();
        let label = if deferred {
            "`defer`, emitted at each exit"
        } else {
            "ordinary statements"
        };
        println!("| {label} | {} | {size} |", ms(codegen[runs / 2]));
    }
    Ok(())
}

fn time_wip(path: &Path, exe: &Path) -> Result<Timings, String> {
    let mut timings = Timings::default();
    let source = timings.time("read", || SourceFile::read(path))?;
    let mut loaded = wip_lang::load_source(&source, &mut timings)?;
    let options = wip_lang::BuildOptions::default();
    wip_lang::build_loaded(&mut loaded, exe, &options, &mut timings).map_err(|_| {
        format!(
            "`{}` does not compile; `wip build` on it shows why",
            path.display()
        )
    })?;
    Ok(timings)
}

fn time_command(command: &mut Process) -> Result<Duration, String> {
    let start = Instant::now();
    let output = command
        .output()
        .map_err(|err| format!("cannot run {command:?}: {err}"))?;
    let elapsed = start.elapsed();
    if !output.status.success() {
        return Err(format!(
            "{command:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(elapsed)
}

fn median(
    runs: usize,
    mut measure: impl FnMut() -> Result<Duration, String>,
) -> Result<Duration, String> {
    let mut times = (0..runs)
        .map(|_| measure())
        .collect::<Result<Vec<_>, _>>()?;
    times.sort();
    Ok(times[runs / 2])
}

fn ms(time: Duration) -> String {
    format!("{:.1}", time.as_secs_f64() * 1000.0)
}

/// Prints the results as Markdown tables.
fn report(rows: &[Row]) {
    println!("## Summary (ms)\n");
    println!(
        "| Wip lines | wip front end | wip codegen | wip to `.o` | clang front end | clang `-O0 -c` | LLVM back end | link |"
    );
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|");
    for row in rows {
        let front_end: Duration = FRONT_END.iter().map(|p| row.wip.get(p)).sum();
        let codegen = row.wip.get("codegen");
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            row.wip_lines,
            ms(front_end),
            ms(codegen),
            ms(front_end + codegen),
            ms(row.clang_front_end),
            ms(row.clang_object),
            ms(row.clang_object.saturating_sub(row.clang_front_end)),
            ms(row.wip.get("link")),
        );
    }

    println!("\n## Throughput (lines per second, to an object file)\n");
    println!("| Wip lines | wip | C lines | clang `-O0 -c` |");
    println!("|---:|---:|---:|---:|");
    for row in rows {
        let front_end: Duration = FRONT_END.iter().map(|p| row.wip.get(p)).sum();
        let wip = front_end + row.wip.get("codegen");
        let rate = |lines: usize, time: Duration| (lines as f64 / time.as_secs_f64()).round();
        println!(
            "| {} | {} | {} | {} |",
            row.wip_lines,
            rate(row.wip_lines, wip),
            row.c_lines,
            rate(row.c_lines, row.clang_object),
        );
    }

    println!("\n## wip phases (ms)\n");
    print!("| phase |");
    for row in rows {
        print!(" {} |", row.wip_lines);
    }
    println!();
    println!("|---|{}", "---:|".repeat(rows.len()));
    if let Some(first) = rows.first() {
        for (phase, _) in &first.wip.phases {
            print!("| {phase} |");
            for row in rows {
                print!(" {} |", ms(row.wip.get(phase)));
            }
            println!();
        }
    }
}
