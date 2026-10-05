//! The `wip` command as people type it.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use wip_lang::{Sources, TempDir, render};
use wip_syntax::{Diagnostic, Span};

fn wip(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("`wip` runs")
}

/// The same, with `input` on the program's standard input.
fn wip_reading(dir: &Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("`wip` runs");
    child
        .stdin
        .take()
        .expect("a pipe")
        .write_all(input.as_bytes())
        .expect("written");
    child.wait_with_output().expect("`wip` finishes")
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("a directory")).expect("created");
    std::fs::write(path, text).expect("written");
}

/// A program of Wip alone compiles no C: the runtime is Wip, and `cc` is
/// asked once, to link. The C compiler here is a script
/// that writes down how it was called and then calls the real one.
#[cfg(unix)]
#[test]
fn a_program_of_wip_alone_compiles_no_c() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::fs\nimport std::future::{Future}\nimport std::io\nimport std::process::{Command}\n\n\
         fn main() = {\n    \
             val answer = Future::run(own () => 6 * 7).get()\n    \
             io::println(\"\\(answer) \\(fs::exists(\"main.wip\"))\")\n    \
             val said = Command::of(\"echo\").arg(\"hello\").output()\n    \
             if said is .Ok(..) {\n        io::println(\"spawned\")\n    }\n\
         }\n",
    );
    let real = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let log = dir.path().join("calls.txt");
    let script = dir.path().join("cc.sh");
    write(
        &script,
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nexec {real} \"$@\"\n",
            log.display()
        ),
    );
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("made runnable");

    let ran = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["run", "main.wip"])
        .env("CC", &script)
        .env("WIP_CACHE_DIR", dir.path().join("cache"))
        .env("WIP_CHECK_LEAKS", "1")
        .output()
        .expect("`wip` runs");
    assert!(ran.status.success(), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "42 true\nspawned\n");
    let calls = std::fs::read_to_string(&log).expect("the compiler was called");
    let calls: Vec<&str> = calls.lines().collect();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].starts_with("-o "), "{calls:?}");
}

/// A debug build compiles a module's C unoptimised and with debug
/// information, and a release build optimised; `--release` is taken by
/// `build`, `run` and `test`, and the program does the same built either
/// way.
#[cfg(unix)]
#[test]
fn a_release_build_optimizes_and_does_the_same() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\nimport twice\n\nfn main() = io::println(\"\\(twice::twice(21))\")\n",
    );
    write(
        &dir.path().join("twice/twice.wip"),
        "extern \"C\" {\n    pub fn twice(n: i64): i64\n}\n",
    );
    write(
        &dir.path().join("twice/twice.c"),
        "#include <stdint.h>\nint64_t twice(int64_t n) { return n * 2; }\n",
    );
    write(
        &dir.path().join("main.test.wip"),
        "@test\nfn itDoubles() = assert(twice::twice(4) == 8)\n",
    );
    let real = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let log = dir.path().join("calls.txt");
    let script = dir.path().join("cc.sh");
    write(
        &script,
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nexec {real} \"$@\"\n",
            log.display()
        ),
    );
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("made runnable");
    let wip_with_cc = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wip"))
            .current_dir(dir.path())
            .args(args)
            .env("CC", &script)
            .env("WIP_CACHE_DIR", dir.path().join("cache"))
            .output()
            .expect("`wip` runs")
    };
    // The C each build compiled, and how.
    let compiled = |log: &Path| -> String {
        let calls = std::fs::read_to_string(log).expect("the compiler was called");
        std::fs::remove_file(log).expect("removed");
        calls
            .lines()
            .filter(|call| call.contains("twice.c"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let debug = wip_with_cc(&["run", "main.wip"]);
    assert!(debug.status.success(), "{debug:?}");
    assert_eq!(String::from_utf8_lossy(&debug.stdout), "42\n");
    let calls = compiled(&log);
    assert!(calls.contains("-O0") && calls.contains("-g"), "{calls}");

    let release = wip_with_cc(&["run", "--release", "main.wip"]);
    assert!(release.status.success(), "{release:?}");
    assert_eq!(String::from_utf8_lossy(&release.stdout), "42\n");
    let calls = compiled(&log);
    assert!(calls.contains("-O2") && calls.contains("-g"), "{calls}");

    let built = wip_with_cc(&["build", "--release", "main.wip", "-o", "doubled"]);
    assert!(built.status.success(), "{built:?}");
    let ran = Command::new(dir.path().join("doubled"))
        .output()
        .expect("it runs");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "42\n");

    let tested = wip_with_cc(&["test", "--release", "main.wip"]);
    let stdout = String::from_utf8_lossy(&tested.stdout);
    assert!(
        tested.status.success() && stdout.contains("1 test passed"),
        "{stdout}"
    );
}

/// A build carries the lines its code was written at, and each function's name
/// and place: the debugger finds `main.wip:8` in `total`, and says where
/// `total` is declared. A debug build describes the variables in scope there
/// too, each in its slot. A release build keeps its lines beside the program;
/// Cranelift's here, which keeps `total` as it was written, where LLVM's adds
/// the numbers as it compiles. `greet`'s text comes before the code in the
/// object, so that an address which forgets where the code starts is caught.
/// The debugger is asked without running the program, which a Mac lets only a
/// developer do.
#[test]
fn a_build_tells_the_debugger_its_lines() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\nfn greet(): str = \"hi\"\n\nfn total(values: &[i64]): i64 = {\n\tvar sum = 0\n\tfor value in values {\n\t\tsum += value\n\t}\n\treturn sum\n}\n\nfn main() = {\n\tval xs = [1, 2, 3]\n\tio::println(greet())\n\tio::println(\"\\(total(&xs))\")\n}\n",
    );
    let built = wip(dir.path(), &["build", "main.wip"]);
    assert!(built.status.success(), "{built:?}");
    tells_the_debugger(dir.path(), "main", true);

    let released = wip(
        dir.path(),
        &[
            "build",
            "--release",
            "--backend",
            "cranelift",
            "main.wip",
            "-o",
            "released",
        ],
    );
    assert!(released.status.success(), "{released:?}");
    tells_the_debugger(dir.path(), "released", false);
    // On Linux the lines leave the program for a file it names, as the
    // distributions ship theirs.
    if !cfg!(target_vendor = "apple") {
        assert!(dir.path().join("released.debug").is_file());
        let program = std::fs::read(dir.path().join("released")).expect("the program");
        let names = |name: &[u8]| program.windows(name.len()).any(|at| at == name);
        assert!(names(b".gnu_debuglink") && !names(b".debug_line"));
    }
}

/// A release build by LLVM carries its lines too, beside
/// the program as Cranelift's are: the debugger finds `main.wip:4`, in
/// code LLVM may have inlined into `main`, and the variable `sum` in scope
/// there, where LLVM keeps it. What it adds up arrives as
/// the program runs, so the line has code. Where there is no `clang` for
/// LLVM, nothing is checked.
#[test]
fn a_release_build_by_llvm_tells_the_debugger_its_lines() {
    if !llvm_builds_here() {
        return;
    }
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "fn total(values: &[cstring]): i64 = {\n\tvar sum = 0\n\tfor value in values {\n\t\tsum += value.toStr().len()\n\t}\n\treturn sum\n}\n\nfn main(args: &[cstring]): i64 = {\n\treturn total(&args)\n}\n",
    );
    let built = wip(
        dir.path(),
        &["build", "--release", "--backend", "llvm", "main.wip"],
    );
    assert!(built.status.success(), "{built:?}");
    let said = |tool: &str, args: &[&str]| {
        Command::new(tool)
            .current_dir(dir.path())
            .args(args)
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default()
    };
    if cfg!(target_vendor = "apple") {
        assert!(dir.path().join("main.dSYM").is_dir());
        if on_path("lldb") {
            let found = said(
                "lldb",
                &[
                    "--batch",
                    "-o",
                    "breakpoint set --file main.wip --line 4",
                    "main",
                ],
            );
            assert!(found.contains("at main.wip:4"), "{found}");
            let address = found
                .split("address = ")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .expect("the breakpoint's address");
            let lookup = format!("image lookup --verbose --address {address}");
            let seen = said("lldb", &["--batch", "-o", &lookup, "main"]);
            assert!(seen.contains("name = \"sum\""), "{seen}");
        }
    } else {
        assert!(dir.path().join("main.debug").is_file());
        if on_path("gdb") {
            let found = said(
                "gdb",
                &[
                    "-batch",
                    "-ex",
                    "info line main.wip:4",
                    "-ex",
                    "info scope main.wip:4",
                    "main",
                ],
            );
            assert!(found.contains("Line 4 of"), "{found}");
            assert!(found.contains("Symbol sum "), "{found}");
        }
    }
}

/// Whether a program of this name is in one of the directories of `PATH`.
fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

/// Asks the debugger where `main.wip:8` is in `program`, which the build
/// in `dir` wrote, and where `total` is declared; and, where `variables`
/// says the build describes them, which variables are in scope there.
fn tells_the_debugger(dir: &Path, program: &str, variables: bool) {
    // A machine without the debugger skips what it would say. Whether it
    // is there is looked up on `PATH`, not by whether starting it fails:
    // under Rosetta, starting a program that is not there does not fail.
    let debugger = |tool: &str, args: &[&str]| {
        if !on_path(tool) {
            return None;
        }
        Command::new(tool)
            .current_dir(dir)
            .args(args)
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let names = ["value", "sum", "values"];
    if cfg!(target_vendor = "apple") {
        // Apple's linker leaves the lines in the objects; the build gathers
        // them beside the program.
        assert!(dir.join(format!("{program}.dSYM")).is_dir());
        let Some(said) = debugger(
            "lldb",
            &[
                "--batch",
                "-o",
                "breakpoint set --file main.wip --line 8",
                "-o",
                "image lookup --verbose --name total",
                program,
            ],
        ) else {
            return;
        };
        assert!(
            said.contains("total + ") && said.contains("at main.wip:8"),
            "{said}"
        );
        assert!(said.contains("decl = main.wip:5"), "{said}");
        // What is in scope at the line's code.
        let address = said
            .split("address = ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("the breakpoint's address");
        let lookup = format!("image lookup --verbose --address {address}");
        let said = debugger("lldb", &["--batch", "-o", &lookup, program]).unwrap_or_default();
        for name in names {
            let described = said.contains(&format!("name = \"{name}\""));
            assert_eq!(described, variables, "{name}: {said}");
        }
    } else {
        let Some(said) = debugger(
            "gdb",
            &[
                "-batch",
                "-ex",
                "info line main.wip:8",
                "-ex",
                "info scope main.wip:8",
                program,
            ],
        ) else {
            return;
        };
        assert!(
            said.contains("Line 8 of") && said.contains("<total+"),
            "{said}"
        );
        for name in names {
            let described = said.contains(&format!("Symbol {name} "));
            assert_eq!(described, variables, "{name}: {said}");
        }
    }
}

/// The debugger shows Wip's values as they are meant, with the scripts
/// `wip debug` loads: a `Vec`, a `Map` and a slice as their
/// elements, a `String` as its text, an enum as the variant it holds, a
/// `char` as itself. The program is run to a breakpoint, which a Mac lets
/// only a developer do; where the debugger cannot run it, or is not there,
/// nothing is asked of it.
#[test]
fn the_debugger_shows_wip_values() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\nimport std::collections::{Map}\n\nenum Shape {\n\tCircle(radius: i64)\n\tEmpty\n}\n\nfn count(items: &[i64]): i64 = {\n\treturn items.len()\n}\n\nfn main() = {\n\tvar numbers: Vec<i64> = Vec()\n\tnumbers.push(4)\n\tnumbers.push(5)\n\tval name = String::of(\"hello\")\n\tvar ages: Map<str, i64> = Map()\n\tages.put(\"ann\", 31)\n\tval shape = Shape::Circle(radius: 2)\n\tval none: Option<i64> = .None\n\tval letter = 'x'\n\tio::println(\"\\(count(&numbers.items())) \\(name) \\(ages.len()) \\(none.isSome()) \\(letter)\")\n\tio::println(\"\\(shape is .Empty)\")\n}\n",
    );
    let built = wip(dir.path(), &["build", "main.wip"]);
    assert!(built.status.success(), "{built:?}");
    let scripts = wip(dir.path(), &["debug", "--scripts"]);
    assert!(scripts.status.success(), "{scripts:?}");
    let listed = String::from_utf8_lossy(&scripts.stdout).into_owned();
    let script = |debugger: &str| {
        let line = listed
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{debugger}: ")))
            .expect("where the script is");
        assert!(Path::new(line).is_file(), "{line}");
        line.to_string()
    };
    let (lldb, gdb) = (script("lldb"), script("gdb"));
    let run = |tool: &str, args: &[&str]| {
        Command::new(tool)
            .current_dir(dir.path())
            .args(args)
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let said = match cfg!(target_vendor = "apple") {
        true => run(
            "lldb",
            &[
                "--batch",
                "-o",
                &format!("command script import {lldb}"),
                "-o",
                "breakpoint set --name count",
                "-o",
                "run",
                "-o",
                "frame variable items",
                "-o",
                "up",
                "-o",
                "frame variable",
                "-o",
                "kill",
                "main",
            ],
        ),
        false => run(
            "gdb",
            &[
                "-batch",
                "-x",
                &gdb,
                "-ex",
                "break count",
                "-ex",
                "run",
                "-ex",
                "info args",
                "-ex",
                "up",
                "-ex",
                "info locals",
                "main",
            ],
        ),
    };
    // No debugger, or one the system does not let run the program.
    let Some(said) = said.filter(|said| said.contains("main.wip:10")) else {
        return;
    };
    // Each as the debugger prints a variable, and not as the source it
    // lists says it.
    let shown: &[&str] = match cfg!(target_vendor = "apple") {
        true => &[
            "(&[i64]) items = len 2 {\n  [0] = 4\n  [1] = 5\n}",
            "(Vec<i64>) numbers = len 2 {\n  [0] = 4\n  [1] = 5\n}",
            "(String) name = \"hello\"",
            "(Map<str, i64>) ages = len 1 {\n  [\"ann\"] = 31\n}",
            "(Shape) shape = .Circle(radius: 2)",
            "(Option<i64>) none = .None",
            "(char32_t) letter = 'x'",
        ],
        false => &[
            "\nitems = len 2 = {4, 5}",
            "\nnumbers = len 2 = {4, 5}",
            "\nname = \"hello\"",
            "\nages = len 1 = {[\"ann\"] = 31}",
            "\nshape = .Circle = {radius = 2}",
            "\nnone = .None",
            "\nletter = 'x'",
        ],
    };
    for shown in shown {
        assert!(said.contains(shown), "{shown}: {said}");
    }

    // `wip debug` starts the system's debugger with its script loaded.
    let mut debugging = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["debug", "main"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("`wip debug` runs");
    debugging
        .stdin
        .take()
        .expect("its input")
        .write_all(b"quit\n")
        .expect("the input is written");
    let debugged = debugging.wait_with_output().expect("it ends");
    assert!(debugged.status.success(), "{debugged:?}");
}

/// `wip run main.wip`, typed in the file's own directory, compiles that
/// directory. A bare file name's parent is the empty path, which used to
/// name no directory at all: nothing was compiled, and the only error had no
/// file to be shown in.
#[test]
fn a_bare_file_name_means_the_current_directory() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import util::{answer}\n\nfn main(): i64 = answer()\n",
    );
    write(
        &dir.path().join("util/util.wip"),
        "pub fn answer(): i64 = 42\n",
    );

    let checked = wip(dir.path(), &["check", "main.wip"]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let run = wip(dir.path(), &["run", "main.wip"]);
    assert_eq!(
        run.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Diagnostics name a file as it was typed, without a leading `./`.
#[test]
fn diagnostics_name_files_as_typed() {
    let dir = TempDir::new().expect("a temporary directory");
    write(&dir.path().join("main.wip"), "fn main(): i64 = nope\n");
    let checked = wip(dir.path(), &["check", "--color", "never", "main.wip"]);
    let stderr = String::from_utf8_lossy(&checked.stderr);
    assert!(!checked.status.success());
    assert!(stderr.contains("[ main.wip:1:18 ]"), "{stderr}");
}

/// The entry's whole directory is compiled, so an entry that does not exist,
/// or is not a `.wip` file, is an error rather than a build of whatever else
/// is there.
#[test]
fn the_entry_must_be_a_wip_file() {
    let dir = TempDir::new().expect("a temporary directory");
    write(&dir.path().join("other.wip"), "fn main(): i64 = 0\n");
    write(&dir.path().join("notes.txt"), "not a program\n");

    let missing = wip(dir.path(), &["check", "typo.wip"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("cannot read `typo.wip`"));

    let text = wip(dir.path(), &["check", "notes.txt"]);
    assert_eq!(text.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&text.stderr).contains("`notes.txt` is not a `.wip` file"));
}

/// A diagnostic that points into no file is shown without an excerpt, never
/// dropped.
#[test]
fn a_diagnostic_outside_every_file_is_shown() {
    let diagnostic = Diagnostic::error(
        wip_syntax::codes::INVALID_MAIN,
        "no `main` function",
        Span::at(0),
        "the program starts at `main`",
    )
    .with_help("add `fn main(): i64 = 0`");
    let rendered = render::render_program(&[diagnostic], &Sources::default(), false);
    assert_eq!(
        rendered,
        "[E0318] Error: no `main` function\n    Help: add `fn main(): i64 = 0`\n"
    );
}

/// A `--` right after the file is the program's too, as every other word
/// after the file is: what it reads as the end of its own options.
#[test]
fn run_passes_a_separator_to_main() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io

fn main(args: &[cstring]) = {
    for i in 1..args.len() {
        io::printCstring(args[i])
    }
}
",
    );
    let run = wip(dir.path(), &["run", "main.wip", "--", "-v", "--", "x"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout.lines().collect::<Vec<_>>(), ["--", "-v", "--", "x"]);
}

/// `wip run` passes what follows the file to the program, which sees it as
/// `args[1]` and on. `args[0]` is the program's name, as in C.
#[test]
fn run_passes_arguments_to_main() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io

fn main(args: &[cstring]): i64 = {
    io::printInt(args.len())
    var i = 0
    while i < args.len() {
        io::printCstring(args[i])
        i = i + 1
    }
    3
}
",
    );
    let run = wip(
        dir.path(),
        &["run", "main.wip", "one", "--two", "three four"],
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(
        run.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 5, "{stdout}");
    assert_eq!(lines[0], "4");
    assert!(
        lines[1].ends_with("program"),
        "the program's name: {}",
        lines[1]
    );
    assert_eq!(lines[2..], ["one", "--two", "three four"]);
}

/// `main` takes nothing, or `&[cstring]` (E0318).
#[test]
fn main_takes_nothing_or_its_arguments() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "fn main(count: i64): i64 = count\n",
    );
    let built = wip(dir.path(), &["build", "--color", "never", "main.wip"]);
    let stderr = String::from_utf8_lossy(&built.stderr);
    assert!(!built.status.success());
    assert!(
        stderr.contains("[E0318] Error: `main` takes no parameters, or the program's arguments"),
        "{stderr}"
    );
    assert!(stderr.contains("not `args: &[cstring]`"), "{stderr}");
}

/// `main` may return a `Result`, whose error must be writable as text
/// (E0318).
#[test]
fn main_returning_a_result_needs_a_text_error() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "struct Boom {\n    code: i64\n}\n\nfn main(): Result<void, Boom> = .Err(Boom(code: 1))\n",
    );
    let built = wip(dir.path(), &["build", "--color", "never", "main.wip"]);
    let stderr = String::from_utf8_lossy(&built.stderr);
    assert!(!built.status.success());
    assert!(
        stderr.contains("[E0318] Error: `main`'s error type, `Boom`, cannot be written as text"),
        "{stderr}"
    );
    assert!(stderr.contains("write `extend Boom: Text`"), "{stderr}");
}

/// A `Result` `main` gives back nothing or an exit code, and nothing else
/// (E0318).
#[test]
fn main_returning_a_result_of_something_else() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "fn main(): Result<bool, i64> = .Ok(true)\n",
    );
    let built = wip(dir.path(), &["build", "--color", "never", "main.wip"]);
    let stderr = String::from_utf8_lossy(&built.stderr);
    assert!(!built.status.success());
    assert!(
        stderr.contains("[E0318] Error: `main` must return a `Result` of `i64` or of nothing"),
        "{stderr}"
    );
    assert!(stderr.contains("its value is `bool`"), "{stderr}");
}

/// The program's arguments and a `Result` together.
#[test]
fn main_takes_arguments_and_returns_a_result() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\nfn main(args: &[cstring]): Result<i64, i64> = {\n    io::printInt(args.len())\n    .Ok(5)\n}\n",
    );
    let run = wip(dir.path(), &["run", "main.wip"]);
    assert_eq!(
        run.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "1");
}

/// A program reads its standard input a line at a time, and the last line
/// counts whether or not it ends with a line break.
#[test]
fn a_program_reads_its_standard_input() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n         fn main(): Result<void, io::IoError> = {\n         \x20   var input = io::stdin()\n         \x20   var line = String()\n         \x20   var count = 0\n         \x20   while input.readLine(&var line)? {\n         \x20       count += 1\n         \x20       io::println(\"\\(count): \\(line)\")\n         \x20       line.clear()\n         \x20   }\n         \x20   io::eprintln(\"read \\(count)\")\n         \x20   return .Ok({})\n         }\n",
    );
    let run = wip_reading(dir.path(), &["run", "main.wip"], "alpha\nbeta\ngamma");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["1: alpha", "2: beta", "3: gamma"]
    );
    assert_eq!(stderr.trim(), "read 3");
}

/// A program writes a file, reads it back a line at a time, and says what
/// went wrong when there is no such file. The `File` closes
/// itself where it ends, which is what its drop is for.
#[test]
fn a_program_reads_and_writes_files() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        r#"import std::io
import std::fs

fn main(): Result<void, io::IoError> = {
    fs::writeFile("notes.txt", "alpha\nbeta\n")?

    var more = fs::File::append("notes.txt")?
    more.write("gamma\n")?
    destroy(move more)

    val text = fs::readFile("notes.txt")?
    io::println("bytes: \(text.len())")

    var file = fs::File::open("notes.txt")?
    var line = String()
    var count = 0
    while file.readLine(&var line)? {
        count += 1
        io::println("\(count): \(line)")
        line.clear()
    }

    match fs::readFile("no-such-file.txt") {
        .Ok(found) => io::println("unreachable")
        .Err(error) => io::println("failed: \(error)")
    }
    return .Ok({})
}
"#,
    );
    let run = wip(dir.path(), &["run", "main.wip"]);
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "bytes: 17",
            "1: alpha",
            "2: beta",
            "3: gamma",
            "failed: no such file or directory",
        ]
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes.txt")).expect("the file"),
        "alpha
beta
gamma
"
    );
}

/// What the environment holds, copied, or nothing.
#[test]
fn a_program_reads_its_environment() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        r#"import std::io

fn main() = {
    match io::env("WIP_DRIVER_TEST") {
        .Some(value) => io::println("set to \(value)")
        .None => io::println("unset")
    }
    match io::env("WIP_DRIVER_TEST_MISSING") {
        .Some(value) => io::println("unreachable")
        .None => io::println("unset")
    }
}
"#,
    );
    let run = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["run", "main.wip"])
        .env("WIP_DRIVER_TEST", "a value")
        .env_remove("WIP_DRIVER_TEST_MISSING")
        .output()
        .expect("`wip` runs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["set to a value", "unset"]
    );
}

/// `wip mir` prints the mid-level IR of every function.
#[test]
fn mir_prints_every_function() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "fn twice(x: i64): i64 = x * 2\n\nfn main(): i64 = twice(21)\n",
    );
    let printed = wip(dir.path(), &["mir", "main.wip"]);
    let stdout = String::from_utf8_lossy(&printed.stdout);
    assert!(
        printed.status.success(),
        "{}",
        String::from_utf8_lossy(&printed.stderr)
    );
    assert!(
        stdout.contains("fn twice(_0: i64) -> _1: i64 {"),
        "{stdout}"
    );
    assert!(stdout.contains("fn main() -> _0: i64 {"), "{stdout}");
    assert!(stdout.contains("call twice(21_i64)"), "{stdout}");
}

/// A negative length stops the program before anything is allocated.
#[test]
fn a_negative_buffer_length_stops_the_program() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "fn length(): i64 = 1 - 3\n\nfn main(): i64 = {\n    val xs = own [0; length()]\n    xs.len()\n}\n",
    );
    let run = wip(dir.path(), &["run", "main.wip"]);
    assert!(
        !run.status.success(),
        "a buffer of length -2 was allocated: {:?}",
        run.status
    );
    assert!(run.stdout.is_empty());
}

/// `-I` and `-L`: where a header and a library are on this machine.
/// The header is written in a directory of its own, which
/// nothing would find without being told.
#[test]
fn include_and_library_paths_are_given_on_the_command_line() {
    let dir = TempDir::new().expect("a temporary directory");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a directory for the header");
    write(
        &elsewhere.join("twice.h"),
        "#ifndef TWICE_H\n#define TWICE_H\nstatic inline int twice(int n) { return n * 2; }\n#endif\n",
    );
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         extern \"C\" {\n    @header(\"twice.h\")\n    fn twice(n: c_int): c_int\n}\n\n\
         fn main() = io::printInt(twice(21) as i64)\n",
    );

    // Without the flag, the C compiler cannot find the header.
    let blind = wip(dir.path(), &["run", "main.wip"]);
    assert!(!blind.status.success(), "{blind:?}");
    let said = String::from_utf8_lossy(&blind.stderr);
    assert!(said.contains("twice.h"), "{said}");

    let told = wip(
        dir.path(),
        &[
            "run",
            "main.wip",
            "-I",
            elsewhere.to_str().expect("a path"),
            "-L",
            elsewhere.to_str().expect("a path"),
        ],
    );
    assert!(told.status.success(), "{told:?}");
    assert_eq!(String::from_utf8_lossy(&told.stdout), "42\n");
}

/// A Wip library a C program links against, with the header the compiler
/// writes for it.
#[test]
fn a_library_c_links_against() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("board.wip"),
        "@export(\"C\")\n\
         fn board_score(row: i64, column: i64): i64 = row * 10 + column\n\n\
         @export(\"C\")\n\
         fn board_best(counts: &[i64]): i64 = {\n    \
             var best = 0\n    \
             for n in counts {\n        \
                 if n > best {\n            \
                     best = n\n        \
                 }\n    \
             }\n    \
             return best\n\
         }\n\n\
         @export(\"C\")\n\
         fn board_greet(name: cstring): i64 = name.toStr().len()\n",
    );
    // The C that calls it lives elsewhere: a `.c` beside the Wip is part
    // of the module.
    let c_dir = dir.path().join("c");
    std::fs::create_dir_all(&c_dir).expect("a directory for the C");

    let built = wip(
        dir.path(),
        &[
            "build",
            "board.wip",
            "--emit",
            "static",
            "--header",
            "c/board.h",
            "-o",
            "libboard.a",
        ],
    );
    assert!(built.status.success(), "{built:?}");

    let header = std::fs::read_to_string(c_dir.join("board.h")).expect("the header is written");
    assert!(
        header.contains("int64_t board_score(int64_t row, int64_t column);"),
        "{header}"
    );
    // A slice reaches C as a pointer and a length.
    assert!(
        header.contains("int64_t board_best(const int64_t *counts, int64_t counts_len);"),
        "{header}"
    );
    assert!(
        header.contains("int64_t board_greet(const char *name);"),
        "{header}"
    );

    write(
        &c_dir.join("main.c"),
        "#include <stdio.h>\n#include \"board.h\"\n\n\
         int main(void) {\n    \
             int64_t counts[] = {3, 9, 4};\n    \
             printf(\"%lld %lld %lld\\n\", (long long)board_score(4, 2),\n        \
                 (long long)board_best(counts, 3), (long long)board_greet(\"lines\"));\n    \
             return 0;\n\
         }\n",
    );
    // The archive holds undefined references to what the library calls,
    // which whoever links it supplies, and the header says what that is.
    // The prelude's maths are Wip's own, but for C's `fma` on x86-64, where an
    // older processor has no instruction for it, which on Linux is in libm.
    let needs_libm = cfg!(all(target_os = "linux", target_arch = "x86_64"));
    assert_eq!(
        header.contains("/* Link what this declares with: -lm */"),
        needs_libm,
        "{header}"
    );
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let mut link: Vec<&str> = vec!["c/main.c", "libboard.a", "-o", "c/program"];
    if needs_libm {
        link.push("-lm");
    }
    let compiled = Command::new(&cc)
        .current_dir(dir.path())
        .args(&link)
        .output()
        .expect("the C compiler runs");
    assert!(compiled.status.success(), "{compiled:?}");

    let ran = Command::new(dir.path().join("c/program"))
        .output()
        .expect("the C program runs");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "42 9 5\n");
}

/// A program holds what it can reach from where it starts, and nothing
/// else of its own or of the standard library's: a
/// function nothing calls, a constant table nothing reads and the
/// prelude's `pow` are in no object, and what is reached through a
/// function value, a drop and another module is.
#[test]
fn a_program_holds_only_what_it_reaches() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         val READ: [i64; 3] = [1, 2, 3]\n\
         val UNREAD: [i64; 3] = [4, 5, 6]\n\n\
         struct Held {\n    text: String\n}\n\n\
         extend Held: Destroy {\n    \
             var fn destroy() = io::println(\"ended\")\n\
         }\n\n\
         fn reached(n: i64): i64 = READ[n]\n\n\
         fn asAValue(n: i64): i64 = n + 1\n\n\
         fn unreached(n: i64): i64 = UNREAD[n] + onlyFromUnreached(n)\n\n\
         fn onlyFromUnreached(n: i64): i64 = (n as f64).pow(2.0) as i64\n\n\
         fn main() = {\n    \
             val held = Held(text: String::of(\"x\"))\n    \
             val f = asAValue\n    \
             io::println(\"\\(reached(1)) \\(f(1)) \\(held.text.len())\")\n\
         }\n",
    );
    let built = wip(dir.path(), &["build", "main.wip", "-o", "program"]);
    assert!(built.status.success(), "{built:?}");
    let ran = Command::new(dir.path().join("program"))
        .output()
        .expect("the program runs");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "2 2 1\nended\n");
    // The symbols of a program that was not stripped name what it holds.
    let bytes = std::fs::read(dir.path().join("program")).expect("the program is written");
    let holds = |name: &str| bytes.windows(name.len()).any(|at| at == name.as_bytes());
    for name in [
        "wip.reached",
        "wip.asAValue",
        "wip.Held.destroy",
        "table#READ",
    ] {
        assert!(
            holds(name),
            "`{name}` is reached, and is not in the program"
        );
    }
    for name in [
        "wip.unreached",
        "wip.onlyFromUnreached",
        "table#UNREAD",
        "wip.std.prelude.power",
    ] {
        assert!(
            !holds(name),
            "`{name}` is not reached, and is in the program"
        );
    }
}

/// A program is built for the processor `--cpu` names, its C too, and
/// does the same on each: the baseline, the compiling
/// machine, and every level of its kind, each run where this machine can.
#[test]
fn a_program_is_built_for_the_processor_named() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         extern \"C\" {\n    fn twice(n: i64): i64\n}\n\n\
         fn main() = {\n    \
             val bits: u64 = 0b1011_0000\n    \
             io::println(\"\\(bits.countOnes()) \\(bits.leadingZeros()) \\((2.5).floor()) \\((0.1).mulAdd(10.0, -1.0)) \\(twice(21))\")\n\
         }\n",
    );
    write(
        &dir.path().join("twice.c"),
        "#include <stdint.h>\nint64_t twice(int64_t n) { return n * 2; }\n",
    );
    let expected = "3 56 2 5.551115123125783e-17 42\n";
    let mut names = vec!["baseline", "native"];
    names.extend(
        wip_codegen::Level::of_host()
            .iter()
            .map(|level| level.name()),
    );
    for name in names {
        let built = wip(
            dir.path(),
            &["build", "--cpu", name, "main.wip", "-o", "program"],
        );
        assert!(built.status.success(), "{name}: {built:?}");
        // What this machine can run: every Arm level, and the x86-64 ones
        // whose features it has.
        let runs = match name {
            "x86-64-v3" => {
                cfg!(target_arch = "x86_64") && x86_has(&["avx2", "fma", "bmi2", "lzcnt"])
            }
            "x86-64-v4" => {
                cfg!(target_arch = "x86_64") && x86_has(&["avx512f", "avx512dq", "avx512vl"])
            }
            _ => true,
        };
        if runs {
            let ran = Command::new(dir.path().join("program"))
                .output()
                .expect("the program runs");
            assert_eq!(String::from_utf8_lossy(&ran.stdout), expected, "{name}");
        }
    }
    let refused = wip(dir.path(), &["build", "--cpu", "pentium", "main.wip"]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("not a processor this compiler builds for"),
        "{refused:?}"
    );
}

/// Whether this machine's x86-64 processor has every one of the features.
fn x86_has(features: &[&str]) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        features.iter().all(|&feature| match feature {
            "avx2" => std::arch::is_x86_feature_detected!("avx2"),
            "fma" => std::arch::is_x86_feature_detected!("fma"),
            "bmi2" => std::arch::is_x86_feature_detected!("bmi2"),
            "lzcnt" => std::arch::is_x86_feature_detected!("lzcnt"),
            "avx512f" => std::arch::is_x86_feature_detected!("avx512f"),
            "avx512dq" => std::arch::is_x86_feature_detected!("avx512dq"),
            "avx512vl" => std::arch::is_x86_feature_detected!("avx512vl"),
            _ => false,
        })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = features;
        false
    }
}

/// The same, as a shared library.
#[test]
fn a_shared_library() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("twice.wip"),
        "@export(\"C\")\nfn wip_twice(n: i64): i64 = n * 2\n",
    );
    let built = wip(dir.path(), &["build", "twice.wip", "--emit", "dynamic"]);
    assert!(built.status.success(), "{built:?}");
    let name = match std::env::consts::OS {
        "macos" | "ios" => "libtwice.dylib",
        _ => "libtwice.so",
    };
    assert!(dir.path().join(name).exists(), "{name} is written");
}

/// A C file is compiled once: a second build finds its object, a header it
/// includes that changes compiles it again, and one whose time moves but
/// whose text does not is still what it was.
#[test]
fn c_is_compiled_once() {
    use wip_lang::c_build::{CFile, compile};
    let dir = TempDir::new().expect("a temporary directory");
    let cache = dir.path().join("cache");
    let source = dir.path().join("tally.c");
    let header = dir.path().join("include/tally.h");
    write(&header, "#define STEP 1\n");
    write(
        &source,
        "#include \"tally.h\"\nint tally(void) { return STEP; }\n",
    );
    let files = [CFile::c(source.clone())];
    let includes = [dir.path().join("include")];
    let first = compile(
        &files,
        &includes,
        &cache,
        false,
        false,
        wip_codegen::Cpu::Baseline,
    )
    .expect("compiled");
    assert_eq!(first.compiled, 1);
    let second = compile(
        &files,
        &includes,
        &cache,
        false,
        false,
        wip_codegen::Cpu::Baseline,
    )
    .expect("compiled");
    assert_eq!(second.compiled, 0);
    assert_eq!(second.objects, first.objects);

    write(&header, "#define STEP 22\n");
    let changed = compile(
        &files,
        &includes,
        &cache,
        false,
        false,
        wip_codegen::Cpu::Baseline,
    )
    .expect("compiled");
    assert_eq!(
        changed.compiled, 1,
        "a changed header compiles its includer"
    );

    // The same text written again: a new time, and the same object.
    write(&header, "#define STEP 22\n");
    let touched = compile(
        &files,
        &includes,
        &cache,
        false,
        false,
        wip_codegen::Cpu::Baseline,
    )
    .expect("compiled");
    assert_eq!(
        touched.compiled, 0,
        "a header with the same text is unchanged"
    );

    // Other flags are another object: position-independent, or optimised
    // for a release build.
    let flagged = compile(
        &files,
        &includes,
        &cache,
        true,
        false,
        wip_codegen::Cpu::Baseline,
    )
    .expect("compiled");
    assert_eq!(flagged.compiled, 1);
    let optimized = compile(
        &files,
        &includes,
        &cache,
        false,
        true,
        wip_codegen::Cpu::Baseline,
    )
    .expect("compiled");
    assert_eq!(optimized.compiled, 1);
}

/// A C file that does not compile says why, naming the compiler.
#[test]
fn c_that_fails_says_so() {
    use wip_lang::c_build::{CFile, compile};
    let dir = TempDir::new().expect("a temporary directory");
    let source = dir.path().join("broken.c");
    write(&source, "int broken(void) { return }\n");
    let error = compile(
        &[CFile::c(source)],
        &[],
        &dir.path().join("cache"),
        false,
        false,
        wip_codegen::Cpu::Baseline,
    )
    .expect_err("broken C");
    assert!(error.contains("failed"), "{error}");
    assert!(error.contains("broken.c"), "{error}");
}

/// On a Mac a module's `.m` file is Objective-C, and `@framework` links
/// what it names.
#[cfg(target_vendor = "apple")]
#[test]
fn objective_c_and_frameworks_on_a_mac() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\nimport clock::{seconds_are_positive}\n\nfn main() = io::println(\"\\(seconds_are_positive())\")\n",
    );
    write(
        &dir.path().join("clock/clock.wip"),
        "@framework(\"Foundation\")\nextern \"C\" {\n    pub fn seconds_are_positive(): bool\n}\n",
    );
    // Objective-C, which a C compiler refuses: a message sent to a class.
    write(
        &dir.path().join("clock/clock.m"),
        "#import <Foundation/Foundation.h>\n#include <stdbool.h>\n\nbool seconds_are_positive(void) {\n    return [[NSDate date] timeIntervalSince1970] > 0;\n}\n",
    );
    let cache = dir.path().join("cache");
    let output = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .env("WIP_CACHE_DIR", &cache)
        .args(["run", "main.wip"])
        .output()
        .expect("`wip` runs");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "true\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An `@include` naming a directory the module does not have is reported
/// by the build, which knows where the module is.
#[test]
fn an_include_that_is_not_there() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import lib::{answer}\n\nfn main(): i64 = answer() as i64\n",
    );
    write(
        &dir.path().join("lib/lib.wip"),
        "@include(\"missing\")\nextern \"C\" {\n    pub fn answer(): c_int\n}\n",
    );
    write(
        &dir.path().join("lib/answer.c"),
        "int answer(void) { return 0; }\n",
    );
    let output = wip(dir.path(), &["build", "main.wip"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`@include(\"missing\")` in module `lib`"),
        "{stderr}"
    );
}

/// `wip test` compiles the `.test.wip` files of every module with the
/// program, runs each `@test` function, and says so. A test
/// sees the names its module keeps to itself, wherever the module is.
#[test]
fn wip_test_runs_the_tests_of_every_module() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\nimport board\n\nfn main() = io::println(\"the program\")\n",
    );
    write(
        &dir.path().join("board/board.wip"),
        "pub fn width(): i64 = 9\n\nfn hidden(): i64 = 4\n",
    );
    write(
        &dir.path().join("board/board.test.wip"),
        "@test\nfn theBoardIsNine() = assert(width() == 9)\n\n\
         @test\nfn testsSeeWhatTheModuleKeeps() = assert(hidden() == 4)\n",
    );
    write(
        &dir.path().join("main.test.wip"),
        "@test\nfn theWidthIsShared() = assert(board::width() == 9)\n",
    );
    let output = wip(dir.path(), &["test", "main.wip"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert_eq!(
        stdout,
        "running 3 tests\n\
         test theWidthIsShared ... ok\n\
         test board::theBoardIsNine ... ok\n\
         test board::testsSeeWhatTheModuleKeeps ... ok\n\
         \n3 tests passed\n"
    );

    // A filter keeps the tests whose name holds it.
    let output = wip(dir.path(), &["test", "main.wip", "Nine"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert_eq!(
        stdout,
        "running 1 test\ntest board::theBoardIsNine ... ok\n\n1 test passed\n"
    );

    // `wip run` compiles the program without its tests: a test file that
    // does not compile is no business of a build.
    write(
        &dir.path().join("board/board.test.wip"),
        "@test\nfn broken() = assert(nothingIsCalledThis())\n",
    );
    let output = wip(dir.path(), &["run", "main.wip"]);
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "the program\n");
    let output = wip(dir.path(), &["test", "main.wip"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("nothingIsCalledThis"),
        "{:?}",
        output
    );
}

/// A test that fails ends the run where it is: the line that names it is
/// unfinished, and the panic says what was asserted and where.
#[test]
fn a_failing_test_names_itself_and_stops_the_run() {
    let dir = TempDir::new().expect("a temporary directory");
    write(&dir.path().join("main.wip"), "fn main() = {}\n");
    write(
        &dir.path().join("main.test.wip"),
        "@test\nfn theSumIsRight() = {\n    var total = 0\n    total += 3\n    \
         assert(total == 7, \"three and four\")\n}\n\n\
         @test\nfn neverReached() = assert(true)\n",
    );
    let output = wip(dir.path(), &["test", "main.wip"]);
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "running 2 tests\ntest theSumIsRight ... "
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("panicked: three and four: total == 7\n  at "),
        "{stderr}"
    );
}

/// A program with no tests is not a failure: `wip test` says there are none.
#[test]
fn wip_test_without_tests_says_so() {
    let dir = TempDir::new().expect("a temporary directory");
    write(&dir.path().join("main.wip"), "fn main() = {}\n");
    let output = wip(dir.path(), &["test", "main.wip"]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "running 0 tests\n");
}

/// A program says which target it is for: `@target` on an
/// item, and a file named for a target. What is not this target's is not
/// compiled, so two of them may share a name, and the prelude says what
/// the target is. The program prints the same thing everywhere, since what
/// differs is which item was compiled to say it.
#[test]
fn targets_choose_what_is_compiled() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\nimport plat\n\n\
         @target(os = \"macos\")\nfn which(): str = \"here\"\n\n\
         @target(os = \"linux\")\nfn which(): str = \"here\"\n\n\
         @target(os = \"windows\")\nfn which(): str = \"here\"\n\n\
         fn main() = {\n    \
             io::println(which())\n    \
             io::println(plat::name())\n    \
             val known = TARGET_OS == \"macos\" || TARGET_OS == \"linux\" ||\n        \
                 TARGET_OS == \"windows\"\n    \
             assert(known && (TARGET_ARCH == \"arm64\" || TARGET_ARCH == \"x86_64\"))\n\
         }\n",
    );
    // One file per target, chosen by its name, and one for every target.
    write(
        &dir.path().join("plat/plat.macos.wip"),
        "pub fn name(): str = \"a file for this one\"\n",
    );
    write(
        &dir.path().join("plat/plat.linux.wip"),
        "pub fn name(): str = \"a file for this one\"\n",
    );
    write(
        &dir.path().join("plat/plat.windows.wip"),
        "pub fn name(): str = \"a file for this one\"\n",
    );
    let output = wip(dir.path(), &["run", "main.wip"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "here\na file for this one\n"
    );

    // `wip parse` shows the file as written: what is compiled is a
    // question for compiling.
    let parsed = wip(dir.path(), &["parse", "main.wip"]);
    let tree = String::from_utf8_lossy(&parsed.stdout);
    assert_eq!(tree.matches("fn which").count(), 3, "{tree}");
}

/// `wip check --target` checks the program as another target compiles it,
/// from this machine: an item or a file for Linux is read
/// on a Mac, and a Mac's on Linux.
#[test]
fn a_program_is_checked_for_another_target() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         @target(os = \"linux\")\n\
         fn place(): str = notDeclaredAnywhere()\n\n\
         @target(vendor = \"apple\")\n\
         fn place(): str = \"a Mac\"\n\n\
         fn main() = {\n    io::println(place())\n}\n",
    );
    write(
        &dir.path().join("wide.x86_64.wip"),
        "fn wide(): i64 = true\n",
    );
    let said = |args: &[&str]| {
        let output = wip(dir.path(), args);
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    };

    let (code, linux) = said(&["check", "--target", "linux-arm64", "main.wip"]);
    assert_eq!(code, Some(1), "{linux}");
    assert!(
        linux.contains("cannot find `notDeclaredAnywhere`"),
        "{linux}"
    );
    assert!(
        linux.contains("1 error"),
        "the x86-64 file is not arm64's: {linux}"
    );

    let (code, mac) = said(&["check", "--target", "macos-arm64", "main.wip"]);
    assert_eq!(code, Some(0), "{mac}");

    let (code, intel) = said(&["check", "--target", "macos-x86_64", "main.wip"]);
    assert_eq!(code, Some(1), "{intel}");
    assert!(intel.contains("wide.x86_64.wip"), "{intel}");

    // Each mistake once, and where not every target found it, which did.
    let (code, all) = said(&["check", "--target", "all", "main.wip"]);
    assert_eq!(code, Some(1), "{all}");
    assert_eq!(
        all.matches("cannot find `notDeclaredAnywhere`").count(),
        1,
        "{all}"
    );
    assert!(
        all.contains("found when checking for linux-arm64, linux-x86_64"),
        "{all}"
    );
    assert!(
        all.contains("found when checking for macos-x86_64, linux-x86_64"),
        "{all}"
    );
    assert!(all.contains("2 errors"), "{all}");

    // The same, named twice over.
    let (_, twice) = said(&[
        "check", "--target", "linux", "--target", "linux", "main.wip",
    ]);
    assert_eq!(
        twice.matches("cannot find `notDeclaredAnywhere`").count(),
        1,
        "{twice}"
    );

    let (code, windows) = said(&["check", "--target", "windows", "main.wip"]);
    assert_eq!(code, Some(2));
    assert!(windows.contains("not written for Windows"), "{windows}");
    let (code, unknown) = said(&["check", "--target", "linux-riscv64", "main.wip"]);
    assert_eq!(code, Some(2));
    assert!(
        unknown.contains("`linux-riscv64` is not a target"),
        "{unknown}"
    );
}

/// A mistake in code shared by every target is said once, without saying
/// which found it.
#[test]
fn a_mistake_every_target_finds_is_said_once() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "fn main() = {\n    val n: i64 = \"text\"\n}\n",
    );
    let output = wip(dir.path(), &["check", "--target", "all", "main.wip"]);
    let said = String::from_utf8_lossy(&output.stderr);
    assert_eq!(said.matches("mismatched types").count(), 1, "{said}");
    assert!(!said.contains("found when checking for"), "{said}");
    assert!(said.contains("1 error"), "{said}");
}

/// Allocations are counted only where something asks: a
/// trace started part way numbers from its start, and a block made before
/// it is freed without a word; the leak check, asked for, still counts
/// every allocation, and a program that leaks ends with status 70.
#[test]
fn allocations_are_counted_only_where_asked() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         extern \"C\" {\n    fn wip_trace_allocs()\n}\n\n\
         fn main() = {\n    \
             val before = own 1\n    \
             wip_trace_allocs()\n    \
             destroy(move before)\n    \
             val after = own 2\n    \
             io::println(\"\\(after)\")\n\
         }\n",
    );
    let ran = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["run", "main.wip"])
        .env_remove("WIP_CHECK_LEAKS")
        .output()
        .expect("`wip` runs");
    assert!(ran.status.success(), "{ran:?}");
    // `before` is freed without a word; `after` is #1, and the text that
    // prints it #2.
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "alloc #1\nalloc #2\n2\nfree #2\nfree #1\n"
    );

    write(
        &dir.path().join("main.wip"),
        // Ended by `exit` with something still held: the leak check's
        // handler runs, and finds it.
        "import std::libc\n\nfn main() = {\n    val kept = own 1\n    libc::exit(0)\n}\n",
    );
    let leak = |env: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_wip"));
        command.current_dir(dir.path()).args(["run", "main.wip"]);
        match env {
            Some(value) => command.env("WIP_CHECK_LEAKS", value),
            None => command.env_remove("WIP_CHECK_LEAKS"),
        };
        command.output().expect("`wip` runs")
    };
    let checked = leak(Some("1"));
    assert_eq!(checked.status.code(), Some(70), "{checked:?}");
    assert!(String::from_utf8_lossy(&checked.stderr).contains("1 allocation was never freed"));
    let unchecked = leak(None);
    assert!(unchecked.status.success(), "{unchecked:?}");
}

/// Whether a release build can be LLVM's here: there is a `clang` that
/// reads its IR.
fn llvm_builds_here() -> bool {
    wip_lang::choose_backend(wip_lang::Profile::Release, Some(wip_lang::Backend::Llvm)).is_ok()
}

/// The LLVM backend builds the program from the
/// same MIR through `clang`, and it does what Cranelift's does — a `&dyn`
/// call that writes, closures, a struct through C's `printf`, owned memory
/// freed. Where there is no `clang` for it, nothing is checked.
#[test]
fn the_llvm_backend_builds_what_cranelift_does() {
    if !llvm_builds_here() {
        return;
    }
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         interface Counter {\n    var fn bump(by: i64)\n    fn get(): i64\n}\n\n\
         struct Cell {\n    n: i64\n}\n\n\
         extend Cell: Counter {\n    var fn bump(by: i64) = self.n += by\n    fn get(): i64 = self.n\n}\n\n\
         fn tick(c: &var dyn Counter) = c.bump(2)\n\n\
         fn main() = {\n    \
             var cell = Cell(n: 1)\n    \
             tick(&var cell)\n    \
             var names: Vec<String> = Vec()\n    \
             for i in 0..3 {\n        names.push(\"n\\(i)\")\n    }\n    \
             val total = names.items().fold(0, (sum, name) => sum + name.len())\n    \
             io::println(\"\\(cell.get()) \\(total) \\(names[2])\")\n\
         }\n",
    );
    let ran = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["run", "--release", "--backend", "llvm", "main.wip"])
        .env("WIP_CHECK_LEAKS", "1")
        .output()
        .expect("`wip` runs");
    assert!(ran.status.success(), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "3 6 n2\n");
}

/// A panic in a program built by LLVM lists its calls as one built by
/// Cranelift does, though LLVM inlines them: a method, a
/// generic function, and a call in the tail of a function, which stays a
/// call. Where there is no `clang` for it, nothing is checked.
#[test]
fn a_panic_built_by_llvm_lists_the_calls_cranelifts_does() {
    if !llvm_builds_here() {
        return;
    }
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         struct Gauge {\n    level: i64\n}\n\n\
         extend Gauge {\n    fn check(): i64 = {\n        if self.level < 0 {\n            \
             panic(\"below zero\")\n        }\n        return self.level\n    }\n}\n\n\
         fn read<T>(gauge: &Gauge, _: T): i64 = gauge.check()\n\n\
         fn last(gauge: &Gauge): i64 = read(gauge, 1)\n\n\
         fn main() = {\n    \
             val gauge = Gauge(level: -1)\n    \
             io::printInt(last(&gauge))\n\
         }\n",
    );
    let run = |backend: &str| {
        Command::new(env!("CARGO_BIN_EXE_wip"))
            .current_dir(dir.path())
            .args(["run", "--release", "--backend", backend, "main.wip"])
            .output()
            .expect("`wip` runs")
    };
    let cranelift = run("cranelift");
    let llvm = run("llvm");
    assert_eq!(llvm.status.code(), Some(101), "{llvm:?}");
    let calls = String::from_utf8_lossy(&llvm.stderr);
    assert_eq!(calls, String::from_utf8_lossy(&cranelift.stderr));
    for call in [
        "in Gauge.check (main.wip:10)",
        "in read<i64> (main.wip:16)",
        "in last (main.wip:18)",
        "in main (main.wip:22)",
    ] {
        assert!(calls.contains(call), "`{call}` is missing from:\n{calls}");
    }
}

/// A release build without a `clang` for LLVM is Cranelift's, and says so;
/// asked for LLVM, it is refused, and so is LLVM for a debug build.
#[test]
fn a_release_build_without_clang_is_cranelifts() {
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\nfn main() = io::println(\"built\")\n",
    );
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wip"))
            .current_dir(dir.path())
            .args(args)
            .env("WIP_CLANG", dir.path().join("no-clang-here"))
            .env("WIP_CACHE_DIR", dir.path().join("cache"))
            .output()
            .expect("`wip` runs")
    };
    let fell_back = run(&["run", "--release", "main.wip"]);
    assert!(fell_back.status.success(), "{fell_back:?}");
    assert_eq!(String::from_utf8_lossy(&fell_back.stdout), "built\n");
    let note = String::from_utf8_lossy(&fell_back.stderr);
    assert!(
        note.contains("note: built by Cranelift: `")
            && note.contains("`, which WIP_CLANG names, is not a clang 15 or newer"),
        "{note}"
    );
    let insisted = run(&["run", "--release", "--backend", "llvm", "main.wip"]);
    assert!(!insisted.status.success(), "{insisted:?}");
    assert!(
        String::from_utf8_lossy(&insisted.stderr)
            .contains("error: the LLVM backend needs a clang 15 or newer"),
        "{insisted:?}"
    );
    let debug = run(&["run", "--backend", "llvm", "main.wip"]);
    assert!(!debug.status.success(), "{debug:?}");
    assert!(
        String::from_utf8_lossy(&debug.stderr).contains("`--backend llvm` builds a release build"),
        "{debug:?}"
    );
    let debug = run(&["run", "main.wip"]);
    assert!(debug.status.success(), "{debug:?}");
    assert!(
        !String::from_utf8_lossy(&debug.stderr).contains("note:"),
        "{debug:?}"
    );
}

/// A `&` parameter is `noalias readonly` to LLVM where nothing changes its
/// place while the call runs: not where the place holds an
/// `Atomic` by value, which changes through a `&`, but where one is only
/// reached through an `own`, which is other memory. An aggregate taken by
/// value is `noalias`, and a closure given one place both by value and by
/// `&` still reads it right. A reference is `nonnull`, and reaches its
/// whole value, aligned. The program runs as Cranelift's does. Where
/// there is no `clang` for LLVM, nothing is checked.
#[test]
fn a_shared_reference_is_read_only_to_llvm() {
    if !llvm_builds_here() {
        return;
    }
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\nimport std::sync::{Atomic}\n\n\
         struct Plain {\n    a: i64\n    b: i64\n}\n\n\
         struct Counted {\n    hits: Atomic<i64>\n}\n\n\
         enum Maybe {\n    Counting(counted: Counted)\n    Not\n}\n\n\
         struct Boxed {\n    counted: own<Counted>\n}\n\n\
         struct Wide {\n    a: i64\n    b: i64\n    c: i64\n    d: i64\n}\n\n\
         fn spanOf(w: Wide): i64 = w.d - w.a\n\n\
         fn sumOf(p: &Plain): i64 = p.a + p.b\n\n\
         fn hitsOf(c: &Counted): i64 = {\n    c.hits.add(1)\n    return c.hits.load()\n}\n\n\
         fn hitsIn(m: &Maybe): i64 = match m {\n    .Counting(counted) => hitsOf(&counted)\n    .Not => 0\n}\n\n\
         fn boxedHits(b: &Boxed): i64 = hitsOf(&b.counted)\n\n\
         fn main() = {\n    \
             val plain = Plain(a: 2, b: 3)\n    \
             val counted = Counted(hits: Atomic::of(0))\n    \
             val maybe = Maybe::Counting(counted: Counted(hits: Atomic::of(10)))\n    \
             val boxed = Boxed(counted: own Counted(hits: Atomic::of(20)))\n    \
             val wide = Wide(a: 1, b: 2, c: 3, d: 7)\n    \
             val both = (w: Wide, at: &Wide) => w.c + at.d\n    \
             io::println(\"\\(sumOf(&plain)) \\(hitsOf(&counted)) \\(hitsOf(&counted)) \\(hitsIn(&maybe)) \\(boxedHits(&boxed)) \\(spanOf(wide)) \\(both(wide, &wide))\")\n\
         }\n",
    );
    let ir = dir.path().join("program.ll");
    let built = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["run", "--release", "--backend", "llvm", "main.wip"])
        .env("WIP_LLVM_IR", &ir)
        .output()
        .expect("`wip` runs");
    assert!(built.status.success(), "{built:?}");
    assert_eq!(String::from_utf8_lossy(&built.stdout), "5 1 2 11 21 6 10\n");
    let ir = std::fs::read_to_string(&ir).expect("the IR");
    let defined = |name: &str| {
        ir.lines()
            .find(|line| line.starts_with("define") && line.contains(&format!("@\"wip.{name}.")))
            .unwrap_or_else(|| panic!("`{name}` is defined"))
            .to_string()
    };
    for name in ["sumOf", "boxedHits"] {
        let line = defined(name);
        assert!(line.contains("ptr noalias readonly nonnull"), "{line}");
    }
    // A reference reaches all of its value, aligned.
    let line = defined("sumOf");
    assert!(
        line.contains("nonnull dereferenceable(16) align 8 %p0"),
        "{line}"
    );
    for name in ["hitsOf", "hitsIn"] {
        let line = defined(name);
        assert!(!line.contains("readonly"), "{line}");
    }
    let line = defined("spanOf");
    assert!(line.contains("ptr noalias %p0"), "{line}");
}

/// The most negative `i128` divided by -1, or its remainder by -1, aborts
/// in a build by either backend: the runtime's division aborts there, and
/// LLVM's `sdiv` and `srem` would be undefined.
/// The divisor is known only as the program runs.
#[cfg(unix)]
#[test]
fn the_most_negative_i128_by_minus_one_aborts() {
    use std::os::unix::process::ExitStatusExt;
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         fn main(args: &[cstring]): i64 = {\n    \
             val most = (1 as i128) << 127\n    \
             val n = args.len() as i128\n    \
             val by = 0 - n % 2\n    \
             val answer = if n > 2 { most % by } else { most / by }\n    \
             io::printInt(answer as i64)\n    \
             return 0\n\
         }\n",
    );
    let mut builds = vec![vec!["build", "main.wip", "-o", "debug"]];
    if llvm_builds_here() {
        builds.push(vec![
            "build",
            "--release",
            "--backend",
            "llvm",
            "main.wip",
            "-o",
            "llvm",
        ]);
    }
    for build in builds {
        let built = wip(dir.path(), &build);
        assert!(built.status.success(), "{built:?}");
        let program = dir.path().join(build[build.len() - 1]);
        for args in [&[][..], &["a", "b"][..]] {
            let ran = Command::new(&program).args(args).output().expect("it runs");
            assert_eq!(ran.status.signal(), Some(6), "{build:?} {args:?}: {ran:?}");
        }
    }
}

/// Checked arithmetic is computed again past its check with LLVM's flags that
/// say it does not wrap — `nsw` signed, `nuw` unsigned — and still panics where
/// the answer does not fit. Where there is no `clang` for LLVM, nothing is
/// checked.
#[test]
fn checked_arithmetic_says_it_does_not_wrap() {
    if !llvm_builds_here() {
        return;
    }
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         fn plus(a: i64, b: i64): i64 = a + b\n\n\
         fn less(a: u32, b: u32): u32 = a - b\n\n\
         fn main(args: &[cstring]) = {\n    \
             io::printInt(plus(40, 2))\n    \
             io::printInt(less(5, 3) as i64)\n    \
             io::printInt(less(1, args.len() as u32 + 1) as i64)\n\
         }\n",
    );
    let ir = dir.path().join("program.ll");
    let ran = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["run", "--release", "--backend", "llvm", "main.wip"])
        .env("WIP_LLVM_IR", &ir)
        .output()
        .expect("`wip` runs");
    assert_eq!(ran.status.code(), Some(101), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "42\n2\n");
    assert!(
        String::from_utf8_lossy(&ran.stderr).contains("panicked"),
        "{ran:?}"
    );
    let ir = std::fs::read_to_string(&ir).expect("the IR");
    assert!(ir.contains(" = add nsw i64 "), "no `add nsw`");
    assert!(ir.contains(" = sub nuw i32 "), "no `sub nuw`");
}

/// What a call through a `&dyn` reads from its table of methods is read as
/// LLVM's invariant, since the table is a constant never freed; what an
/// owned closure's drop reads from its environment's header is not, since
/// that memory is freed and used again. Where there is no
/// `clang` for LLVM, nothing is checked.
#[test]
fn a_table_of_methods_is_read_as_invariant() {
    if !llvm_builds_here() {
        return;
    }
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\n\
         interface Step {\n    fn size(): i64\n}\n\n\
         struct Stride {\n    by: i64\n}\n\n\
         extend Stride: Step {\n    fn size(): i64 = self.by\n}\n\n\
         fn walk(step: &dyn Step, times: i64): i64 = {\n    \
             var total = 0\n    \
             var i = 0\n    \
             while i < times {\n        total += step.size()\n        i += 1\n    }\n    \
             return total\n\
         }\n\n\
         fn main() = {\n    \
             val stride = Stride(by: 3)\n    \
             val extra = 4\n    \
             val add = own (x: i64) => x + extra\n    \
             io::printInt(add(walk(&stride, 5)))\n\
         }\n",
    );
    let ir = dir.path().join("program.ll");
    let ran = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["run", "--release", "--backend", "llvm", "main.wip"])
        .env("WIP_LLVM_IR", &ir)
        .env("WIP_CHECK_LEAKS", "1")
        .output()
        .expect("`wip` runs");
    assert!(ran.status.success(), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "19\n");
    let ir = std::fs::read_to_string(&ir).expect("the IR");
    let body = |name: &str| -> String {
        let head = format!("@\"wip.{name}.");
        let start = ir
            .match_indices("\ndefine ")
            .map(|(at, _)| at + 1)
            .find(|&at| {
                ir[at..]
                    .lines()
                    .next()
                    .is_some_and(|line| line.contains(&head))
            })
            .unwrap_or_else(|| panic!("`{name}` is defined"));
        let end = start + ir[start..].find("\n}\n").expect("the end of a function");
        ir[start..end].to_string()
    };
    assert!(body("walk").contains("!invariant.load"), "{}", body("walk"));
    assert!(
        !body("main").contains("!invariant.load"),
        "{}",
        body("main")
    );
}

/// A release build by LLVM describes its variables, which
/// gdb shows through `wip debug`'s script each as it is or, where LLVM
/// kept only part of it, as `<optimized out>` — never as what it does not
/// hold. Where gdb, a `clang` for LLVM or a system that lets gdb run the
/// program is missing, nothing is checked.
#[test]
fn a_release_build_by_llvm_shows_wip_values_or_none() {
    if cfg!(target_vendor = "apple") || !on_path("gdb") || !llvm_builds_here() {
        return;
    }
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\nimport std::collections::{Map}\n\n\
         fn main() = {\n\
         \tvar numbers: Vec<i64> = Vec()\n\
         \tnumbers.push(4)\n\
         \tnumbers.push(5)\n\
         \tval name = String::of(\"hello\")\n\
         \tvar ages: Map<str, i64> = Map()\n\
         \tages.put(\"ann\", 31)\n\
         \tval none: Option<i64> = .None\n\
         \tval letter = 'x'\n\
         \tio::println(\"\\(numbers.len()) \\(name) \\(ages.len()) \\(none.isSome()) \\(letter)\")\n\
         }\n",
    );
    let built = wip(
        dir.path(),
        &["build", "--release", "--backend", "llvm", "main.wip"],
    );
    assert!(built.status.success(), "{built:?}");
    let scripts = wip(dir.path(), &["debug", "--scripts"]);
    let listed = String::from_utf8_lossy(&scripts.stdout).into_owned();
    let gdb = listed
        .lines()
        .find_map(|line| line.strip_prefix("gdb: "))
        .expect("where the script is")
        .to_string();
    let said = Command::new("gdb")
        .current_dir(dir.path())
        .args([
            "-batch",
            "-x",
            &gdb,
            "-ex",
            "break main.wip:13",
            "-ex",
            "run",
            "-ex",
            "info locals",
            "main",
        ])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    if !said.contains("main.wip:13") {
        return;
    }
    let mut shown_whole = 0;
    for (name, whole) in [
        ("numbers", "len 2 = {4, 5}"),
        ("name", "\"hello\""),
        ("ages", "len 1 = {[\"ann\"] = 31}"),
        ("none", ".None"),
        ("letter", "'x'"),
    ] {
        let line = said
            .lines()
            .find(|line| line.starts_with(&format!("{name} = ")))
            .unwrap_or_else(|| panic!("`{name}` is described: {said}"));
        let value = &line[name.len() + 3..];
        assert!(
            value == whole || value == "<optimized out>",
            "`{name}` is shown as {value}: {said}"
        );
        if value == whole {
            shown_whole += 1;
        }
    }
    assert!(shown_whole > 0, "nothing is shown whole: {said}");
}
