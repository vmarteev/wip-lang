//! Synthetic programs for measuring compile time.
//!
//! [`generate`] writes one program twice, in Wip and in C, so that `wip` can
//! be timed against `clang -O0` doing the same work. The program is made of
//! independent units. Each has a struct, an enum with payloads, a `match`,
//! loops, an `own` allocation, a `&var` parameter and a `defer`. `main` calls
//! every unit through group functions and prints two numbers, which the two
//! versions must agree on (see the test below).

/// Units per group function.
const GROUP: usize = 100;

pub struct Generated {
    pub wip: String,
    pub c: String,
    pub wip_lines: usize,
    pub c_lines: usize,
}

/// A program of about `lines` lines of Wip, and the same program in C.
pub fn generate(lines: usize) -> Generated {
    generate_repeated(lines, 1)
}

/// The same program, with `main` running the whole computation `repeat`
/// times, so that a run lasts long enough to time.
/// Each time from a seed of its own, the first 1, as the program spread
/// over modules has it: from the same seed, an optimiser may compute the
/// work once and hoist it out of the loop, which LLVM's build of the Wip
/// did.
pub fn generate_repeated(lines: usize, repeat: usize) -> Generated {
    let mut wip = String::from(WIP_PRELUDE);
    let mut c = String::from(C_PRELUDE);
    let mut wip_lines = count_lines(WIP_PRELUDE);
    let mut units = 0;
    // Each unit also costs a line in its group function.
    while units == 0 || wip_lines + units < lines {
        let text = fill(WIP_UNIT, units);
        wip_lines += count_lines(&text);
        wip.push_str(&text);
        c.push_str(&fill(C_UNIT, units));
        units += 1;
    }

    let groups = units.div_ceil(GROUP);
    for g in 0..groups {
        wip.push_str(&format!(
            "fn group{g}(seed: i64, count: &var i64): i64 = {{\n    var total = 0\n"
        ));
        c.push_str(&format!(
            "static int64_t group{g}(int64_t seed, int64_t *count) {{\n    int64_t total = 0;\n"
        ));
        for k in g * GROUP..((g + 1) * GROUP).min(units) {
            wip.push_str(&format!(
                "    total = total + unit{k}(seed + {k}, &var count)\n"
            ));
            c.push_str(&format!(
                "    total = total + unit{k}(seed + {k}, count);\n"
            ));
        }
        wip.push_str("    total\n}\n\n");
        c.push_str("    return total;\n}\n\n");
    }

    wip.push_str("fn main(): i64 = {\n    var count = 0\n    var total = 0\n    var r = 0\n");
    c.push_str(
        "int main(void) {\n    int64_t count = 0;\n    int64_t total = 0;\n    int64_t r = 0;\n",
    );
    wip.push_str(&format!("    while r < {repeat} {{\n"));
    c.push_str(&format!("    while (r < {repeat}) {{\n"));
    for g in 0..groups {
        wip.push_str(&format!(
            "        total = total + group{g}(r + 1, &var count)\n"
        ));
        c.push_str(&format!(
            "        total = total + group{g}(r + 1, &count);\n"
        ));
    }
    wip.push_str("        r = r + 1\n    }\n");
    c.push_str("        r = r + 1;\n    }\n");
    wip.push_str("    printf(\"%lld\\n\", total)\n    printf(\"%lld\\n\", count)\n    0\n}\n");
    c.push_str("    wip_print_i64(total);\n    wip_print_i64(count);\n    return 0;\n}\n");

    Generated {
        wip_lines: count_lines(&wip),
        c_lines: count_lines(&c),
        wip,
        c,
    }
}

/// A program of `functions` functions, each with `defers` deferred
/// assignments and `returns` early returns, for measuring what it costs to
/// emit deferred code again at every exit. With `deferred` false, the same
/// assignments are ordinary statements at the top of each function. They run
/// once on every path either way, so both versions print the same numbers.
pub fn generate_defers(functions: usize, defers: usize, returns: usize, deferred: bool) -> String {
    let mut wip = String::from(WIP_PRELUDE);
    let keyword = if deferred { "defer " } else { "" };
    for k in 0..functions {
        wip.push_str(&format!("fn f{k}(n: i64, c: &var i64): i64 = {{\n"));
        for d in 1..=defers {
            wip.push_str(&format!("    {keyword}c = c + {d}\n"));
        }
        for r in 0..returns {
            wip.push_str(&format!(
                "    if n == {r} {{\n        return {}\n    }}\n",
                r * 10 + k
            ));
        }
        wip.push_str(&format!("    n + {k}\n}}\n\n"));
    }
    wip.push_str("fn main(): i64 = {\n    var c = 0\n    var total = 0\n");
    for k in 0..functions {
        wip.push_str(&format!(
            "    total = total + f{k}({}, &var c)\n",
            k % (returns + 1)
        ));
    }
    wip.push_str("    printf(\"%lld\\n\", total)\n    printf(\"%lld\\n\", c)\n    0\n}\n");
    wip
}

/// The same units spread over `modules` modules, as `(path, text)` pairs
/// relative to the program's root: `main.wip`, then `p0/units.wip` and so on.
/// Each module becomes its own object file on its own
/// thread, which is what this is for. The units and seeds are the ones
/// `generate` uses, so the program prints the same two numbers.
pub fn generate_modules(lines: usize, modules: usize) -> Vec<(String, String)> {
    let modules = modules.max(1);
    let mut units = 0;
    let mut written = 0;
    while units == 0 || written + units < lines {
        written += count_lines(&fill(WIP_UNIT, units));
        units += 1;
    }
    let groups = units.div_ceil(GROUP);
    // A unit lives in the module of the group that calls it.
    let mut files = vec![String::from("// Generated by `wip-bench`. Do not edit.\n\n"); modules];
    for k in 0..units {
        files[(k / GROUP) % modules].push_str(&fill(WIP_UNIT, k));
    }
    for g in 0..groups {
        let file = &mut files[g % modules];
        file.push_str(&format!(
            "pub fn group{g}(seed: i64, count: &var i64): i64 = {{\n    var total = 0\n"
        ));
        for k in g * GROUP..((g + 1) * GROUP).min(units) {
            file.push_str(&format!(
                "    total = total + unit{k}(seed + {k}, &var count)\n"
            ));
        }
        file.push_str("    total\n}\n\n");
    }
    let mut main = String::from("// Generated by `wip-bench`. Do not edit.\n\nimport std::io\n");
    for p in 0..modules {
        main.push_str(&format!("import p{p}\n"));
    }
    main.push_str("\nfn main(): i64 = {\n    var count = 0\n    var total = 0\n");
    for g in 0..groups {
        main.push_str(&format!(
            "    total = total + p{}::group{g}(1, &var count)\n",
            g % modules
        ));
    }
    main.push_str("    io::printInt(total)\n    io::printInt(count)\n    0\n}\n");
    let mut out = vec![("main.wip".to_string(), main)];
    for (p, text) in files.into_iter().enumerate() {
        out.push((format!("p{p}/units.wip"), text));
    }
    out
}

fn count_lines(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count()
}

/// A unit template with `$k` replaced by the unit's number, and `$c1` to
/// `$c8` by small constants that vary from unit to unit.
fn fill(template: &str, k: usize) -> String {
    let mut text = template.replace("$k", &k.to_string());
    for i in 1..=8 {
        let value = (k * 7 + i * 13) % 9 + 1;
        // `$c1` is compared with a remainder of 3.
        let value = if i == 1 { value % 3 } else { value };
        text = text.replace(&format!("$c{i}"), &value.to_string());
    }
    text
}

const WIP_PRELUDE: &str = "\
// Generated by `wip-bench`. Do not edit.

// It prints through C's `printf`, as the C version of it does, so that
// both do the same work.
extern \"C\" {
    fn printf(format: cstring, ...): c_int
}

";

const C_PRELUDE: &str = "\
// Generated by `wip-bench`. Do not edit.

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

/* What `io::printInt` does, in C. */
static void wip_print_i64(int64_t value) {
    printf(\"%lld\\n\", (long long)value);
}

/* What the prelude's allocator does, in C: a numbered header in front
   of the memory, and the counters, changed atomically. */
static int64_t wip_made, wip_live;

static void *wip_alloc(int64_t size) {
    char *block = malloc((size_t)(size <= 0 ? 1 : size) + 16);
    if (!block) {
        abort();
    }
    *(int64_t *)block = __atomic_add_fetch(&wip_made, 1, __ATOMIC_SEQ_CST);
    __atomic_add_fetch(&wip_live, 1, __ATOMIC_SEQ_CST);
    return block + 16;
}

static void wip_free(void *ptr) {
    if (ptr) {
        __atomic_sub_fetch(&wip_live, 1, __ATOMIC_SEQ_CST);
        free((char *)ptr - 16);
    }
}

";

const WIP_UNIT: &str = "\
struct Point$k {
    x: i64
    y: i64
}

enum Shape$k {
    Square(corner: Point$k, side: i64)
    Rect(min: Point$k, max: Point$k)
    Empty
}

fn area$k(s: &Shape$k): i64 = match s {
    .Square(side, ..) => side * side
    .Rect(min, max) => (max.x - min.x) * (max.y - min.y)
    .Empty => 0
}

fn shift$k(p: &var Point$k, dx: i64, dy: i64) = {
    p.x = p.x + dx
    p.y = p.y + dy
}

fn sum_to$k(n: i64): i64 = {
    var total = 0
    var i = 0
    while i < n {
        if i % 3 == $c1 {
            total = total + i * $c2
        } else {
            total = total - i
        }
        i = i + 1
    }
    total
}

fn boxed$k(seed: i64): own<Shape$k> = {
    val corner = Point$k(x: seed, y: seed + $c3)
    own Shape$k::Square(corner: corner, side: seed % $c4 + 1)
}

fn unit$k(seed: i64, count: &var i64): i64 = {
    defer count = count + 1
    var p = Point$k(x: seed, y: $c5)
    shift$k(&var p, $c6, seed % 7)
    val shapes = [
        Shape$k::Rect(min: Point$k(x: 0, y: 0), max: p),
        Shape$k::Square(corner: p, side: $c7),
        Shape$k::Empty,
    ]
    var total = sum_to$k(seed % 10 + $c8)
    var i = 0
    while i < shapes.len() {
        total = total + area$k(&shapes[i])
        i = i + 1
    }
    val b = boxed$k(seed)
    total + area$k(&b)
}

";

const C_UNIT: &str = "\
typedef struct Point$k {
    int64_t x;
    int64_t y;
} Point$k;

typedef struct Shape$k {
    uint8_t tag;
    union {
        struct {
            Point$k corner;
            int64_t side;
        } square;
        struct {
            Point$k min;
            Point$k max;
        } rect;
    } u;
} Shape$k;

static int64_t area$k(const Shape$k *s) {
    switch (s->tag) {
    case 0:
        return s->u.square.side * s->u.square.side;
    case 1:
        return (s->u.rect.max.x - s->u.rect.min.x) * (s->u.rect.max.y - s->u.rect.min.y);
    default:
        return 0;
    }
}

static void shift$k(Point$k *p, int64_t dx, int64_t dy) {
    p->x = p->x + dx;
    p->y = p->y + dy;
}

static int64_t sum_to$k(int64_t n) {
    int64_t total = 0;
    int64_t i = 0;
    while (i < n) {
        if (i % 3 == $c1) {
            total = total + i * $c2;
        } else {
            total = total - i;
        }
        i = i + 1;
    }
    return total;
}

static Shape$k *boxed$k(int64_t seed) {
    Point$k corner = {seed, seed + $c3};
    Shape$k *s = wip_alloc(sizeof *s);
    s->tag = 0;
    s->u.square.corner = corner;
    s->u.square.side = seed % $c4 + 1;
    return s;
}

static int64_t unit$k(int64_t seed, int64_t *count) {
    Point$k p = {seed, $c5};
    shift$k(&p, $c6, seed % 7);
    Shape$k shapes[3];
    shapes[0].tag = 1;
    shapes[0].u.rect.min = (Point$k){0, 0};
    shapes[0].u.rect.max = p;
    shapes[1].tag = 0;
    shapes[1].u.square.corner = p;
    shapes[1].u.square.side = $c7;
    shapes[2].tag = 2;
    int64_t total = sum_to$k(seed % 10 + $c8);
    int64_t i = 0;
    while (i < 3) {
        total = total + area$k(&shapes[i]);
        i = i + 1;
    }
    Shape$k *b = boxed$k(seed);
    int64_t result = total + area$k(b);
    *count = *count + 1;
    wip_free(b);
    return result;
}

";

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;
    use wip_lang::{SourceFile, TempDir};

    fn run(exe: &Path) -> String {
        let output = Command::new(exe).output().expect("the program runs");
        assert!(output.status.success(), "{} failed", exe.display());
        String::from_utf8(output.stdout).expect("the output is UTF-8")
    }

    /// The two versions of a small program compile without complaint and
    /// print the same numbers, so that the measurements compare the same
    /// work.
    #[test]
    fn wip_and_c_agree() {
        let program = generate(2_000);
        assert!(
            (1_950..2_100).contains(&program.wip_lines),
            "{} lines",
            program.wip_lines
        );
        let dir = TempDir::new().expect("a temporary directory");

        let source = SourceFile {
            name: "gen.wip".to_string(),
            text: program.wip,
        };
        let wip_exe = dir.path().join("wip");
        match wip_lang::build(&source, &wip_exe) {
            Ok(warnings) => assert!(warnings.is_empty(), "the generated Wip has warnings"),
            Err(_) => panic!("the generated Wip does not compile; `wip build` on it shows why"),
        }

        let c_path = dir.path().join("gen.c");
        std::fs::write(&c_path, &program.c).expect("the C file is written");
        let c_exe = dir.path().join("c");
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let status = Command::new(&cc)
            .arg("-o")
            .arg(&c_exe)
            .arg(&c_path)
            .arg("-lm")
            .status()
            .expect("the C compiler runs");
        assert!(status.success(), "the generated C does not compile");

        let wip_output = run(&wip_exe);
        assert_eq!(wip_output.lines().count(), 2, "{wip_output}");
        assert_eq!(wip_output, run(&c_exe));
    }

    /// Spread over several modules, compiled separately and linked, the
    /// program prints what the single-module program prints.
    #[test]
    fn modules_agree_with_one_module() {
        let dir = TempDir::new().expect("a temporary directory");
        let root = dir.path().join("program");
        for (path, text) in generate_modules(2_000, 3) {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().expect("a directory")).expect("created");
            std::fs::write(&file, text).expect("written");
        }
        let mut loaded = wip_lang::load(&root.join("main.wip")).expect("loaded");
        let spread_exe = dir.path().join("spread");
        let options = wip_lang::BuildOptions::default();
        match wip_lang::build_loaded(
            &mut loaded,
            &spread_exe,
            &options,
            &mut wip_lang::Timings::default(),
        ) {
            Ok(warnings) => assert!(warnings.is_empty(), "the generated Wip has warnings"),
            Err(_) => panic!("the generated modules do not compile; `wip build` shows why"),
        }

        let single = SourceFile {
            name: "gen.wip".to_string(),
            text: generate(2_000).wip,
        };
        let single_exe = dir.path().join("single");
        assert!(wip_lang::build(&single, &single_exe).is_ok());
        let spread = run(&spread_exe);
        assert_eq!(spread.lines().count(), 2, "{spread}");
        assert_eq!(spread, run(&single_exe));
    }

    /// The two versions of the `defer` program compile without complaint and
    /// print the same numbers.
    #[test]
    fn defers_and_statements_agree() {
        let dir = TempDir::new().expect("a temporary directory");
        let outputs: Vec<String> = [true, false]
            .into_iter()
            .map(|deferred| {
                let source = SourceFile {
                    name: "defers.wip".to_string(),
                    text: generate_defers(20, 3, 3, deferred),
                };
                let exe = dir.path().join(format!("defers_{deferred}"));
                match wip_lang::build(&source, &exe) {
                    Ok(warnings) => assert!(warnings.is_empty(), "the generated Wip has warnings"),
                    Err(_) => panic!("the generated Wip does not compile; `wip build` shows why"),
                }
                run(&exe)
            })
            .collect();
        assert_eq!(outputs[0].lines().count(), 2, "{}", outputs[0]);
        assert_eq!(outputs[0], outputs[1]);
    }
}
