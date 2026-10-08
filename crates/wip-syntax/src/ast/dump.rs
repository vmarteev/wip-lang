//! A readable, deterministic rendering of the syntax tree, for `wip parse` and
//! snapshot tests. One node per line, children indented under their parent,
//! and each line ends with the node's span as `@line:col..line:col`. A
//! `role:` prefix says which child of the parent a node is where the order
//! alone would not.

use std::fmt::Write;

use super::*;
use crate::{Interner, LineIndex};

pub fn dump(ast: &Ast, src: &str, interner: &Interner) -> String {
    dump_at(ast, src, 0, interner)
}

/// Dumps a file whose spans start at `base`.
pub fn dump_at(ast: &Ast, src: &str, base: u32, interner: &Interner) -> String {
    let mut printer = Printer {
        ast,
        src,
        interner,
        base,
        lines: LineIndex::new(src),
        out: String::new(),
        depth: 0,
    };
    for item in &ast.items {
        printer.item(item);
    }
    printer.out
}

struct Printer<'a> {
    ast: &'a Ast,
    src: &'a str,
    interner: &'a Interner,
    base: u32,
    lines: LineIndex,
    out: String,
    depth: usize,
}

impl<'a> Printer<'a> {
    fn line(&mut self, role: &str, text: &str, span: Span) {
        let (l1, c1) = self.lines.line_col(self.src, span.lo - self.base);
        let (l2, c2) = self.lines.line_col(self.src, span.hi - self.base);
        let indent = "  ".repeat(self.depth);
        let role = if role.is_empty() {
            String::new()
        } else {
            format!("{role}: ")
        };
        writeln!(self.out, "{indent}{role}{text}  @{l1}:{c1}..{l2}:{c2}").unwrap();
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self)) {
        self.depth += 1;
        f(self);
        self.depth -= 1;
    }

    fn sym(&self, sym: Symbol) -> &'a str {
        self.interner.resolve(sym)
    }

    /// The annotations before a declaration, each on a line of its own.
    fn annotations(&mut self, annotations: &'a [Annotation]) {
        for annotation in annotations {
            let mut text = format!("@{}", self.sym(annotation.name.sym));
            if annotation.parens.is_some() {
                let args: Vec<String> = annotation
                    .args
                    .iter()
                    .map(|arg| {
                        let value = match &arg.value {
                            AnnotationValue::Int(v) => v.to_string(),
                            AnnotationValue::Str(sym) => format!("{:?}", self.sym(*sym)),
                            AnnotationValue::Bool(v) => v.to_string(),
                            AnnotationValue::Name(sym) => self.sym(*sym).to_string(),
                            AnnotationValue::Error => "?".to_string(),
                        };
                        match arg.name {
                            Some(name) => format!("{} = {value}", self.sym(name.sym)),
                            None => value,
                        }
                    })
                    .collect();
                text.push_str(&format!("({})", args.join(", ")));
            }
            self.line("", &text, annotation.span);
        }
    }

    fn item(&mut self, item: &'a Item) {
        let exported = |is_pub: bool| if is_pub { "pub " } else { "" };
        match item {
            Item::Import(i) => {
                let path: Vec<&str> = i.path.iter().map(|n| self.sym(n.sym)).collect();
                let mut text = format!("import {}", path.join("::"));
                if let Some(items) = &i.items {
                    let items: Vec<String> = items
                        .iter()
                        .map(|item| {
                            // `self` in the list is the module itself.
                            let name = match item.name {
                                Some(name) => self.sym(name.sym),
                                None => "self",
                            };
                            match item.alias {
                                Some(alias) => format!("{name} as {}", self.sym(alias.sym)),
                                None => name.to_string(),
                            }
                        })
                        .collect();
                    text.push_str(&format!("::{{{}}}", items.join(", ")));
                }
                if let Some(alias) = i.alias {
                    text.push_str(&format!(" as {}", self.sym(alias.sym)));
                }
                self.line("", &text, i.span);
            }
            // `type Id = i64`.
            Item::Type(t) => {
                let text = format!("{}type {}", exported(t.is_pub), self.sym(t.name.sym));
                self.line("", &text, t.span);
                self.nested(|p| {
                    p.generics(&t.generics);
                    p.ty("", t.ty);
                });
            }
            Item::Struct(s) => {
                self.annotations(&s.annotations);
                // `extern struct` is C's layout, and `extern union` is C's
                // one-of.
                let kind = match (s.is_extern, s.is_union) {
                    (true, true) => "extern union",
                    (true, false) => "extern struct",
                    _ if s.is_view => "view struct",
                    _ => "struct",
                };
                let text = format!("{}{kind} {}", exported(s.is_pub), self.sym(s.name.sym));
                self.line("", &text, s.span);
                self.nested(|p| {
                    p.generics(&s.generics);
                    s.fields.iter().for_each(|f| p.field("field", f));
                    s.methods.iter().for_each(|m| p.method(m));
                });
            }
            Item::Enum(e) => {
                self.annotations(&e.annotations);
                let kind = if e.is_view { "view enum" } else { "enum" };
                let text = format!("{}{kind} {}", exported(e.is_pub), self.sym(e.name.sym));
                self.line("", &text, e.span);
                self.nested(|p| {
                    p.generics(&e.generics);
                    for v in &e.variants {
                        p.line("", &format!("variant {}", p.sym(v.name.sym)), v.span);
                        p.nested(|p| v.fields.iter().for_each(|f| p.field("field", f)));
                    }
                    e.methods.iter().for_each(|m| p.method(m));
                });
            }
            Item::Assert(a) => {
                self.annotations(&a.annotations);
                self.line("", "assert", a.span);
                self.nested(|p| p.expr("assert", a.assert));
            }
            Item::Val(v) => {
                self.annotations(&v.annotations);
                let text = format!("{}val {}", exported(v.is_pub), self.sym(v.name.sym));
                self.line("", &text, v.span);
                self.nested(|p| {
                    if let Some(ty) = v.ty {
                        p.ty("type", ty);
                    }
                    p.expr("value", v.value);
                });
            }
            Item::Fn(f) => {
                self.annotations(&f.annotations);
                if f.is_pub {
                    self.line("", &format!("pub fn {}", self.sym(f.sig.name.sym)), f.span);
                    self.nested(|p| {
                        p.generics(&f.sig.generics);
                        f.sig
                            .params
                            .iter()
                            .for_each(|field| p.field("param", field));
                        if let Some(ret) = f.sig.ret {
                            p.ty("ret", ret);
                        }
                        p.lends_from(&f.sig.lends_from);
                        if let Some(body) = f.body {
                            p.expr("body", body);
                        }
                    });
                    return;
                }
                self.sig(&f.sig, f.span);
                self.nested(|p| {
                    if let Some(body) = f.body {
                        p.expr("body", body);
                    }
                });
            }
            Item::Interface(i) => {
                self.annotations(&i.annotations);
                let text = format!("{}interface {}", exported(i.is_pub), self.sym(i.name.sym));
                self.line("", &text, i.span);
                self.nested(|p| {
                    for method in &i.methods {
                        let (receiver, _) = method.receiver;
                        let name = p.sym(method.sig.name.sym);
                        let default = if method.default.is_some() {
                            " with default"
                        } else {
                            ""
                        };
                        let text = format!("{} {name}{default}", receiver.text());
                        p.line("", &text, method.span);
                        p.nested(|p| {
                            p.generics(&method.sig.generics);
                            method.sig.params.iter().for_each(|f| p.field("param", f));
                            if let Some(ret) = method.sig.ret {
                                p.ty("ret", ret);
                            }
                            p.lends_from(&method.sig.lends_from);
                            if let Some(body) = method.default {
                                p.expr("body", body);
                            }
                        });
                    }
                });
            }
            Item::Extend(block) => {
                self.annotations(&block.annotations);
                let path: Vec<&str> = block.path.iter().map(|n| self.sym(n.sym)).collect();
                let mut text = match block.slice_of {
                    Some(elem) => format!("extend [{}]", self.sym(elem.sym)),
                    None => format!("extend {}", path.join("::")),
                };
                if !block.generics.is_empty() {
                    let names: Vec<&str> = block
                        .generics
                        .iter()
                        .map(|p| self.sym(p.name.sym))
                        .collect();
                    text.push_str(&format!("<{}>", names.join(", ")));
                }
                if let Some(interface) = block.interface {
                    text.push_str(&format!(": {}", self.sym(interface.sym)));
                }
                self.line("", &text, block.span);
                self.nested(|p| {
                    if let Some(args) = &block.interface_args {
                        args.args.iter().for_each(|&arg| p.ty("interface arg", arg));
                    }
                    block.methods.iter().for_each(|m| p.method(m));
                });
            }
            Item::Extern(e) => {
                self.annotations(&e.annotations);
                let abi =
                    &self.src[(e.abi.lo - self.base) as usize..(e.abi.hi - self.base) as usize];
                self.line("", format!("extern {abi}").trim_end(), e.span);
                self.nested(|p| {
                    for declared in &e.types {
                        let exported = if declared.is_pub { "pub " } else { "" };
                        let name = declared.name;
                        p.line(
                            "",
                            &format!("{exported}type {}", p.sym(name.sym)),
                            name.span,
                        );
                    }
                    for declared in &e.globals {
                        let exported = if declared.is_pub { "pub " } else { "" };
                        let keyword = if declared.is_mut { "var" } else { "val" };
                        let name = p.sym(declared.name.sym);
                        p.line("", &format!("{exported}{keyword} {name}"), declared.span);
                        p.nested(|p| p.ty("type", declared.ty));
                    }
                    for declared in &e.fns {
                        let exported = if declared.is_pub { "pub " } else { "" };
                        p.sig_with(exported, &declared.sig, declared.sig.span);
                    }
                });
            }
        }
    }

    /// A method of a struct or an enum, with the word that declares how it
    /// takes its receiver.
    fn method(&mut self, decl: &'a FnDecl) {
        self.annotations(&decl.annotations);
        let (receiver, _) = decl.receiver.expect("a method has a receiver");
        let exported = if decl.is_pub { "pub " } else { "" };
        let text = format!(
            "{exported}{} {}",
            receiver.text(),
            self.sym(decl.sig.name.sym)
        );
        self.line("", &text, decl.span);
        self.nested(|p| {
            p.generics(&decl.sig.generics);
            decl.sig.params.iter().for_each(|f| p.field("param", f));
            if let Some(ret) = decl.sig.ret {
                p.ty("ret", ret);
            }
            p.lends_from(&decl.sig.lends_from);
            if let Some(body) = decl.body {
                p.expr("body", body);
            }
        });
    }

    /// Prints `fn name` with its parameters and return type nested under it.
    /// The caller prints the body, if there is one, at the same depth.
    fn sig(&mut self, sig: &'a FnSig, span: Span) {
        self.sig_with("", sig, span);
    }

    /// The same, with what goes before `fn`: `pub` on an exported C
    /// declaration.
    fn sig_with(&mut self, before: &str, sig: &'a FnSig, span: Span) {
        self.line("", &format!("{before}fn {}", self.sym(sig.name.sym)), span);
        self.nested(|p| {
            p.generics(&sig.generics);
            sig.params.iter().for_each(|f| p.field("param", f));
            // `...`: it takes more than it declares.
            if let Some(variadic) = sig.variadic {
                p.line("", "more arguments", variadic);
            }
            if let Some(ret) = sig.ret {
                p.ty("ret", ret);
            }
            p.lends_from(&sig.lends_from);
        });
    }

    /// What `from` says the result borrows, one line each.
    fn lends_from(&mut self, paths: &[LendPath]) {
        for path in paths {
            let mut text = match path.root {
                Some(name) => self.sym(name.sym).to_string(),
                None => "self".to_string(),
            };
            for field in &path.fields {
                text.push('.');
                text.push_str(self.sym(field.sym));
            }
            self.line("from", &text, path.span);
        }
    }

    /// `<T, U: copy>` of a generic item.
    fn generics(&mut self, generics: &[GenericParam]) {
        for param in generics {
            let decided = if param.decided.is_some() {
                "decided "
            } else {
                ""
            };
            let mut text = format!("{decided}type param {}", self.sym(param.name.sym));
            if !param.bounds.is_empty() {
                let bounds: Vec<&str> = param.bounds.iter().map(|b| self.sym(b.name.sym)).collect();
                text.push_str(&format!(": {}", bounds.join(" + ")));
            }
            self.line("", &text, param.span);
            // The types a constraint's interface takes.
            let args: Vec<TypeId> = param
                .bounds
                .iter()
                .flat_map(|b| b.args.iter().copied())
                .collect();
            if !args.is_empty() {
                self.nested(|p| args.iter().for_each(|&arg| p.ty("constraint arg", arg)));
            }
            // What a use that leaves it out means.
            if let Some(default) = param.default {
                self.nested(|p| p.ty("default", default));
            }
        }
    }

    /// `<A, B>` in an expression, with the segment it follows.
    fn type_args(&mut self, type_args: Option<&'a TypeArgs>) {
        let Some(type_args) = type_args else { return };
        self.line(
            "",
            &format!("type args after segment {}", type_args.after + 1),
            type_args.span,
        );
        self.nested(|p| type_args.args.iter().for_each(|&arg| p.ty("", arg)));
    }

    /// `while outer`, `break outer`: the word and the loop's name where
    /// one was written.
    fn labelled(&self, word: &str, label: Option<&Name>) -> String {
        match label {
            Some(label) => format!("{word} {}", self.sym(label.sym)),
            None => word.to_string(),
        }
    }

    fn field(&mut self, what: &str, field: &'a Field) {
        self.line(
            "",
            &format!("{what} {}", self.sym(field.name.sym)),
            field.span,
        );
        self.nested(|p| {
            p.ty("", field.ty);
            if let Some(default) = field.default {
                p.expr("default", default);
            }
        });
    }

    fn ty(&mut self, role: &str, id: TypeId) {
        let ty = &self.ast.types[id];
        match &ty.kind {
            TypeKind::Named { name, args } => {
                self.line(role, self.sym(*name), ty.span);
                self.nested(|p| args.iter().for_each(|&arg| p.ty("arg", arg)));
            }
            TypeKind::Path { segments, args } => {
                let names: Vec<&str> = segments.iter().map(|n| self.sym(n.sym)).collect();
                self.line(role, &names.join("::"), ty.span);
                self.nested(|p| args.iter().for_each(|&arg| p.ty("arg", arg)));
            }
            TypeKind::Own(inner) => {
                self.line(role, "own", ty.span);
                self.nested(|p| p.ty("", *inner));
            }
            TypeKind::Ref { var, inner } => {
                self.line(role, if *var { "ref var" } else { "ref" }, ty.span);
                self.nested(|p| p.ty("", *inner));
            }
            TypeKind::Array { elem, len } => {
                let len = match len {
                    ArrayLen::Int(n) => n.to_string(),
                    ArrayLen::Name(name) => self.sym(name.sym).to_string(),
                };
                self.line(role, &format!("array {len}"), ty.span);
                self.nested(|p| p.ty("", *elem));
            }
            TypeKind::Slice(elem) => {
                self.line(role, "slice", ty.span);
                self.nested(|p| p.ty("", *elem));
            }
            TypeKind::Dyn(name, args) => {
                self.line(role, &format!("dyn {}", self.sym(name.sym)), ty.span);
                self.nested(|p| args.iter().for_each(|&arg| p.ty("arg", arg)));
            }
            TypeKind::Fn { params, ret } => {
                self.line(role, "fn type", ty.span);
                self.nested(|p| {
                    params.iter().for_each(|param| p.field("param", param));
                    p.ty("ret", *ret);
                });
            }
            TypeKind::Error => self.line(role, "error", ty.span),
        }
    }

    fn block(&mut self, role: &str, block: &'a Block) {
        self.line(role, "block", block.span);
        self.nested(|p| block.stmts.iter().for_each(|&s| p.stmt("", s)));
    }

    fn stmt(&mut self, role: &str, id: StmtId) {
        let stmt = &self.ast.stmts[id];
        match &stmt.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                init,
            } => {
                let keyword = if *mutable { "var" } else { "val" };
                self.line(
                    role,
                    &format!("{keyword} {}", self.sym(name.sym)),
                    stmt.span,
                );
                self.nested(|p| {
                    if let Some(ty) = ty {
                        p.ty("type", *ty);
                    }
                    p.expr("init", *init);
                });
            }
            StmtKind::Guard {
                mutable,
                pattern,
                value,
                else_block,
            } => {
                self.line(
                    role,
                    if *mutable { "var else" } else { "val else" },
                    stmt.span,
                );
                self.nested(|p| {
                    p.pattern(pattern);
                    p.expr("value", *value);
                    if let Some(else_block) = else_block {
                        p.block("else", else_block);
                    }
                });
            }
            StmtKind::Defer(e) => {
                self.line(role, "defer", stmt.span);
                self.nested(|p| p.expr("", *e));
            }
            StmtKind::Return(e) => {
                self.line(role, "return", stmt.span);
                self.nested(|p| e.iter().for_each(|&e| p.expr("", e)));
            }
            StmtKind::Yield(e) => {
                self.line(role, "yield", stmt.span);
                self.nested(|p| p.expr("", *e));
            }
            StmtKind::While { label, cond, body } => {
                let text = self.labelled("while", label.as_ref());
                self.line(role, &text, stmt.span);
                self.nested(|p| {
                    p.expr("cond", *cond);
                    p.block("body", body);
                });
            }
            StmtKind::For {
                label,
                binding,
                source,
                body,
            } => {
                let text = self.labelled("for", label.as_ref());
                self.line(role, &text, stmt.span);
                self.nested(|p| {
                    p.pattern(binding);
                    match *source {
                        ForSource::Elements(elements) => p.expr("elements", elements),
                        ForSource::Range { lo, hi, inclusive } => {
                            p.expr("from", lo);
                            p.expr(if inclusive { "through" } else { "to" }, hi);
                        }
                    }
                    p.block("body", body);
                });
            }
            StmtKind::Break(label) => {
                let text = self.labelled("break", label.as_ref());
                self.line(role, &text, stmt.span);
            }
            StmtKind::Continue(label) => {
                let text = self.labelled("continue", label.as_ref());
                self.line(role, &text, stmt.span);
            }
            StmtKind::Expr(e) => {
                self.line(role, "expr", stmt.span);
                self.nested(|p| p.expr("", *e));
            }
        }
    }

    fn expr(&mut self, role: &str, id: ExprId) {
        let expr = &self.ast.exprs[id];
        let span = expr.span;
        match &expr.kind {
            ExprKind::Int(v) => self.line(role, &format!("int {v}"), span),
            ExprKind::Float(v) => self.line(role, &format!("float {v:?}"), span),
            ExprKind::Str(s) => self.line(role, &format!("str {:?}", self.sym(*s)), span),
            ExprKind::Char(c) => self.line(role, &format!("char {}", show_char(*c)), span),
            ExprKind::Byte(b) => self.line(role, &format!("byte {}", show_byte(*b)), span),
            ExprKind::Bool(b) => self.line(role, &format!("bool {b}"), span),
            ExprKind::Null => self.line(role, "null", span),
            ExprKind::Name(s) => self.line(role, &format!("name {}", self.sym(*s)), span),
            ExprKind::SelfRef => self.line(role, "self", span),
            ExprKind::Paren(inner) => {
                self.line(role, "paren", span);
                self.nested(|p| p.expr("", *inner));
            }
            ExprKind::Block(block) => self.block(role, block),
            ExprKind::If {
                cond,
                then_block,
                else_branch,
            } => {
                self.line(role, "if", span);
                self.nested(|p| {
                    p.expr("cond", *cond);
                    p.block("then", then_block);
                    if let Some(e) = else_branch {
                        p.expr("else", *e);
                    }
                });
            }
            ExprKind::Assert {
                cond,
                note,
                message,
            } => {
                let text = message.map_or(String::new(), |m| format!(" {:?}", self.sym(m)));
                self.line(role, &format!("assert{text}"), span);
                self.nested(|p| {
                    p.expr("cond", *cond);
                    if let Some(note) = note {
                        p.expr("note", *note);
                    }
                });
            }
            ExprKind::Path {
                leading_dot,
                segments,
                type_args,
            } => {
                let names: Vec<&str> = segments.iter().map(|n| self.sym(n.sym)).collect();
                let dot = if *leading_dot { "." } else { "" };
                self.line(role, &format!("path {dot}{}", names.join("::")), span);
                self.nested(|p| p.type_args(type_args.as_ref()));
            }
            ExprKind::Lend(value) => {
                self.line(role, "lend", span);
                self.nested(|p| p.expr("", *value));
            }
            ExprKind::Array(elems) => {
                self.line(role, "array", span);
                self.nested(|p| elems.iter().for_each(|&e| p.expr("", e)));
            }
            ExprKind::ForElement {
                binding,
                source,
                body,
            } => {
                self.line(role, "for element", span);
                self.nested(|p| {
                    p.pattern(binding);
                    match *source {
                        ForSource::Elements(elements) => p.expr("elements", elements),
                        ForSource::Range { lo, hi, inclusive } => {
                            p.expr("from", lo);
                            p.expr(if inclusive { "through" } else { "to" }, hi);
                        }
                    }
                    p.block("body", body);
                });
            }
            ExprKind::ArrayRepeat {
                elem,
                count: RepeatCount::Literal(count),
            } => {
                self.line(role, &format!("array repeat {count}"), span);
                self.nested(|p| p.expr("", *elem));
            }
            ExprKind::ArrayRepeat {
                elem,
                count: RepeatCount::Expr(count),
            } => {
                self.line(role, "array repeat", span);
                self.nested(|p| {
                    p.expr("", *elem);
                    p.expr("count", *count);
                });
            }
            ExprKind::Match { scrutinee, arms } => {
                self.line(role, "match", span);
                self.nested(|p| {
                    p.expr("scrutinee", *scrutinee);
                    for arm in arms {
                        p.line("", "arm", arm.span);
                        p.nested(|p| {
                            p.pattern(&arm.pattern);
                            p.expr("body", arm.body);
                        });
                    }
                });
            }
            ExprKind::Unary { op, operand, .. } => {
                self.line(role, &format!("unary {}", op.text()), span);
                self.nested(|p| p.expr("", *operand));
            }
            ExprKind::Binary {
                op,
                lhs,
                rhs,
                wrapping,
                ..
            } => {
                // `+%` is `+` that wraps, and the dump says which was
                // written.
                let wraps = if *wrapping { "%" } else { "" };
                self.line(role, &format!("binary {}{wraps}", op.text()), span);
                self.nested(|p| {
                    p.expr("", *lhs);
                    p.expr("", *rhs);
                });
            }
            ExprKind::Lambda { params, ret, body } => {
                self.line(role, "lambda", span);
                self.nested(|p| {
                    for param in params {
                        p.line("", &format!("param {}", p.sym(param.name.sym)), param.span);
                        if let Some(ty) = param.ty {
                            p.nested(|p| p.ty("", ty));
                        }
                    }
                    if let Some(ret) = ret {
                        p.ty("ret", *ret);
                    }
                    p.expr("body", *body);
                });
            }
            ExprKind::Try(inner) => {
                self.line(role, "try", span);
                self.nested(|p| p.expr("", *inner));
            }
            ExprKind::Cast { expr, ty } => {
                self.line(role, "cast", span);
                self.nested(|p| {
                    p.expr("", *expr);
                    p.ty("to", *ty);
                });
            }
            ExprKind::Assign {
                target,
                op,
                wrapping,
                value,
            } => {
                let wraps = if *wrapping { "%" } else { "" };
                match op {
                    Some((op, _)) => {
                        self.line(role, &format!("assign {}{wraps}=", op.text()), span)
                    }
                    None => self.line(role, "assign", span),
                }
                self.nested(|p| {
                    p.expr("target", *target);
                    p.expr("value", *value);
                });
            }
            ExprKind::Field { base, name } => {
                self.line(role, &format!("field {}", self.sym(name.sym)), span);
                self.nested(|p| p.expr("", *base));
            }
            ExprKind::Call {
                callee,
                args,
                names,
                rest,
            } => {
                self.line(role, "call", span);
                self.nested(|p| {
                    p.expr("callee", *callee);
                    for (&arg, name) in args.iter().zip(names) {
                        match name {
                            Some(name) => p.expr(&format!("arg {}", p.sym(name.sym)), arg),
                            None => p.expr("arg", arg),
                        }
                    }
                    if let Some(rest) = rest {
                        p.expr("rest", *rest);
                    }
                });
            }
            ExprKind::Is {
                scrutinee,
                pattern,
                negated,
            } => {
                self.line(role, if *negated { "!is" } else { "is" }, span);
                self.nested(|p| {
                    p.expr("scrutinee", *scrutinee);
                    p.pattern(pattern);
                });
            }
            ExprKind::Index { base, index } => {
                self.line(role, "index", span);
                self.nested(|p| {
                    p.expr("base", *base);
                    p.expr("index", *index);
                });
            }
            ExprKind::SubSlice {
                base,
                lo,
                hi,
                inclusive,
            } => {
                self.line(
                    role,
                    if *inclusive { "slice through" } else { "slice" },
                    span,
                );
                self.nested(|p| {
                    p.expr("base", *base);
                    if let Some(lo) = lo {
                        p.expr("lo", *lo);
                    }
                    if let Some(hi) = hi {
                        p.expr("hi", *hi);
                    }
                });
            }
            ExprKind::Error => self.line(role, "error", span),
        }
    }

    fn pattern(&mut self, pattern: &Pattern) {
        let text = self.pattern_text(pattern, true);
        self.line("pattern", &text, pattern.span);
    }

    /// A pattern as one line, which a binder's own pattern is part of.
    /// Only the outermost says what kind it is; nested,
    /// it reads as it was written.
    fn pattern_text(&mut self, pattern: &Pattern, top: bool) -> String {
        match &pattern.kind {
            PatternKind::Wildcard => "_".to_string(),
            PatternKind::Binding(sym) => {
                let kind = if top { "binding " } else { "" };
                format!("{kind}{}", self.sym(*sym))
            }
            PatternKind::Variant {
                leading_dot,
                segments,
                binders,
                rest,
            } => {
                let binders = match binders {
                    None => String::new(),
                    Some(bs) => {
                        let mut names: Vec<String> = bs
                            .iter()
                            .map(|b| match &b.pattern {
                                // A binder takes a pattern, and a name is
                                // one of those, which reads as the rename
                                // it has always been.
                                Some(pattern) => format!(
                                    "{}: {}",
                                    self.sym(b.field.sym),
                                    self.pattern_text(pattern, false)
                                ),
                                None => self.sym(b.field.sym).to_string(),
                            })
                            .collect();
                        if *rest {
                            names.push("..".to_string());
                        }
                        format!("({})", names.join(", "))
                    }
                };
                let names: Vec<&str> = segments.iter().map(|n| self.sym(n.sym)).collect();
                let dot = if *leading_dot { "." } else { "" };
                let kind = if top { "variant " } else { "" };
                format!("{kind}{dot}{}{binders}", names.join("::"))
            }
            // A number, a `bool` or text the value must equal, and `|`
            // for any one of several.
            PatternKind::Int {
                negative,
                magnitude,
            } => {
                let sign = if *negative { "-" } else { "" };
                format!("int {sign}{magnitude}")
            }
            PatternKind::Bool(value) => format!("bool {value}"),
            PatternKind::Str(sym) => format!("str \"{}\"", self.sym(*sym)),
            PatternKind::Char(c) => format!("char {}", show_char(*c)),
            PatternKind::Byte(b) => format!("byte {}", show_byte(*b)),
            // `lo..hi`, `lo..=hi`, an end left off.
            PatternKind::Range { lo, hi, inclusive } => {
                let bound = |bound: &Option<RangeBound>| match bound {
                    None => String::new(),
                    Some(bound) => match &bound.kind {
                        RangeBoundKind::Int {
                            negative,
                            magnitude,
                        } => format!("{}{magnitude}", if *negative { "-" } else { "" }),
                        RangeBoundKind::Char(c) => show_char(*c),
                        RangeBoundKind::Byte(b) => show_byte(*b),
                        RangeBoundKind::Const(path) => path
                            .iter()
                            .map(|n| self.sym(n.sym))
                            .collect::<Vec<_>>()
                            .join("::"),
                    },
                };
                let dots = if *inclusive { "..=" } else { ".." };
                format!("range {}{dots}{}", bound(lo), bound(hi))
            }
            PatternKind::Any(alternatives) => {
                let texts: Vec<String> = alternatives
                    .iter()
                    .map(|p| self.pattern_text(p, top))
                    .collect();
                texts.join(" | ")
            }
            // `[first, ..rest]`.
            PatternKind::Slice(elements) => {
                let parts: Vec<String> = elements
                    .iter()
                    .map(|element| match element {
                        SliceElement::Pattern(p) => self.pattern_text(p, false),
                        SliceElement::Rest { name, .. } => {
                            format!("..{}", name.map_or("", |n| self.sym(n.sym)))
                        }
                    })
                    .collect();
                format!("slice [{}]", parts.join(", "))
            }
            PatternKind::Error => "error".to_string(),
        }
    }
}

/// A character as a dump shows it: `'x'`, escaped where it is not
/// printable.
fn show_byte(value: u8) -> String {
    format!("b{:?}", value as char)
}

fn show_char(value: u32) -> String {
    match char::from_u32(value) {
        Some(c) => format!("{:?}", c),
        None => format!("'\\u{{{value:x}}}'"),
    }
}
