//! The MIR of small programs, as snapshots. A change to a snapshot is a
//! change to what every backend compiles.

use wip_syntax::Interner;

use crate::*;

const PROGRAM: &str = "extern \"C\" {
    fn wip_print_i64(x: i64)
}

enum Link {
    More(value: i64, next: own<Link>)
    End
}

// A `defer` and a drop at the end of the same block: the drop comes first,
// since it was registered last.
fn scoped(n: i64): i64 = {
    defer wip_print_i64(n)
    val list = own Link::More(value: n, next: own Link::End)
    n
}

// A bounds check, and a `match` whose arm binds a field by alias.
fn head(xs: &[i64], link: &Link): i64 = match link {
    .More(value, ..) => value + xs[0]
    .End => 0
}
";

/// The type-checked program, and its interner.
fn checked() -> (wip_hir::Program, Interner) {
    let mut interner = Interner::new();
    let lexed = wip_syntax::lex(PROGRAM, &mut interner);
    let parsed = wip_syntax::parse(PROGRAM, &lexed);
    let lowered = wip_hir::lower_file(&parsed.ast, &interner);
    assert!(
        !lowered.diagnostics.iter().any(|d| d.is_error()),
        "{:#?}",
        lowered.diagnostics
    );
    (lowered.program, interner)
}

#[test]
fn drops_defer_checks_and_match() {
    let (program, interner) = checked();
    let mut out = String::new();
    for (_, def) in program.fns.iter() {
        let Some(body) = &def.body else { continue };
        let mir = lower_fn(&program, &interner, def, body);
        validate(&mir);
        out.push_str(&dump(&program, &interner, interner.resolve(def.name), &mir));
    }
    insta::assert_snapshot!(out);
}

/// The drop function of `own<Link>` follows `next` in a loop, not by
/// calling itself.
#[test]
fn drop_function_loops_over_a_chain() {
    let (program, interner) = checked();
    let (link, _) = program
        .enums
        .iter()
        .find(|(_, e)| interner.resolve(e.name) == "Link")
        .expect("the enum");
    let link = program
        .types
        .find(TyKind::Enum(link, wip_hir::TyList::EMPTY))
        .expect("its type");
    let mir = lower_drop_fn(&program, &interner, link);
    validate(&mir);
    let text = dump(&program, &interner, "drop_link", &mir);
    assert!(!text.contains("drop_fn<"), "no call to itself:\n{text}");
    insta::assert_snapshot!(text);
}

/// Two places that once dropped twice (`ok/216` and `ok/218` run them too): a
/// field moved out of a field, whose local then drops only what is left of it;
/// and an arm that took its value out of a temporary and leaves by `break`,
/// after which the temporary is not dropped again.
const DROPS: &str = "struct Node { value: i64 }
struct Inner { kept: own<Node>, left: own<Node> }
struct Outer { inner: Inner, other: own<Node> }
enum Maybe { Some(node: own<Node>), None }

fn made(): Maybe = .Some(own Node { value: 1 })

fn nested() = {
    val outer = Outer {
        inner: Inner { kept: own Node { value: 1 }, left: own Node { value: 2 } },
        other: own Node { value: 3 },
    }
    val taken = move outer.inner.kept
}

fn leaving() = {
    while true {
        match made() {
            .Some(node) => {
                break
            }
            .None => {
                break
            }
        }
    }
}
";

#[test]
fn a_move_out_of_a_field_and_out_of_an_arm_drops_once() {
    let mut interner = Interner::new();
    let lexed = wip_syntax::lex(DROPS, &mut interner);
    let parsed = wip_syntax::parse(DROPS, &lexed);
    let lowered = wip_hir::lower_file(&parsed.ast, &interner);
    assert!(
        !lowered.diagnostics.iter().any(|d| d.is_error()),
        "{:#?}",
        lowered.diagnostics
    );
    let program = lowered.program;
    let mut out = String::new();
    for name in ["nested", "leaving"] {
        let (_, def) = program
            .fns
            .iter()
            .find(|(_, def)| interner.resolve(def.name) == name)
            .expect("declared");
        let body = def.body.as_ref().expect("a body");
        let mir = lower_fn(&program, &interner, def, body);
        validate(&mir);
        out.push_str(&dump(&program, &interner, name, &mir));
    }
    insta::assert_snapshot!(out);
}

const SMALL: &str = "struct Pair {
    a: i64
    b: f64
}

enum Maybe {
    Some(value: i64)
    Nothing
}

@inline
fn found(n: i64): Maybe = if n > 0 { .Some(value: n) } else { .Nothing }

// A struct copied whole and written by field, and an enum an inlined call
// answers and a `match` takes apart: each split into locals in a release
// build.
fn small(n: i64): i64 = {
    val pair = Pair(a: n, b: 1.5)
    var copy = pair
    copy.a += 1
    match found(copy.a) {
        .Some(value) => value
        .Nothing => 0
    }
}
";

#[test]
fn small_aggregates_become_scalars() {
    let mut interner = Interner::new();
    let lexed = wip_syntax::lex(SMALL, &mut interner);
    let parsed = wip_syntax::parse(SMALL, &lexed);
    let lowered = wip_hir::lower_file(&parsed.ast, &interner);
    assert!(
        !lowered.diagnostics.iter().any(|d| d.is_error()),
        "{:#?}",
        lowered.diagnostics
    );
    let program = lowered.program;
    let (_, def) = program
        .fns
        .iter()
        .find(|(_, def)| interner.resolve(def.name) == "small")
        .expect("the function is there");
    let mut mir = lower_fn(&program, &interner, def, def.body.as_ref().expect("a body"));
    inline(&program, &interner, &mut mir);
    simplify(&program, &mut mir);
    scalars(&program, &mut mir);
    validate(&mir);
    // No local of the struct or the enum is left: each became its fields.
    let left: Vec<String> = mir
        .locals
        .iter()
        .filter(|decl| {
            matches!(
                program.types.kind(decl.ty),
                wip_hir::TyKind::Struct(..) | wip_hir::TyKind::Enum(..)
            )
        })
        .map(|decl| program.ty_name(decl.ty, &interner))
        .collect();
    assert!(left.is_empty(), "aggregates left whole: {left:?}");
}
