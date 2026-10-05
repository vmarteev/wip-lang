//! What a place in a checked program names: where it was declared, and
//! what to show for it.
//!
//! Every expression keeps the span it was written at, so the smallest one
//! around a place says what is there: a local, a function called, a field,
//! a struct built. The names that leave no expression — a type in a
//! signature, a constant — the checker records in [`Program::names`], and
//! each declaration answers for its own name.

use wip_hir::{
    ConstId, DefaultValue, EnumId, ExprKind, FnId, GlobalId, LocalId, LocalKind, Named, Pattern,
    Program, StructId, Ty, TyKind, TypeDef, Types,
};
use wip_syntax::{Interner, Span};

/// What a place names.
#[derive(Clone, Copy)]
pub(super) enum Target {
    Local(FnId, LocalId),
    Fn(FnId),
    Field(StructId, u32),
    Struct(StructId),
    Enum(EnumId),
    Variant(EnumId, u32),
    Type(Ty),
    Const(ConstId),
    Global(GlobalId),
    /// A value with no name, as a literal: only its type.
    Value(Ty),
}

/// What is at a place: the span that holds it, where it was declared, and
/// what it is, as Wip would write it.
pub struct Hit {
    pub span: Span,
    pub(super) target: Target,
    pub declared: Option<Span>,
    pub shown: String,
}

/// Every place in `program` that names something: the span that holds the
/// name, what it names, and the spans inside it that are not the name — a
/// call's arguments, a field's value — so the name can be found in it.
fn walk(program: &Program, interner: &Interner, visit: &mut impl FnMut(Span, Target, &[Span])) {
    for &(span, named) in &program.names {
        match named {
            Named::Type(ty) => visit(span, Target::Type(ty), &[]),
            Named::Const(id) => visit(span, Target::Const(id), &[]),
            Named::Owner(TypeDef::Struct(id)) => visit(span, Target::Struct(id), &[]),
            Named::Owner(TypeDef::Enum(id)) => visit(span, Target::Enum(id), &[]),
            Named::Owner(TypeDef::Builtin(_)) => {}
            Named::Variant(id, variant) => visit(span, Target::Variant(id, variant), &[]),
        }
    }
    for (id, def) in program.fns.iter() {
        // An instance is its generic function again, with the types filled
        // in; what the compiler wrote has no place in the source, and what
        // `@derive` wrote is placed at the annotation.
        if def.instance_of.is_some() || def.generated.is_some() || program.derived.contains(&id) {
            continue;
        }
        // A lambda's function has no name, nor has a default's.
        if !def.is_lambda && !interner.is_hidden(def.name) {
            visit(def.name_span, Target::Fn(id), &[]);
        }
        let Some(body) = &def.body else { continue };
        for (local, l) in body.locals.iter() {
            if !interner.is_hidden(l.name) {
                visit(l.span, Target::Local(id, local), &[]);
            }
        }
        visit_exprs(program, interner, Some(id), &body.exprs, visit);
    }
    // What a field or a parameter is when it is not given: written in the
    // declaration, and checked on its own.
    let defaults = program
        .structs
        .iter()
        .flat_map(|(_, def)| def.fields.iter().filter_map(|f| f.default.as_deref()))
        .chain(program.enums.iter().flat_map(|(_, def)| {
            def.variants
                .iter()
                .flat_map(|v| v.fields.iter().filter_map(|f| f.default.as_deref()))
        }))
        .chain(
            program
                .fns
                .iter()
                .flat_map(|(_, def)| def.params.iter().filter_map(|p| p.default.as_deref())),
        );
    for default in defaults {
        // One that is code is a function of its own, walked with the rest.
        if let DefaultValue::Constant { exprs, .. } = default {
            visit_exprs(program, interner, None, exprs, visit);
        }
    }
    for (id, def) in program.structs.iter() {
        visit(def.span, Target::Struct(id), &[]);
        for (i, field) in def.fields.iter().enumerate() {
            visit(field.span, Target::Field(id, i as u32), &[]);
        }
    }
    for (id, def) in program.enums.iter() {
        visit(def.span, Target::Enum(id), &[]);
        for (i, variant) in def.variants.iter().enumerate() {
            visit(variant.span, Target::Variant(id, i as u32), &[]);
        }
    }
    for (id, def) in program.consts.iter() {
        visit(def.span, Target::Const(id), &[]);
    }
    for (id, def) in program.globals.iter() {
        visit(def.span, Target::Global(id), &[]);
    }
}

/// The places the expressions of one body, or of one default, name
/// something; `owner` is the function whose locals they are.
fn visit_exprs(
    program: &Program,
    interner: &Interner,
    owner: Option<FnId>,
    exprs: &la_arena::Arena<wip_hir::Expr>,
    visit: &mut impl FnMut(Span, Target, &[Span]),
) {
    let spans =
        |ids: &[wip_hir::ExprId]| -> Vec<Span> { ids.iter().map(|&e| exprs[e].span).collect() };
    for (_, e) in exprs.iter() {
        let (target, inner) = match &e.kind {
            ExprKind::Local(local) => match owner {
                Some(id)
                    if !program.fns[id]
                        .body
                        .as_ref()
                        .is_some_and(|b| interner.is_hidden(b.locals[*local].name)) =>
                {
                    (Target::Local(id, *local), Vec::new())
                }
                _ => continue,
            },
            ExprKind::FnRef { id, .. } => (Target::Fn(*id), Vec::new()),
            ExprKind::Call { callee, args, .. } => (Target::Fn(*callee), spans(args)),
            ExprKind::DynCall {
                interface,
                index,
                args,
                ..
            } => match program.interfaces[*interface].methods.get(*index as usize) {
                Some(method) => (Target::Fn(method.id), spans(args)),
                None => continue,
            },
            ExprKind::Field { base, index } => match program.types.kind(exprs[*base].ty) {
                TyKind::Struct(owner, _) => (Target::Field(owner, *index), spans(&[*base])),
                _ => continue,
            },
            ExprKind::Struct { id, fields, .. } => (Target::Struct(*id), spans(fields)),
            ExprKind::Union { id, field } => (
                Target::Struct(*id),
                field.iter().map(|&(_, value)| exprs[value].span).collect(),
            ),
            ExprKind::Variant {
                id, variant, args, ..
            } => (Target::Variant(*id, *variant), spans(args)),
            // A variant a pattern names: in `x is .Some(a)`, and in a
            // `match`'s arms, outside their guards and bodies.
            ExprKind::Is {
                scrutinee,
                pattern: Pattern::Variant { variant, .. },
            } => match enum_of(program, exprs[*scrutinee].ty) {
                Some(owner) => (Target::Variant(owner, *variant), spans(&[*scrutinee])),
                None => continue,
            },
            ExprKind::Match { scrutinee, arms } => {
                if let Some(owner) = enum_of(program, exprs[*scrutinee].ty) {
                    for arm in arms {
                        if let Pattern::Variant { variant, .. } = arm.pattern {
                            let mut inner = spans(&[arm.body]);
                            inner.extend(arm.guard.map(|g| exprs[g].span));
                            visit(arm.span, Target::Variant(owner, variant), &inner);
                        }
                    }
                }
                continue;
            }
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) => {
                (Target::Value(e.ty), Vec::new())
            }
            _ => continue,
        };
        visit(e.span, target, &inner);
    }
}

/// The enum a scrutinee of this type is, through references.
fn enum_of(program: &Program, ty: Ty) -> Option<EnumId> {
    match program.types.kind(ty) {
        TyKind::Enum(id, _) => Some(id),
        TyKind::Ref(inner, _) | TyKind::Own(inner) => enum_of(program, inner),
        _ => None,
    }
}

/// What is at `pos`, a place in `program`'s sources.
pub fn at(program: &Program, interner: &Interner, pos: u32) -> Option<Hit> {
    let mut best: Option<(Span, Target)> = None;
    walk(program, interner, &mut |span, target, _| {
        let inside = span.lo <= pos && pos <= span.hi;
        // Of two the same size, the later: an expression is made after the
        // ones inside it, and a value the checker filled in has the span
        // of what it was filled into.
        let smaller = best.is_none_or(|(b, _)| span.hi - span.lo <= b.hi - b.lo);
        if inside && smaller {
            best = Some((span, target));
        }
    });
    let (span, target) = best?;
    let shown = Shown { program, interner };
    let (declared, shown) = shown.target(target);
    Some(Hit {
        span,
        target,
        declared,
        shown,
    })
}

/// Every place in `program` that names `written`, declared at
/// `declared`, the declaration among them: the span of the name itself, as
/// a rename would replace it. What the compiler wrote for a declaration —
/// an enum's `fromIndex` — shares its span but not its name, and is not
/// counted. `text_of` gives the text a span is in, and where that text
/// begins.
pub fn references<'a>(
    program: &Program,
    interner: &Interner,
    (written, declared): (&str, Span),
    text_of: impl Fn(Span) -> Option<(&'a str, u32)>,
) -> Vec<Span> {
    let shown = Shown { program, interner };
    let mut found = Vec::new();
    walk(program, interner, &mut |span, target, inner| {
        if shown.declared(target) != Some(declared) {
            return;
        }
        let Some(name) = shown.name_of(target).filter(|name| name == written) else {
            return;
        };
        let Some((text, base)) = text_of(span) else {
            return;
        };
        if let Some(name_span) = name_in(text, base, span, &name, inner) {
            found.push(name_span);
        }
    });
    found.sort_by_key(|s| (s.lo, s.hi));
    found.dedup();
    found
}

/// Where `name` is written in `span` of `text` (which begins at `base`),
/// as a whole word, outside the spans `inner`.
pub(super) fn name_in(
    text: &str,
    base: u32,
    span: Span,
    name: &str,
    inner: &[Span],
) -> Option<Span> {
    let lo = span.lo.checked_sub(base)? as usize;
    let hi = (span.hi.checked_sub(base)? as usize).min(text.len());
    let within = text.get(lo..hi)?;
    let word = |c: char| c.is_alphanumeric() || c == '_';
    within.match_indices(name).find_map(|(i, _)| {
        let before = within[..i].chars().next_back();
        let after = within[i + name.len()..].chars().next();
        if before.is_some_and(word) || after.is_some_and(word) {
            return None;
        }
        let at = span.lo + i as u32;
        let found = Span::new(at, at + name.len() as u32);
        // An inner span that holds the whole span was not written inside
        // it: the checker filled it in, as a field's default.
        let nested = inner
            .iter()
            .filter(|s| !(s.lo <= span.lo && span.hi <= s.hi))
            .any(|s| s.lo <= found.lo && found.hi <= s.hi);
        (!nested).then_some(found)
    })
}

/// The name a target is written with, and the module that declares it,
/// for what can be imported by name: a function, a type, a constant.
pub(super) fn importable(
    program: &Program,
    interner: &Interner,
    target: Target,
) -> Option<(String, u32)> {
    let p = program;
    let module = match target {
        Target::Fn(f) if p.fns[f].owner.is_none() => p.fns[f].module,
        Target::Struct(id) => p.structs[id].module,
        Target::Enum(id) => p.enums[id].module,
        Target::Type(ty) => match p.types.kind(ty) {
            TyKind::Struct(id, _) => p.structs[id].module,
            TyKind::Enum(id, _) => p.enums[id].module,
            _ => return None,
        },
        Target::Const(id) => p.consts[id].module,
        Target::Global(id) => p.globals[id].module,
        _ => return None,
    };
    let name = Shown { program, interner }.name_of(target)?;
    Some((name, module))
}

/// The name a target is written with.
pub(super) fn name_of(program: &Program, interner: &Interner, target: Target) -> Option<String> {
    Shown { program, interner }.name_of(target)
}

/// What a target is, as Wip would write it.
pub(super) fn describe(program: &Program, interner: &Interner, target: Target) -> String {
    Shown { program, interner }.target(target).1
}

struct Shown<'a> {
    program: &'a Program,
    interner: &'a Interner,
}

impl Shown<'_> {
    fn ty(&self, ty: Ty) -> String {
        self.program.ty_name(ty, self.interner)
    }

    fn name(&self, sym: wip_syntax::Symbol) -> &str {
        self.interner.resolve(sym)
    }

    /// Where a target was declared.
    fn declared(&self, target: Target) -> Option<Span> {
        let p = self.program;
        Some(match target {
            Target::Local(f, local) => p.fns[f].body.as_ref()?.locals[local].span,
            Target::Fn(f) => p.fns[f].name_span,
            Target::Field(owner, i) => p.structs[owner].fields.get(i as usize)?.span,
            Target::Struct(id) => p.structs[id].span,
            Target::Enum(id) => p.enums[id].span,
            Target::Variant(id, i) => p.enums[id].variants.get(i as usize)?.span,
            Target::Type(ty) => match p.types.kind(ty) {
                TyKind::Struct(id, _) => p.structs[id].span,
                TyKind::Enum(id, _) => p.enums[id].span,
                _ => return None,
            },
            Target::Const(id) => p.consts[id].span,
            Target::Global(id) => p.globals[id].span,
            Target::Value(_) => return None,
        })
    }

    /// The name a target is written with.
    fn name_of(&self, target: Target) -> Option<String> {
        let p = self.program;
        let sym = match target {
            Target::Local(f, local) => p.fns[f].body.as_ref()?.locals[local].name,
            Target::Fn(f) => p.fns[f].name,
            Target::Field(owner, i) => p.structs[owner].fields.get(i as usize)?.name,
            Target::Struct(id) => p.structs[id].name,
            Target::Enum(id) => p.enums[id].name,
            Target::Variant(id, i) => p.enums[id].variants.get(i as usize)?.name,
            Target::Type(ty) => match p.types.kind(ty) {
                TyKind::Struct(id, _) => p.structs[id].name,
                TyKind::Enum(id, _) => p.enums[id].name,
                _ => return None,
            },
            Target::Const(id) => p.consts[id].name,
            Target::Global(id) => p.globals[id].name,
            Target::Value(_) => return None,
        };
        Some(self.name(sym).to_string())
    }

    /// Where the target was declared, and what it is.
    fn target(&self, target: Target) -> (Option<Span>, String) {
        let p = self.program;
        match target {
            Target::Local(f, local) => {
                let Some(body) = &p.fns[f].body else {
                    return (None, String::new());
                };
                let l = &body.locals[local];
                let keyword = match l.kind {
                    LocalKind::Param | LocalKind::Binding => "",
                    LocalKind::Let { .. } => "val ",
                    LocalKind::Var => "var ",
                };
                (
                    Some(l.span),
                    format!("{keyword}{}: {}", self.name(l.name), self.ty(l.ty)),
                )
            }
            Target::Fn(f) => {
                let def = &p.fns[f];
                let params: Vec<String> = def
                    .params
                    .iter()
                    .map(|param| format!("{}: {}", self.name(param.name), self.ty(param.ty)))
                    .collect();
                let owner = match def.owner {
                    Some(owner) if !matches!(owner, wip_hir::TypeDef::Builtin(_)) => {
                        format!("{}.", p.owner_name(owner, self.interner))
                    }
                    _ => String::new(),
                };
                let mut shown = format!("fn {owner}{}({})", self.name(def.name), params.join(", "));
                if def.ret != Types::UNIT {
                    shown.push_str(&format!(": {}", self.ty(def.ret)));
                }
                (Some(def.name_span), shown)
            }
            Target::Field(owner, index) => {
                let def = &p.structs[owner];
                let Some(field) = def.fields.get(index as usize) else {
                    return (None, String::new());
                };
                let shown = format!(
                    "{}.{}: {}",
                    self.name(def.name),
                    self.name(field.name),
                    self.ty(field.ty)
                );
                (Some(field.span), shown)
            }
            Target::Struct(id) => {
                let def = &p.structs[id];
                let keyword = if def.is_union { "union" } else { "struct" };
                (Some(def.span), format!("{keyword} {}", self.name(def.name)))
            }
            Target::Enum(id) => {
                let def = &p.enums[id];
                (Some(def.span), format!("enum {}", self.name(def.name)))
            }
            Target::Variant(id, index) => {
                let def = &p.enums[id];
                let Some(variant) = def.variants.get(index as usize) else {
                    return (None, String::new());
                };
                let mut shown = format!("{}.{}", self.name(def.name), self.name(variant.name));
                if !variant.fields.is_empty() {
                    let fields: Vec<String> = variant
                        .fields
                        .iter()
                        .map(|f| format!("{}: {}", self.name(f.name), self.ty(f.ty)))
                        .collect();
                    shown.push_str(&format!("({})", fields.join(", ")));
                }
                (Some(variant.span), shown)
            }
            Target::Type(ty) => match p.types.kind(ty) {
                TyKind::Struct(id, _) => {
                    let keyword = if p.structs[id].is_union {
                        "union"
                    } else {
                        "struct"
                    };
                    (
                        Some(p.structs[id].span),
                        format!("{keyword} {}", self.ty(ty)),
                    )
                }
                TyKind::Enum(id, _) => (Some(p.enums[id].span), format!("enum {}", self.ty(ty))),
                _ => (None, self.ty(ty)),
            },
            Target::Const(id) => {
                let def = &p.consts[id];
                (
                    Some(def.span),
                    format!("val {}: {}", self.name(def.name), self.ty(def.ty)),
                )
            }
            Target::Global(id) => {
                let def = &p.globals[id];
                let keyword = if def.is_mut { "var" } else { "val" };
                (
                    Some(def.span),
                    format!("{keyword} {}: {}", self.name(def.name), self.ty(def.ty)),
                )
            }
            Target::Value(ty) => (None, self.ty(ty)),
        }
    }
}

/// The `///` lines just above the line `at` is on, past any annotations:
/// what the declaration there says of itself.
pub fn documentation(text: &str, at: usize) -> String {
    let at = at.min(text.len());
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let mut lines = Vec::new();
    for line in text[..start].lines().rev() {
        let line = line.trim();
        if let Some(doc) = line.strip_prefix("///") {
            lines.push(doc.strip_prefix(' ').unwrap_or(doc));
        } else if !line.starts_with('@') {
            break;
        }
    }
    lines.reverse();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::documentation;

    #[test]
    fn documentation_is_the_comment_above_past_annotations() {
        let text = "// not this\n/// The area.\n/// In units.\n@inline\nfn area(): i64 = 1\n";
        let at = text.find("fn").expect("fn");
        assert_eq!(documentation(text, at), "The area.\nIn units.");
        assert_eq!(documentation(text, 3), "");
    }
}
