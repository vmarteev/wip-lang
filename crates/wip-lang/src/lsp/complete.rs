//! Completion: what could be written where the cursor is.
//!
//! Where completion is asked for, the text is rarely a program: `ball.`
//! does not parse. So the server writes a placeholder name at the cursor
//! — `ball.__wipComplete` — and checks that text instead. What comes
//! before the placeholder is then checked as it always is, and the
//! placeholder is reported and ignored. What the text before the cursor
//! ends with says what is asked:
//!
//! - after `.`, the fields and methods of the value before it;
//! - after `a::`, the items of the module `a`, or the static functions and
//!   variants of the type `a`;
//! - anywhere else, the names in scope: the function's locals declared
//!   above the cursor, the module's items, what it imports, the prelude's
//!   items, and the keywords.

use std::path::Path;

use serde_json::{Value, json};
use wip_hir::BuiltinOwner;
use wip_hir::{FnId, Program, Receiver, Ty, TyKind, TypeDef};
use wip_syntax::ast::Item;
use wip_syntax::{Interner, Span};

use super::navigate::{Target, describe};
use crate::{Loaded, SourceFile, canonical_file};

/// The name written at the cursor, so that the text there parses.
pub const PLACEHOLDER: &str = "__wipComplete";

/// What the text before the cursor asks for.
#[derive(Debug, PartialEq)]
pub enum Asked {
    /// Members of the value that ends at this byte offset: the `.`'s.
    Member { dot: usize },
    /// Items of what this path names.
    Path(Vec<String>),
    /// The names in scope.
    Scope,
}

/// Whether a word is one of Wip's keywords, which no name may be.
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// What is asked at `cursor` in `text`.
pub fn asked(text: &str, cursor: usize) -> Asked {
    let before = &text[..cursor];
    let start = before
        .trim_end_matches(|c: char| c.is_alphanumeric() || c == '_')
        .len();
    let head = &before[..start];
    if let Some(path) = head.strip_suffix("::") {
        let written = path
            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .next()
            .unwrap_or_default();
        let segments: Vec<String> = written
            .split("::")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if !segments.is_empty() {
            return Asked::Path(segments);
        }
    }
    // `a.` asks for a member, but `0..` is a range.
    if head.ends_with('.') && !head.ends_with("..") {
        return Asked::Member { dot: start - 1 };
    }
    Asked::Scope
}

// The protocol's numbers for kinds of completion.
const METHOD: u32 = 2;
const FUNCTION: u32 = 3;
const FIELD: u32 = 5;
const VARIABLE: u32 = 6;
const INTERFACE: u32 = 8;
const MODULE: u32 = 9;
const ENUM: u32 = 13;
const KEYWORD: u32 = 14;
const ENUM_MEMBER: u32 = 20;
const CONSTANT: u32 = 21;
const STRUCT: u32 = 22;

const KEYWORDS: &[&str] = &[
    "fn",
    "val",
    "var",
    "if",
    "else",
    "then",
    "match",
    "while",
    "for",
    "in",
    "return",
    "break",
    "continue",
    "struct",
    "enum",
    "interface",
    "extend",
    "import",
    "extern",
    "type",
    "pub",
    "is",
    "as",
    "own",
    "move",
    "lend",
    "from",
    "defer",
    "assert",
    "true",
    "false",
    "null",
    "static",
    "dyn",
];

struct Items<'a> {
    program: &'a Program,
    interner: &'a Interner,
    /// The module the cursor is in, which sees what it does not export.
    module: u32,
    items: Vec<Value>,
    seen: std::collections::HashSet<String>,
}

impl Items<'_> {
    fn push(&mut self, label: &str, kind: u32, target: Option<Target>) {
        if label.is_empty() || label == PLACEHOLDER || label.starts_with('<') {
            return;
        }
        if !self.seen.insert(label.to_string()) {
            return;
        }
        let mut item = json!({ "label": label, "kind": kind });
        if let Some(target) = target {
            item["detail"] = json!(describe(self.program, self.interner, target));
        }
        self.items.push(item);
    }

    fn name(&self, sym: wip_syntax::Symbol) -> String {
        self.interner.resolve(sym).to_string()
    }

    fn sees(&self, module: u32, is_pub: bool) -> bool {
        is_pub || module == self.module
    }

    /// What can follow `value.`: its fields, and its methods.
    fn members(&mut self, ty: Ty) {
        let p = self.program;
        let mut ty = ty;
        while let TyKind::Ref(inner, _) | TyKind::Own(inner) = p.types.kind(ty) {
            ty = inner;
        }
        let kind = p.types.kind(ty);
        if let TyKind::Struct(id, _) = kind {
            let def = &p.structs[id];
            for (i, field) in def.fields.iter().enumerate() {
                if self.sees(def.module, field.is_pub) {
                    let name = self.name(field.name);
                    self.push(&name, FIELD, Some(Target::Field(id, i as u32)));
                }
            }
        }
        let methods: Vec<FnId> = match kind {
            TyKind::Struct(id, _) => p.structs[id].methods.clone(),
            TyKind::Enum(id, _) => p.enums[id].methods.clone(),
            TyKind::Dyn(interface, _) => p.interfaces[interface]
                .methods
                .iter()
                .map(|m| m.id)
                .collect(),
            TyKind::Array(..) => builtin(p, BuiltinOwner::Slice),
            kind => BuiltinOwner::of(kind).map_or_else(Vec::new, |owner| builtin(p, owner)),
        };
        let methods = methods.into_iter().chain(extensions(p, ty));
        for id in methods {
            let def = &p.fns[id];
            if matches!(def.receiver, None | Some(Receiver::Static)) {
                continue;
            }
            if self.sees(def.module, def.is_pub) {
                let name = self.name(def.name);
                self.push(&name, METHOD, Some(Target::Fn(id)));
            }
        }
    }

    /// What can follow `Type::`: its static functions, and an enum's
    /// variants.
    fn statics(&mut self, owner: TypeDef) {
        let p = self.program;
        let methods = match owner {
            TypeDef::Struct(id) => p.structs[id].methods.clone(),
            TypeDef::Enum(id) => {
                for (i, variant) in p.enums[id].variants.iter().enumerate() {
                    let name = self.name(variant.name);
                    self.push(&name, ENUM_MEMBER, Some(Target::Variant(id, i as u32)));
                }
                p.enums[id].methods.clone()
            }
            TypeDef::Builtin(owner) => builtin(p, owner),
        };
        for id in methods {
            let def = &p.fns[id];
            if def.receiver == Some(Receiver::Static) && self.sees(def.module, def.is_pub) {
                let name = self.name(def.name);
                self.push(&name, FUNCTION, Some(Target::Fn(id)));
            }
        }
    }

    /// The items a module declares: all of them from inside it, and what
    /// it exports from outside.
    fn module_items(&mut self, module: u32) {
        let p = self.program;
        for (id, def) in p.fns.iter() {
            let free = def.owner.is_none() && !def.is_lambda && def.generated.is_none();
            if free
                && def.instance_of.is_none()
                && def.module == module
                && self.sees(module, def.is_pub)
            {
                let name = self.name(def.name);
                self.push(&name, FUNCTION, Some(Target::Fn(id)));
            }
        }
        for (id, def) in p.structs.iter() {
            // What the compiler made — a closure's environment, a
            // generator — has no name to write.
            if def.module == module
                && self.sees(module, def.is_pub)
                && !def.is_env
                && !def.is_tuple
                && def.generator.is_none()
            {
                let name = self.name(def.name);
                self.push(&name, STRUCT, Some(Target::Struct(id)));
            }
        }
        for (id, def) in p.enums.iter() {
            if def.module == module && self.sees(module, def.is_pub) {
                let name = self.name(def.name);
                self.push(&name, ENUM, Some(Target::Enum(id)));
            }
        }
        for (_, def) in p.interfaces.iter() {
            if def.module == module && self.sees(module, def.is_pub) {
                let name = self.name(def.name);
                self.push(&name, INTERFACE, None);
            }
        }
        for (id, def) in p.consts.iter() {
            if def.module == module && self.sees(module, def.is_pub) {
                let name = self.name(def.name);
                self.push(&name, CONSTANT, Some(Target::Const(id)));
            }
        }
        for (id, def) in p.globals.iter() {
            if def.module == module && self.sees(module, def.is_pub) {
                let name = self.name(def.name);
                self.push(&name, VARIABLE, Some(Target::Global(id)));
            }
        }
    }

    /// The struct or enum a name means, preferring the module's own.
    fn type_named(&self, name: &str) -> Option<TypeDef> {
        let p = self.program;
        let structs = p
            .structs
            .iter()
            .filter(|(_, d)| self.interner.resolve(d.name) == name && self.sees(d.module, d.is_pub))
            .map(|(id, d)| (d.module, TypeDef::Struct(id)));
        let enums = p
            .enums
            .iter()
            .filter(|(_, d)| self.interner.resolve(d.name) == name && self.sees(d.module, d.is_pub))
            .map(|(id, d)| (d.module, TypeDef::Enum(id)));
        let found: Vec<(u32, TypeDef)> = structs.chain(enums).collect();
        found
            .iter()
            .find(|(module, _)| *module == self.module)
            .or(found.first())
            .map(|&(_, def)| def)
    }
}

/// What the interfaces `ty` implements give it through their extensions,
/// where the implementation's types meet the condition:
/// `text.chars().` lists `max`, and not `sum`.
fn extensions(program: &Program, ty: Ty) -> Vec<FnId> {
    let kind = program.types.kind(ty);
    let owner = match kind {
        TyKind::Struct(id, _) => TypeDef::Struct(id),
        TyKind::Enum(id, _) => TypeDef::Enum(id),
        kind => match BuiltinOwner::of(kind) {
            Some(builtin) => TypeDef::Builtin(builtin),
            None => return Vec::new(),
        },
    };
    let own: Vec<Ty> = match kind {
        TyKind::Struct(_, list) | TyKind::Enum(_, list) => program.types.list(list).to_vec(),
        _ => Vec::new(),
    };
    let mut found = Vec::new();
    for implementation in program.impls.iter().filter(|i| i.ty == owner) {
        let args: Option<Vec<Ty>> = program
            .types
            .list(implementation.args)
            .iter()
            .map(|&arg| program.types.try_subst_find(arg, &own))
            .collect();
        let Some(args) = args else { continue };
        for &method in &program.interfaces[implementation.interface].extensions {
            let generics = &program.fns[method].generics;
            let met = args.iter().enumerate().all(|(at, &arg)| {
                generics.get(at + 1).is_none_or(|param| {
                    param
                        .interfaces
                        .iter()
                        .all(|c| program.implements(arg, c.interface))
                })
            });
            if met {
                found.push(method);
            }
        }
    }
    found
}

fn builtin(program: &Program, owner: BuiltinOwner) -> Vec<FnId> {
    program
        .builtins
        .get(&owner)
        .map_or_else(Vec::new, |b| b.methods.clone())
}

/// What could be written at `cursor` in the open file `path`, whose text
/// the program was checked with the placeholder written at the cursor.
/// `base` is where the file's spans begin in the program.
pub fn items(
    loaded: &Loaded,
    program: &Program,
    (path, text): (&Path, &str),
    base: u32,
    cursor: usize,
) -> Value {
    let pos = base + cursor as u32;
    // The function the cursor is in, if it is in one.
    let around = program.fns.iter().find(|(_, def)| {
        def.instance_of.is_none()
            && !def.is_lambda
            && def.body.as_ref().is_some_and(|body| {
                body.value
                    .is_some_and(|v| body.exprs[v].span.lo <= pos && pos <= body.exprs[v].span.hi)
            })
    });
    let module = around
        .map(|(_, def)| def.module)
        .or_else(|| module_of(loaded, path))
        .unwrap_or(0);
    let mut items = Items {
        program,
        interner: &loaded.interner,
        module,
        items: Vec::new(),
        seen: Default::default(),
    };
    match asked(text, cursor) {
        Asked::Member { dot } => {
            let dot = base + dot as u32;
            // The largest expression that ends at the `.`, in any body:
            // `a.b` of `a.b.`, not `b`.
            let mut best: Option<(Span, Ty)> = None;
            for (_, def) in program.fns.iter() {
                let Some(body) = &def.body else { continue };
                for (_, e) in body.exprs.iter() {
                    if e.span.hi == dot && best.is_none_or(|(b, _)| e.span.lo < b.lo) {
                        best = Some((e.span, e.ty));
                    }
                }
            }
            if let Some((_, ty)) = best {
                items.members(ty);
            }
        }
        Asked::Path(segments) => {
            let written = segments.join("::");
            let prefix = &program.modules[module as usize];
            let own = prefix.split("::").next().filter(|_| prefix.contains("::"));
            let imported = imports(text, path);
            // An alias stands for the path it was imported as.
            let written = match imported.iter().find(|(name, _)| *name == segments[0]) {
                Some((_, full)) => {
                    let mut full = full.clone();
                    for segment in &segments[1..] {
                        full.push_str("::");
                        full.push_str(segment);
                    }
                    full
                }
                None => written,
            };
            let candidates = [
                written.clone(),
                own.map(|own| format!("{own}::{written}"))
                    .unwrap_or_default(),
            ];
            let found = program
                .modules
                .iter()
                .position(|m| !m.is_empty() && candidates.contains(m));
            match found {
                Some(target) => items.module_items(target as u32),
                None => {
                    let last = segments.last().map(String::as_str).unwrap_or_default();
                    let owner = items.type_named(last).or_else(|| {
                        builtin_named(last)
                            .filter(|b| program.builtins.contains_key(b))
                            .map(TypeDef::Builtin)
                    });
                    if let Some(owner) = owner {
                        items.statics(owner);
                    }
                }
            }
        }
        Asked::Scope => {
            if let Some((id, def)) = around
                && let Some(body) = &def.body
            {
                // The locals declared above the cursor, the nearest last,
                // so it is the one a repeated name keeps.
                let mut locals: Vec<_> = body
                    .locals
                    .iter()
                    .filter(|(_, l)| l.span.hi <= pos)
                    .collect();
                locals.sort_by_key(|(_, l)| std::cmp::Reverse(l.span.lo));
                for (local, l) in locals {
                    let name = items.name(l.name);
                    items.push(&name, VARIABLE, Some(Target::Local(id, local)));
                }
            }
            items.module_items(module);
            for (name, _) in imports(text, path) {
                items.push(&name, MODULE, None);
            }
            if let Some(prelude) = program.modules.iter().position(|m| m == wip_hir::PRELUDE) {
                items.module_items(prelude as u32);
            }
            for keyword in KEYWORDS {
                items.push(keyword, KEYWORD, None);
            }
        }
    }
    json!({ "isIncomplete": false, "items": items.items })
}

/// The built-in type a name means, for `i64::` and the like.
fn builtin_named(name: &str) -> Option<BuiltinOwner> {
    use wip_hir::{FloatTy, IntTy};
    Some(match name {
        "i8" => BuiltinOwner::Int(IntTy::I8),
        "i16" => BuiltinOwner::Int(IntTy::I16),
        "i32" => BuiltinOwner::Int(IntTy::I32),
        "i64" => BuiltinOwner::Int(IntTy::I64),
        "u8" => BuiltinOwner::Int(IntTy::U8),
        "u16" => BuiltinOwner::Int(IntTy::U16),
        "u32" => BuiltinOwner::Int(IntTy::U32),
        "u64" => BuiltinOwner::Int(IntTy::U64),
        "f32" => BuiltinOwner::Float(FloatTy::F32),
        "f64" => BuiltinOwner::Float(FloatTy::F64),
        "bool" => BuiltinOwner::Bool,
        "char" => BuiltinOwner::Char,
        "str" => BuiltinOwner::Str,
        _ => return None,
    })
}

/// The module whose directory holds `path`.
fn module_of(loaded: &Loaded, path: &Path) -> Option<u32> {
    let file = canonical_file(path);
    let dir = file.parent()?;
    loaded
        .modules
        .iter()
        .position(|m| {
            !m.dir.as_os_str().is_empty() && canonical_file(&m.dir.join("x")).parent() == Some(dir)
        })
        .map(|i| i as u32)
}

/// The names the file's imports bind to modules, and the path each names.
fn imports(text: &str, path: &Path) -> Vec<(String, String)> {
    let parsed = crate::parse_file(&SourceFile {
        name: path.display().to_string(),
        text: text.to_string(),
    });
    let resolve = |sym| parsed.interner.resolve(sym).to_string();
    let mut bound = Vec::new();
    for item in &parsed.ast.items {
        let Item::Import(decl) = item else { continue };
        let full: Vec<String> = decl.path.iter().map(|n| resolve(n.sym)).collect();
        let full = full.join("::");
        match &decl.items {
            // Only the names in braces are bound, and `self` binds the module.
            Some(names) => {
                for name in names.iter().filter(|n| n.name.is_none()) {
                    let bound_as = name.alias.map(|a| resolve(a.sym)).unwrap_or_else(|| {
                        full.rsplit("::").next().unwrap_or_default().to_string()
                    });
                    bound.push((bound_as, full.clone()));
                }
            }
            None => {
                let bound_as = match decl.alias {
                    Some(alias) => resolve(alias.sym),
                    None => full.rsplit("::").next().unwrap_or_default().to_string(),
                };
                bound.push((bound_as, full));
            }
        }
    }
    bound
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_asked_is_read_from_before_the_cursor() {
        let at_end = |text: &str| asked(text, text.len());
        assert_eq!(at_end("\tval a = ball."), Asked::Member { dot: 13 });
        assert_eq!(at_end("\tval a = ball.pa"), Asked::Member { dot: 13 });
        assert_eq!(
            at_end("\tgame::board::Bo"),
            Asked::Path(vec!["game".to_string(), "board".to_string()])
        );
        assert_eq!(at_end("\tfor i in 0.."), Asked::Scope);
        assert_eq!(at_end("\tval a = to"), Asked::Scope);
    }
}
