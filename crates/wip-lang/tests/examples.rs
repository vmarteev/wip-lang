//! The programs in `examples/`, each a directory with a `main.wip`: every
//! one is checked, its tests are run where it has any, and a few are run
//! as a reader would run them, so that an example cannot quietly stop
//! working.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The examples' directories, by name.
fn examples() -> Vec<PathBuf> {
    let root = Path::new("../../examples");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .expect("the examples are there")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn wip(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir("../..")
        .args(args)
        .output()
        .expect("`wip` runs")
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Whether a directory holds tests of its own module, `.test.wip` files.
fn has_tests(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().ends_with(".test.wip"))
}

/// The packages inside an example other than the example itself: a
/// library it depends on, whose tests are its own.
fn packages_inside(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.join("package.wip").is_file() {
                found.push(path.clone());
            }
            packages_inside(&path, found);
        }
    }
}

#[test]
fn every_example_checks_and_passes_its_tests() {
    let all = examples();
    assert!(all.len() >= 9, "the examples are there: {all:?}");
    for dir in all {
        let name = dir.display().to_string().replace("../../", "");
        let main = format!("{name}/main.wip");
        assert!(
            Path::new("../..").join(&main).is_file(),
            "{name} has a main.wip"
        );
        let checked = wip(&["check", &main]);
        assert!(
            checked.status.success(),
            "{name} checks:\n{}",
            said(&checked)
        );
        if has_tests(&dir) {
            let tested = wip(&["test", &main]);
            assert!(
                tested.status.success(),
                "{name}'s tests pass:\n{}",
                said(&tested)
            );
        }
        let mut inside = Vec::new();
        packages_inside(&dir, &mut inside);
        for package in inside {
            let package = package.display().to_string().replace("../../", "");
            let tested = wip(&["test", &package]);
            assert!(
                tested.status.success(),
                "{package}'s tests pass:\n{}",
                said(&tested)
            );
        }
    }
}

#[test]
fn hello_says_hello() {
    let run = wip(&["run", "examples/hello/main.wip"]);
    assert!(run.status.success(), "{}", said(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "hello, world\n");
}

/// `wordcount` reads its options with `std::args`, as `wip run` passes
/// them: a cluster with a value in it, a file, and a mistake.
#[test]
fn wordcount_counts_with_the_options_it_is_given() {
    let dir = wip_lang::TempDir::new().expect("a temporary directory");
    let file = dir.path().join("text.txt");
    std::fs::write(&file, "The cat saw the dog, and the dog saw the cat.\n").expect("written");
    let file = file.display().to_string();
    let run = wip(&["run", "examples/wordcount/main.wip", "-in2", &file]);
    assert!(run.status.success(), "{}", said(&run));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "      4  the\n      2  cat\n"
    );
    let wrong = wip(&["run", "examples/wordcount/main.wip", "--top", "two"]);
    assert_eq!(wrong.status.code(), Some(1), "{}", said(&wrong));
    assert!(
        said(&wrong).contains("error: `--top` takes a number, and `two` is not one"),
        "{}",
        said(&wrong)
    );
}

/// What `shapes-package` prints, its dependency's version among it.
#[test]
fn a_package_runs_with_what_it_depends_on() {
    let run = wip(&["run", "examples/shapes-package/main.wip"]);
    assert!(run.status.success(), "{}", said(&run));
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.ends_with("shapes 1.0.0, with geometry 0.2.0\n"),
        "{stdout}"
    );
}
