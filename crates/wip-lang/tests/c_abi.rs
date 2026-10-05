//! Structs by value across the C boundary, checked against C itself.
//!
//! The compiler passes a struct to C the way the target's ABI says, with
//! no C in between. A wrong classification does not crash — it hands C
//! the wrong numbers — so this test generates structs of many shapes and,
//! for each, C that knows what every field should hold:
//!
//! - Wip passes one to C, and C checks it;
//! - C returns one, and C checks it again through a pointer, which has
//!   always been right;
//! - Wip passes one after enough scalars that the registers run out, and
//!   four of one shape with a `float` among them, so that on arm64 one
//!   part of the registers is left and the next struct does not fit;
//! - C calls a Wip `@export("C")` function with one, and checks what comes
//!   back, and calls the two above as exports too;
//! - Wip calls that exported function itself.
//!
//! It runs three times: as the compiler builds it, with `WIP_FORCE_SHIMS`,
//! which sends every call through a wrapper in C, the path that was the
//! only one before, and as a release build, which is LLVM's where there
//! is a `clang` for it. All must pass, on every target the
//! gate runs.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use wip_lang::TempDir;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Int { bits: u32, signed: bool },
    F32,
    F64,
    Bool,
}

/// A struct, or a union, whose fields all start at 0.
struct Shape {
    union: bool,
    fields: Vec<Field>,
}

#[derive(Clone)]
enum Field {
    Prim(Kind),
    Array(Kind, u32),
    Nested(usize),
}

impl Kind {
    fn wip(self) -> String {
        match self {
            Kind::Int { bits, signed } => format!("{}{bits}", if signed { "i" } else { "u" }),
            Kind::F32 => "f32".into(),
            Kind::F64 => "f64".into(),
            Kind::Bool => "bool".into(),
        }
    }

    fn c(self) -> String {
        match self {
            Kind::Int { bits, signed } => {
                format!("{}int{bits}_t", if signed { "" } else { "u" })
            }
            Kind::F32 => "float".into(),
            Kind::F64 => "double".into(),
            Kind::Bool => "_Bool".into(),
        }
    }

    fn size(self) -> u32 {
        match self {
            Kind::Int { bits, .. } => bits / 8,
            Kind::F32 => 4,
            Kind::F64 => 8,
            Kind::Bool => 1,
        }
    }

    /// The value the `n`th scalar of shape `k` holds: small, exact in a
    /// `float`, and different from its neighbours.
    fn value(self, k: usize, n: usize) -> (String, String) {
        match self {
            Kind::Int { signed, .. } => {
                let v = ((k * 7 + n * 13 + 1) % 100) as i64;
                let v = if signed { v - 50 } else { v };
                (v.to_string(), v.to_string())
            }
            Kind::F32 | Kind::F64 => {
                let v = ((k * 3 + n * 5) % 40) as f64 * 0.25 + 0.5;
                let text = format!("{v:?}");
                let c = if self == Kind::F32 {
                    format!("{text}f")
                } else {
                    text.clone()
                };
                (text, c)
            }
            Kind::Bool => {
                let v = (k + n) % 2 == 1;
                (v.to_string(), (v as i32).to_string())
            }
        }
    }
}

const PRIMS: &[Kind] = &[
    Kind::Int {
        bits: 8,
        signed: false,
    },
    Kind::Int {
        bits: 8,
        signed: true,
    },
    Kind::Int {
        bits: 16,
        signed: true,
    },
    Kind::Int {
        bits: 32,
        signed: true,
    },
    Kind::Int {
        bits: 32,
        signed: false,
    },
    Kind::Int {
        bits: 64,
        signed: true,
    },
    Kind::F32,
    Kind::F64,
    Kind::Bool,
];

/// The shapes: the ones a real binding passes, then a spread of the rest,
/// then unions — each written and read through its first field, which the
/// generator makes as large as the union, so that every byte is checked.
fn shapes() -> Vec<Shape> {
    use Field::*;
    let f = Prim(Kind::F32);
    let d = Prim(Kind::F64);
    let i32_ = Prim(Kind::Int {
        bits: 32,
        signed: true,
    });
    let u8_ = Prim(Kind::Int {
        bits: 8,
        signed: false,
    });
    let i64_ = Prim(Kind::Int {
        bits: 64,
        signed: true,
    });
    let mut shapes: Vec<Vec<Field>> = vec![
        vec![f.clone(), f.clone()],                               // Vector2
        vec![f.clone(), f.clone(), f.clone()],                    // Vector3
        vec![f.clone(), f.clone(), f.clone(), f.clone()],         // Vector4, Rectangle
        vec![u8_.clone(), u8_.clone(), u8_.clone(), u8_.clone()], // Color
        vec![Array(Kind::F32, 16)],                               // Matrix
        vec![f.clone(); 5], // five floats: no longer homogeneous
        vec![d.clone(), d.clone()],
        vec![d.clone(), d.clone(), d.clone(), d.clone()], // 32 bytes, still homogeneous
        vec![f.clone(), i32_.clone()],                    // a mixed eightbyte
        vec![d.clone(), f.clone()],
        vec![f.clone(), d.clone()],
        vec![i64_.clone(), d.clone()],
        vec![
            Prim(Kind::Int {
                bits: 8,
                signed: true,
            }),
            d.clone(),
        ],
        vec![Array(
            Kind::Int {
                bits: 8,
                signed: false,
            },
            3,
        )], // three bytes
        vec![i64_.clone(), i64_.clone(), i64_.clone()], // 24 bytes of integers
        vec![i32_.clone(), i32_.clone(), i32_.clone()], // 12 bytes of integers
        vec![Prim(Kind::Bool), f.clone()],
        vec![Nested(0), Nested(0)], // two Vector2: four floats
        vec![Nested(1), f.clone()], // Vector3 and a float
        vec![Nested(3), f.clone()], // Color and a float
    ];
    // The rest from a fixed seed, so a failure is the same failure again.
    let mut seed: u64 = 0x5eed_cafe;
    let mut next = |below: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % below
    };
    for _ in 0..60 {
        let count = 1 + next(6);
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            let field = match next(10) {
                0 => Array(PRIMS[next(PRIMS.len())], 1 + next(4) as u32),
                1 if shapes.len() > 1 => Nested(next(shapes.len().min(20))),
                _ => Prim(PRIMS[next(PRIMS.len())]),
            };
            fields.push(field);
        }
        shapes.push(fields);
    }
    let mut shapes: Vec<Shape> = shapes
        .into_iter()
        .map(|fields| Shape {
            union: false,
            fields,
        })
        .collect();
    let union = |fields: Vec<Field>| Shape {
        union: true,
        fields,
    };
    let first_union = shapes.len();
    shapes.extend([
        union(vec![Array(Kind::F32, 2), f.clone()]), // two floats' worth: homogeneous
        union(vec![d.clone(), f.clone()]),           // two floating types: not
        union(vec![d.clone(), i32_.clone()]),        // an integer over a double
        union(vec![Array(Kind::F32, 4), Array(Kind::F32, 2)]), // four floats' worth
        union(vec![Array(Kind::F64, 3), i64_.clone()]), // over sixteen bytes
        union(vec![
            Array(
                Kind::Int {
                    bits: 32,
                    signed: true,
                },
                4,
            ),
            Array(Kind::F32, 4),
        ]), // sixteen bytes, integer and floating
        union(vec![Array(Kind::F32, 3), f.clone()]), // three floats' worth
        union(vec![Array(Kind::F32, 5), f.clone()]), // five: no longer homogeneous
    ]);
    // Unions from the same seed: a first field, and smaller ones over it.
    for _ in 0..12 {
        let kind = PRIMS[next(PRIMS.len())];
        let first = match next(2) {
            0 => Prim(kind),
            _ => Array(kind, 1 + next(4) as u32),
        };
        let room = match &first {
            Prim(kind) => kind.size(),
            Array(kind, len) => kind.size() * len,
            Nested(_) => unreachable!("a first field of scalars"),
        };
        let mut fields = vec![first];
        for _ in 0..1 + next(3) {
            let kind = PRIMS[next(PRIMS.len())];
            if kind.size() > room {
                continue;
            }
            let most = room / kind.size();
            fields.push(match next(2) {
                0 => Prim(kind),
                _ => Array(kind, 1 + next(most as usize) as u32),
            });
        }
        shapes.push(union(fields));
    }
    // Structs that hold a union.
    shapes.push(Shape {
        union: false,
        fields: vec![Nested(first_union), f.clone()], // three floats' worth
    });
    shapes.push(Shape {
        union: false,
        fields: vec![Nested(first_union + 1), i32_.clone()],
    });
    shapes
}

/// What one shape writes: its Wip literal, its C initialiser, and the C
/// that checks each scalar through a pointer.
struct Written {
    wip: String,
    c: String,
    checks: Vec<String>,
}

fn write_value(shapes: &[Shape], index: usize, k: usize, n: &mut usize, path: &str) -> Written {
    let mut wip = format!("S{index}(");
    let mut c = String::from("{ ");
    let mut checks = Vec::new();
    // A union is written through its first field, which C's initialiser
    // sets too when it names none.
    let written = match shapes[index].union {
        true => 1,
        false => shapes[index].fields.len(),
    };
    for (i, field) in shapes[index].fields[..written].iter().enumerate() {
        let name = format!("f{i}");
        let at = format!("{path}.{name}");
        if i > 0 {
            wip.push_str(", ");
            c.push_str(", ");
        }
        match field {
            Field::Prim(kind) => {
                let (w, cv) = kind.value(k, *n);
                *n += 1;
                let _ = write!(wip, "{name}: {w}");
                c.push_str(&cv);
                checks.push(format!("s{at} == {cv}"));
            }
            Field::Array(kind, len) => {
                let mut ws = Vec::new();
                let mut cs = Vec::new();
                for j in 0..*len {
                    let (w, cv) = kind.value(k, *n);
                    *n += 1;
                    checks.push(format!("s{at}[{j}] == {cv}"));
                    ws.push(w);
                    cs.push(cv);
                }
                let _ = write!(wip, "{name}: [{}]", ws.join(", "));
                let _ = write!(c, "{{ {} }}", cs.join(", "));
            }
            Field::Nested(inner) => {
                let inner = write_value(shapes, *inner, k, n, &at);
                let _ = write!(wip, "{name}: {}", inner.wip);
                c.push_str(&inner.c);
                checks.extend(inner.checks);
            }
        }
    }
    wip.push(')');
    c.push_str(" }");
    Written { wip, c, checks }
}

/// The program and its C, for every shape.
fn program() -> (String, String, usize) {
    let shapes = shapes();
    let mut wip = String::from("import std::io\n\nextern \"C\" {\n");
    let mut c = String::from("#include <stdint.h>\n\n");
    let mut tests = String::new();
    let ints: Vec<String> = (0..7).map(|i| format!("i{i}: i64")).collect();
    let doubles: Vec<String> = (0..8).map(|i| format!("d{i}: f64")).collect();
    let c_ints: Vec<String> = (0..7).map(|i| format!("int64_t i{i}")).collect();
    let c_doubles: Vec<String> = (0..8).map(|i| format!("double d{i}")).collect();
    let int_args: Vec<String> = (0..7).map(|i| (i + 1).to_string()).collect();
    let double_args: Vec<String> = (0..8).map(|i| format!("{}.5", i + 1)).collect();
    let int_checks: Vec<String> = (0..7).map(|i| format!("i{i} == {}", i + 1)).collect();
    let double_checks: Vec<String> = (0..8).map(|i| format!("d{i} == {}.5", i + 1)).collect();

    let keyword = |index: usize| match shapes[index].union {
        true => "union",
        false => "struct",
    };
    for (k, shape) in shapes.iter().enumerate() {
        let fields = &shape.fields;
        let s = format!("S{k}");
        // What C calls the type: `struct S3` or `union S3`.
        let t = format!("{} {s}", keyword(k));
        // The declaration, in Wip and in C.
        let _ = writeln!(wip, "    {t} {{");
        let _ = writeln!(c, "{t} {{");
        for (i, field) in fields.iter().enumerate() {
            let (w, cdecl) = match field {
                Field::Prim(kind) => (kind.wip(), format!("{} f{i}", kind.c())),
                Field::Array(kind, len) => (
                    format!("[{}; {len}]", kind.wip()),
                    format!("{} f{i}[{len}]", kind.c()),
                ),
                Field::Nested(inner) => (
                    format!("S{inner}"),
                    format!("{} S{inner} f{i}", keyword(*inner)),
                ),
            };
            let comma = if i + 1 < fields.len() { "," } else { "" };
            let _ = writeln!(wip, "        f{i}: {w}{comma}");
            let _ = writeln!(c, "    {cdecl};");
        }
        let _ = writeln!(wip, "    }}");
        let _ = writeln!(c, "}};\n");

        let value = write_value(&shapes, k, k, &mut 0, "");
        let checks = value.checks.join(" && ").replace("s.", "s->");
        let _ = writeln!(
            c,
            "int32_t check_ptr_{s}(const {t} *s) {{ return {checks}; }}\n\
             int32_t check_{s}({t} s) {{ return check_ptr_{s}(&s); }}\n\
             {t} make_{s}(void) {{ {t} s = {init}; return s; }}\n\
             int32_t late_{s}({ci}, {cd}, {t} s, int64_t tail) {{\n    \
                 return {ic} && {dc} && tail == 99 && check_ptr_{s}(&s);\n}}\n\
             int32_t four_{s}({t} a, {t} b, {t} c, float f, {t} d,\n    \
                 int64_t tail) {{\n    \
                 return check_ptr_{s}(&a) && check_ptr_{s}(&b) && check_ptr_{s}(&c) && f == 0.5f\n        \
                     && check_ptr_{s}(&d) && tail == 99;\n}}\n\
             int32_t wip_check_{s}({t} s);\n\
             {t} wip_echo_{s}({t} s, int64_t n);\n\
             int32_t wip_late_{s}({ci}, {cd}, {t} s, int64_t tail);\n\
             int32_t wip_four_{s}({t} a, {t} b, {t} c, float f, {t} d,\n    \
                 int64_t tail);\n\
             int32_t drive_{s}(void) {{\n    \
                 {t} s = make_{s}();\n    \
                 if (!wip_check_{s}(s)) return 0;\n    \
                 if (!wip_late_{s}(1, 2, 3, 4, 5, 6, 7, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, s, 99)) return 0;\n    \
                 if (!wip_four_{s}(s, s, s, 0.5f, s, 99)) return 0;\n    \
                 {t} back = wip_echo_{s}(s, 5);\n    \
                 return check_ptr_{s}(&back);\n}}\n",
            init = value.c,
            ci = c_ints.join(", "),
            cd = c_doubles.join(", "),
            ic = int_checks.join(" && "),
            dc = double_checks.join(" && "),
        );
        let _ = writeln!(
            wip,
            "    fn check_ptr_{s}(s: &{s}): i32\n    \
                 fn check_{s}(s: {s}): i32\n    \
                 fn make_{s}(): {s}\n    \
                 fn late_{s}({}, {}, s: {s}, tail: i64): i32\n    \
                 fn four_{s}(a: {s}, b: {s}, c: {s}, f: f32, d: {s}, tail: i64): i32\n    \
                 fn drive_{s}(): i32\n",
            ints.join(", "),
            doubles.join(", "),
        );
        let _ = writeln!(
            tests,
            "@export(\"C\")\n\
             fn wip_check_{s}(s: {s}): i32 = check_ptr_{s}(&s)\n\n\
             @export(\"C\")\n\
             fn wip_echo_{s}(s: {s}, n: i64): {s} = s\n\n\
             @export(\"C\")\n\
             fn wip_late_{s}({ints}, {doubles}, s: {s}, tail: i64): i32 =\n    \
                 if i0 == 1 && i6 == 7 && d0 == 1.5 && d7 == 8.5 && tail == 99 {{ check_ptr_{s}(&s) }} else {{ 0 }}\n\n\
             @export(\"C\")\n\
             fn wip_four_{s}(a: {s}, b: {s}, c: {s}, f: f32, d: {s}, tail: i64): i32 =\n    \
                 if f == 0.5 && tail == 99 && check_ptr_{s}(&a) == 1 && check_ptr_{s}(&b) == 1 &&\n        \
                     check_ptr_{s}(&c) == 1 {{ check_ptr_{s}(&d) }} else {{ 0 }}\n\n\
             fn test_{s}(): i64 = {{\n    \
                 var failed = 0\n    \
                 val s = {literal}\n    \
                 if check_{s}(s) != 1 {{ io::println(\"{s}: passed to C\"); failed += 1 }}\n    \
                 val made = make_{s}()\n    \
                 if check_ptr_{s}(&made) != 1 {{ io::println(\"{s}: returned by C\"); failed += 1 }}\n    \
                 if late_{s}({}, {}, s, 99) != 1 {{ io::println(\"{s}: after the registers\"); failed += 1 }}\n    \
                 if four_{s}(s, s, s, 0.5, s, 99) != 1 {{ io::println(\"{s}: four, a float among them\"); failed += 1 }}\n    \
                 if drive_{s}() != 1 {{ io::println(\"{s}: C calling Wip\"); failed += 1 }}\n    \
                 val echoed = wip_echo_{s}(s, 5)\n    \
                 if check_ptr_{s}(&echoed) != 1 {{ io::println(\"{s}: Wip calling its export\"); failed += 1 }}\n    \
                 failed\n}}\n",
            int_args.join(", "),
            double_args.join(", "),
            literal = value.wip,
            ints = ints.join(", "),
            doubles = doubles.join(", "),
        );
    }
    wip.push_str("}\n\n");
    wip.push_str(&tests);
    wip.push_str("fn main(): i64 = {\n    var failed = 0\n");
    for k in 0..shapes.len() {
        let _ = writeln!(wip, "    failed += test_S{k}()");
    }
    wip.push_str("    io::println(\"\\(failed) failed\")\n    failed\n}\n");
    (wip, c, shapes.len())
}

/// How a program is built and run.
#[derive(Clone, Copy, Debug)]
enum Way {
    /// A debug build, as the compiler builds it: Cranelift's, calling C
    /// itself.
    Directly,
    /// Every call into C through a wrapper in C.
    ThroughWrappers,
    /// A release build: LLVM's, where there is a `clang` for it.
    Released,
}

const WAYS: [Way; 3] = [Way::Directly, Way::ThroughWrappers, Way::Released];

fn run(dir: &Path, way: Way) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wip"));
    command.current_dir(dir).arg("run");
    if let Way::Released = way {
        command.arg("--release");
    }
    command.arg("main.wip");
    match way {
        Way::ThroughWrappers => command.env("WIP_FORCE_SHIMS", "1"),
        _ => command.env_remove("WIP_FORCE_SHIMS"),
    };
    let output = command.output().expect("`wip` runs");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn structs_cross_by_value_the_way_c_passes_them() {
    let dir = TempDir::new().expect("a temporary directory");
    let (wip, c, count) = program();
    std::fs::write(dir.path().join("main.wip"), &wip).expect("written");
    std::fs::write(dir.path().join("structs.c"), &c).expect("written");
    assert!(count >= 80, "the corpus has {count} shapes");
    for way in WAYS {
        let (code, stdout, stderr) = run(dir.path(), way);
        assert_eq!(
            (code, stdout.as_str()),
            (Some(0), "0 failed\n"),
            "{way:?}:\n{stdout}{stderr}"
        );
    }
}

/// A union crosses from C into an export like a struct,
/// but an export that takes a struct by value cannot also be a Wip
/// function value: C passes it in registers and Wip passes its address.
/// Whichever backend builds it.
#[test]
fn an_export_taken_as_a_value_is_refused() {
    let dir = TempDir::new().expect("a temporary directory");
    std::fs::write(
        dir.path().join("main.wip"),
        "extern \"C\" {\n    union Bits {\n        whole: i64,\n        half: i32\n    }\n}\n\n\
         @export(\"C\")\nfn takesBits(bits: Bits): i64 = bits.whole\n\n\
         @export(\"C\")\nfn takesPair(pair: Pair): i64 = pair.a\n\n\
         extern struct Pair {\n    a: i64,\n    b: i64\n}\n\n\
         fn main(): i64 = {\n    val f = takesPair\n    f(Pair(a: 1, b: 2)) + takesBits(Bits(whole: 0))\n}\n",
    )
    .expect("written");
    for way in [Way::Directly, Way::Released] {
        let (code, _, stderr) = run(dir.path(), way);
        assert_ne!(code, Some(0), "{way:?}");
        assert!(!stderr.contains("takesBits"), "{way:?}: {stderr}");
        assert!(
            stderr.contains("E0901")
                && stderr.contains(
                    "`takesPair` is called by C, and cannot also be a Wip function value"
                ),
            "{way:?}: {stderr}"
        );
    }
}

/// One extra argument of a variadic call: its Wip expression, and what C
/// reads it back as once promoted — a letter for `va_arg`'s type and the
/// value, as the checker below parses them.
fn extra(kind: usize, k: usize, n: usize) -> (String, String) {
    let v = ((k * 11 + n * 7) % 90) as i64 - 30;
    let small = v.rem_euclid(100);
    let f = ((k * 3 + n * 5) % 40) as f64 * 0.25 - 2.0;
    match kind {
        0 => (format!("({v} as i8)"), format!("i{v}")),
        1 => (format!("({small} as u8)"), format!("i{small}")),
        2 => (format!("({} as i16)", v * 100), format!("i{}", v * 100)),
        3 => (format!("({small} as u16)"), format!("i{small}")),
        4 => (format!("({} as i32)", v * 1000), format!("i{}", v * 1000)),
        5 => (
            format!("({} as u32)", small * 1000),
            format!("u{}", small * 1000),
        ),
        6 => (
            format!("({} as i64)", v * 100_000_000),
            format!("l{}", v * 100_000_000),
        ),
        7 => (
            format!("({} as u64)", small * 1_000_000_000),
            format!("L{}", small * 1_000_000_000),
        ),
        8 => {
            let b = (k + n).is_multiple_of(2);
            (b.to_string(), format!("i{}", b as i32))
        }
        9 => (format!("({f:?} as f32)"), format!("d{f:?}")),
        10 => (format!("{f:?}"), format!("d{f:?}")),
        11 => (format!("\"s{k}x{n}\""), format!("ss{k}x{n}")),
        _ => unreachable!("twelve kinds"),
    }
}

/// The C that checks a call: it reads the extra arguments as the
/// specification says and compares each with what it should be, so a
/// wrong register or a wrong promotion is a wrong answer.
const VARIADIC_C: &str = r#"#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

static int check(const char *spec, va_list ap) {
    const char *p = spec;
    while (*p) {
        char type = *p++;
        const char *end = strchr(p, ';');
        char text[64];
        size_t n = (size_t)(end - p);
        memcpy(text, p, n);
        text[n] = 0;
        switch (type) {
            case 'i': if (va_arg(ap, int) != (int)strtoll(text, 0, 10)) return 0; break;
            case 'u': if (va_arg(ap, unsigned) != (unsigned)strtoull(text, 0, 10)) return 0; break;
            case 'l': if (va_arg(ap, long long) != strtoll(text, 0, 10)) return 0; break;
            case 'L': if (va_arg(ap, unsigned long long) != strtoull(text, 0, 10)) return 0; break;
            case 'd': if (va_arg(ap, double) != strtod(text, 0)) return 0; break;
            case 's': if (strcmp(va_arg(ap, const char *), text) != 0) return 0; break;
            default: return 0;
        }
        p = end + 1;
    }
    return 1;
}

int32_t v0(const char *spec, ...) {
    va_list ap; va_start(ap, spec); int r = check(spec, ap); va_end(ap);
    return r;
}

int32_t v1(int64_t a, const char *spec, ...) {
    va_list ap; va_start(ap, spec); int r = check(spec, ap); va_end(ap);
    return r && a == 11;
}

int32_t v2(double a, const char *spec, ...) {
    va_list ap; va_start(ap, spec); int r = check(spec, ap); va_end(ap);
    return r && a == 1.5;
}

int32_t v3(int8_t a, float b, const char *spec, ...) {
    va_list ap; va_start(ap, spec); int r = check(spec, ap); va_end(ap);
    return r && a == -3 && b == 0.25f;
}

int32_t vmany(int64_t i0, int64_t i1, int64_t i2, int64_t i3, int64_t i4, int64_t i5,
              int64_t i6, int64_t i7, double d0, double d1, double d2, double d3, double d4,
              double d5, double d6, double d7, const char *spec, ...) {
    va_list ap; va_start(ap, spec); int r = check(spec, ap); va_end(ap);
    return r && i0 == 1 && i1 == 2 && i2 == 3 && i3 == 4 && i4 == 5 && i5 == 6 && i6 == 7
        && i7 == 8 && d0 == 1.5 && d1 == 2.5 && d2 == 3.5 && d3 == 4.5 && d4 == 5.5
        && d5 == 6.5 && d6 == 7.5 && d7 == 8.5;
}
"#;

/// Calls that pass more than they declare, of every scalar type C
/// promotes and some it does not, after named arguments of each kind and
/// after enough of them that the registers are gone.
fn variadic_program() -> (String, usize) {
    let mut wip = String::from(
        "import std::io\n\n\
         extern \"C\" {\n    \
         fn v0(spec: cstring, ...): i32\n    \
         fn v1(a: i64, spec: cstring, ...): i32\n    \
         fn v2(a: f64, spec: cstring, ...): i32\n    \
         fn v3(a: i8, b: f32, spec: cstring, ...): i32\n    \
         fn vmany(i0: i64, i1: i64, i2: i64, i3: i64, i4: i64, i5: i64, i6: i64, i7: i64, \
         d0: f64, d1: f64, d2: f64, d3: f64, d4: f64, d5: f64, d6: f64, d7: f64, \
         spec: cstring, ...): i32\n}\n\n",
    );
    let mut seed: u64 = 0x0da7_a5e7;
    let mut next = |below: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % below
    };
    let calls = 250;
    let per_fn = 25;
    for chunk in 0..calls / per_fn {
        let _ = writeln!(wip, "fn calls{chunk}(): i64 = {{\n    var failed = 0");
        for k in chunk * per_fn..(chunk + 1) * per_fn {
            let count = next(13);
            let mut exprs = Vec::new();
            let mut spec = String::new();
            for n in 0..count {
                let (expr, read) = extra(next(12), k, n);
                exprs.push(expr);
                spec.push_str(&read);
                spec.push(';');
            }
            let named = match k % 5 {
                0 => String::new(),
                1 => "11, ".into(),
                2 => "1.5, ".into(),
                3 => "(-3 as i8), (0.25 as f32), ".into(),
                _ => "1, 2, 3, 4, 5, 6, 7, 8, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, ".into(),
            };
            let callee = ["v0", "v1", "v2", "v3", "vmany"][k % 5];
            let extras = if exprs.is_empty() {
                String::new()
            } else {
                format!(", {}", exprs.join(", "))
            };
            let _ = writeln!(
                wip,
                "    if {callee}({named}\"{spec}\"{extras}) != 1 {{ io::println(\"call {k}: {callee}({spec})\"); failed += 1 }}"
            );
        }
        wip.push_str("    failed\n}\n\n");
    }
    wip.push_str("fn main(): i64 = {\n    var failed = 0\n");
    for chunk in 0..calls / per_fn {
        let _ = writeln!(wip, "    failed += calls{chunk}()");
    }
    wip.push_str("    io::println(\"\\(failed) failed\")\n    failed\n}\n");
    (wip, calls)
}

#[test]
fn variadic_calls_pass_what_c_reads() {
    let dir = TempDir::new().expect("a temporary directory");
    let (wip, _) = variadic_program();
    std::fs::write(dir.path().join("main.wip"), &wip).expect("written");
    std::fs::write(dir.path().join("variadic.c"), VARIADIC_C).expect("written");
    for way in WAYS {
        let (code, stdout, stderr) = run(dir.path(), way);
        assert_eq!(
            (code, stdout.as_str()),
            (Some(0), "0 failed\n"),
            "{way:?}:\n{stdout}{stderr}"
        );
    }
}

/// C's variables, read and written where they are used rather than
/// through C of the compiler's writing: every width, each
/// written by one side and read by the other.
#[test]
fn c_variables_are_c_s_own() {
    let dir = TempDir::new().expect("a temporary directory");
    std::fs::write(
        dir.path().join("globals.c"),
        "#include <stdint.h>\n\
         int8_t small = -5;\n\
         uint16_t middle = 60000;\n\
         int32_t word = -70000;\n\
         int64_t wide = 5000000000;\n\
         float single = 0.25f;\n\
         double twice = 2.5;\n\
         _Bool flag = 1;\n\
         const char *name = \"C's\";\n\
         int32_t check(void) {\n    \
             return small == 7 && middle == 1234 && word == 42 && wide == -9000000000\n        \
                 && single == 1.5f && twice == -0.75 && flag == 0;\n\
         }\n",
    )
    .expect("written");
    std::fs::write(
        dir.path().join("main.wip"),
        "import std::io\n\n\
         extern \"C\" {\n    \
             var small: i8\n    var middle: u16\n    var word: i32\n    var wide: i64\n    \
             var single: f32\n    var twice: f64\n    var flag: bool\n    val name: cstring\n    \
             fn check(): i32\n}\n\n\
         fn main(): i64 = {\n    \
             val read = small == -5 && middle == 60000 && word == -70000 && wide == 5000000000 &&\n        \
                 single == 0.25 && twice == 2.5 && flag && name.toStr() == \"C's\"\n    \
             small = 7\n    middle = 1234\n    word = 42\n    wide = -9000000000\n    \
             single = 1.5\n    twice = -0.75\n    flag = false\n    \
             io::println(\"\\(read) \\(check())\")\n    0\n}\n",
    )
    .expect("written");
    for way in WAYS {
        let (code, stdout, stderr) = run(dir.path(), way);
        assert_eq!(
            (code, stdout.as_str()),
            (Some(0), "true 1\n"),
            "{way:?}:\n{stdout}{stderr}"
        );
    }
}

/// A program that asks for no macro, no `static inline`, no variable
/// behind a header and no opaque field gets no C of the compiler's writing,
/// on any target: std's own calls into C are all made directly — floats
/// included, which on x86-64 means `strfromd` rather than a variadic call
/// that passes a double.
#[test]
fn std_needs_no_generated_c() {
    let dir = TempDir::new().expect("a temporary directory");
    let cache = dir.path().join("cache");
    std::fs::write(
        dir.path().join("main.wip"),
        "import std::io\nimport std::fs\n\n\
         fn main(): i64 = {\n    \
             io::printFloat(0.1 + 0.2)\n    \
             io::println(\"\\(1234567.0) \\(0.5) \\(-2.5e-10)\")\n    \
             io::printInt(42)\n    \
             val found = fs::exists(\"main.wip\")\n    \
             val unset = io::env(\"WIP_NOT_SET\").isNone()\n    \
             io::println(\"\\(found) \\(unset)\")\n    \
             0\n}\n",
    )
    .expect("written");
    let output = Command::new(env!("CARGO_BIN_EXE_wip"))
        .current_dir(dir.path())
        .args(["run", "main.wip"])
        .env("WIP_CACHE_DIR", &cache)
        .env_remove("WIP_FORCE_SHIMS")
        .output()
        .expect("`wip` runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "0.30000000000000004\n1234567 0.5 -2.5e-10\n42\ntrue true\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fn generated(dir: &Path) -> Vec<std::path::PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .flat_map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    generated(&path)
                } else if path.file_name().is_some_and(|n| n == "wip_shims.c") {
                    vec![path]
                } else {
                    Vec::new()
                }
            })
            .collect()
    }
    let found = generated(&cache);
    assert!(found.is_empty(), "C of the compiler's writing: {found:?}");
}
