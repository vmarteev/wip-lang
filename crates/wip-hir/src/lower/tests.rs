use super::*;

#[test]
fn suggestions() {
    assert_eq!(suggest("pritn", ["print", "main"]), Some("print"));
    assert_eq!(suggest("cuont", ["count", "total"]), Some("count"));
    assert_eq!(suggest("Pint", ["Point", "Shape"]), Some("Point"));
    assert_eq!(suggest("x", ["y", "z"]), None);
    assert_eq!(suggest("count", ["total", "main"]), None);
    assert_eq!(edit_distance("kitten", "sitting"), 3);
    assert_eq!(edit_distance("hieght", "height"), 1);
}

/// Lowers two modules — the prelude, then the root — and returns the
/// messages, for rules that only the prelude can break.
fn prelude_errors(prelude: &str, root: &str) -> Vec<String> {
    with_prelude(prelude, root)
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

/// Lowers two modules, the prelude and then the root.
fn with_prelude(prelude: &str, root: &str) -> Lowered {
    let interner = wip_syntax::Interner::new();
    let lex_parse = |text: &str, interner: &mut wip_syntax::Interner| {
        let lexed = wip_syntax::lex(text, interner);
        wip_syntax::parse(text, &lexed).ast
    };
    let mut interner = interner;
    let prelude_ast = lex_parse(prelude, &mut interner);
    let root_ast = lex_parse(root, &mut interner);
    let modules = [
        ModuleAst {
            path: crate::PRELUDE.to_string(),
            files: vec![&prelude_ast],
            prefix: String::new(),
            depends: Vec::new(),
            dir: None,
            root: None,
        },
        ModuleAst {
            path: String::new(),
            files: vec![&root_ast],
            prefix: String::new(),
            depends: Vec::new(),
            dir: None,
            root: None,
        },
    ];
    lower(&modules, &interner)
}

/// A tuple written as a `match`'s scrutinee has the prelude's tuple type
/// inside the prelude too, where the prelude is the module being lowered
/// rather than another. It was the error type there,
/// reported nowhere, and the MIR met it.
#[test]
fn a_tuple_matched_in_place_inside_the_prelude_has_its_type() {
    let lowered = with_prelude(
        "pub struct Tuple2<A, B> {\n    pub var _0: A\n    pub var _1: B\n}\n\npub fn both(a: bool, b: bool): bool = match (a, b) {\n    (true, true) => true\n    _ => false\n}\n",
        "fn main() = {}\n",
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let both = lowered
        .program
        .fns
        .iter()
        .find(|(_, f)| f.body.is_some() && f.params.len() == 2)
        .map(|(_, f)| f)
        .expect("the prelude's function was lowered");
    let body = both.body.as_ref().expect("it has a body");
    assert!(
        body.exprs.iter().all(|(_, e)| e.ty != Types::ERROR),
        "no expression of it has the error type"
    );
}

#[test]
fn a_slice_has_no_move_method() {
    let messages = prelude_errors(
        "extend [T] {\n    pub move fn take(): i64 = 0\n}\n",
        "fn main() = {}\n",
    );
    assert!(
        messages.iter().any(|m| m.contains("no value to move")),
        "{messages:?}"
    );
}

#[test]
fn a_builtin_type_cannot_clean_up_after_itself() {
    let messages = prelude_errors(
        "pub interface Destroy {\n    var fn destroy()\n}\n\nextend i64: Destroy {\n    var fn destroy() = {}\n}\n",
        "fn main() = {}\n",
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("cannot clean up after itself")),
        "{messages:?}"
    );
}
