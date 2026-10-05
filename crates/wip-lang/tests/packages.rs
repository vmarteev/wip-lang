//! Packages that depend on each other: a monorepo of a
//! library and two programs, and each way a `package.wip` or an import
//! across packages can be wrong.

use std::path::Path;
use std::process::{Command, Output};

use wip_lang::TempDir;

fn wip(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("`wip` runs")
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("a directory")).expect("created");
    std::fs::write(path, text).expect("written");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// `engine`, a library with a public module and an internal one; `game`,
/// a program that depends on it; and `editor`, a program that depends on
/// both.
fn monorepo() -> TempDir {
    let dir = TempDir::new().expect("a temporary directory");
    let root = dir.path();
    write(
        &root.join("engine/package.wip"),
        "@version(\"0.3.0\")\npackage engine\n",
    );
    write(
        &root.join("engine/engine.wip"),
        "import render\n\npub fn frames(): i64 = render::frame(3)\n",
    );
    write(
        &root.join("engine/render/render.wip"),
        "import internal::pool\n\n\
         pub fn frame(n: i64): i64 = n * pool::size()\n\n\
         pub fn engineVersion(): str = package::VERSION\n",
    );
    write(
        &root.join("engine/render/render.test.wip"),
        "@test\nfn framesScale() = assert(frame(2) == 20)\n",
    );
    write(
        &root.join("engine/internal/pool/pool.wip"),
        "pub fn size(): i64 = 10\n",
    );
    write(
        &root.join("game/package.wip"),
        "@version(\"1.4.0\")\n@depends(\"engine\", path = \"../engine\")\npackage game\n",
    );
    write(
        &root.join("game/main.wip"),
        "import std::io\nimport engine\nimport engine::render\nimport board\n\n\
         fn main() = {\n    \
             io::println(\"\\(package::NAME) \\(package::VERSION)\")\n    \
             io::println(render::engineVersion())\n    \
             io::printInt(engine::frames() + board::cells())\n\
         }\n",
    );
    write(
        &root.join("game/board/board.wip"),
        "pub fn cells(): i64 = 81\n",
    );
    write(
        &root.join("game/main.test.wip"),
        "import std::fs\nimport board\n\n\
         @test\nfn boardHasCells() = assert(board::cells() == 81)\n\n\
         @test\nfn levelsAreBesideIt() = assert(fs::exists(\"levels/first.txt\"))\n",
    );
    write(&root.join("game/levels/first.txt"), "#########\n");
    write(
        &root.join("editor/package.wip"),
        "@version(\"0.1.0\")\n\
         @depends(\"engine\", path = \"../engine\")\n\
         @depends(\"game\", path = \"../game\")\n\
         package editor\n",
    );
    write(
        &root.join("editor/main.wip"),
        "import std::io\nimport game::board\nimport engine::render\n\n\
         fn main() = {\n    \
             io::println(\"\\(package::NAME) \\(package::VERSION)\")\n    \
             io::printInt(board::cells() + render::frame(1))\n\
         }\n",
    );
    dir
}

/// Each program builds its dependencies from source, and `package::` in a
/// module means that module's package: `engine`'s version from `engine`'s
/// code, called by `game`.
#[test]
fn programs_build_the_packages_they_depend_on() {
    let repo = monorepo();
    let game = wip(repo.path(), &["run", "game"]);
    assert_eq!(
        stdout(&game),
        "game 1.4.0\n0.3.0\n111\n",
        "{}",
        stderr(&game)
    );
    // `editor` reaches `engine` directly and through `game`: one `engine`.
    let editor = wip(repo.path(), &["run", "editor"]);
    assert_eq!(stdout(&editor), "editor 0.1.0\n91\n", "{}", stderr(&editor));
}

/// A package is built by its directory or its `package.wip`, and the
/// program is named as the package names itself, in the package's root:
/// from above, where a directory of that name is the package itself, the
/// program is written into it.
#[test]
fn a_package_is_built_as_itself() {
    let repo = monorepo();
    let game_dir = repo.path().join("game");
    let built = wip(&game_dir, &["build", "package.wip"]);
    assert!(built.status.success(), "{}", stderr(&built));
    assert!(game_dir.join("game").is_file());
    let from_above = wip(repo.path(), &["build", "editor"]);
    assert!(from_above.status.success(), "{}", stderr(&from_above));
    assert!(repo.path().join("editor/editor").is_file());
    let named = wip(repo.path(), &["build", "editor", "-o", "editor-bin"]);
    assert!(named.status.success(), "{}", stderr(&named));
    assert!(repo.path().join("editor-bin").is_file());
}

/// `wip test game` runs `game`'s tests; `engine`'s are run by testing it,
/// library as it is.
#[test]
fn a_package_tests_itself() {
    let repo = monorepo();
    let game = wip(repo.path(), &["test", "game"]);
    let game_out = stdout(&game);
    assert!(game_out.contains("boardHasCells"), "{game_out}");
    // Its tests run in its root, so the path a test names is the
    // package's, wherever the command was typed.
    assert!(game_out.contains("levelsAreBesideIt ... ok"), "{game_out}");
    assert!(!game_out.contains("framesScale"), "{game_out}");
    let engine = wip(repo.path(), &["test", "engine"]);
    assert!(
        stdout(&engine).contains("render::framesScale ... ok"),
        "{}",
        stderr(&engine)
    );
    // A filter is matched against the name as it is printed, module and
    // all: `render::` runs the module's tests.
    let by_module = wip(repo.path(), &["test", "engine", "render::"]);
    assert!(
        stdout(&by_module).contains("running 1 test"),
        "{}",
        stdout(&by_module)
    );
}

/// A module of the package that nothing imports yet is still checked, and
/// its tests run; a hidden directory, `target`, and a
/// package inside the package are not modules of it, and `wip build`
/// builds what the program imports.
#[test]
fn every_module_of_a_package_is_checked_and_tested() {
    let dir = TempDir::new().expect("a temporary directory");
    let root = dir.path();
    write(&root.join("package.wip"), "package tool\n");
    write(
        &root.join("main.wip"),
        "import used\n\nfn main() = used::hello()\n",
    );
    write(&root.join("used/used.wip"), "pub fn hello() = {}\n");
    write(
        &root.join("lonely/lonely.wip"),
        "fn half(n: i64): i64 = n / 2\n",
    );
    write(
        &root.join("lonely/lonely.test.wip"),
        "@test\nfn halves() = assert(half(8) == 4)\n",
    );
    write(
        &root.join("lonely/deeper/deeper.wip"),
        "fn one(): i64 = 1\n",
    );
    for skipped in [
        ".hidden/h.wip",
        "target/t.wip",
        "inner/package.wip",
        "inner/i.wip",
    ] {
        write(&root.join(skipped), "fn wrong(): i64 = \"not a number\"\n");
    }
    let tested = wip(root, &["test", "."]);
    let out = stdout(&tested);
    assert!(
        out.contains("lonely::halves ... ok"),
        "{out}{}",
        stderr(&tested)
    );
    let checked = wip(root, &["check", "."]);
    assert!(checked.status.success(), "{}", stderr(&checked));
    // An error in the module nobody imports is found by `check`, and is
    // nothing to `build`, which compiles what the program reaches.
    write(
        &root.join("lonely/lonely.wip"),
        "fn half(n: i64): i64 = \"half\"\n",
    );
    let checked = wip(root, &["check", "."]);
    assert!(!checked.status.success());
    assert!(
        stderr(&checked).contains("lonely.wip"),
        "{}",
        stderr(&checked)
    );
    let built = wip(root, &["build", "main.wip", "-o", "tool"]);
    assert!(built.status.success(), "{}", stderr(&built));
}

/// One wrong package: the monorepo, and a program beside it that says
/// `package.wip` and `main.wip`, checked. What it reports is returned.
fn refused(package_wip: &str, main_wip: &str) -> String {
    let repo = monorepo();
    write(&repo.path().join("bad/package.wip"), package_wip);
    write(&repo.path().join("bad/main.wip"), main_wip);
    let checked = wip(repo.path(), &["check", "--color", "never", "bad/main.wip"]);
    assert!(
        !checked.status.success(),
        "accepted:\n{package_wip}\n{main_wip}"
    );
    stderr(&checked)
}

#[test]
fn another_package_s_internal_module_is_refused() {
    let said = refused(
        "@depends(\"engine\", path = \"../engine\")\npackage bad\n",
        "import engine::internal::pool\n\nfn main() = {}\n",
    );
    assert!(said.contains("[E0215]"), "{said}");
    assert!(said.contains("is internal to package `engine`"), "{said}");
}

#[test]
fn a_package_reached_but_not_depended_on_is_refused() {
    let said = refused(
        "@depends(\"game\", path = \"../game\")\npackage bad\n",
        "import engine::render\n\nfn main() = {}\n",
    );
    assert!(
        said.contains("this package does not depend on `engine`"),
        "{said}"
    );
}

#[test]
fn a_dependency_is_named_as_it_names_itself() {
    let said = refused(
        "@depends(\"eng\", path = \"../engine\")\npackage bad\n",
        "fn main() = {}\n",
    );
    assert!(said.contains("is package `engine`, not `eng`"), "{said}");
    assert!(said.contains("it says this"), "{said}");
}

#[test]
fn what_a_package_wip_may_say() {
    let said = refused(
        "@version(\"1.4\")\n@license(\"MIT\")\n\
         @depends(\"net\", git = \"https://example.com/net\", rev = \"v1.0.0\")\n\
         @depends(\"nothing\", path = \"../nowhere\")\n\
         package bad\n",
        "fn main() = {}\n",
    );
    assert!(said.contains("`1.4` is not a semantic version"), "{said}");
    assert!(
        said.contains("`@license` is not something a package says"),
        "{said}"
    );
    assert!(said.contains("which is not built yet"), "{said}");
    assert!(said.contains("has no `package.wip`"), "{said}");
}

#[test]
fn a_package_wip_holds_only_the_package() {
    let said = refused("package bad\nfn main() = {}\n", "fn main() = {}\n");
    assert!(said.contains("holds nothing after its `package`"), "{said}");
}

#[test]
fn packages_that_depend_on_each_other_are_refused() {
    let repo = monorepo();
    let root = repo.path();
    write(
        &root.join("c1/package.wip"),
        "@depends(\"c2\", path = \"../c2\")\npackage c1\n",
    );
    write(&root.join("c1/main.wip"), "fn main() = {}\n");
    write(
        &root.join("c2/package.wip"),
        "@depends(\"c1\", path = \"../c1\")\npackage c2\n",
    );
    write(&root.join("c2/c2.wip"), "pub fn x(): i64 = 1\n");
    let checked = wip(root, &["check", "c1/main.wip"]);
    assert!(
        stderr(&checked).contains("depend on each other"),
        "{}",
        stderr(&checked)
    );
}

#[test]
fn a_module_named_like_a_dependency_is_refused() {
    let repo = monorepo();
    let root = repo.path();
    write(
        &root.join("bad/package.wip"),
        "@depends(\"engine\", path = \"../engine\")\npackage bad\n",
    );
    write(
        &root.join("bad/main.wip"),
        "import engine::render\n\nfn main() = {}\n",
    );
    write(&root.join("bad/engine/mine.wip"), "pub fn x(): i64 = 1\n");
    let checked = wip(root, &["check", "bad/main.wip"]);
    assert!(
        stderr(&checked).contains("is both a package this one depends on and a module of its own"),
        "{}",
        stderr(&checked)
    );
}

/// `package::VERSION` without an `@version`, and `package::NAME` without a
/// `package.wip`: each says what to write, and where.
#[test]
fn a_name_or_version_that_was_never_given() {
    let said = refused(
        "package bad\n",
        "import std::io\n\nfn main() = io::println(package::VERSION)\n",
    );
    assert!(said.contains("this package says no version"), "{said}");
    let dir = TempDir::new().expect("a temporary directory");
    write(
        &dir.path().join("main.wip"),
        "import std::io\n\nfn main() = io::println(package::NAME)\n",
    );
    let checked = wip(dir.path(), &["check", "main.wip"]);
    assert!(
        stderr(&checked).contains("this program is not a package"),
        "{}",
        stderr(&checked)
    );
    // A directory is built as a package only if it is one.
    let built = wip(
        dir.path().parent().expect("a parent"),
        &["build", &dir.path().display().to_string()],
    );
    assert!(
        stderr(&built).contains("with no `package.wip`"),
        "{}",
        stderr(&built)
    );
}

/// `wip fmt` lays a package out as its `@format` says, the command line
/// overrides it, and `--check` writes nothing and says which files would
/// change.
#[test]
fn a_package_says_how_it_is_formatted() {
    let dir = TempDir::new().expect("a temporary directory");
    let root = dir.path();
    write(
        &root.join("package.wip"),
        "@format(width = 100, indent = \"spaces\", size = 2)\npackage laid\n",
    );
    let messy = "fn main() = {\nval x = 1\n}\n";
    write(&root.join("main.wip"), messy);
    let check = wip(root, &["fmt", "--check", "."]);
    assert_eq!(check.status.code(), Some(1), "{}", stderr(&check));
    assert!(stdout(&check).contains("main.wip"), "{}", stdout(&check));
    assert_eq!(
        std::fs::read_to_string(root.join("main.wip")).expect("read"),
        messy
    );

    let formatted = wip(root, &["fmt", "."]);
    assert!(formatted.status.success(), "{}", stderr(&formatted));
    assert_eq!(
        std::fs::read_to_string(root.join("main.wip")).expect("read"),
        "fn main() = {\n  val x = 1\n}\n"
    );
    let again = wip(root, &["fmt", "--check", "."]);
    assert!(again.status.success(), "{}", stdout(&again));

    let tabs = wip(root, &["fmt", "--tabs", "main.wip"]);
    assert!(tabs.status.success(), "{}", stderr(&tabs));
    assert_eq!(
        std::fs::read_to_string(root.join("main.wip")).expect("read"),
        "fn main() = {\n\tval x = 1\n}\n"
    );
}

/// `wip fmt -` lays out standard input, as the package of the file
/// `--stdin-path` names says, and writes nothing when it does not parse,
/// so an editor keeps its buffer.
#[test]
fn standard_input_is_formatted_as_its_package_says() {
    let dir = TempDir::new().expect("a temporary directory");
    let root = dir.path();
    write(
        &root.join("package.wip"),
        "@format(indent = \"spaces\", size = 2)\npackage laid\n",
    );
    let format = |args: &[&str], text: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_wip"))
            .current_dir(root)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("`wip` runs");
        std::io::Write::write_all(&mut child.stdin.take().expect("stdin"), text.as_bytes())
            .expect("written");
        child.wait_with_output().expect("`wip` ends")
    };
    let messy = "fn main() = {\nval x = 1\n}\n";
    let laid = format(&["fmt", "-", "--stdin-path", "main.wip"], messy);
    assert!(laid.status.success(), "{}", stderr(&laid));
    assert_eq!(stdout(&laid), "fn main() = {\n  val x = 1\n}\n");

    let broken = format(&["fmt", "-", "--stdin-path", "main.wip"], "fn main( = {\n");
    assert_eq!(broken.status.code(), Some(1));
    assert_eq!(stdout(&broken), "");
    assert!(stderr(&broken).contains("main.wip"), "{}", stderr(&broken));
}

#[test]
fn a_format_wip_does_not_know_is_refused() {
    let said = refused(
        "@format(indent = \"both\")\npackage bad\n",
        "fn main() = {}\n",
    );
    assert!(
        said.contains("neither `\"tabs\"` nor `\"spaces\"`"),
        "{said}"
    );
}
