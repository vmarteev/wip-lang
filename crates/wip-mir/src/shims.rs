//! The C the compiler writes for itself.
//!
//! Most calls into C are made directly, a struct by value included.
//! Where one cannot be — on a target whose rules are not
//! written, or for a C function taken as a Wip function value, which is
//! called the way Wip calls — the compiler writes a small C function whose
//! signature it *can* express, and `cc` decides how the fields travel:
//!
//! ```c
//! struct Rect { int32_t x; int32_t y; };
//! extern struct Rect grow(struct Rect r, int32_t by);
//!
//! void wip_shim_7(struct Rect *out, const struct Rect *r, int32_t by) {
//!     *out = grow(*r, by);
//! }
//! ```
//!
//! Wip already passes an aggregate by its address, and an aggregate result
//! by an address the caller gives — so that shim is the signature code
//! generation would emit anyway. Only the symbol changes.
//!
//! A call that passes more than the declaration names is written the same
//! way, and for the same reason: Cranelift cannot make it, and a front end
//! that tried would have to apply C's own promotions — `float` to `double`,
//! small integers to `int` — which is where Odin and Zig both shipped bugs:
//!
//! ```c
//! extern int32_t printf(const char *a0, ...);
//!
//! int32_t wip_shim_131(const char *a0, int32_t a1, double a2) {
//!     return printf(a0, a1, a2);
//! }
//! ```
//!
//! So is a call to what a header defines rather than declares — a macro
//! or a `static inline`, which has no symbol — and a read or write of a
//! variable C owns.
use std::fmt::Write;

use wip_syntax::Interner;

use wip_hir::*;

/// What a function that needs one is called in the generated C.
pub fn shim_name(id: FnId) -> String {
    format!("wip_shim_{}", u32::from(id.into_raw()))
}

/// The C for every call that needs one, or `None` when none do. Which do
/// is `wip_mir::c_abi::shimmed`'s to say, since it depends on how the
/// target passes a struct.
pub fn write_shims(
    program: &Program,
    interner: &Interner,
    needed: impl Fn(FnId) -> bool,
) -> Option<String> {
    let needed: Vec<(FnId, &FnDef)> = program.fns.iter().filter(|&(id, _)| needed(id)).collect();
    if needed.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(
        "/* Written by the Wip compiler:\n   \
         the calls where C decides how the arguments travel, since Wip\n   \
         does not. */\n\
         #include <stdint.h>\n\n",
    );
    // The headers the declarations name, before anything that uses them.
    // A header may define what it declares — a macro, a `static inline` —
    // so what it says is what the wrappers below call.
    let mut headers: Vec<&str> = Vec::new();
    let named = program
        .structs
        .iter()
        .filter_map(|(_, def)| def.header)
        .chain(needed.iter().filter_map(|(_, def)| def.header));
    for header in named {
        let header = interner.resolve(header);
        if !headers.contains(&header) {
            headers.push(header);
        }
    }
    for header in &headers {
        // `@header("<math.h>")` is the system's, `@header("shape.h")` the
        // module's own, which its directory is searched for.
        if header.starts_with('<') {
            let _ = writeln!(out, "#include {header}");
        } else {
            let _ = writeln!(out, "#include \"{header}\"");
        }
    }
    if !headers.is_empty() {
        out.push('\n');
    }
    // Every C layout the program declares, in the order they were written,
    // since one may hold another.
    for (_, def) in program.structs.iter() {
        // A struct the header declares is C's, written once, by C. Wip
        // writes only the layouts it was told and no header claims.
        if !def.is_extern || def.header.is_some() {
            continue;
        }
        let name = interner.resolve(def.name);
        let keyword = if def.is_union { "union" } else { "struct" };
        let _ = writeln!(out, "{keyword} {name} {{");
        for field in &def.fields {
            let field_name = interner.resolve(field.name);
            let _ = writeln!(
                out,
                "    {};",
                declaration(program, interner, field.ty, field_name)
            );
        }
        out.push_str("};\n\n");
    }
    // A variable C owns is declared once, however many accessors read it,
    // and not at all when a header has declared it already.
    for (id, def) in program.globals.iter() {
        let used = needed
            .iter()
            .any(|(_, f)| f.accesses == Some(Access::Global(id)));
        if !used || def.header.is_some() {
            continue;
        }
        let ty = c_type(program, interner, def.ty);
        let name = interner.resolve(def.symbol.unwrap_or(def.name));
        let _ = writeln!(out, "extern {};\n", declarator(&ty, name));
    }
    for (id, def) in needed {
        write_shim(&mut out, program, interner, id, def);
    }
    Some(out)
}

fn write_shim(out: &mut String, program: &Program, interner: &Interner, id: FnId, def: &FnDef) {
    // What C calls it, which `@symbol` may have said.
    let name = interner.resolve(def.symbol.unwrap_or(def.name));
    // Reading and writing what C owns: a variable, or a field of a struct C
    // lays out. Both are a pair of accessors whose bodies say the same thing.
    if let Some(access) = def.accesses {
        let shim = shim_name(id);
        // The field of a struct is reached through the pointer the
        // accessor is given; a variable is simply named.
        let (reached, first) = match access {
            Access::Global(_) => (name.to_string(), 0),
            Access::Field(owner, index) => {
                let field = interner.resolve(program.structs[owner].fields[index as usize].name);
                (format!("a0->{field}"), 1)
            }
        };
        let taken: Vec<String> = def
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| declaration(program, interner, p.ty, &format!("a{i}")))
            .collect();
        let taken = if taken.is_empty() {
            "void".to_string()
        } else {
            taken.join(", ")
        };
        match def.params.len() > first {
            // The last argument is the value to write.
            true => {
                let _ = writeln!(out, "void {shim}({taken}) {{");
                let _ = writeln!(out, "    {reached} = a{first};");
            }
            false => {
                let ty = c_type(program, interner, def.ret);
                let _ = writeln!(out, "{ty} {shim}({taken}) {{");
                let _ = writeln!(out, "    return {reached};");
            }
        }
        out.push_str("}\n\n");
        return;
    }
    let ret = c_type(program, interner, def.ret);
    let params: Vec<(String, String)> = def
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| (c_type(program, interner, p.ty), format!("a{i}")))
        .collect();

    // What C says the function is.
    let declared: Vec<String> = params
        .iter()
        .map(|(ty, name)| declarator(ty, name))
        .collect();
    // A variadic function is declared as it was written — the parameters
    // it names, and then `...`. The shim below it is the one call, of
    // fixed arity, and C applies the promotions.
    let arguments = match def.variadic_of {
        Some(original) => {
            let declared = &program.fns[original].params;
            let mut written: Vec<String> = declared
                .iter()
                .enumerate()
                .map(|(i, p)| declarator(&c_type(program, interner, p.ty), &format!("a{i}")))
                .collect();
            written.push("...".to_string());
            written.join(", ")
        }
        None if declared.is_empty() => "void".to_string(),
        None => declared.join(", "),
    };
    // A header has said what it is already, and it may have defined it —
    // a second declaration of a macro is not C.
    if def.header.is_none() {
        let _ = writeln!(out, "extern {ret} {name}({arguments});\n");
    }

    // What Wip calls: a struct is an address, here and back.
    let aggregate_result = matches!(program.types.kind(def.ret), TyKind::Struct(..));
    let mut taken: Vec<String> = Vec::new();
    if aggregate_result {
        taken.push(format!("{ret} *out"));
    }
    let mut passed: Vec<String> = Vec::new();
    for (i, (ty, param)) in params.iter().enumerate() {
        let by_address = matches!(program.types.kind(def.params[i].ty), TyKind::Struct(..));
        if by_address {
            taken.push(format!("{ty} *{param}"));
            passed.push(format!("*{param}"));
        } else {
            taken.push(declarator(ty, param));
            passed.push(param.clone());
        }
    }
    let shim = shim_name(id);
    let taken = if taken.is_empty() {
        "void".to_string()
    } else {
        taken.join(", ")
    };
    let passed = passed.join(", ");
    let call = format!("{name}({passed})");
    // A struct comes back through the address the caller gave; anything
    // else comes back the way it went in C, and the shim must hand it on.
    if aggregate_result {
        let _ = writeln!(out, "void {shim}({taken}) {{");
        let _ = writeln!(out, "    *out = {call};");
    } else if def.ret == Types::UNIT {
        let _ = writeln!(out, "void {shim}({taken}) {{");
        let _ = writeln!(out, "    {call};");
    } else {
        let _ = writeln!(out, "{ret} {shim}({taken}) {{");
        let _ = writeln!(out, "    return {call};");
    }
    out.push_str("}\n\n");
}

/// A declaration of `name` with type `ty`, where C puts part of the type
/// after the name: an array's length.
fn declaration(program: &Program, interner: &Interner, ty: Ty, name: &str) -> String {
    match program.types.kind(ty) {
        TyKind::Array(elem, len) => {
            let inner = declaration(program, interner, elem, name);
            format!("{inner}[{len}]")
        }
        _ => declarator(&c_type(program, interner, ty), name),
    }
}

/// A declaration of `name` with type `ty`, which for a pointer puts the
/// star where C wants it.
fn declarator(ty: &str, name: &str) -> String {
    if ty.ends_with('*') {
        format!("{ty}{name}")
    } else {
        format!("{ty} {name}")
    }
}

/// The C that means the same as a Wip type. Fixed widths, since a C
/// declaration written here must agree with the one in the library's
/// header on every target Wip supports.
fn c_type(program: &Program, interner: &Interner, ty: Ty) -> String {
    match program.types.kind(ty) {
        TyKind::Int(t) => {
            let signed = t.signed();
            match t.bits() {
                8 | 16 | 32 | 64 if signed => format!("int{}_t", t.bits()),
                8 | 16 | 32 | 64 => format!("uint{}_t", t.bits()),
                _ if signed => "intptr_t".to_string(),
                _ => "uintptr_t".to_string(),
            }
        }
        TyKind::Float(FloatTy::F32) => "float".to_string(),
        TyKind::Float(_) => "double".to_string(),
        TyKind::Bool => "_Bool".to_string(),
        TyKind::Char => "uint32_t".to_string(),
        TyKind::Cstring => "const char *".to_string(),
        // A pointer to a layout C knows is that layout's pointer, so C
        // checks the call; anything else is `void *`.
        TyKind::Ptr(inner) if matches!(program.types.kind(inner), TyKind::Struct(..)) => {
            format!("{} *", c_type(program, interner, inner))
        }
        TyKind::Ptr(_) => "void *".to_string(),
        TyKind::Struct(id, _) => {
            let keyword = if program.structs[id].is_union {
                "union"
            } else {
                "struct"
            };
            format!("{keyword} {}", interner.resolve(program.structs[id].name))
        }
        // An array lent is a pointer to its first element, as C's
        // `T x[N]` parameter is.
        TyKind::Ref(inner, _) => match program.types.kind(inner) {
            TyKind::Array(elem, _) => format!("{} *", c_type(program, interner, elem)),
            _ => format!("{} *", c_type(program, interner, inner)),
        },
        _ => "void".to_string(),
    }
}

/// The C declarations for what a library exports: one prototype per
/// `@export("C")` function, and the structs those mention, which C
/// declared in the first place.
pub fn write_header(program: &Program, interner: &Interner, guard: &str) -> String {
    let exported: Vec<&FnDef> = program
        .fns
        .iter()
        .map(|(_, def)| def)
        // The runtime's entry points are exported from the prelude for the
        // code generator and for C beside the program, and are no
        // library's to declare.
        .filter(|def| def.exports_c && !program.modules[def.module as usize].starts_with("std"))
        .collect();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "/* Written by `wip build --header`: what this\n   \
         library exports, for the C that calls it. */"
    );
    // An archive carries undefined references to whatever the library
    // calls, and whoever links it must supply them: the libraries the
    // program's `@link`s name, which on Linux include the maths that a
    // Mac has in libSystem.
    let linked: Vec<String> = program
        .libraries
        .iter()
        .map(|name| format!("-l{name}"))
        .chain(
            program
                .frameworks
                .iter()
                .map(|name| format!("-framework {name}")),
        )
        .collect();
    if !linked.is_empty() {
        let _ = writeln!(
            out,
            "/* Link what this declares with: {} */",
            linked.join(" ")
        );
    }
    let _ = writeln!(out, "#ifndef {guard}\n#define {guard}\n");
    let _ = writeln!(out, "#include <stdbool.h>\n#include <stdint.h>\n");
    // The structs the signatures name. A struct that crosses the boundary
    // is C's layout, so its fields are written out.
    let mut named: Vec<StructId> = Vec::new();
    for def in &exported {
        let tys = def
            .params
            .iter()
            .map(|p| p.ty)
            .chain(std::iter::once(def.ret));
        for ty in tys {
            let mut ty = ty;
            while let TyKind::Ptr(inner) | TyKind::Ref(inner, _) = program.types.kind(ty) {
                ty = inner;
            }
            if let TyKind::Struct(id, _) = program.types.kind(ty)
                && !named.contains(&id)
            {
                named.push(id);
            }
        }
    }
    for id in named {
        let def = &program.structs[id];
        let keyword = if def.is_union { "union" } else { "struct" };
        let name = interner.resolve(def.name);
        let _ = writeln!(out, "{keyword} {name} {{");
        for field in &def.fields {
            let ty = c_type(program, interner, field.ty);
            let field_name = interner.resolve(field.name);
            let _ = writeln!(out, "    {};", declarator(&ty, field_name));
        }
        let _ = writeln!(out, "}};\n");
    }
    for def in &exported {
        let ret = c_type(program, interner, def.ret);
        let name = interner.resolve(def.name);
        let mut params: Vec<String> = Vec::new();
        for p in &def.params {
            let name = interner.resolve(p.name);
            // A slice and a `str` go to C as a pointer and a length, two
            // arguments.
            let elem = match program.types.kind(p.ty) {
                TyKind::Ref(inner, kind) => match program.types.kind(inner) {
                    TyKind::Slice(elem) => Some((c_type(program, interner, elem), kind)),
                    _ => None,
                },
                TyKind::Str => Some(("char".to_string(), wip_hir::RefKind::Shared)),
                _ => None,
            };
            match elem {
                Some((elem, kind)) => {
                    let konst = match kind {
                        wip_hir::RefKind::Shared => "const ",
                        wip_hir::RefKind::Var => "",
                    };
                    params.push(format!("{konst}{elem} *{name}"));
                    params.push(format!("int64_t {name}_len"));
                }
                None => params.push(declarator(&c_type(program, interner, p.ty), name)),
            }
        }
        let params = match params.is_empty() {
            true => "void".to_string(),
            false => params.join(", "),
        };
        let _ = writeln!(out, "{};", declarator(&ret, &format!("{name}({params})")));
    }
    let _ = writeln!(out, "\n#endif");
    out
}
