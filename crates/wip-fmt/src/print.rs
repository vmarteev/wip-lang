//! The syntax tree as a document: every construct, with
//! the line breaks it may take where Wip allows one — after `(`, `[`, `{`,
//! a comma, a binary operator, `=` and `=>` — and the comments found
//! between its tokens.

use wip_syntax::ast::*;
use wip_syntax::{Interner, MAX_TUPLE, MIN_TUPLE, Span, Symbol};

use crate::comments::{Comment, blank_between};
use crate::doc::{Doc, choice, concat, flat, group, if_break, indent, join, rigid, text};

pub struct Printer<'a> {
    src: &'a str,
    ast: &'a Ast,
    interner: &'a Interner,
    comments: Vec<Comment>,
    /// The first comment not written yet.
    next: usize,
}

impl<'a> Printer<'a> {
    pub fn new(src: &'a str, ast: &'a Ast, interner: &'a Interner, comments: Vec<Comment>) -> Self {
        Printer {
            src,
            ast,
            interner,
            comments,
            next: 0,
        }
    }

    fn sym(&self, sym: Symbol) -> &'a str {
        self.interner.resolve(sym)
    }

    /// What was written at `span`: a literal is printed as it was.
    fn source(&self, span: Span) -> &'a str {
        &self.src[span.lo as usize..span.hi as usize]
    }

    // ---- Comments and blank lines --------------------------------------

    /// Where the next thing really begins: at its first comment on a line
    /// of its own, if one comes before it.
    fn start_of(&self, at: u32) -> u32 {
        match self.comments.get(self.next) {
            Some(c) if c.at < at && c.own_line => c.at,
            _ => at,
        }
    }

    /// What goes between two things of a list of declarations or
    /// statements: a line break, or a blank line where the file had one.
    fn between(&self, prev_end: u32, next_start: u32) -> Doc {
        let start = self.start_of(next_start);
        if blank_between(self.src, prev_end as usize, start as usize) {
            Doc::Blank
        } else {
            Doc::Hard
        }
    }

    /// The comments before `at`: one that ended a line is put back at the
    /// end of its line, and one on a line of its own is written on one,
    /// before what follows.
    fn leading(&mut self, at: u32) -> Doc {
        let mut parts = Vec::new();
        let mut last_end: Option<u32> = None;
        while let Some(c) = self.comments.get(self.next).cloned() {
            if c.at >= at {
                break;
            }
            self.next += 1;
            if !c.own_line {
                parts.push(Doc::LineEnd(format!(" {}", c.text)));
                continue;
            }
            if let Some(end) = last_end
                && blank_between(self.src, end as usize, c.at as usize)
            {
                parts.push(Doc::Blank);
            } else if last_end.is_some() {
                parts.push(Doc::Hard);
            }
            parts.push(text(c.text.clone()));
            last_end = Some(c.at + c.text.len() as u32);
        }
        if let Some(end) = last_end {
            parts.push(if blank_between(self.src, end as usize, at as usize) {
                Doc::Blank
            } else {
                Doc::Hard
            });
        }
        concat(parts)
    }

    /// The comments left before `end`, inside something that is closing:
    /// each on a line of its own, after what came before, or at the end of
    /// the last line.
    fn trailing(&mut self, end: u32) -> Doc {
        let mut parts = Vec::new();
        let mut last_end: Option<u32> = None;
        while let Some(c) = self.comments.get(self.next).cloned() {
            if c.at >= end {
                break;
            }
            self.next += 1;
            if !c.own_line {
                parts.push(Doc::LineEnd(format!(" {}", c.text)));
                continue;
            }
            let blank = match last_end {
                Some(e) => blank_between(self.src, e as usize, c.at as usize),
                None => c.blank_before,
            };
            parts.push(if blank { Doc::Blank } else { Doc::Hard });
            parts.push(text(c.text.clone()));
            last_end = Some(c.at + c.text.len() as u32);
        }
        concat(parts)
    }

    // ---- A file --------------------------------------------------------

    pub fn file(&mut self) -> Doc {
        let mut parts = Vec::new();
        let mut prev_end: Option<u32> = None;
        let items = self.ast.items.clone();
        for item in &items {
            let span = item_span(item);
            if let Some(end) = prev_end {
                parts.push(self.between(end, span.lo));
            }
            parts.push(self.leading(span.lo));
            parts.push(self.item(item));
            prev_end = Some(span.hi);
        }
        let rest = self.trailing(u32::MAX);
        if prev_end.is_none() {
            // Nothing but comments.
            let rest = strip_leading_break(rest);
            return rest;
        }
        parts.push(rest);
        concat(parts)
    }

    /// A `package.wip`: its annotations, and `package NAME`.
    pub fn package(&mut self, annotations: &[Annotation], name: Option<Name>) -> Doc {
        let mut parts = Vec::new();
        for annotation in annotations {
            parts.push(self.leading(annotation.span.lo));
            parts.push(self.annotation(annotation));
            parts.push(Doc::Hard);
        }
        if let Some(name) = name {
            parts.push(self.leading(name.span.lo));
            parts.push(text(format!("package {}", self.sym(name.sym))));
        }
        parts.push(self.trailing(u32::MAX));
        concat(parts)
    }

    // ---- Items ---------------------------------------------------------

    fn annotation(&self, annotation: &Annotation) -> Doc {
        let name = self.sym(annotation.name.sym);
        if annotation.parens.is_none() {
            return text(format!("@{name}"));
        }
        let args: Vec<String> = annotation
            .args
            .iter()
            .map(|arg| {
                let written = self.source(arg.span);
                match arg.name {
                    Some(key) => {
                        let value = written.split_once('=').map_or(written, |(_, v)| v).trim();
                        format!("{} = {value}", self.sym(key.sym))
                    }
                    None => written.trim().to_string(),
                }
            })
            .collect();
        text(format!("@{name}({})", args.join(", ")))
    }

    /// Annotations, each on a line of its own, before what they are on.
    fn annotations(&mut self, annotations: &[Annotation]) -> Doc {
        let mut parts = Vec::new();
        for annotation in annotations {
            parts.push(self.leading(annotation.span.lo));
            parts.push(self.annotation(annotation));
            parts.push(Doc::Hard);
        }
        concat(parts)
    }

    fn item(&mut self, item: &Item) -> Doc {
        match item {
            Item::Import(i) => self.import(i),
            Item::Val(v) => {
                let head = concat(vec![
                    self.annotations(&v.annotations),
                    text(format!("{}val {}", pub_(v.is_pub), self.sym(v.name.sym))),
                    self.type_annotation(v.ty),
                ]);
                let value = self.assigned(v.value);
                concat(vec![head, value])
            }
            Item::Assert(a) => concat(vec![self.annotations(&a.annotations), self.expr(a.assert)]),
            Item::Struct(s) => self.struct_decl(s, false),
            Item::Enum(e) => self.enum_decl(e),
            Item::Fn(f) => self.fn_decl(f),
            Item::Extern(e) => self.extern_block(e),
            Item::Extend(e) => self.extend_block(e),
            Item::Interface(i) => self.interface(i),
            Item::Type(t) => concat(vec![
                text(format!(
                    "{}type {}{} = ",
                    pub_(t.is_pub),
                    self.sym(t.name.sym),
                    self.generics(&t.generics)
                )),
                self.ty(t.ty),
            ]),
        }
    }

    /// `import a::b::{c, d}` on one line where it fits, and otherwise the
    /// names one to a line, one deeper, with no comma after the last.
    fn import(&mut self, i: &ImportDecl) -> Doc {
        let path: Vec<&str> = i.path.iter().map(|n| self.sym(n.sym)).collect();
        let mut parts = vec![text(format!("import {}", path.join("::")))];
        if let Some(items) = &i.items {
            let names: Vec<Doc> = items
                .iter()
                .map(|item| {
                    let name = item.name.map_or("self", |n| self.sym(n.sym));
                    text(match item.alias {
                        Some(alias) => format!("{name} as {}", self.sym(alias.sym)),
                        None => name.to_string(),
                    })
                })
                .collect();
            parts.push(text("::"));
            parts.push(self.list_ending("{", names, "}", false));
        }
        if let Some(alias) = i.alias {
            parts.push(text(format!(" as {}", self.sym(alias.sym))));
        }
        concat(parts)
    }

    fn generics(&self, generics: &[GenericParam]) -> String {
        if generics.is_empty() {
            return String::new();
        }
        let params: Vec<String> = generics
            .iter()
            .map(|g| {
                let name = self.sym(g.name.sym);
                // `type Item`: a type each implementation decides.
                let mut text = match g.decided {
                    Some(_) => format!("type {name}"),
                    None => name.to_string(),
                };
                if !g.bounds.is_empty() {
                    let bounds: Vec<String> = g.bounds.iter().map(|b| self.bound(b)).collect();
                    text.push_str(&format!(": {}", bounds.join(" + ")));
                }
                // `K = T`.
                if let Some(default) = g.default {
                    text.push_str(&format!(" = {}", self.ty_text(default)));
                }
                text
            })
            .collect();
        format!("<{}>", params.join(", "))
    }

    fn bound(&self, b: &Bound) -> String {
        let name = self.sym(b.name.sym);
        if b.args.is_empty() {
            return name.to_string();
        }
        let args: Vec<String> = b.args.iter().map(|&t| self.ty_text(t)).collect();
        format!("{name}<{}>", args.join(", "))
    }

    fn type_annotation(&self, ty: Option<TypeId>) -> Doc {
        match ty {
            Some(ty) => concat(vec![text(": "), self.ty(ty)]),
            None => concat(vec![]),
        }
    }

    fn struct_decl(&mut self, s: &StructDecl, in_block: bool) -> Doc {
        let mut head = pub_(s.is_pub).to_string();
        if s.is_view {
            head.push_str("view ");
        }
        if s.is_extern && !in_block {
            head.push_str("extern ");
        }
        head.push_str(if s.is_union { "union " } else { "struct " });
        head.push_str(self.sym(s.name.sym));
        head.push_str(&self.generics(&s.generics));
        let mut body: Vec<(Span, Doc)> = Vec::new();
        for field in &s.fields {
            let lead = self.leading(field.span.lo);
            // A line break separates declarations, so a
            // field, as a variant or an arm, needs no comma.
            body.push((field.span, concat(vec![lead, self.field(field)])));
        }
        let methods_start = body.len();
        for method in &s.methods {
            let lead = self.leading(method.span.lo);
            body.push((method.span, concat(vec![lead, self.fn_decl(method)])));
        }
        let annotations = self.annotations(&s.annotations);
        // A struct with no fields and no methods is its name alone, unless
        // its braces hold a comment, which keeps them. A `view` or `extern`
        // one keeps its braces: it is refused without them.
        if body.is_empty() && !s.is_view && !s.is_extern {
            let braced = s.span.hi > 0 && self.src.as_bytes()[s.span.hi as usize - 1] == b'}';
            let inside = if braced {
                self.trailing(s.span.hi - 1)
            } else {
                concat(vec![])
            };
            return match inside {
                Doc::Concat(ref parts) if parts.is_empty() => concat(vec![annotations, text(head)]),
                inside => concat(vec![
                    annotations,
                    text(head),
                    text(" {"),
                    indent(inside),
                    Doc::Hard,
                    text("}"),
                ]),
            };
        }
        let braces = self.braced(
            body,
            s.span.hi,
            Some(methods_start).filter(|&m| m > 0 && m < s.fields.len() + s.methods.len()),
        );
        concat(vec![annotations, text(head), text(" "), braces])
    }

    /// `{ … }` around declarations, one per line, with the blank lines the
    /// file had between them; `force_blank_at` puts a blank line before
    /// that one, as between a struct's fields and its methods.
    fn braced(&mut self, body: Vec<(Span, Doc)>, close: u32, force_blank_at: Option<usize>) -> Doc {
        if body.is_empty() {
            let inside = self.trailing(close.saturating_sub(1));
            return match inside {
                Doc::Concat(ref parts) if parts.is_empty() => text("{}"),
                inside => concat(vec![text("{"), indent(inside), Doc::Hard, text("}")]),
            };
        }
        let mut inner = Vec::new();
        let mut prev: Option<u32> = None;
        for (i, (span, doc)) in body.into_iter().enumerate() {
            match prev {
                None => inner.push(Doc::Hard),
                Some(_) if force_blank_at == Some(i) => inner.push(Doc::Blank),
                Some(end) => inner.push(self.between(end, span.lo)),
            }
            inner.push(doc);
            prev = Some(span.hi);
        }
        inner.push(self.trailing(close.saturating_sub(1)));
        concat(vec![text("{"), indent(concat(inner)), Doc::Hard, text("}")])
    }

    fn field(&mut self, f: &Field) -> Doc {
        let mut parts = vec![text(format!(
            "{}{}{}: ",
            pub_(f.is_pub),
            if f.is_var { "var " } else { "" },
            self.sym(f.name.sym)
        ))];
        parts.push(self.ty(f.ty));
        if let Some(default) = f.default {
            parts.push(text(" = "));
            parts.push(self.expr(default));
        }
        concat(parts)
    }

    fn enum_decl(&mut self, e: &EnumDecl) -> Doc {
        let head = format!(
            "{}{}enum {}{}",
            pub_(e.is_pub),
            if e.is_view { "view " } else { "" },
            self.sym(e.name.sym),
            self.generics(&e.generics)
        );
        let mut body: Vec<(Span, Doc)> = Vec::new();
        for v in &e.variants {
            let lead = self.leading(v.span.lo);
            let mut doc = vec![lead, text(self.sym(v.name.sym))];
            if !v.fields.is_empty() {
                let fields: Vec<Doc> = v.fields.iter().map(|f| self.field(f)).collect();
                doc.push(self.list("(", fields, ")"));
            }
            body.push((v.span, concat(doc)));
        }
        let methods_start = body.len();
        for method in &e.methods {
            let lead = self.leading(method.span.lo);
            body.push((method.span, concat(vec![lead, self.fn_decl(method)])));
        }
        let annotations = self.annotations(&e.annotations);
        let force = Some(methods_start).filter(|&m| m > 0 && !e.methods.is_empty());
        let braces = self.braced(body, e.span.hi, force);
        concat(vec![annotations, text(head), text(" "), braces])
    }

    fn fn_sig(&mut self, keyword: &str, is_pub: bool, sig: &FnSig) -> Doc {
        let mut params: Vec<Doc> = sig
            .params
            .iter()
            .map(|p| {
                let lead = self.leading(p.span.lo);
                concat(vec![lead, self.field(p)])
            })
            .collect();
        if sig.variadic.is_some() {
            params.push(text("..."));
        }
        let mut parts = vec![
            text(format!(
                "{}{keyword} {}{}",
                pub_(is_pub),
                self.sym(sig.name.sym),
                self.generics(&sig.generics)
            )),
            self.list_ending("(", params, ")", sig.variadic.is_none()),
        ];
        if let Some(ret) = sig.ret {
            parts.push(text(": "));
            parts.push(self.ty(ret));
        }
        // What the result borrows: `from a, self.ast`.
        if !sig.lends_from.is_empty() {
            let paths: Vec<String> = sig
                .lends_from
                .iter()
                .map(|path| {
                    let mut written = match path.root {
                        Some(name) => self.sym(name.sym).to_string(),
                        None => "self".to_string(),
                    };
                    for field in &path.fields {
                        written.push('.');
                        written.push_str(self.sym(field.sym));
                    }
                    written
                })
                .collect();
            parts.push(text(format!(" from {}", paths.join(", "))));
        }
        concat(parts)
    }

    fn fn_decl(&mut self, f: &FnDecl) -> Doc {
        let annotations = self.annotations(&f.annotations);
        let keyword = f.receiver.map_or("fn", |(r, _)| r.text());
        let sig = self.fn_sig(keyword, f.is_pub, &f.sig);
        let body = match f.body {
            Some(body) => self.body(body),
            None => concat(vec![]),
        };
        concat(vec![annotations, group(concat(vec![sig, body]))])
    }

    fn extern_block(&mut self, e: &ExternBlock) -> Doc {
        let annotations = self.annotations(&e.annotations);
        let head = format!("extern {}", self.source(e.abi));
        // Its members in the order they were written: the tree keeps them
        // by kind.
        enum Member<'b> {
            Fn(&'b ExternFn),
            Type(&'b ExternType),
            Global(&'b ExternGlobal),
            Struct(&'b StructDecl),
        }
        let mut members: Vec<(Span, Member)> = Vec::new();
        for f in &e.fns {
            let start = f
                .annotations
                .first()
                .map_or(f.sig.span, |a| a.span.to(f.sig.span));
            members.push((start, Member::Fn(f)));
        }
        for t in &e.types {
            let start = t
                .annotations
                .first()
                .map_or(t.name.span, |a| a.span.to(t.name.span));
            members.push((start, Member::Type(t)));
        }
        for g in &e.globals {
            let start = g.annotations.first().map_or(g.span, |a| a.span.to(g.span));
            members.push((start, Member::Global(g)));
        }
        for s in &e.structs {
            members.push((s.span, Member::Struct(s)));
        }
        members.sort_by_key(|(span, _)| span.lo);
        let mut body: Vec<(Span, Doc)> = Vec::new();
        for (span, member) in members {
            let lead = self.leading(span.lo);
            let doc = match member {
                Member::Fn(f) => {
                    let annotations = self.annotations(&f.annotations);
                    let sig = self.fn_sig("fn", f.is_pub, &f.sig);
                    concat(vec![annotations, group(sig)])
                }
                Member::Type(t) => {
                    let annotations = self.annotations(&t.annotations);
                    concat(vec![
                        annotations,
                        text(format!("{}type {}", pub_(t.is_pub), self.sym(t.name.sym))),
                    ])
                }
                Member::Global(g) => {
                    let annotations = self.annotations(&g.annotations);
                    concat(vec![
                        annotations,
                        text(format!(
                            "{}{} {}: ",
                            pub_(g.is_pub),
                            if g.is_mut { "var" } else { "val" },
                            self.sym(g.name.sym)
                        )),
                        self.ty(g.ty),
                    ])
                }
                Member::Struct(s) => self.struct_decl(s, true),
            };
            body.push((span, concat(vec![lead, doc])));
        }
        let braces = self.braced(body, e.span.hi, None);
        concat(vec![annotations, text(head), text(" "), braces])
    }

    fn extend_block(&mut self, e: &ExtendBlock) -> Doc {
        let annotations = self.annotations(&e.annotations);
        let mut head = String::from("extend ");
        match e.slice_of {
            Some(element) => {
                // `extend [T: Ord] { … }`.
                let bounds: Vec<String> = e
                    .generics
                    .iter()
                    .find(|g| g.name.sym == element.sym)
                    .map(|g| g.bounds.iter().map(|b| self.bound(b)).collect())
                    .unwrap_or_default();
                head.push('[');
                head.push_str(self.sym(element.sym));
                if !bounds.is_empty() {
                    head.push_str(": ");
                    head.push_str(&bounds.join(" + "));
                }
                head.push(']');
            }
            None => {
                let path: Vec<&str> = e.path.iter().map(|n| self.sym(n.sym)).collect();
                head.push_str(&path.join("::"));
                head.push_str(&self.generics(&e.generics));
            }
        }
        if let Some(interface) = e.interface {
            head.push_str(": ");
            head.push_str(self.sym(interface.sym));
            if let Some(args) = &e.interface_args {
                let args: Vec<String> = args.args.iter().map(|&t| self.ty_text(t)).collect();
                head.push_str(&format!("<{}>", args.join(", ")));
            }
        }
        let mut body: Vec<(Span, Doc)> = Vec::new();
        for method in &e.methods {
            let lead = self.leading(method.span.lo);
            body.push((method.span, concat(vec![lead, self.fn_decl(method)])));
        }
        let braces = self.braced(body, e.span.hi, None);
        concat(vec![annotations, text(head), text(" "), braces])
    }

    fn interface(&mut self, i: &InterfaceDecl) -> Doc {
        let annotations = self.annotations(&i.annotations);
        let head = format!(
            "{}interface {}{}",
            pub_(i.is_pub),
            self.sym(i.name.sym),
            self.generics(&i.generics)
        );
        let mut body: Vec<(Span, Doc)> = Vec::new();
        for m in &i.methods {
            let lead = self.leading(m.span.lo);
            let sig = self.fn_sig(m.receiver.0.text(), false, &m.sig);
            let default = match m.default {
                Some(body) => self.body(body),
                None => concat(vec![]),
            };
            body.push((
                m.span,
                concat(vec![lead, group(concat(vec![sig, default]))]),
            ));
        }
        let braces = self.braced(body, i.span.hi, None);
        concat(vec![annotations, text(head), text(" "), braces])
    }

    // ---- Lists ---------------------------------------------------------

    /// `open a, b close` on one line if it fits, and otherwise each on a
    /// line of its own, one deeper, with a trailing comma.
    fn list(&self, open: &str, items: Vec<Doc>, close: &str) -> Doc {
        self.list_ending(open, items, close, true)
    }

    /// The same, with a comma after the last item where it is broken or
    /// none: none after `...`, which ends a C function's parameters, and
    /// none after an import's last name.
    fn list_ending(&self, open: &str, items: Vec<Doc>, close: &str, comma: bool) -> Doc {
        if items.is_empty() {
            return text(format!("{open}{close}"));
        }
        let trailing = if comma {
            if_break(text(","), text(""))
        } else {
            text("")
        };
        group(concat(vec![
            text(open),
            indent(concat(vec![
                Doc::Soft,
                join(items, concat(vec![text(","), Doc::Line])),
            ])),
            trailing,
            Doc::Soft,
            text(close),
        ]))
    }

    // ---- Types ---------------------------------------------------------

    fn ty(&self, id: TypeId) -> Doc {
        text(self.ty_text(id))
    }

    fn ty_text(&self, id: TypeId) -> String {
        let ty = &self.ast.types[id];
        match &ty.kind {
            TypeKind::Named { name, args } => {
                if let Some(n) = tuple_arity(*name) {
                    let _ = n;
                    let elems: Vec<String> = args.iter().map(|&t| self.ty_text(t)).collect();
                    return format!("({})", elems.join(", "));
                }
                let name = self.sym(*name);
                if args.is_empty() {
                    name.to_string()
                } else {
                    let args: Vec<String> = args.iter().map(|&t| self.ty_text(t)).collect();
                    format!("{name}<{}>", args.join(", "))
                }
            }
            TypeKind::Path { segments, args } => {
                let path: Vec<&str> = segments.iter().map(|n| self.sym(n.sym)).collect();
                let path = path.join("::");
                if args.is_empty() {
                    path
                } else {
                    let args: Vec<String> = args.iter().map(|&t| self.ty_text(t)).collect();
                    format!("{path}<{}>", args.join(", "))
                }
            }
            TypeKind::Own(inner) => format!("own<{}>", self.ty_text(*inner)),
            TypeKind::Ref { var, inner } => {
                let inner = self.ty_text(*inner);
                // `& &i64`: two borrows, which written together are `&&`.
                let space = if !*var && inner.starts_with('&') {
                    " "
                } else {
                    ""
                };
                format!("&{}{space}{inner}", if *var { "var " } else { "" })
            }
            TypeKind::Array { elem, len } => {
                let len = match len {
                    ArrayLen::Int(n) => n.to_string(),
                    ArrayLen::Name(name) => self.sym(name.sym).to_string(),
                };
                format!("[{}; {len}]", self.ty_text(*elem))
            }
            TypeKind::Slice(elem) => format!("[{}]", self.ty_text(*elem)),
            TypeKind::Dyn(name, args) => {
                let name = self.sym(name.sym);
                if args.is_empty() {
                    format!("dyn {name}")
                } else {
                    let args: Vec<String> = args.iter().map(|&t| self.ty_text(t)).collect();
                    format!("dyn {name}<{}>", args.join(", "))
                }
            }
            TypeKind::Fn { params, ret } => {
                let params: Vec<String> = params
                    .iter()
                    .map(|p| format!("{}: {}", self.sym(p.name.sym), self.ty_text(p.ty)))
                    .collect();
                format!("({}) => {}", params.join(", "), self.ty_text(*ret))
            }
            TypeKind::Error => self.source(ty.span).to_string(),
        }
    }

    // ---- Statements and blocks -----------------------------------------

    /// A block. `flat` says it may be written on one line where it is one
    /// expression that fits: a value, as a branch of an `if` that answers
    /// one or a lambda's body is; a function's body, a loop's and a
    /// statement's are always broken.
    fn block(&mut self, b: &Block) -> Doc {
        self.block_as(b, true)
    }

    fn block_as(&mut self, b: &Block, flat: bool) -> Doc {
        let close = b.span.hi.saturating_sub(1);
        if b.stmts.is_empty() {
            let inside = self.trailing(close);
            return match inside {
                Doc::Concat(ref parts) if parts.is_empty() => text("{}"),
                inside => concat(vec![text("{"), indent(inside), Doc::Hard, text("}")]),
            };
        }
        // One expression, and nothing said about it: on one line if it
        // fits, as `if a < b { a } else { b }` is written.
        // A `break`, `continue` or `return` alone is as short as a value.
        if let [only] = b.stmts[..]
            && flat
            && matches!(
                self.ast.stmts[only].kind,
                StmtKind::Break(_)
                    | StmtKind::Continue(_)
                    | StmtKind::Return(_)
                    | StmtKind::Yield(_)
            )
            && !self.comment_before(close)
        {
            let e = self.stmt(only);
            return group(concat(vec![
                text("{"),
                indent(concat(vec![Doc::Line, e])),
                Doc::Line,
                text("}"),
            ]));
        }
        if let [only] = b.stmts[..]
            && flat
            && let StmtKind::Expr(e) = self.ast.stmts[only].kind
            && !self.comment_before(close)
            && !is_block_like(&self.ast.exprs[e].kind)
        {
            let e = self.expr(e);
            return group(concat(vec![
                text("{"),
                indent(concat(vec![Doc::Line, e])),
                Doc::Line,
                text("}"),
            ]));
        }
        let mut inner = Vec::new();
        let mut prev: Option<u32> = None;
        for &stmt in &b.stmts {
            let span = self.ast.stmts[stmt].span;
            match prev {
                None => inner.push(Doc::Hard),
                Some(end) => inner.push(self.between(end, span.lo)),
            }
            inner.push(self.leading(span.lo));
            inner.push(self.stmt(stmt));
            prev = Some(span.hi);
        }
        inner.push(self.trailing(close));
        concat(vec![text("{"), indent(concat(inner)), Doc::Hard, text("}")])
    }

    /// Whether a comment comes before `end` that has not been written.
    fn comment_before(&self, end: u32) -> bool {
        self.comments.get(self.next).is_some_and(|c| c.at < end)
    }

    fn stmt(&mut self, id: StmtId) -> Doc {
        let stmt = self.ast.stmts[id].clone();
        match &stmt.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                init,
            } => concat(vec![
                text(format!(
                    "{} {}",
                    if *mutable { "var" } else { "val" },
                    self.sym(name.sym)
                )),
                self.type_annotation(*ty),
                self.assigned(*init),
            ]),
            StmtKind::Guard {
                mutable,
                pattern,
                value,
                else_block,
            } => {
                let keyword = if *mutable { "var " } else { "val " };
                let mut parts = vec![text(keyword), self.pattern(pattern), self.assigned(*value)];
                if let Some(block) = else_block {
                    parts.push(text(" else "));
                    parts.push(self.block_as(block, false));
                }
                concat(parts)
            }
            StmtKind::Defer(e) => concat(vec![text("defer "), self.expr(*e)]),
            StmtKind::Return(None) => text("return"),
            StmtKind::Return(Some(e)) => concat(vec![text("return "), self.expr(*e)]),
            StmtKind::Yield(e) => concat(vec![text("yield "), self.expr(*e)]),
            StmtKind::While { label, cond, body } => concat(vec![
                text(self.label(label.as_ref())),
                text("while "),
                self.expr(*cond),
                text(" "),
                self.block_as(body, false),
            ]),
            StmtKind::For {
                label,
                binding,
                source,
                body,
            } => {
                let source = match source {
                    ForSource::Elements(e) => self.expr(*e),
                    ForSource::Range { lo, hi, inclusive } => concat(vec![
                        self.expr(*lo),
                        text(if *inclusive { "..=" } else { ".." }),
                        self.expr(*hi),
                    ]),
                };
                concat(vec![
                    text(self.label(label.as_ref())),
                    text("for "),
                    self.pattern(binding),
                    text(" in "),
                    source,
                    text(" "),
                    self.block_as(body, false),
                ])
            }
            StmtKind::Break(label) => text(match label {
                Some(l) => format!("break {}", self.sym(l.sym)),
                None => "break".to_string(),
            }),
            StmtKind::Continue(label) => text(match label {
                Some(l) => format!("continue {}", self.sym(l.sym)),
                None => "continue".to_string(),
            }),
            // An `if` without an `else` is a statement, and broken as one;
            // with one, it answers a value, as the last of a block does.
            StmtKind::Expr(e)
                if matches!(
                    self.ast.exprs[*e].kind,
                    ExprKind::If {
                        else_branch: None,
                        ..
                    }
                ) =>
            {
                self.if_chain(*e, true)
            }
            StmtKind::Expr(e) => self.expr(*e),
        }
    }

    /// `if … { } else if … { } else { }`, as one: flat only where it is one
    /// `if` and one `else`, each a single expression, that fit on the line
    /// together, and broken otherwise, every branch alike.
    fn if_chain(&mut self, id: ExprId, statement: bool) -> Doc {
        match self.if_forms(id, statement) {
            (Some(then), braced) => choice(then, braced),
            (None, braced) => braced,
        }
    }

    /// An `if` as the `then` form, where every branch is one expression or
    /// one jump, and with braces. The `then` form is written where each
    /// condition fits on its line with its `then`, on a line where the
    /// whole fits and broken before each `else` where it does not; braces
    /// where a condition must be broken.
    fn if_forms(&mut self, id: ExprId, statement: bool) -> (Option<Doc>, Doc) {
        let branches = self.branches(id);
        if !branches.iter().all(|(_, b)| self.then_branch(b)) {
            return (None, self.braced_chain(&branches, statement));
        }
        // Each is printed from the same comment on.
        let next = self.next;
        let then = self.then_chain(&branches);
        let after = self.next;
        self.next = next;
        let braced = self.braced_chain(&branches, statement);
        debug_assert_eq!(self.next, after, "both forms write the same comments");
        (Some(then), braced)
    }

    fn braced_chain(&mut self, branches: &[(Option<ExprId>, Block)], statement: bool) -> Doc {
        // One `if` and one `else`, each a single expression — or, among a
        // list's elements, one `if` that yields one value.
        // An `if` alone that does anything else is a statement, and broken
        // as one wherever it stands.
        let one_yield = |b: &Block| matches!(b.stmts[..], [only] if matches!(self.ast.stmts[only].kind, StmtKind::Yield(_)));
        let simple = !statement
            && ((branches.len() == 2 && branches[1].0.is_none())
                || (branches.len() == 1 && one_yield(&branches[0].1)))
            && branches.iter().all(|(_, b)| self.one_expression(b));
        let mut parts = Vec::new();
        for (i, (cond, b)) in branches.iter().enumerate() {
            if i > 0 {
                parts.push(text(" else "));
            }
            if let Some(cond) = cond {
                parts.push(text("if "));
                parts.push(self.expr(*cond));
                parts.push(text(" "));
            }
            parts.push(if simple {
                self.open_block(b)
            } else {
                self.block_as(b, false)
            });
        }
        if simple {
            group(concat(parts))
        } else {
            concat(parts)
        }
    }

    /// The branches of an `if` and the `else if`s after it, each with its
    /// condition, and the last `else`'s without one.
    fn branches(&self, id: ExprId) -> Vec<(Option<ExprId>, Block)> {
        let mut branches: Vec<(Option<ExprId>, Block)> = Vec::new();
        let mut at = id;
        loop {
            match self.ast.exprs[at].kind.clone() {
                ExprKind::If {
                    cond,
                    then_block,
                    else_branch,
                } => {
                    branches.push((Some(cond), then_block));
                    match else_branch {
                        Some(next) => at = next,
                        None => break,
                    }
                }
                ExprKind::Block(b) => {
                    branches.push((None, b));
                    break;
                }
                _ => unreachable!("an `else` is a block or an `if`"),
            }
        }
        branches
    }

    /// Whether a branch is what a `then` form holds: one
    /// expression — not a block, a `match` or another `if`, which keep
    /// their braces — or one `return`, `break` or `continue`, with no
    /// comment in it.
    fn then_branch(&self, b: &Block) -> bool {
        let [only] = b.stmts[..] else {
            return false;
        };
        let fits = match &self.ast.stmts[only].kind {
            StmtKind::Expr(e) => {
                !matches!(
                    self.ast.exprs[*e].kind,
                    ExprKind::Match { .. } | ExprKind::Block(_) | ExprKind::If { .. }
                ) || self.is_text(*e)
            }
            StmtKind::Return(_) | StmtKind::Break(_) | StmtKind::Continue(_) => true,
            _ => false,
        };
        fits && !self
            .comments
            .iter()
            .skip(self.next)
            .any(|c| c.at > b.span.lo && c.at < b.span.hi)
    }

    /// `if c then a else if d then b else e`: flat where it fits, and
    /// otherwise each `else` at the start of a line of its own, under the
    /// `if`, as a `when` is laid out. A branch too long for
    /// its line goes on the next, one deeper.
    fn then_chain(&mut self, branches: &[(Option<ExprId>, Block)]) -> Doc {
        let mut parts = Vec::new();
        for (i, (cond, b)) in branches.iter().enumerate() {
            if i > 0 {
                parts.push(Doc::Line);
                parts.push(text("else "));
            }
            let [only] = b.stmts[..] else {
                unreachable!("one expression")
            };
            let branch = match self.ast.stmts[only].kind {
                StmtKind::Expr(e) => self.expr(e),
                _ => self.stmt(only),
            };
            match cond {
                Some(cond) => {
                    parts.push(rigid(concat(vec![
                        text("if "),
                        self.expr(*cond),
                        text(" then"),
                    ])));
                    parts.push(group(indent(concat(vec![Doc::Line, branch]))));
                }
                None => parts.push(branch),
            }
        }
        group(concat(parts))
    }

    /// Whether a block is one expression, or one `yield`,
    /// with no comment in it.
    fn one_expression(&self, b: &Block) -> bool {
        match b.stmts[..] {
            [only] => {
                (matches!(self.ast.stmts[only].kind, StmtKind::Expr(e) if !is_block_like(&self.ast.exprs[e].kind))
                    || matches!(self.ast.stmts[only].kind, StmtKind::Yield(_)))
                    && !self
                        .comments
                        .iter()
                        .skip(self.next)
                        .any(|c| c.at > b.span.lo && c.at < b.span.hi)
            }
            _ => false,
        }
    }

    /// `{ x }` whose line breaks are the enclosing group's: flat with it, or
    /// broken with it.
    fn open_block(&mut self, b: &Block) -> Doc {
        let [only] = b.stmts[..] else {
            unreachable!("one expression")
        };
        let e = match self.ast.stmts[only].kind {
            StmtKind::Expr(e) => self.expr(e),
            StmtKind::Yield(_) => self.stmt(only),
            _ => unreachable!("an expression or a `yield`"),
        };
        concat(vec![
            text("{"),
            indent(concat(vec![Doc::Line, e])),
            Doc::Line,
            text("}"),
        ])
    }

    fn label(&self, label: Option<&Name>) -> String {
        label.map_or(String::new(), |l| format!("{}: ", self.sym(l.sym)))
    }

    /// A function's ` = body`: a block is always broken, as a function's
    /// body is; anything else is laid out as a value is.
    fn body(&mut self, body: ExprId) -> Doc {
        if let ExprKind::Block(b) = &self.ast.exprs[body].kind
            && !self.is_interpolated(body)
        {
            let b = b.clone();
            return concat(vec![text(" = "), self.block_as(&b, false)]);
        }
        self.assigned(body)
    }

    /// ` = value`: on the same line when the value breaks well there — a
    /// block, a call, a literal of several parts — and otherwise tried on
    /// the next line, one deeper, before the value itself is broken.
    fn assigned(&mut self, value: ExprId) -> Doc {
        let hugs = hugs(&self.ast.exprs[value].kind) || self.is_text(value);
        // An `if` is asked for its forms once: printing it writes the
        // comments inside it, and a second print would find none.
        let doc = match self.ast.exprs[value].kind {
            ExprKind::If { .. } => match self.if_forms(value, false) {
                (Some(then), braced) => {
                    return choice(
                        concat(vec![
                            text(" ="),
                            group(indent(concat(vec![Doc::Line, then]))),
                        ]),
                        concat(vec![text(" = "), braced]),
                    );
                }
                (None, braced) => braced,
            },
            ExprKind::Binary { .. } => self.binary_as(value, false),
            _ => self.expr(value),
        };
        if hugs {
            concat(vec![text(" = "), doc])
        } else {
            concat(vec![
                text(" ="),
                group(indent(concat(vec![Doc::Line, doc]))),
            ])
        }
    }

    // ---- Expressions ---------------------------------------------------

    /// An array of number literals, `-1` among them, with no comment
    /// inside it.
    fn is_table(&self, elems: &[ExprId], span: Span) -> bool {
        let number = |id: ExprId| match &self.ast.exprs[id].kind {
            ExprKind::Int(_) | ExprKind::Float(_) => true,
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
                ..
            } => matches!(
                self.ast.exprs[*operand].kind,
                ExprKind::Int(_) | ExprKind::Float(_)
            ),
            _ => false,
        };
        let commented = self
            .comments
            .get(self.next)
            .is_some_and(|c| c.at >= span.lo && c.at < span.hi);
        elems.iter().all(|&el| number(el)) && !commented
    }

    fn is_interpolated(&self, id: ExprId) -> bool {
        let e = &self.ast.exprs[id];
        matches!(e.kind, ExprKind::Block(_)) && self.source(e.span).starts_with('"')
    }

    /// A string laid out as it was written, which stays on the line it
    /// starts on: one with `\(…)` in it, or a text block.
    fn is_text(&self, id: ExprId) -> bool {
        let e = &self.ast.exprs[id];
        self.is_interpolated(id)
            || matches!(e.kind, ExprKind::Str(_)) && self.source(e.span).starts_with("\"\"\"")
    }

    /// A text block, plain or interpolated: what is written
    /// between two `\"\"\"`s.
    fn is_text_block(&self, id: ExprId) -> bool {
        self.is_text(id) && self.source(self.ast.exprs[id].span).starts_with("\"\"\"")
    }

    /// A text block, its lines one deeper than the line it
    /// starts on, and the closing `\"\"\"` with them: what each line has
    /// past the old closing `\"\"\"`'s indentation is kept, which is what
    /// keeps the text the same.
    fn text_block(&self, source: &str) -> Doc {
        let lines: Vec<&str> = source.split('\n').collect();
        let closing = lines.last().copied().unwrap_or_default();
        let old = &closing[..closing.len() - closing.trim_start_matches([' ', '\t']).len()];
        let mut parts = vec![text("\"\"\"")];
        for line in &lines[1..lines.len().saturating_sub(1)] {
            let rest = line
                .strip_prefix(old)
                .unwrap_or_else(|| line.trim_start_matches([' ', '\t']));
            parts.push(Doc::Hard);
            parts.push(text(rest.trim_end_matches([' ', '\t', '\r'])));
        }
        parts.push(Doc::Hard);
        parts.push(text(closing.trim_start_matches([' ', '\t'])));
        indent(concat(parts))
    }

    pub fn expr(&mut self, id: ExprId) -> Doc {
        let e = self.ast.exprs[id].clone();
        match &e.kind {
            ExprKind::Str(_) if self.is_text(id) => self.text_block(self.source(e.span)),
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Char(_)
            | ExprKind::Byte(_) => text(self.source(e.span)),
            // `for x in xs { value }`, an element of a list literal:
            // its body is a value, so it may be one line.
            ExprKind::ForElement {
                binding,
                source,
                body,
            } => {
                let source = match source {
                    ForSource::Elements(e) => self.expr(*e),
                    ForSource::Range { lo, hi, inclusive } => concat(vec![
                        self.expr(*lo),
                        text(if *inclusive { "..=" } else { ".." }),
                        self.expr(*hi),
                    ]),
                };
                concat(vec![
                    text("for "),
                    self.pattern(binding),
                    text(" in "),
                    source,
                    text(" "),
                    self.block(body),
                ])
            }
            // `"read \(rows) rows"`, which the parser rewrote into the
            // calls it means: as it was written.
            ExprKind::Block(_) if self.is_interpolated(id) => {
                self.skip_comments_in(e.span);
                let source = self.source(e.span);
                match source.starts_with("\"\"\"") {
                    true => self.text_block(source),
                    false => text(source),
                }
            }
            ExprKind::Bool(b) => text(if *b { "true" } else { "false" }),
            ExprKind::Null => text("null"),
            ExprKind::Name(sym) => text(self.sym(*sym)),
            ExprKind::SelfRef => text("self"),
            ExprKind::Paren(inner) => concat(vec![text("("), self.expr(*inner), text(")")]),
            ExprKind::Block(b) => self.block(b),
            ExprKind::If { .. } => self.if_chain(id, false),
            ExprKind::Path {
                leading_dot,
                segments,
                type_args,
            } => {
                let dot = if *leading_dot { "." } else { "" };
                text(format!(
                    "{dot}{}",
                    self.path_text(segments, type_args.as_ref())
                ))
            }
            ExprKind::Assert { cond, note, .. } => {
                let mut args = vec![self.expr(*cond)];
                if let Some(note) = note {
                    args.push(self.expr(*note));
                }
                concat(vec![text("assert"), self.list("(", args, ")")])
            }
            ExprKind::Lend(inner) => concat(vec![text("lend "), self.expr(*inner)]),
            // A table of numbers fills its lines, as Prettier fills one:
            // a line for each number would be most of the file.
            ExprKind::Array(elems) if elems.len() > 1 && self.is_table(elems, e.span) => {
                let mut words: Vec<String> = elems
                    .iter()
                    .map(|&el| match &self.ast.exprs[el].kind {
                        ExprKind::Unary { operand, .. } => {
                            format!("-{},", self.source(self.ast.exprs[*operand].span))
                        }
                        _ => format!("{},", self.source(self.ast.exprs[el].span)),
                    })
                    .collect();
                if let Some(last) = words.last_mut() {
                    last.pop();
                }
                group(concat(vec![
                    text("["),
                    indent(concat(vec![Doc::Soft, Doc::Fill(words)])),
                    if_break(text(","), text("")),
                    Doc::Soft,
                    text("]"),
                ]))
            }
            ExprKind::Array(elems) => {
                let elems: Vec<Doc> = elems
                    .iter()
                    .map(|&el| {
                        let lead = self.leading(self.ast.exprs[el].span.lo);
                        concat(vec![lead, self.expr(el)])
                    })
                    .collect();
                self.list("[", elems, "]")
            }
            ExprKind::ArrayRepeat { elem, count } => {
                let count = match count {
                    RepeatCount::Literal(n) => text(n.to_string()),
                    RepeatCount::Expr(e) => self.expr(*e),
                };
                concat(vec![
                    text("["),
                    self.expr(*elem),
                    text("; "),
                    count,
                    text("]"),
                ])
            }
            ExprKind::Match { scrutinee, arms } => {
                let head = concat(vec![text("match "), self.expr(*scrutinee), text(" ")]);
                let mut body: Vec<(Span, Doc)> = Vec::new();
                for arm in arms {
                    let lead = self.leading(arm.span.lo);
                    let mut parts = vec![lead, self.pattern(&arm.pattern)];
                    if let Some(guard) = arm.guard {
                        parts.push(text(" if "));
                        parts.push(self.expr(guard));
                    }
                    match self.lone_jump(arm.body) {
                        // A block that only leaves is written as the jump.
                        Some(jump) => {
                            parts.push(text(" => "));
                            parts.push(self.stmt(jump));
                        }
                        None => parts.push(self.arrow_body(arm.body)),
                    }
                    body.push((arm.span, group(concat(parts))));
                }
                let braces = self.braced(body, e.span.hi, None);
                concat(vec![head, braces])
            }
            ExprKind::Unary { op, operand, .. } => {
                // `& &x`: two borrows, which written together are `&&`.
                let borrows_a_borrow = matches!(op, UnaryOp::Ref)
                    && matches!(
                        self.ast.exprs[*operand].kind,
                        ExprKind::Unary {
                            op: UnaryOp::Ref | UnaryOp::RefVar,
                            ..
                        }
                    );
                let space = borrows_a_borrow
                    || matches!(op, UnaryOp::RefVar | UnaryOp::Move | UnaryOp::Own);
                concat(vec![
                    text(format!("{}{}", op.text(), if space { " " } else { "" })),
                    self.expr(*operand),
                ])
            }
            ExprKind::Binary { .. } => self.binary(id),
            ExprKind::Is {
                scrutinee,
                pattern,
                negated,
            } => concat(vec![
                self.expr(*scrutinee),
                text(if *negated { " !is " } else { " is " }),
                self.pattern(pattern),
            ]),
            ExprKind::Try(inner) => concat(vec![self.expr(*inner), text("?")]),
            ExprKind::Lambda { params, ret, body } => {
                let params: Vec<Doc> = params
                    .iter()
                    .map(|p| match p.ty {
                        Some(ty) => concat(vec![
                            text(format!("{}: ", self.sym(p.name.sym))),
                            self.ty(ty),
                        ]),
                        None => text(self.sym(p.name.sym)),
                    })
                    .collect();
                let mut parts = vec![self.list("(", params, ")")];
                if let Some(ret) = ret {
                    parts.push(text(": "));
                    parts.push(self.ty(*ret));
                }
                parts.push(self.arrow_body(*body));
                concat(parts)
            }
            ExprKind::Cast { expr, ty } => {
                concat(vec![self.expr(*expr), text(" as "), self.ty(*ty)])
            }
            ExprKind::Assign {
                target,
                op,
                wrapping,
                value,
            } => {
                let wraps = if *wrapping { "%" } else { "" };
                let op = op.map_or(String::new(), |(op, _)| format!("{}{wraps}", op.text()));
                let target = self.expr(*target);
                let hugs = hugs(&self.ast.exprs[*value].kind) || self.is_text(*value);
                let value_doc = self.expr(*value);
                if hugs {
                    concat(vec![target, text(format!(" {op}= ")), value_doc])
                } else {
                    concat(vec![
                        target,
                        text(format!(" {op}=")),
                        group(indent(concat(vec![Doc::Line, value_doc]))),
                    ])
                }
            }
            ExprKind::Field { base, name } => {
                if let Some(chain) = self.chain(id) {
                    return chain;
                }
                let name = match tuple_field_index(name.sym) {
                    Some(i) => i.to_string(),
                    None => self.sym(name.sym).to_string(),
                };
                concat(vec![self.expr(*base), text(format!(".{name}"))])
            }
            ExprKind::Call {
                callee,
                args,
                names,
                rest,
            } => {
                // `(a, b)`: the prelude's `Tuple2(a, b)`, written as the
                // sugar it was.
                if let ExprKind::Name(sym) = self.ast.exprs[*callee].kind
                    && tuple_arity(sym) == Some(args.len())
                    && rest.is_none()
                    && names.iter().all(Option::is_none)
                {
                    let elems: Vec<Doc> = args
                        .iter()
                        .map(|&arg| {
                            let lead = self.leading(self.ast.exprs[arg].span.lo);
                            concat(vec![lead, self.expr(arg)])
                        })
                        .collect();
                    return self.list("(", elems, ")");
                }
                if let Some(chain) = self.chain(id) {
                    return chain;
                }
                let callee_doc = self.expr(*callee);
                self.call_rest(&e, callee_doc, args, names, *rest)
            }
            ExprKind::Index { base, index } => concat(vec![
                self.expr(*base),
                text("["),
                self.expr(*index),
                text("]"),
            ]),
            ExprKind::SubSlice {
                base,
                lo,
                hi,
                inclusive,
            } => {
                let lo = lo.map_or(concat(vec![]), |lo| self.expr(lo));
                let hi = hi.map_or(concat(vec![]), |hi| self.expr(hi));
                concat(vec![
                    self.expr(*base),
                    text("["),
                    lo,
                    text(if *inclusive { "..=" } else { ".." }),
                    hi,
                    text("]"),
                ])
            }
            ExprKind::Error => text(self.source(e.span)),
        }
    }

    /// A call's arguments, after what is called: `callee_doc` is the
    /// callee as written, or a chain's `.name`.
    fn call_rest(
        &mut self,
        e: &Expr,
        callee_doc: Doc,
        args: &[ExprId],
        names: &[Option<Name>],
        rest: Option<ExprId>,
    ) -> Doc {
        // The last argument hugs the parentheses where it is a
        // literal of several parts or a lambda, and the others are
        // short: `push(Entry {` … `})`, as Prettier writes it.
        // Named or not: `.Column(children: own [` … `])`. A text
        // block does too: `println("""` … `""")`.
        if let Some(&last) = args.last()
            && rest.is_none()
            && (hugs_as_argument(&self.ast.exprs[last].kind) || self.is_text(last))
            && args[..args.len() - 1]
                .iter()
                .all(|&a| is_short(&self.ast.exprs[a].kind))
            && !self.comment_before(e.span.hi)
        {
            let mut parts = vec![callee_doc, text("(")];
            for (&arg, name) in args[..args.len() - 1].iter().zip(names) {
                if let Some(name) = name {
                    parts.push(text(format!("{}: ", self.sym(name.sym))));
                }
                parts.push(self.expr(arg));
                parts.push(text(", "));
            }
            if let Some(Some(name)) = names.last() {
                parts.push(text(format!("{}: ", self.sym(name.sym))));
            }
            parts.push(self.expr(last));
            parts.push(text(")"));
            return concat(parts);
        }
        let callee = callee_doc;
        let mut args: Vec<Doc> = args
            .iter()
            .zip(names)
            .map(|(&arg, name)| {
                let lead = self.leading(self.ast.exprs[arg].span.lo);
                match name {
                    Some(name) => concat(vec![
                        lead,
                        text(format!("{}: ", self.sym(name.sym))),
                        self.expr(arg),
                    ]),
                    None => concat(vec![lead, self.expr(arg)]),
                }
            })
            .collect();
        // `..base` comes last.
        if let Some(rest) = &rest {
            let lead = self.leading(self.ast.exprs[*rest].span.lo);
            args.push(concat(vec![lead, text(".."), self.expr(*rest)]));
        }
        concat(vec![callee, self.list("(", args, ")")])
    }

    /// A chain of three calls or more on a value,
    /// `xs.walk().filter(…).count()`: on one line where it fits, and
    /// otherwise broken after each `.` that a call follows, but the first,
    /// the rest one deeper. A field stays with the call after it,
    /// `self.row.entries.items()`, and a shorter chain is laid out as any
    /// call is, its last argument hugging the parentheses. A line that ends
    /// in `.` goes on, where one that starts with `.` would begin a variant.
    fn chain(&mut self, id: ExprId) -> Option<Doc> {
        // Each `.name`, innermost last: the field, and the call it is the
        // callee of.
        let mut segments: Vec<(ExprId, Option<ExprId>)> = Vec::new();
        let mut at = id;
        loop {
            match &self.ast.exprs[at].kind {
                ExprKind::Call { callee, .. }
                    if let ExprKind::Field { base, .. } = self.ast.exprs[*callee].kind =>
                {
                    segments.push((*callee, Some(at)));
                    at = base;
                }
                ExprKind::Field { base, .. } => {
                    segments.push((at, None));
                    at = *base;
                }
                _ => break,
            }
        }
        if segments.iter().filter(|(_, call)| call.is_some()).count() < 3 {
            return None;
        }
        segments.reverse();
        let head = self.expr(at);
        let mut first = Vec::new();
        let mut later = Vec::new();
        // The fields since the last call, which go with the next one.
        let mut fields: Vec<String> = Vec::new();
        let mut calls = 0;
        for (field, call) in segments {
            let ExprKind::Field { name, .. } = &self.ast.exprs[field].kind else {
                unreachable!("a segment is a field")
            };
            let name = match tuple_field_index(name.sym) {
                Some(index) => index.to_string(),
                None => self.sym(name.sym).to_string(),
            };
            fields.push(name);
            let Some(call) = call else {
                continue;
            };
            let named = text(std::mem::take(&mut fields).join("."));
            let e = self.ast.exprs[call].clone();
            let ExprKind::Call {
                args, names, rest, ..
            } = &e.kind
            else {
                unreachable!("a call")
            };
            let segment = self.call_rest(&e, named, args, names, *rest);
            if calls == 0 {
                first.push(text("."));
                first.push(segment);
            } else {
                later.push(text("."));
                later.push(Doc::Soft);
                later.push(segment);
            }
            calls += 1;
        }
        // Fields after the last call stay on its line.
        for name in fields {
            later.push(text(format!(".{name}")));
        }
        Some(group(concat(vec![
            head,
            concat(first),
            indent(concat(later)),
        ])))
    }

    /// ` => body`: on the same line when the body is a block or breaks well
    /// there, and otherwise tried on the next line, one deeper.
    /// The `return`, `break` or `continue` an arm's block holds and
    /// nothing else — no comment either, which the bare jump would lose.
    fn lone_jump(&self, body: ExprId) -> Option<StmtId> {
        let ExprKind::Block(block) = &self.ast.exprs[body].kind else {
            return None;
        };
        let [only] = block.stmts[..] else {
            return None;
        };
        let jumps = matches!(
            self.ast.stmts[only].kind,
            StmtKind::Return(_) | StmtKind::Break(_) | StmtKind::Continue(_)
        );
        (jumps && !self.comment_before(block.span.hi)).then_some(only)
    }

    fn arrow_body(&mut self, body: ExprId) -> Doc {
        // A text block opens on the arrow's line, as a block does; a string
        // of one line moves to the next where the arm is too long, rather
        // than the pattern being broken to make room for it.
        // An interpolated string is a block to the parser, and is asked
        // about as text first.
        let hugs = if self.is_text(body) {
            self.is_text_block(body)
        } else {
            hugs(&self.ast.exprs[body].kind)
        };
        // An `if` is asked for its forms once, as in `assigned`.
        let doc = match self.ast.exprs[body].kind {
            ExprKind::If { .. } => match self.if_forms(body, false) {
                (Some(then), braced) => {
                    return choice(
                        concat(vec![
                            text(" =>"),
                            group(indent(concat(vec![Doc::Line, then]))),
                        ]),
                        concat(vec![text(" => "), braced]),
                    );
                }
                (None, braced) => braced,
            },
            _ => self.expr(body),
        };
        if hugs {
            concat(vec![text(" => "), doc])
        } else {
            concat(vec![
                text(" =>"),
                group(indent(concat(vec![Doc::Line, doc]))),
            ])
        }
    }

    /// A chain of one precedence, `a + b - c`, broken after its operators,
    /// each operand after the first one deeper.
    fn binary(&mut self, id: ExprId) -> Doc {
        self.binary_as(id, true)
    }

    /// The same, where `deeper` says whether the operands after the first
    /// go one deeper: not where the chain is a whole value that an `=` has
    /// moved to its own line, one deeper already.
    fn binary_as(&mut self, id: ExprId, deeper: bool) -> Doc {
        let ExprKind::Binary { op, .. } = self.ast.exprs[id].kind else {
            unreachable!("a binary expression")
        };
        let level = precedence(op);
        let mut operands = Vec::new();
        let mut ops = Vec::new();
        let mut at = id;
        // Left-nested operators of the same precedence are one chain.
        loop {
            match self.ast.exprs[at].kind {
                ExprKind::Binary {
                    op,
                    lhs,
                    rhs,
                    wrapping,
                    ..
                } if precedence(op) == level => {
                    operands.push(rhs);
                    ops.push(format!("{}{}", op.text(), if wrapping { "%" } else { "" }));
                    at = lhs;
                }
                _ => {
                    operands.push(at);
                    break;
                }
            }
        }
        operands.reverse();
        ops.reverse();
        let first = self.expr(operands[0]);
        let mut rest = Vec::new();
        for (op, &operand) in ops.iter().zip(&operands[1..]) {
            rest.push(text(format!(" {op}")));
            rest.push(Doc::Line);
            rest.push(self.expr(operand));
        }
        let rest = concat(rest);
        group(concat(vec![
            first,
            if deeper { indent(rest) } else { rest },
        ]))
    }

    /// Passes over the comments inside something printed as it was
    /// written, which carries them already.
    fn skip_comments_in(&mut self, span: Span) {
        while self
            .comments
            .get(self.next)
            .is_some_and(|c| c.at >= span.lo && c.at < span.hi)
        {
            self.next += 1;
        }
    }

    fn path_text(&self, segments: &[Name], type_args: Option<&TypeArgs>) -> String {
        let mut out = String::new();
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                out.push_str("::");
            }
            out.push_str(self.sym(segment.sym));
            // `after` counts segments from zero: `Vec<i64>::new` has its
            // arguments after segment 0.
            if let Some(args) = type_args
                && args.after == i
            {
                let args: Vec<String> = args.args.iter().map(|&t| self.ty_text(t)).collect();
                out.push_str(&format!("<{}>", args.join(", ")));
            }
        }
        out
    }

    // ---- Patterns ------------------------------------------------------

    fn pattern(&mut self, p: &Pattern) -> Doc {
        match &p.kind {
            PatternKind::Wildcard => text("_"),
            PatternKind::Binding(sym) => text(self.sym(*sym)),
            PatternKind::Int { .. }
            | PatternKind::Str(_)
            | PatternKind::Char(_)
            | PatternKind::Byte(_) => text(self.source(p.span)),
            PatternKind::Bool(b) => text(if *b { "true" } else { "false" }),
            // `'a'..='z'`, `..0`, `10..`: its ends as written, with nothing
            // between them and the dots.
            PatternKind::Range { lo, hi, inclusive } => {
                let end = |bound: &Option<wip_syntax::ast::RangeBound>| {
                    bound
                        .as_ref()
                        .map_or(String::new(), |b| self.source(b.span).to_string())
                };
                let dots = if *inclusive { "..=" } else { ".." };
                text(format!("{}{dots}{}", end(lo), end(hi)))
            }
            // On one line where they fit, and otherwise as many to a line
            // as fit, broken before a `|`; one to a line where one cannot
            // be laid out on a line of its own.
            PatternKind::Any(alternatives) => {
                let parts: Vec<Doc> = alternatives.iter().map(|a| self.pattern(a)).collect();
                let words: Option<Vec<String>> = parts.iter().map(flat).collect();
                match words {
                    Some(words) => Doc::Fill(
                        words
                            .into_iter()
                            .enumerate()
                            .map(|(i, word)| if i == 0 { word } else { format!("| {word}") })
                            .collect(),
                    ),
                    None => group(join(parts, concat(vec![Doc::Line, text("| ")]))),
                }
            }
            PatternKind::Variant {
                leading_dot,
                segments,
                binders,
                rest,
            } => {
                if !*leading_dot
                    && segments.len() == 1
                    && let Some(n) = tuple_arity(segments[0].sym)
                    && let Some(binders) = binders
                    && binders.len() == n
                {
                    let parts: Vec<Doc> = binders
                        .iter()
                        .map(|b| match &b.pattern {
                            Some(p) => self.pattern(p),
                            None => text(self.sym(b.field.sym)),
                        })
                        .collect();
                    return self.list("(", parts, ")");
                }
                let dot = if *leading_dot { "." } else { "" };
                let path: Vec<&str> = segments.iter().map(|n| self.sym(n.sym)).collect();
                let head = text(format!("{dot}{}", path.join("::")));
                let Some(binders) = binders else {
                    return head;
                };
                let mut parts: Vec<Doc> = binders
                    .iter()
                    .map(|b| match &b.pattern {
                        // A pattern where the variant's one field is: by
                        // position.
                        Some(p) if b.field.sym == Symbol::positional() => self.pattern(p),
                        // `field: pattern`, or the field's name on its own
                        // where it binds that name.
                        Some(p)
                            if !(matches!(p.kind, PatternKind::Binding(s) if s == b.field.sym)
                                && p.span == b.field.span) =>
                        {
                            concat(vec![
                                text(format!("{}: ", self.sym(b.field.sym))),
                                self.pattern(p),
                            ])
                        }
                        _ => text(self.sym(b.field.sym)),
                    })
                    .collect();
                if *rest {
                    parts.push(text(".."));
                }
                concat(vec![head, self.list("(", parts, ")")])
            }
            // `[first, ..rest]`.
            PatternKind::Slice(elements) => {
                let parts: Vec<Doc> = elements
                    .iter()
                    .map(|element| match element {
                        SliceElement::Pattern(p) => self.pattern(p),
                        SliceElement::Rest { name, .. } => {
                            text(format!("..{}", name.map_or("", |n| self.sym(n.sym))))
                        }
                    })
                    .collect();
                self.list("[", parts, "]")
            }
            PatternKind::Error => text(self.source(p.span)),
        }
    }
}

fn pub_(is_pub: bool) -> &'static str {
    if is_pub { "pub " } else { "" }
}

/// A tuple's arity, where the name is the prelude's `TupleN`.
fn tuple_arity(sym: Symbol) -> Option<usize> {
    (MIN_TUPLE..=MAX_TUPLE).find(|&n| Symbol::tuple(n) == sym)
}

/// A tuple's element, where the name is `_0` and the rest.
fn tuple_field_index(sym: Symbol) -> Option<usize> {
    (0..MAX_TUPLE).find(|&i| Symbol::tuple_field(i) == sym)
}

/// Whether a value breaks well where it starts, so that ` = ` and ` => `
/// keep it on their line rather than moving it to the next.
fn hugs(kind: &ExprKind) -> bool {
    matches!(
        kind,
        ExprKind::Block(_)
            | ExprKind::Match { .. }
            | ExprKind::If { .. }
            | ExprKind::Call { .. }
            | ExprKind::Array(_)
            | ExprKind::Lambda { .. }
            | ExprKind::Assert { .. }
            // `own [ … ]` and `own Point { … }` break inside as what they
            // hold does.
            | ExprKind::Unary {
                op: UnaryOp::Own,
                ..
            }
            // A loop that is a generator breaks inside its body, as a
            // block does.
            | ExprKind::ForElement { .. }
    )
}

/// Whether a call's last argument may hug its parentheses: something
/// that breaks inside its own brackets.
fn hugs_as_argument(kind: &ExprKind) -> bool {
    // A call that names its arguments, or takes `..base`, builds a struct
    // or a variant, and is a literal of several parts as braces were.
    if let ExprKind::Call { names, rest, .. } = kind
        && (rest.is_some() || names.iter().any(Option::is_some))
    {
        return true;
    }
    matches!(
        kind,
        ExprKind::Lambda { .. } | ExprKind::Array(_) | ExprKind::Block(_)
    ) || matches!(
        kind,
        ExprKind::Unary {
            op: UnaryOp::Own,
            ..
        }
    )
}

/// Whether an argument is short enough to stand before a hugging one: a
/// name, a literal, a path or a field.
fn is_short(kind: &ExprKind) -> bool {
    matches!(
        kind,
        ExprKind::Name(_)
            | ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Char(_)
            | ExprKind::Byte(_)
            | ExprKind::Bool(_)
            | ExprKind::Path { .. }
            | ExprKind::SelfRef
            | ExprKind::Field { .. }
            | ExprKind::Unary {
                op: UnaryOp::Ref | UnaryOp::RefVar | UnaryOp::Neg,
                ..
            }
    )
}

/// Whether an expression is itself a block of statements, which a block
/// around it does not put on one line.
fn is_block_like(kind: &ExprKind) -> bool {
    matches!(kind, ExprKind::Match { .. } | ExprKind::Block(_))
}

/// How tightly a binary operator binds, as the parser has it.
fn precedence(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Or => 1,
        BinaryOp::And => 2,
        BinaryOp::Eq | BinaryOp::Ne => 3,
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => 4,
        BinaryOp::BitOr => 5,
        BinaryOp::BitXor => 6,
        BinaryOp::BitAnd => 7,
        BinaryOp::Shl | BinaryOp::Shr => 8,
        BinaryOp::Add | BinaryOp::Sub => 9,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 10,
    }
}

fn item_span(item: &Item) -> Span {
    let (annotations, span): (&[Annotation], Span) = match item {
        Item::Import(i) => (&[], i.span),
        Item::Val(v) => (&v.annotations, v.span),
        Item::Assert(a) => (&a.annotations, a.span),
        Item::Struct(s) => (&s.annotations, s.span),
        Item::Enum(e) => (&e.annotations, e.span),
        Item::Fn(f) => (&f.annotations, f.span),
        Item::Extern(e) => (&e.annotations, e.span),
        Item::Extend(e) => (&e.annotations, e.span),
        Item::Interface(i) => (&i.annotations, i.span),
        Item::Type(t) => (&[], t.span),
    };
    match annotations.first() {
        Some(a) if a.span.lo < span.lo => a.span.to(span),
        _ => span,
    }
}

/// A file of comments alone starts with them, not with a line break.
fn strip_leading_break(doc: Doc) -> Doc {
    match doc {
        Doc::Concat(mut parts) => {
            if matches!(parts.first(), Some(Doc::Hard | Doc::Blank)) {
                parts.remove(0);
            }
            Doc::Concat(parts)
        }
        other => other,
    }
}
