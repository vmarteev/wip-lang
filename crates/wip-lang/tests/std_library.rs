//! The standard library's own tests, which are written in Wip and run by
//! `wip test`. They live in `tests/std`, a module of their
//! own: the library is compiled into the compiler, so a `.test.wip` file
//! beside it would ship in every program.

use std::path::Path;
use std::process::Command;

#[test]
fn the_standard_library_passes_its_own_tests() {
    passes(&["test", "tests/std/main.wip"]);
}

/// Built for use, the library passes them too: a release build does what a
/// debug build does.
#[test]
fn the_standard_library_passes_them_built_for_release() {
    passes(&["test", "--release", "tests/std/main.wip"]);
}

fn passes(args: &[&str]) {
    // They run from the repository's root, as `wip test tests/std/main.wip`
    // does by hand: the tests of `std::fs` read the repository they are
    // in, so where they are run from is part of what they are.
    let root = Path::new("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("`wip test` runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    // A run that compiled no tests would pass silently, which is the one
    // way this test could stop checking anything.
    let ran: usize = stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("running "))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);
    assert!(ran >= 20, "only {ran} tests ran:\n{stdout}");
}

/// The standard library checks for every target it is written for, on
/// whatever machine runs this: its code for Linux is read
/// on a Mac, and a Mac's on Linux, which a build reads only there.
#[test]
fn the_standard_library_checks_for_every_target() {
    let dir = wip_lang::TempDir::new().expect("a temporary directory");
    let mut program = String::new();
    for module in wip_lang::modules() {
        if module != "std::prelude" {
            program.push_str(&format!("import {module}\n"));
        }
    }
    program.push_str("\nfn main() = {}\n");
    let main = dir.path().join("main.wip");
    std::fs::write(&main, &program).expect("written");
    for target in wip_lang::targets::Target::all() {
        let mut loaded = wip_lang::load_for(
            &main,
            wip_lang::Mode::Program,
            target,
            &mut wip_lang::Timings::default(),
        )
        .expect("loaded");
        let checked = wip_lang::check_loaded(&mut loaded, &mut wip_lang::Timings::default());
        let errors: Vec<_> = checked
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        assert!(
            errors.is_empty(),
            "for {}:\n{}",
            target.name(),
            wip_lang::render::render_program(&errors, &loaded.sources, false)
        );
    }
}

/// A temporary made in one branch of what its statement evaluates — the
/// panic's message an arena's index builds where a handle is stale — is
/// dropped where the branches join, and its flag is written on every path
/// before it is read. Cranelift's code read an unwritten
/// flag as false, and LLVM's as anything.
#[test]
fn no_flag_is_read_before_it_is_written() {
    let dir = wip_lang::TempDir::new().expect("a temporary directory");
    let main = dir.path().join("main.wip");
    std::fs::write(
        &main,
        "import std::collections::{Arena, Handle}\n\n\
         fn read(nodes: &Arena<i64>, at: Handle<i64>): i64 = nodes[at]\n\n\
         fn main() = {}\n",
    )
    .expect("written");
    let mut loaded = wip_lang::load_for(
        &main,
        wip_lang::Mode::Program,
        wip_lang::targets::Target::host(),
        &mut wip_lang::Timings::default(),
    )
    .expect("loaded");
    let checked = wip_lang::check_loaded(&mut loaded, &mut wip_lang::Timings::default());
    assert!(!checked.diagnostics.iter().any(|d| d.is_error()));
    let program = &checked.program;
    let (id, _) = program
        .fns
        .iter()
        .find(|(_, def)| loaded.interner.resolve(def.name) == "read")
        .expect("`read` is there");
    let body = wip_mir::function_body(program, &loaded.interner, id).expect("a body");
    let flags: Vec<_> = wip_mir::reads_before_writes(&body)
        .into_iter()
        .filter(|(local, _)| body.local(*local).ty == wip_hir::Types::BOOL)
        .collect();
    assert!(
        flags.is_empty(),
        "flags read before they are written: {flags:?}"
    );
}
