//! Every example in the language reference, compiled.
//!
//! `docs/language/` describes the language as it is, and each of its Wip
//! examples is a program the compiler must accept. The fence says what is
//! expected of it:
//!
//! - ```` ```wip ```` — it must compile. A block that declares no items is
//!   wrapped in a `main`, so an example can be a few statements.
//! - ```` ```wip,run ```` — it must compile and run to a successful end. An
//!   example that claims something says it with `assert`,
//!   so running it is what checks the claim.
//! - ```` ```wip,error=E0303 ```` — it must be refused, with that code. This
//!   is how the reference shows what the language does not allow.
//! - ```` ```wip,ignore ```` — prose in Wip's clothing: a fragment that
//!   stands for something, not a program.
//!
//! An example that needs more than one file — a module, imported by the
//! program beside it — writes `// file: util/util.wip` where the next file
//! begins; everything before the first such line is the program itself.
//!
//! An example that fails names its file, the line its fence is on, and what
//! the compiler said, so it can be fixed where it is written.
//!
//! Each example is also laid out as `wip fmt` lays it out, with four spaces
//! and in 80 columns, since a reader copies what the reference shows.

use std::path::{Path, PathBuf};
use std::process::Command;

use wip_lang::TempDir;

/// One fenced block, as the harness found it.
struct Example {
    /// The document it came from, as a reader would name it.
    file: String,
    /// The line the fence is on, counted from one.
    line: usize,
    expected: Expected,
    code: String,
}

#[derive(PartialEq, Eq)]
enum Expected {
    Compiles,
    /// It compiles, runs, and ends well.
    Runs,
    /// It is refused, with this code.
    Refused(String),
}

#[test]
fn every_example_in_the_reference_compiles() {
    let dir = Path::new("../../docs/language");
    assert!(
        dir.is_dir(),
        "the language reference is at {}",
        dir.display()
    );
    let mut examples = collect(dir);
    // The front door is checked too: what it shows must work.
    let readme = Path::new("../../README.md");
    let text = std::fs::read_to_string(readme).expect("the repository has a README");
    examples.extend(examples_in("README.md", &text));
    assert!(
        examples.len() >= 5,
        "the reference has examples: found {}",
        examples.len()
    );
    // Each example is its own program in its own directory, so they are
    // checked on as many threads as the machine has.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = examples.len().div_ceil(threads);
    let failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = examples
            .chunks(chunk)
            .map(|chunk| scope.spawn(move || chunk.iter().filter_map(check).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a thread finished"))
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} of {} examples failed:\n\n{}",
        failures.len(),
        examples.len(),
        failures.join("\n\n")
    );
}

#[test]
fn every_example_in_the_reference_is_formatted() {
    let mut examples = collect(Path::new("../../docs/language"));
    let text = std::fs::read_to_string("../../README.md").expect("the repository has a README");
    examples.extend(examples_in("README.md", &text));
    let unformatted: Vec<String> = examples.iter().filter_map(unformatted).collect();
    assert!(
        unformatted.is_empty(),
        "{} examples are not laid out as `wip fmt` lays them out:\n\n{}",
        unformatted.len(),
        unformatted.join("\n\n")
    );
}

/// What `wip fmt` would write for an example, where that is not what the
/// reference shows. A block of statements is laid out as a `main`'s body,
/// four columns wider so that it fits in 80 on its own. A file that does
/// not parse is left alone: an example may be refused for its syntax.
fn unformatted(example: &Example) -> Option<String> {
    let style = |width| wip_fmt::Style {
        width,
        tabs: false,
        size: 4,
    };
    let mut wrong = Vec::new();
    for (index, (name, written)) in split_files(&example.code).iter().enumerate() {
        if !name.ends_with(".wip") {
            continue;
        }
        // The blank line before the next `// file:` is the page's, not the
        // file's.
        let text = &format!("{}\n", written.trim_end_matches('\n'));
        let formatted = if name.ends_with("package.wip") {
            wip_fmt::format(text, style(80), true)
        } else if index == 0 && !text.lines().any(starts_item) {
            let body: String = text.lines().map(indented).collect();
            wip_fmt::format(&format!("fn main() = {{\n{body}}}\n"), style(84), false).map(|out| {
                let lines: Vec<&str> = out.lines().collect();
                lines[1..lines.len() - 1]
                    .iter()
                    .map(|line| format!("{}\n", line.strip_prefix("    ").unwrap_or(line)))
                    .collect()
            })
        } else {
            wip_fmt::format(text, style(80), false)
        };
        match formatted {
            Ok(out) if out != *text => wrong.push(format!("// file: {name}\n{}", indent(&out))),
            Ok(_) | Err(wip_fmt::Error::Parse(_)) => {}
            Err(wip_fmt::Error::Bug(bug)) => wrong.push(format!("// file: {name}: {bug}")),
        }
    }
    if wrong.is_empty() {
        return None;
    }
    Some(format!(
        "{}:{}: `wip fmt` writes it as\n{}",
        example.file,
        example.line,
        wrong.join("\n")
    ))
}

fn indented(line: &str) -> String {
    if line.is_empty() {
        "\n".to_string()
    } else {
        format!("    {line}\n")
    }
}

/// The examples of every document in `dir`, in reading order.
fn collect(dir: &Path) -> Vec<Example> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("the reference is readable")
        .flatten()
        .map(|entry| dir.join(entry.file_name()))
        .filter(|file| file.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    let mut examples = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("a document is readable");
        let name = format!("docs/language/{}", file.file_name().unwrap().display());
        examples.extend(examples_in(&name, &text));
    }
    examples
}

/// The fenced Wip blocks of one document.
fn examples_in(file: &str, text: &str) -> Vec<Example> {
    let mut examples = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((at, line)) = lines.next() {
        let Some(info) = line.strip_prefix("```") else {
            continue;
        };
        let info = info.trim();
        // Every other fence — `ebnf`, `sh`, the output of a program — is
        // read past, to its end, so that nothing inside it is mistaken for
        // an example.
        let expected = match info {
            "wip" => Some(Expected::Compiles),
            "wip,run" => Some(Expected::Runs),
            "wip,ignore" => None,
            _ => info
                .strip_prefix("wip,error=")
                .map(|code| Expected::Refused(code.to_string())),
        };
        let mut code = String::new();
        for (_, line) in lines.by_ref() {
            if line.starts_with("```") {
                break;
            }
            code.push_str(line);
            code.push('\n');
        }
        if let Some(expected) = expected {
            examples.push(Example {
                file: file.to_string(),
                line: at + 1,
                expected,
                code,
            });
        }
    }
    examples
}

/// What the example is, as a whole program: a block that declares no items
/// is a `main`'s body, and one that declares items but no `main` gets an
/// empty one.
fn program(code: &str) -> String {
    let declares = code.lines().any(starts_item);
    if !declares {
        let body: String = code
            .lines()
            .map(|line| format!("    {line}\n"))
            .collect::<String>();
        return format!("fn main() = {{\n{body}}}\n");
    }
    if code.lines().any(|line| line.starts_with("fn main")) {
        return code.to_string();
    }
    format!("{code}\nfn main() = {{}}\n")
}

/// Whether a line of an example begins an item, rather than a statement.
fn starts_item(line: &str) -> bool {
    [
        "fn ",
        "pub ",
        "struct ",
        "enum ",
        "interface ",
        "extend ",
        "import ",
        "extern ",
        "type ",
        "view ",
        "@",
        "lend ",
        "var fn",
        "move fn",
        "static fn",
    ]
    .iter()
    .any(|start| line.starts_with(start))
}

/// The files an example is written in, as written: `main.wip`, and
/// whatever a `// file: …` line names after it.
fn split_files(code: &str) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = vec![("main.wip".to_string(), String::new())];
    for line in code.lines() {
        match line.trim().strip_prefix("// file: ") {
            Some(name) => files.push((name.trim().to_string(), String::new())),
            None => {
                let text = &mut files.last_mut().expect("a file is open").1;
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    files
}

/// The files an example is compiled from: the entry made a program, and
/// the files beside it, which are modules and declare items and nothing
/// else.
fn files(code: &str) -> Vec<(String, String)> {
    let mut files = split_files(code);
    files[0].1 = program(&files[0].1);
    files
}

/// Compiles one example, and says what went wrong if anything did.
fn check(example: &Example) -> Option<String> {
    let dir = TempDir::new().expect("a temporary directory");
    let entry = dir.path().join("main.wip");
    let written = files(&example.code);
    let program = written[0].1.clone();
    for (name, text) in &written {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("created");
        std::fs::write(&path, text).expect("written");
    }
    let command = match example.expected {
        Expected::Runs => "run",
        _ => "check",
    };
    let output = Command::new(env!("CARGO_BIN_EXE_wip"))
        .args([command, &entry.display().to_string()])
        .output()
        .expect("`wip` runs");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let wrong = match &example.expected {
        Expected::Compiles | Expected::Runs if output.status.success() => return None,
        Expected::Compiles => "this example does not compile".to_string(),
        Expected::Runs => "this example does not run to a successful end".to_string(),
        Expected::Refused(code) => {
            if !said.contains(&format!("[{code}]")) {
                format!("this example must be refused with {code}")
            } else {
                return None;
            }
        }
    };
    let all: String = written
        .iter()
        .skip(1)
        .map(|(name, text)| format!("// file: {name}\n{text}"))
        .collect();
    Some(format!(
        "{}:{}: {wrong}\n{}\n{}",
        example.file,
        example.line,
        indent(&format!("{program}{all}")),
        indent(said.trim_end())
    ))
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}\n"))
        .collect::<String>()
}
