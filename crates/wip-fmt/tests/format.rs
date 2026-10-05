//! `wip fmt`: what it writes, and that it never writes a
//! different program.

use std::path::{Path, PathBuf};

use wip_fmt::{Error, Style, format};

fn spaces(width: usize) -> Style {
    Style {
        width,
        tabs: false,
        size: 4,
    }
}

fn formatted(src: &str, style: Style) -> String {
    match format(src, style, false) {
        Ok(out) => out,
        Err(Error::Parse(d)) => panic!("does not parse: {d:?}"),
        Err(Error::Bug(what)) => panic!("{what}"),
    }
}

/// Every `.wip` file under `dir`.
fn wip_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a directory").flatten() {
        let path = entry.path();
        if path.is_dir() {
            wip_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "wip") {
            out.push(path);
        }
    }
}

/// The standard library and every case that parses: each is formatted as
/// the same program, with the same comments, stably — which `format`
/// checks itself, and reports as a bug where it is not so.
#[test]
fn every_file_in_the_repository_formats_as_itself() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    wip_files(&root.join("std"), &mut files);
    wip_files(&root.join("tests/cases/ok"), &mut files);
    assert!(files.len() > 200, "{} files", files.len());
    let mut bugs = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("readable");
        for style in [Style::default(), spaces(80)] {
            if let Err(Error::Bug(what)) = format(&text, style, false) {
                bugs.push(format!("{}: {what}", file.display()));
            }
        }
    }
    assert!(bugs.is_empty(), "{}", bugs.join("\n"));
}

#[test]
fn tabs_by_default_and_spaces_when_asked() {
    let src = "fn main() = {\n  val x = 1\n}\n";
    assert_eq!(
        formatted(src, Style::default()),
        "fn main() = {\n\tval x = 1\n}\n"
    );
    assert_eq!(
        formatted(src, spaces(120)),
        "fn main() = {\n    val x = 1\n}\n"
    );
}

#[test]
fn spacing_is_the_same_everywhere() {
    let src =
        "fn area( s:&Shape ):f64=match s{\n  .Circle(r)=>3.14*r*r\n    .Rect(w,h) => w*h\n}\n";
    assert_eq!(
        formatted(src, spaces(120)),
        "fn area(s: &Shape): f64 = match s {\n    .Circle(r) => 3.14 * r * r\n    .Rect(w, h) => w * h\n}\n"
    );
}

/// A call stays beside its `=` and breaks its arguments, one to a line,
/// with a trailing comma.
/// An arm whose block only leaves is written as the jump, as it may be
/// written; one with a comment in it keeps its block.
#[test]
fn an_arm_that_only_leaves_is_the_jump() {
    let src = "fn f(n: i64): i64 = match n {\n    0 => { return 1 }\n    1 => {\n        // why\n        return 2\n    }\n    _ => n\n}\n";
    assert_eq!(
        formatted(src, spaces(100)),
        "fn f(n: i64): i64 = match n {\n    0 => return 1\n    1 => {\n        // why\n        return 2\n    }\n    _ => n\n}\n"
    );
}

#[test]
fn a_list_that_does_not_fit_is_one_to_a_line() {
    let src = "fn main() = compute(alpha, beta, gamma, delta)\n";
    assert_eq!(
        formatted(src, spaces(30)),
        "fn main() = compute(\n    alpha,\n    beta,\n    gamma,\n    delta,\n)\n"
    );
}

#[test]
fn a_chain_breaks_after_its_operators() {
    let src = "fn main() = {\n    val total = first + second + third + fourth\n}\n";
    assert_eq!(
        formatted(src, spaces(30)),
        "fn main() = {\n    val total =\n        first +\n        second +\n        third +\n        fourth\n}\n"
    );
}

/// A line break separates a struct's fields and an enum's variants, as it
/// does a match's arms, so none is written with a comma; a variant's own
/// fields are a list, and keep theirs.
#[test]
fn declarations_have_no_commas() {
    let src = "struct Point {\n    x: i64,\n    y: i64 = 0,\n}\n\nenum Shape { Circle(r: f64, at: Point), Dot, }\n";
    assert_eq!(
        formatted(src, spaces(120)),
        "struct Point {\n    x: i64\n    y: i64 = 0\n}\n\nenum Shape {\n    Circle(r: f64, at: Point)\n    Dot\n}\n"
    );
}

#[test]
fn an_if_of_one_expression_each_is_written_with_then() {
    // A branch of one expression, a call, a value or a jump, needs no
    // braces; a branch of two statements keeps them, and so
    // does one with a comment in it.
    let src = "fn f(a: i64): i64 = {\n    if a > 0 { g() }\n    if a > 9 {\n        return 0\n    }\n    if a > 5 {\n        g()\n        h()\n    }\n    if a > 3 {\n        // why\n        return 1\n    }\n    if a < 0 { 1 } else { 2 }\n}\n";
    assert_eq!(
        formatted(src, spaces(80)),
        "fn f(a: i64): i64 = {\n    if a > 0 then g()\n    if a > 9 then return 0\n    if a > 5 {\n        g()\n        h()\n    }\n    if a > 3 {\n        // why\n        return 1\n    }\n    if a < 0 then 1 else 2\n}\n"
    );
}

#[test]
fn a_long_then_chain_breaks_before_each_else() {
    let src = "fn d(count: i64): String = if count == 0 { \"nothing\" } else if count == 1 { \"one item\" } else { \"many items, more than the line holds\" }\n";
    let want = "fn d(count: i64): String =\n    if count == 0 then \"nothing\"\n    else if count == 1 then \"one item\"\n    else \"many items, more than the line holds\"\n";
    assert_eq!(formatted(src, spaces(80)), want);
    assert_eq!(formatted(want, spaces(80)), want);
}

#[test]
fn a_condition_that_must_be_broken_keeps_the_braces() {
    // A `then` after a broken condition would leave the branch under the
    // condition's lines, looking like more of it.
    let src = "fn main() = {\n    if index < 0 || index >= occupants.len() || occupants[index].isSome() then panic(\"distinct\")\n    if a then b()\n}\n";
    let want = "fn main() = {\n    if index < 0 ||\n        index >= occupants.len() ||\n        occupants[index].isSome() {\n        panic(\"distinct\")\n    }\n    if a then b()\n}\n";
    assert_eq!(formatted(src, spaces(40)), want);
    assert_eq!(formatted(want, spaces(40)), want);

    // One condition of a chain that must be broken takes the whole chain.
    let src = "fn f(x: i64): i64 = if x < 0 then 1 else if x == firstLimit || x == secondLimit then 2 else 3\n";
    let want = "fn f(x: i64): i64 = if x < 0 {\n    1\n} else if x == firstLimit ||\n    x == secondLimit {\n    2\n} else {\n    3\n}\n";
    assert_eq!(formatted(src, spaces(40)), want);
    assert_eq!(formatted(want, spaces(40)), want);
}

#[test]
fn a_long_chain_breaks_after_its_dots() {
    // A line that ends in `.` goes on; one that starts with `.` would begin
    // a variant. Fields stay with the call after them.
    let src = "fn main() = {\n    val n = self.words.walk().filter(own (w) => w.len() > 2).map(own (w) => w.len()).count()\n}\n";
    let want = "fn main() = {\n    val n = self.words.walk().\n        filter(own (w) => w.len() > 2).\n        map(own (w) => w.len()).\n        count()\n}\n";
    assert_eq!(formatted(src, spaces(60)), want);
    assert_eq!(formatted(want, spaces(60)), want);

    // A chain that fits is one line, and one of two calls is laid out as
    // any call is.
    let short = "fn main() = {\n    val n = words.walk().count()\n}\n";
    assert_eq!(formatted(short, spaces(60)), short);
    let two = "fn main() = {\n    val n = self.items().copyWithin(from: first, to: second, count: n)\n}\n";
    assert_eq!(
        formatted(two, spaces(60)),
        "fn main() = {\n    val n = self.items().copyWithin(\n        from: first,\n        to: second,\n        count: n,\n    )\n}\n"
    );
}

#[test]
fn the_last_argument_hugs_the_parentheses() {
    // A struct built with named fields breaks inside its own parentheses,
    // as braces did.
    let src =
        "fn main() = {\n    list.push(Entry(name: firstValueHere, kind: secondValueHere))\n}\n";
    assert_eq!(
        formatted(src, spaces(40)),
        "fn main() = {\n    list.push(Entry(\n        name: firstValueHere,\n        kind: secondValueHere,\n    ))\n}\n"
    );
}

#[test]
fn a_tuple_is_written_as_one() {
    let src = "fn main() = {\n    val pair = (1,   \"one\")\n}\n";
    assert_eq!(
        formatted(src, spaces(120)),
        "fn main() = {\n    val pair = (1, \"one\")\n}\n"
    );
}

#[test]
fn comments_and_blank_lines_are_kept() {
    let src = "// A file.\n\nimport std::io\n\n\n\nfn main() = {\n    // First.\n    val x = 1 // one\n\n    // Then.\n    io::printInt(x)\n    // At the end.\n}\n";
    assert_eq!(
        formatted(src, spaces(120)),
        "// A file.\n\nimport std::io\n\nfn main() = {\n    // First.\n    val x = 1 // one\n\n    // Then.\n    io::printInt(x)\n    // At the end.\n}\n"
    );
}

#[test]
fn literals_are_written_as_they_were() {
    let src = "fn main() = {\n    val a = 0xFF_00\n    val b = \"tab\\there \\(a) and \\u{e9}\"\n    val c = 1_000.5\n}\n";
    assert_eq!(formatted(src, spaces(120)), src);
}

#[test]
fn tuples_and_positional_patterns_are_written_as_tuples() {
    let src = "fn f(p: (i64, i64)): i64 = match p {\n    (a, b) => a + b\n}\nfn g(o: Option<i64>): i64 = match o {\n    .Some(x) => x\n    .None => 0\n}\nfn h(): (i64, i64) = (1, 2).0\n";
    assert_eq!(formatted(src, spaces(120)), src);
}

#[test]
fn a_file_that_does_not_parse_is_left_alone() {
    assert!(matches!(
        format("fn main( = {}\n", Style::default(), false),
        Err(Error::Parse(_))
    ));
}

#[test]
fn a_package_wip_is_formatted_too() {
    let src = "@version( \"1.4.0\" )\n@depends(\"engine\",path=\"../engine\")\npackage   game\n";
    assert_eq!(
        format(src, Style::default(), true).expect("formats"),
        "@version(\"1.4.0\")\n@depends(\"engine\", path = \"../engine\")\npackage game\n"
    );
}

/// A text block is re-indented one deeper than the line it starts on, and
/// what each line has past the closing `"""`'s indentation stays.
#[test]
fn text_blocks_move_with_their_code() {
    let src = "fn main() = {\n  if true {\n val s = \"\"\"\n     one  \n\n       two\n     \"\"\"\n  }\n}\n";
    let want = "fn main() = {\n\tif true {\n\t\tval s = \"\"\"\n\t\t\tone\n\n\t\t\t  two\n\t\t\t\"\"\"\n\t}\n}\n";
    assert_eq!(formatted(src, Style::default()), want);
}

/// A text block as the last argument hugs the parentheses, plain or
/// interpolated, so a call prints a block of lines as it reads.
#[test]
fn a_text_block_hugs_the_parentheses() {
    for body in ["one\n\t\ttwo", "one \\(x)\n\t\ttwo"] {
        let src = format!("fn main() = {{\n\tio::println(\"\"\"\n\t\t{body}\n\t\t\"\"\")\n}}\n");
        assert_eq!(formatted(&src, Style::default()), src);
        let opened = src.replace("println(\"", "println(\n\t\t\"");
        assert_eq!(formatted(&opened, Style::default()), src);
    }
}

/// An arm too long for its line moves its body to the next, and keeps its
/// pattern on one; a text block stays on the arrow's line.
#[test]
fn a_long_arm_breaks_after_its_arrow() {
    let src = "fn f(p: P): String = match p {\n\t.Nested(parent, child) => \"\\(parent) and \\(child), which is inside it, cannot both be chosen for one move\"\n\t.Twice(path) => \"\\(path) twice\"\n}\n";
    let want = "fn f(p: P): String = match p {\n\t.Nested(parent, child) =>\n\t\t\"\\(parent) and \\(child), which is inside it, cannot both be chosen for one move\"\n\t.Twice(path) => \"\\(path) twice\"\n}\n";
    assert_eq!(formatted(src, Style::default()), want);
    assert_eq!(formatted(want, Style::default()), want);
    let block = "fn f(p: P): String = match p {\n\t.Long(path) => \"\"\"\n\t\t\\(path) is a very long name that goes on and on past the width of a line\n\t\t\"\"\"\n}\n";
    assert_eq!(formatted(block, Style::default()), block);
}

/// An array of numbers too long for a line fills its lines, as Prettier
/// fills one, rather than taking a line for each number; anything else in
/// it, or a comment, and each element has its line again.
#[test]
fn tables_of_numbers_fill_their_lines() {
    let numbers: Vec<String> = (0..40).map(|n| format!("0x{n:02X}")).collect();
    let src = format!("val TABLE: [u8; 40] = [{}]\n", numbers.join(", "));
    let out = formatted(&src, spaces(40));
    assert_eq!(
        out,
        "val TABLE: [u8; 40] = [\n    0x00, 0x01, 0x02, 0x03, 0x04, 0x05,\n    0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,\n    0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11,\n    0x12, 0x13, 0x14, 0x15, 0x16, 0x17,\n    0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,\n    0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23,\n    0x24, 0x25, 0x26, 0x27,\n]\n"
    );
    let mixed = "val MIXED: [i64; 3] = [1, -2, 3]\n";
    assert_eq!(formatted(mixed, spaces(100)), mixed);
    let named = "val NAMED = [first, second, third]\n";
    assert_eq!(
        formatted(named, spaces(20)),
        "val NAMED = [\n    first,\n    second,\n    third,\n]\n"
    );
}
