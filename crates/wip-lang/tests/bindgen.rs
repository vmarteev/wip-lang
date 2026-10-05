//! The binding generator, which is written in Wip: its own tests, run by `wip
//! test` as the standard library's are.

use std::path::Path;
use std::process::Command;

#[test]
fn the_binding_generator_passes_its_own_tests() {
    let root = Path::new("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(root)
        .args(["test", "tools/bindgen/main.wip"])
        .output()
        .expect("`wip test` runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    // A run that compiled no tests would pass silently.
    assert!(!stdout.contains("running 0 tests"), "{stdout}");
}

/// The generator asks clang for a header's syntax tree, and only clang can
/// write one. Where clang is not installed there is
/// nothing to test rather than something broken: the Debian image
/// `scripts/linux.sh` uses carries gcc and no clang.
fn clang_is_installed() -> bool {
    Command::new("clang")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// What `wip bindgen` writes for `tests/bindgen/shapes.h` with `args`.
fn written_for(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir("../..")
        .args(["bindgen", "tests/bindgen/shapes.h"])
        .args(args)
        .output()
        .expect("`wip bindgen` runs");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    stdout
}

/// What the generator writes for a header with one of everything in it.
/// The file it produces is committed and read by people, so what changes
/// about it should be visible in a review.
#[test]
fn shapes() {
    if !clang_is_installed() {
        eprintln!("skipped: clang is not installed, and the generator is its front end");
        return;
    }
    insta::assert_snapshot!(written_for(&["--link", "shapes", "--module", "shapes"]));
}

/// A header read with a setting declares what that setting makes it
/// declare, and the file says the setting, so the C compiled with the
/// program is configured the same way.
#[test]
fn shapes_configured() {
    if !clang_is_installed() {
        eprintln!("skipped: clang is not installed, and the generator is its front end");
        return;
    }
    let written = written_for(&["-D", "SHAPES_WIDE", "--module", "shapes"]);
    assert!(
        written.contains("pub val SHAPES_WIDTH: i64 = 64"),
        "{written}"
    );
    assert!(
        written.contains("@define(\"SHAPES_WIDE\")\n@header(\"shapes.h\")"),
        "{written}"
    );
    let plain = written_for(&["--module", "shapes"]);
    assert!(plain.contains("pub val SHAPES_WIDTH: i64 = 32"), "{plain}");
    assert!(!plain.contains("@define"), "{plain}");
}

/// The words a C name is renamed around are the language's keywords,
/// every one: bindgen keeps its own list, which the lexer's is read
/// against here, so that a keyword added to the language is not a name
/// bindgen writes as it is.
#[test]
fn bindgen_renames_every_keyword() {
    let lexer = std::fs::read_to_string("../wip-syntax/src/token.rs").expect("the lexer is there");
    let start = lexer
        .find("pub fn keyword(")
        .expect("the lexer reads keywords");
    let end = start + lexer[start..].find("_ => return None").expect("and stops");
    let mut keywords: Vec<&str> = lexer[start..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"')?.split_once("\" =>"))
        .map(|(word, _)| word)
        .collect();
    keywords.sort_unstable();

    let types =
        std::fs::read_to_string("../../tools/bindgen/types/types.wip").expect("bindgen is there");
    let list = &types[types.find("val KEYWORDS").expect("bindgen lists them")..];
    let list = &list[list.find("= [").expect("the list begins")..];
    let list = &list[..list.find("\n]").expect("the list ends")];
    let mut renamed: Vec<&str> = list
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"')?.strip_suffix("\","))
        .collect();
    renamed.sort_unstable();

    assert_eq!(
        renamed, keywords,
        "bindgen's KEYWORDS and the lexer's keywords"
    );
}

/// `wip bindgen`'s own command line, read with `std::args`: what is wrong
/// with it is said, with the usage, before clang is asked anything.
#[test]
fn bindgen_reads_its_command_line() {
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wip"))
            .current_dir("../..")
            .arg("bindgen")
            .args(args)
            .output()
            .expect("`wip bindgen` runs")
    };
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("usage: wip bindgen <header>"));

    for (args, said) in [
        (
            &["--frobnicate"][..],
            "error: `--frobnicate` is not an option",
        ),
        (&["a.h", "b.h"][..], "error: unexpected argument `b.h`"),
        (&["a.h", "-o"][..], "error: `-o` needs a value"),
        (&[][..], "usage: wip bindgen <header>"),
    ] {
        let out = run(args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {stderr}");
        assert!(stderr.contains(said), "{args:?}: {stderr}");
    }
}
