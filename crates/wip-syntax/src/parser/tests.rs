use super::*;
use crate::{Interner, Severity, lex};

fn parse_str(src: &str) -> (Parsed, Interner) {
    let mut interner = Interner::new();
    let lexed = lex(src, &mut interner);
    assert!(lexed.diagnostics.is_empty());
    (parse(src, &lexed), interner)
}

/// A file of a multi-file program is lexed and parsed with its spans based
/// where the file starts, so one span names a place in one file.
#[test]
fn spans_start_at_the_base() {
    let src = "fn first(): i64 = 1\n";
    let mut interner = Interner::new();
    let lexed = crate::lex_at(src, 1000, &mut interner);
    let parsed = crate::parse_at(src, 1000, &lexed);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let Item::Fn(f) = &parsed.ast.items[0] else {
        panic!("expected a function")
    };
    // "fn " is three bytes, and the name is four more.
    assert_eq!((f.sig.name.span.lo, f.sig.name.span.hi), (1003, 1008));
    assert_eq!(interner.resolve(f.sig.name.sym), "first");
}

fn codes(src: &str) -> Vec<&'static str> {
    parse_str(src)
        .0
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect()
}

/// The statements of `fn f() = { src }`, each rendered as an S-expression.
fn stmts(src: &str) -> Vec<String> {
    let src = format!("fn f() = {{\n{src}\n}}");
    let (parsed, interner) = parse_str(&src);
    // A warning says how something reads, not what it parsed as, and these
    // check the shape: `a` and then `.b` is two statements, and says so
    // (E0126).
    let errors: Vec<&Diagnostic> = parsed
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{src:?}: {errors:#?}");
    let Item::Fn(f) = &parsed.ast.items[0] else {
        panic!("not a function")
    };
    let body = f.body.expect("a function with a body");
    let ExprKind::Block(body) = &parsed.ast.exprs[body].kind else {
        panic!("not a block")
    };
    body.stmts
        .iter()
        .map(|&s| match &parsed.ast.stmts[s].kind {
            StmtKind::Expr(e) => render(&parsed.ast, &interner, *e),
            StmtKind::Let { name, init, .. } => {
                format!(
                    "(val {} {})",
                    interner.resolve(name.sym),
                    render(&parsed.ast, &interner, *init)
                )
            }
            StmtKind::Return(e) => match e {
                Some(e) => format!("(return {})", render(&parsed.ast, &interner, *e)),
                None => "(return)".to_string(),
            },
            other => format!("{other:?}"),
        })
        .collect()
}

/// Parses `src` as a single expression statement.
fn sexpr(src: &str) -> String {
    let rendered = stmts(src);
    assert_eq!(rendered.len(), 1, "{src:?}: {rendered:?}");
    rendered.into_iter().next().unwrap()
}

fn render(ast: &Ast, interner: &Interner, id: ExprId) -> String {
    let r = |id| render(ast, interner, id);
    match &ast.exprs[id].kind {
        ExprKind::Int(v) => v.to_string(),
        ExprKind::Name(s) => interner.resolve(*s).to_string(),
        ExprKind::Paren(e) => format!("({})", r(*e)),
        ExprKind::Unary { op, operand, .. } => format!("({} {})", op.text(), r(*operand)),
        ExprKind::Binary { op, lhs, rhs, .. } => {
            format!("({} {} {})", op.text(), r(*lhs), r(*rhs))
        }
        ExprKind::Cast { expr, ty } => {
            let TypeKind::Named { name: sym, .. } = ast.types[*ty].kind else {
                panic!("not a named type")
            };
            format!("(as {} {})", r(*expr), interner.resolve(sym))
        }
        ExprKind::Assign {
            target,
            op,
            wrapping,
            value,
        } => {
            let op = op.map_or(String::new(), |(op, _)| op.text().to_string());
            let wraps = if *wrapping { "%" } else { "" };
            format!("({op}{wraps}= {} {})", r(*target), r(*value))
        }
        ExprKind::Field { base, name } => {
            format!("(. {} {})", r(*base), interner.resolve(name.sym))
        }
        ExprKind::Index { base, index } => format!("([] {} {})", r(*base), r(*index)),
        ExprKind::SubSlice {
            base,
            lo,
            hi,
            inclusive,
        } => format!(
            "([{}] {} {} {})",
            if *inclusive { "..=" } else { ".." },
            r(*base),
            lo.map_or("-".to_string(), r),
            hi.map_or("-".to_string(), r)
        ),
        ExprKind::Call {
            callee,
            args,
            names,
            rest,
        } => {
            let mut args: Vec<String> = args
                .iter()
                .zip(names)
                .map(|(&a, name)| match name {
                    Some(name) => format!("{}: {}", interner.resolve(name.sym), r(a)),
                    None => r(a),
                })
                .collect();
            if let Some(rest) = rest {
                args.push(format!("..{}", r(*rest)));
            }
            format!("(call {} {})", r(*callee), args.join(" "))
        }
        ExprKind::If {
            then_block,
            else_branch,
            ..
        } => format!(
            "(if {} {})",
            then_block.stmts.len(),
            else_branch.map_or("-".to_string(), r)
        ),
        ExprKind::Block(block) => format!("(block {})", block.stmts.len()),
        ExprKind::Array(elems) => {
            let elems: Vec<String> = elems.iter().map(|&e| r(e)).collect();
            format!("[{}]", elems.join(" "))
        }
        ExprKind::Path {
            leading_dot,
            segments,
            type_args,
        } => {
            let names: Vec<&str> = segments.iter().map(|n| interner.resolve(n.sym)).collect();
            let dot = if *leading_dot { "." } else { "" };
            let args = type_args.as_ref().map_or(String::new(), |t| {
                format!(" <{} after {}>", t.args.len(), t.after)
            });
            format!("(path {dot}{}{args})", names.join("::"))
        }
        ExprKind::Match { arms, .. } => format!("(match {})", arms.len()),
        ExprKind::Is {
            scrutinee, negated, ..
        } => format!("({}is {})", if *negated { "!" } else { "" }, r(*scrutinee)),
        other => format!("{other:?}"),
    }
}

#[test]
fn precedence_and_associativity() {
    let cases = [
        ("a + b * c", "(+ a (* b c))"),
        ("a - b - c", "(- (- a b) c)"),
        ("a / b % c", "(% (/ a b) c)"),
        ("a = b = c", "(= a (= b c))"),
        ("a || b && c", "(|| a (&& b c))"),
        ("a && b || c", "(|| (&& a b) c)"),
        ("a < b == c < d", "(== (< a b) (< c d))"),
        ("!x == y", "(== (! x) y)"),
        ("-a.b", "(- (. a b))"),
        ("-f(a) * -c", "(* (- (call f a)) (- c))"),
        ("own f(x)", "(own (call f x))"),
        ("own own 1", "(own (own 1))"),
        ("&a[i]", "(& ([] a i))"),
        ("move p.next", "(move (. p next))"),
        ("a.b.c(d)[e]", "([] (call (. (. a b) c) d) e)"),
        ("(a + b) * c", "(* ((+ a b)) c)"),
        ("x = a + b", "(= x (+ a b))"),
        // Compound assignment, as loose as `=`.
        ("x += a * b", "(+= x (* a b))"),
        ("xs[i] -= 1", "(-= ([] xs i) 1)"),
        ("x *= y /= 2", "(*= x (/= y 2))"),
        ("x %= a == b", "(%= x (== a b))"),
        ("x &= m | 1", "(&= x (| m 1))"),
        ("x |= 1 << b", "(|= x (<< 1 b))"),
        ("x ^= y", "(^= x y)"),
        ("x <<= 2", "(<<= x 2)"),
        ("x >>= a >> 1", "(>>= x (>> a 1))"),
        ("x+=-1", "(+= x (- 1))"),
        ("a < b", "(< a b)"),
        ("a <= b", "(<= a b)"),
        ("-x as i64", "(as (- x) i64)"),
        ("a * b as f64", "(* a (as b f64))"),
        ("a as i32 as i64 + 1", "(+ (as (as a i32) i64) 1)"),
        ("a | b ^ c & d", "(| a (^ b (& c d)))"),
        ("a & b << 1 + 2", "(& a (<< b (+ 1 2)))"),
        ("x & m == 0", "(== (& x m) 0)"),
        ("a >> b > c", "(> (>> a b) c)"),
        ("a < b | c", "(< a (| b c))"),
        ("a << b >> c", "(>> (<< a b) c)"),
        ("!a & b", "(& (! a) b)"),
        ("a * b << c as i64", "(<< (* a b) (as c i64))"),
        ("a is .Some(x) && x > 0", "(&& (is a) (> x 0))"),
        ("!a is .None || b", "(|| (is (! a)) b)"),
        (
            "f(x) is Shape::Rect(width, ..) == c",
            "(== (is (call f x)) c)",
        ),
        ("move a is .Some(x)", "(is (move a))"),
    ];
    for (src, expected) in cases {
        assert_eq!(sexpr(src), expected, "{src}");
    }
}

/// Literals in other bases, with `_` separators.
/// After a name, `<` starts type arguments only when `(`, `{` or `::`
/// follows the matching `>` (grammar R7).
#[test]
fn type_arguments_or_comparisons() {
    let cases = [
        ("f<i64>(x)", "(call (path f <1 after 0>) x)"),
        ("f<own<T>, [i64; 3]>()", "(call (path f <2 after 0>) )"),
        ("Option<i64>::None", "(path Option::None <1 after 0>)"),
        (
            "pkg::Option<i64>::None",
            "(path pkg::Option::None <1 after 1>)",
        ),
        (
            "Pair<A, B>(first: a)",
            "(call (path Pair <2 after 0>) first: a)",
        ),
        ("a < b", "(< a b)"),
        ("a < b > c", "(> (< a b) c)"),
        ("f(a < b, c > d)", "(call f (< a b) (> c d))"),
        (
            "f(a < xs[i], c > (d))",
            "(call f (< a ([] xs i)) (> c (d)))",
        ),
        ("f(a < (b), c > (d))", "(call f (< a (b)) (> c (d)))"),
        ("f(a < (b) > (c))", "(call f (> (< a (b)) (c)))"),
        ("f<[i64; 3], &var T>(x)", "(call (path f <2 after 0>) x)"),
        (
            "f(g<(item: &T, n: [i64; 2]) => (x: T) => void>)",
            "(call f (path g <1 after 0>))",
        ),
        ("a < b + 1 > (c)", "(> (< a (+ b 1)) (c))"),
        ("x as i64 < y", "(< (as x i64) y)"),
        ("a << b >> (c)", "(>> (<< a b) (c))"),
        ("f(less<i64>, x)", "(call f (path less <1 after 0>) x)"),
        ("f(x, less<i64>)", "(call f x (path less <1 after 0>))"),
        (
            "f(apply<(v: i64) => bool>)",
            "(call f (path apply <1 after 0>))",
        ),
        ("f(a < b, c > d)", "(call f (< a b) (> c d))"),
        (
            "f(x, width: 2, dashed: a < b)",
            "(call f x width: 2 dashed: (< a b))",
        ),
        (
            "f(g<i64>(1), n: x::y)",
            "(call f (call (path g <1 after 0>) 1) n: (path x::y))",
        ),
    ];
    for (src, expected) in cases {
        assert_eq!(sexpr(src), expected, "{src}");
    }
}

#[test]
fn integer_literal_values() {
    let cases = [
        ("0xff", "255"),
        ("0xFF_FF", "65535"),
        ("0o755", "493"),
        ("0b1010_1010", "170"),
        ("1_000_000", "1000000"),
        (
            "0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF",
            "340282366920938463463374607431768211455",
        ),
    ];
    for (src, expected) in cases {
        assert_eq!(sexpr(src), expected, "{src}");
    }
    // One more digit does not fit in 128 bits.
    assert_eq!(
        codes("fn f() = 0x1_0000_0000_0000_0000_0000_0000_0000_0000"),
        ["E0111"]
    );
}

#[test]
fn a_variant_under_an_expression_warns() {
    // It reads as `io::println(1).Ok(2)`, and is two statements.
    assert_eq!(codes("fn f() = {\n    g(1)\n    .Ok(2)\n}"), ["E0126"]);
    // A binding and an assignment cannot take a `.`, so neither is
    // reported, and saying `return` says which was meant.
    assert_eq!(
        codes("fn f() = {\n    val x = 1\n    .Ok(x)\n}"),
        [] as [&str; 0]
    );
    assert_eq!(
        codes("fn f() = {\n    var x = 1\n    x += 1\n    .Ok(x)\n}"),
        [] as [&str; 0]
    );
    assert_eq!(
        codes("fn f() = {\n    g(1)\n    return .Ok(2)\n}"),
        [] as [&str; 0]
    );
}

#[test]
fn line_breaks() {
    let cases: &[(&str, &[&str])] = &[
        // A complete expression ends at a line break.
        ("a\n- b", &["a", "(- b)"]),
        ("f\n(x)", &["f", "(x)"]),
        ("a\n[0]", &["a", "[0]"]),
        // An incomplete one continues.
        ("a +\nb", &["(+ a b)"]),
        ("val x =\n1", &["(val x 1)"]),
        // `else` continues the line above; a `.` begins a variant instead,
        // so it starts a new statement.
        ("a\n.b", &["a", "(path .b)"]),
        ("a.b", &["(. a b)"]),
        ("if c { 1 }\nelse { 2 }", &["(if 1 (block 1))"]),
        // Line breaks are whitespace inside parentheses and conditions.
        ("f(a\n, b)", &["(call f a b)"]),
        ("(a\n+ b)", &["((+ a b))"]),
        ("if a\n&& b { 1 }", &["(if 1 -)"]),
        // `;` separates statements on one line.
        ("val x = 1; val y = 2", &["(val x 1)", "(val y 2)"]),
        // `return` followed by a line break returns nothing.
        ("return\n1", &["(return)", "1"]),
        // A `{` on the next line starts a block.
        ("p\n{ 1 }", &["p", "(block 1)"]),
        // Arguments across lines, with commas.
        ("P(\nx: 1,\ny: 2,\n)", &["(call P x: 1 y: 2)"]),
        ("match e {\nA => 1\nB => 2\n}", &["(match 2)"]),
        ("match e { A => 1, B => 2 }", &["(match 2)"]),
    ];
    for (src, expected) in cases {
        assert_eq!(stmts(src), *expected, "{src:?}");
    }
}

#[test]
fn functions_and_extern_blocks() {
    let src = "extern \"C\" {\n fn a(x: i64): i64\n fn b(); fn c()\n}\n\
               fn add(x: i64, y: i64): i64 = x + y\n\
               fn square(x: i64): i64 = {\n val s = x * x\n s\n}\n\
               fn main() = a(1)\n";
    let (parsed, _) = parse_str(src);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    assert_eq!(parsed.ast.items.len(), 4);
    let Item::Extern(e) = &parsed.ast.items[0] else {
        panic!("not an extern block")
    };
    assert_eq!(e.fns.len(), 3);
}

/// An import names a module, or items of it in braces.
#[test]
fn import_lists() {
    let src = "import std::io::{print}\n\
               import a::b::{self, C, d as e}\n\
               import f::{\n    g\n    h,\n}\n\
               import i as j\n";
    let (parsed, interner) = parse_str(src);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let imports: Vec<String> = parsed
        .ast
        .items
        .iter()
        .map(|item| {
            let Item::Import(i) = item else {
                panic!("not an import")
            };
            let path: Vec<&str> = i.path.iter().map(|n| interner.resolve(n.sym)).collect();
            let mut text = path.join("::");
            for item in i.items.iter().flatten() {
                match item.name {
                    Some(name) => text.push_str(&format!(" {}", interner.resolve(name.sym))),
                    None => text.push_str(" self(module)"),
                }
                if let Some(alias) = item.alias {
                    text.push_str(&format!("={}", interner.resolve(alias.sym)));
                }
            }
            if let Some(alias) = i.alias {
                text.push_str(&format!(" as {}", interner.resolve(alias.sym)));
            }
            text
        })
        .collect();
    assert_eq!(
        imports,
        [
            "std::io print",
            "a::b self(module) C d=e",
            "f g h",
            "i as j"
        ]
    );
}

#[test]
fn struct_literal_rules() {
    // A struct is built with a call, in a condition as anywhere.
    let (parsed, _) = parse_str("fn f() = { val p = P(x: 1, ..q)\n if P(x: 1).x == 1 {} }");
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    // In a condition, `x {}` is a name and an empty block.
    let (parsed, _) = parse_str("fn f() = { if x {}\n while y {} }");
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    // Braces are E0133, once each, with a fix that writes the call.
    assert_eq!(codes("fn f() = { val p = P { x: 1 } }"), ["E0133"]);
    assert_eq!(
        codes("fn f() = { if p == P { x: 1 } { return } }"),
        ["E0133"]
    );
    let (parsed, _) = parse_str("fn f() = { val p = P { x, y: 2, ..q } }");
    let fix = parsed.diagnostics[0].fix.as_ref().expect("a fix");
    assert_eq!(fix.edits[0].replacement, "P(x: x, y: 2, ..q)");
}

#[test]
fn error_codes() {
    let cases: &[(&str, &[&str])] = &[
        ("fn f() = { val x = 1 val y = 2 }", &["E0112"]),
        ("fn f() = {\n val x = 1\n + 2\n}", &["E0113"]),
        ("fn f() = { let x = 1 }", &["E0114"]),
        ("fn f() -> i64 = 1", &["E0115"]),
        ("fn f() { }", &["E0106"]),
        ("fn f()", &["E0106"]),
        ("fn f() = match e { A => 1 B => 2 }", &["E0104"]),
        ("struct P { x: i64 y: i64 }", &["E0112"]),
        ("extern \"C\" { fn a() fn b() }", &["E0112"]),
        ("extern \"C\" { fn a(): i64 = 1 }", &["E0106"]),
        ("import a::{}", &["E0119"]),
        ("import a::{b} as c", &["E0119"]),
    ];
    for (src, expected) in cases {
        assert_eq!(codes(src), *expected, "{src:?}");
    }
}

/// Every fix the parser suggests must produce source that parses cleanly.
#[test]
fn fixes_apply_cleanly() {
    let cases = [
        "fn f() = { val x = 1 val y = 2 }",
        "fn f() = {\n val x = 1\n + 2\n}",
        "fn f() = { let x = 1 }",
        "fn f() -> i64 = 1",
        "fn f() { }",
        "fn f() = match e { A => 1 B => 2 }",
        "struct P { x: i64 y: i64 }",
        "struct S { a i64 }",
        "fn f(a: i64 b: i64) = {}",
        "fn f() = { return (1 + 2 }",
        "struct S { a: own<i64, }",
        "struct P { x: i64 } fn f(p: P) = { if p == P { x: 1 } {} }",
        "fn f(x: i64) = g(&&x)",
        "enum E { A() }",
        "extern \"C\" { fn f(): i64 = 1 }",
        "extern { fn f() }",
        "import a::{}",
    ];
    for src in cases {
        let (parsed, _) = parse_str(src);
        assert_eq!(
            parsed.diagnostics.len(),
            1,
            "{src:?}: {:#?}",
            parsed.diagnostics
        );
        let diagnostic = &parsed.diagnostics[0];
        let fix = diagnostic
            .fix
            .as_ref()
            .unwrap_or_else(|| panic!("{src:?}: no fix for {diagnostic:#?}"));
        let fixed = fix.apply(src);
        let (reparsed, _) = parse_str(&fixed);
        assert!(
            reparsed.diagnostics.is_empty(),
            "{src:?} fixed to {fixed:?}: {:#?}",
            reparsed.diagnostics
        );
    }
}

#[test]
fn every_error_has_a_span_inside_the_source() {
    let src = "fn f( = { val = ; if { match x { A(b) => } } struct";
    let (parsed, _) = parse_str(src);
    assert!(!parsed.diagnostics.is_empty());
    for d in &parsed.diagnostics {
        assert!(d.primary.span.hi as usize <= src.len(), "{d:#?}");
    }
}
