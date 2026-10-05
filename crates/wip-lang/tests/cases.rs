//! File-driven tests over `tests/cases/`. Every case is a directory, so it is
//! a module like any other program and can be run as it is:
//! `wip run tests/cases/ok/01_hello/main.wip`.
//!
//! - `ok/<case>/main.wip`: the program must produce no diagnostics. Its syntax
//!   tree is snapshotted in `syntax.snap`, and the output of compiling and
//!   running it (or the list of features the code generator does not support
//!   yet) in `run.snap`.
//! - `err/<case>/main.wip`: the rendered diagnostics are snapshotted in
//!   `diagnostics.snap`.
//!
//! A case may have more files, in its directory or in directories below it:
//! the module's other files, and the modules it imports. The syntax
//! snapshot shows every file of the case's own module, under its name.
//!
//! Regenerate with `INSTA_UPDATE=always cargo test`, then review the diff.
//! Snapshots record whatever the phases implemented so far report, so a case
//! aimed at a later phase snapshots as `(no diagnostics)` or "not supported
//! yet" until that phase exists.

use std::process::{Command, Stdio};

use datatest_stable::Utf8Path;
use wip_lang::{BuildError, Loaded, TempDir, Timings, render};
use wip_syntax::ast;

const CASES: &str = "../../tests/cases";

/// Loads the program a case's `main.wip` starts.
fn load(path: &Utf8Path) -> datatest_stable::Result<Loaded> {
    wip_lang::load(path.as_std_path())
        .map_err(|message| -> Box<dyn std::error::Error> { message.into() })
}

/// Where a case's snapshots go: the case's own directory. `insta` reads the
/// path relative to this file's directory, one level below the paths the
/// harness is given.
fn snapshot_path(path: &Utf8Path) -> String {
    let dir = path.parent().expect("a case directory");
    format!("../{dir}")
}

/// A name as it appears in diagnostics, relative to the case's directory:
/// `main.wip`, `util/util.wip`.
fn in_case(path: &Utf8Path, name: &str) -> String {
    let dir = format!("{}/", path.parent().expect("a case directory"));
    name.strip_prefix(&dir).unwrap_or(name).to_string()
}

/// The syntax tree of every file of the case's own module, in the order the
/// files are read.
fn syntax_of(loaded: &Loaded, path: &Utf8Path) -> String {
    let module = loaded.modules.first().expect("the root module");
    let mut out = String::new();
    for ((name, text, base), parsed) in loaded.sources.entries().zip(&module.files) {
        out.push_str(&format!("=== {} ===\n", in_case(path, name)));
        out.push_str(&ast::dump_at(&parsed.ast, text, base, &loaded.interner));
    }
    out
}

/// Runs a built program and reports its exit code and output, the way the
/// snapshots record it.
fn report_of(exe: &std::path::Path) -> datatest_stable::Result<String> {
    // The runtime reports any `own` value that was never freed. Standard
    // input is empty, whatever the suite was started from: a case that
    // reads would otherwise wait on a terminal for ever.
    let output = Command::new(exe)
        .env("WIP_CHECK_LEAKS", "1")
        .stdin(Stdio::null())
        .output()?;
    let status = match output.status.code() {
        Some(code) => format!("exit code {code}"),
        None => format!("terminated: {}", output.status),
    };
    let mut report = format!(
        "{status}\n--- stdout ---\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    if !output.stderr.is_empty() {
        report.push_str(&format!(
            "--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(report)
}

/// A case that must compile: its syntax tree, and what running it prints.
fn ok(path: &Utf8Path, _text: String) -> datatest_stable::Result<()> {
    let mut loaded = load(path)?;
    let tree = syntax_of(&loaded, path);
    let dir = TempDir::new()?;
    let exe = dir.path().join("program");
    let options = wip_lang::BuildOptions::default();
    let built = wip_lang::build_loaded(&mut loaded, &exe, &options, &mut Timings::default());
    let report = match built {
        Ok(_) => report_of(&exe)?,
        Err(BuildError::Diagnostics(diagnostics))
            if diagnostics
                .iter()
                .all(|d| d.code == wip_syntax::codes::UNSUPPORTED) =>
        {
            let mut report = String::from("not supported by the code generator yet:\n");
            for d in &diagnostics {
                report.push_str(&format!("- {}\n", d.message));
            }
            report
        }
        Err(BuildError::Diagnostics(diagnostics)) => {
            let rendered = render::render_program(&diagnostics, &loaded.sources, false);
            return Err(format!("expected no diagnostics, got:\n{rendered}").into());
        }
        Err(BuildError::Link(message)) => return Err(message.into()),
    };
    insta::with_settings!({
        snapshot_path => snapshot_path(path),
        prepend_module_to_snapshot => false,
        omit_expression => true,
    }, {
        insta::assert_snapshot!("syntax", tree);
        insta::assert_snapshot!("run", report);
    });
    // A release build does what the debug build does: only faster.
    let release = wip_lang::BuildOptions {
        profile: wip_lang::Profile::Release,
        ..wip_lang::BuildOptions::default()
    };
    let released = dir.path().join("released");
    if wip_lang::build_loaded(&mut loaded, &released, &release, &mut Timings::default()).is_ok() {
        let released = report_of(&released)?;
        if released != report {
            return Err(format!(
                "the release build does something else:\n--- debug ---\n{report}\n--- release ---\n{released}"
            )
            .into());
        }
    }
    Ok(())
}

/// A case that must be reported: the diagnostics of every phase up to the
/// move checker, rendered as the driver renders them.
fn err(path: &Utf8Path, _text: String) -> datatest_stable::Result<()> {
    let mut loaded = load(path)?;
    let diagnostics = wip_lang::check_loaded(&mut loaded, &mut Timings::default()).diagnostics;
    let rendered = if diagnostics.is_empty() {
        "(no diagnostics)\n".to_string()
    } else {
        // File names relative to `tests/cases/`, so snapshots do not depend
        // on where the test runs.
        render::render_program(&diagnostics, &loaded.sources, false)
            .replace(&format!("{CASES}/"), "")
    };
    insta::with_settings!({
        snapshot_path => snapshot_path(path),
        prepend_module_to_snapshot => false,
        omit_expression => true,
    }, {
        insta::assert_snapshot!("diagnostics", rendered);
    });
    Ok(())
}

datatest_stable::harness! {
    { test = ok, root = "../../tests/cases/ok", pattern = r"^.*/main\.wip$" },
    { test = err, root = "../../tests/cases/err", pattern = r"^.*/main\.wip$" },
}
