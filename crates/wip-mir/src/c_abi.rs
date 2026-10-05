//! How C passes a value, for the calls the compiler makes itself.
//!
//! Wip passes an aggregate by its address; C passes a struct in registers,
//! by rules that differ per target. This module says, for one C function,
//! how each struct parameter and the result travel, on the three targets
//! Wip supports. It covers what Wip can write at the boundary — scalars,
//! pointers, and structs of those — and answers `None` for anything else,
//! which then goes through a wrapper in C as it always did.
//!
//! Apple arm64 and Linux arm64 follow AAPCS64, and for this subset they
//! agree: Apple's divergences are about variadic arguments and about small
//! arguments on the stack. x86-64 is the System V ABI.

use rustc_hash::FxHashSet;
use wip_hir::{Access, ExprKind, FloatTy, FnDef, FnId, Program, Ty, TyKind, Types};
use wip_syntax::{Diagnostic, Interner, codes};

use crate::Layouts;

/// The processor, which is what decides how C passes a struct. Windows
/// would be a third set of rules, and is not written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    /// Linux arm64: AAPCS64 as written.
    Arm64,
    /// Apple arm64: AAPCS64, but with arguments on the stack packed at
    /// their natural alignment rather than in eight-byte slots.
    AppleArm64,
    X86_64,
}

impl Arch {
    /// Whether this is AAPCS64, Apple's or Linux's: the two agree on
    /// registers and differ only on the stack.
    pub fn is_arm64(self) -> bool {
        matches!(self, Arch::Arm64 | Arch::AppleArm64)
    }

    /// The machine the compiler is running on, which is the machine it
    /// compiles for. `None` where the rules are not written.
    pub fn host() -> Option<Arch> {
        if cfg!(windows) {
            return None;
        }
        if cfg!(target_arch = "aarch64") {
            Some(if cfg!(target_vendor = "apple") {
                Arch::AppleArm64
            } else {
                Arch::Arm64
            })
        } else if cfg!(target_arch = "x86_64") {
            Some(Arch::X86_64)
        } else {
            None
        }
    }
}

/// A register-sized piece of a struct: where it starts in the struct's
/// bytes, and the register it travels in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Part {
    pub offset: u32,
    pub reg: Reg,
}

/// What a piece is loaded as. An `F64` on x86-64 may be two `f32`s side by
/// side, which is how System V puts them in one SSE register.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reg {
    I64,
    F32,
    F64,
}

/// How one parameter travels.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Pass {
    /// As Wip passes it already: a scalar, or a `str` or `&[T]` as a
    /// pointer and a length.
    Plain,
    /// In registers, each piece loaded from the struct's bytes.
    Parts(Vec<Part>),
    /// A pointer to a copy the caller makes (arm64, over sixteen bytes).
    Copy,
    /// The bytes themselves, on the stack, rounded up to eight
    /// (x86-64's MEMORY class).
    Stack(u32),
    /// On arm64, a struct the registers left cannot hold: zeros that fill
    /// the rest of its class's registers, and then its pieces, which go on
    /// the stack where C puts the struct (AAPCS64 C.3 and C.11).
    Spilled { pad: Pad, parts: Vec<Part> },
}

/// Registers filled with zeros, so that what follows goes on the stack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pad {
    pub ints: u32,
    pub floats: u32,
}

/// How the result comes back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Answer {
    /// A scalar, or nothing.
    Plain,
    /// In registers, each piece stored to the struct's bytes.
    Parts(Vec<Part>),
    /// Through an address the caller gives: x8 on arm64, the first
    /// argument on x86-64.
    Hidden,
}

/// How a C function's parameters and result travel.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CCall {
    pub params: Vec<Pass>,
    pub ret: Answer,
}

impl CCall {
    /// Whether anything travels differently from how Wip passes it.
    pub fn is_plain(&self) -> bool {
        self.ret == Answer::Plain && self.params.iter().all(|p| *p == Pass::Plain)
    }
}

/// Whether `WIP_FORCE_SHIMS` asks for every call into C to go through a
/// wrapper, which is how the direct path is checked against the one that
/// came before it. A Wip function C calls has no wrapper to go through,
/// so it is unaffected.
fn forced() -> bool {
    std::env::var_os("WIP_FORCE_SHIMS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// The extern functions whose calls still go through C the compiler
/// writes: a variadic call, what a header defines rather than declares, an
/// accessor of C's variables, and a struct by value this module does not
/// classify. A function used as a value keeps its wrapper
/// too, since a Wip function value is called the way Wip calls.
pub fn shimmed(program: &Program, arch: Option<Arch>) -> FxHashSet<FnId> {
    let arch = arch.filter(|_| !forced());
    let mut layouts = Layouts::new();
    let mut shimmed: FxHashSet<FnId> = program
        .fns
        .iter()
        .filter(|(_, def)| def.is_extern)
        .filter(|(_, def)| {
            (def.variadic_of.is_some()
                && !arch.is_some_and(|arch| variadic_direct(program, def, arch)))
                || def.header.is_some()
                || match def.accesses {
                    // A variable C owns is a symbol, loaded and stored
                    // directly, unless a header may make it a macro.
                    // A field of a struct only C lays out
                    // is always C's to reach.
                    Some(Access::Global(global)) => {
                        arch.is_none() || program.globals[global].header.is_some()
                    }
                    Some(Access::Field(..)) => true,
                    None => false,
                }
                || (by_value(program, def)
                    && arch
                        .and_then(|arch| classify(program, &mut layouts, def, arch))
                        .is_none())
        })
        .map(|(id, _)| id)
        .collect();
    for (_, def) in program.fns.iter() {
        let Some(body) = &def.body else { continue };
        for (_, expr) in body.exprs.iter() {
            if let ExprKind::FnRef { id, .. } = expr.kind {
                let referenced = &program.fns[id];
                if referenced.is_extern && by_value(program, referenced) {
                    shimmed.insert(id);
                }
            }
        }
    }
    shimmed
}

/// The pieces a struct is passed in once it is on the stack, laid out so
/// that Cranelift's stack slots hold its bytes where C's would. Integer
/// pieces are eight bytes on every target. A floating aggregate is its
/// members on Apple, which packs stack arguments at their natural
/// alignment as it packs the members; on Linux every stack argument is
/// eight-byte aligned and rounded up to eight, so it is eight-byte pieces
/// of its bytes, carried as `f64` so that no general register left takes
/// one.
fn spilled(arch: Arch, size: u32, parts: Vec<Part>) -> Vec<Part> {
    let floating = parts.iter().all(|part| part.reg != Reg::I64);
    if !floating || arch == Arch::AppleArm64 {
        return parts;
    }
    (0..size.div_ceil(8))
        .map(|i| Part {
            offset: i * 8,
            reg: Reg::F64,
        })
        .collect()
}

/// Whether a call that passes more than its declaration names — one of
/// the declarations `resolve_variadic_calls` makes, one per set of types
/// — is made directly. Its extra arguments are scalars;
/// a struct among its named ones, or a header that may
/// make it a macro, keeps the wrapper, and so does a floating extra
/// argument on x86-64, where the callee reads `%al` to know whether to
/// save the vector registers, and Cranelift cannot set it.
pub fn variadic_direct(program: &Program, def: &FnDef, arch: Arch) -> bool {
    let Some(original) = def.variadic_of else {
        return false;
    };
    let named = program.fns[original].params.len();
    if def.header.is_some() || program.fns[original].header.is_some() || by_value(program, def) {
        return false;
    }
    let extras = &def.params[named..];
    let scalar = extras.iter().all(|p| promoted(program, p.ty).is_some());
    let floating = extras
        .iter()
        .any(|p| matches!(program.types.kind(p.ty), TyKind::Float(_)));
    scalar && (arch.is_arm64() || !floating)
}

/// What C's default argument promotions make of an extra argument of a
/// variadic call: a `float` becomes a `double`, and an integer narrower
/// than `int` becomes an `int`, extended by its own sign (C11 6.5.2.2).
/// `None` for what cannot be an extra argument at all.
pub fn promoted(program: &Program, ty: Ty) -> Option<Promoted> {
    Some(match program.types.kind(ty) {
        TyKind::Float(_) => Promoted::Double,
        TyKind::Bool => Promoted::Int {
            bits: 32,
            signed: false,
        },
        TyKind::Char => Promoted::Int {
            bits: 32,
            signed: false,
        },
        TyKind::Int(t) if t.bits() < 32 => Promoted::Int {
            bits: 32,
            signed: t.signed(),
        },
        TyKind::Int(t) if t.bits() <= 64 => Promoted::Int {
            bits: t.bits(),
            signed: t.signed(),
        },
        TyKind::Cstring | TyKind::Ptr(_) | TyKind::Fn(..) => Promoted::Int {
            bits: 64,
            signed: false,
        },
        TyKind::Ref(inner, _) if !matches!(program.types.kind(inner), TyKind::Slice(_)) => {
            Promoted::Int {
                bits: 64,
                signed: false,
            }
        }
        _ => return None,
    })
}

/// An extra argument of a variadic call, as C's promotions leave it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Promoted {
    Int { bits: u32, signed: bool },
    Double,
}

/// Whether a struct crosses by value, in either direction.
pub fn by_value(program: &Program, def: &FnDef) -> bool {
    let is_struct = |ty: Ty| matches!(program.types.kind(ty), TyKind::Struct(..));
    is_struct(def.ret) || def.params.iter().any(|p| is_struct(p.ty))
}

/// How `def`'s parameters and result travel on `arch`, or `None` where a
/// wrapper in C has to decide: a struct of no size.
pub fn classify(
    program: &Program,
    layouts: &mut Layouts,
    def: &FnDef,
    arch: Arch,
) -> Option<CCall> {
    let mut regs = Registers::new(arch);
    let ret = match program.types.kind(def.ret) {
        TyKind::Struct(..) => {
            let layout = layouts.of(program, def.ret);
            let leaves = leaves_of(program, layouts, def.ret)?;
            if layout.size == 0 {
                return None;
            }
            match pieces(arch, layout.size, &leaves) {
                Some(parts) => Answer::Parts(parts),
                None => {
                    // x86-64's hidden pointer is the first argument; arm64's
                    // is x8, which no argument uses.
                    if arch == Arch::X86_64 {
                        regs.ints -= 1;
                    }
                    Answer::Hidden
                }
            }
        }
        _ => Answer::Plain,
    };
    let mut params = Vec::with_capacity(def.params.len());
    for param in &def.params {
        let ty = param.ty;
        let pass = match program.types.kind(ty) {
            TyKind::Struct(..) => {
                let layout = layouts.of(program, ty);
                let leaves = leaves_of(program, layouts, ty)?;
                if layout.size == 0 {
                    return None;
                }
                match pieces(arch, layout.size, &leaves) {
                    Some(parts) => {
                        let ints = parts.iter().filter(|p| p.reg == Reg::I64).count() as u32;
                        let floats = parts.len() as u32 - ints;
                        if regs.ints >= ints && regs.floats >= floats {
                            regs.ints -= ints;
                            regs.floats -= floats;
                            Pass::Parts(parts)
                        } else if arch.is_arm64() {
                            // C.3 and C.11: the struct goes on the stack
                            // whole, and the registers of its class are
                            // spent, so no later argument of that class is
                            // in one either. Filling them with zeros says so
                            // to Cranelift.
                            let pad = if floats > 0 {
                                Pad {
                                    ints: 0,
                                    floats: std::mem::take(&mut regs.floats),
                                }
                            } else {
                                Pad {
                                    ints: std::mem::take(&mut regs.ints),
                                    floats: 0,
                                }
                            };
                            Pass::Spilled {
                                pad,
                                parts: spilled(arch, layout.size, parts),
                            }
                        } else {
                            // "If there are no registers available for any
                            // eightbyte of an argument, the whole argument
                            // is passed on the stack", which is MEMORY, and
                            // uses no register.
                            Pass::Stack(layout.size.next_multiple_of(8))
                        }
                    }
                    None if arch.is_arm64() => {
                        regs.take_int();
                        Pass::Copy
                    }
                    None => Pass::Stack(layout.size.next_multiple_of(8)),
                }
            }
            TyKind::Float(_) => {
                regs.take_float();
                Pass::Plain
            }
            // A `str` and a `&[T]` are a pointer and a length.
            _ if is_pair(program, ty) => {
                regs.take_int();
                regs.take_int();
                Pass::Plain
            }
            _ => {
                regs.take_int();
                Pass::Plain
            }
        };
        params.push(pass);
    }
    Some(CCall { params, ret })
}

/// Whether a type reaches C as a pointer and a length.
fn is_pair(program: &Program, ty: Ty) -> bool {
    match program.types.kind(ty) {
        TyKind::Str | TyKind::Slice(_) => true,
        TyKind::Ref(inner, _) => matches!(program.types.kind(inner), TyKind::Slice(_)),
        _ => false,
    }
}

/// The argument registers left, counted as C counts them.
struct Registers {
    ints: u32,
    floats: u32,
}

impl Registers {
    fn new(arch: Arch) -> Registers {
        match arch {
            // x0–x7 and v0–v7.
            Arch::Arm64 | Arch::AppleArm64 => Registers { ints: 8, floats: 8 },
            // rdi, rsi, rdx, rcx, r8, r9; xmm0–xmm7.
            Arch::X86_64 => Registers { ints: 6, floats: 8 },
        }
    }

    fn take_int(&mut self) {
        self.ints = self.ints.saturating_sub(1);
    }

    fn take_float(&mut self) {
        self.floats = self.floats.saturating_sub(1);
    }
}

/// A scalar inside a struct, at its offset from the struct's start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Leaf {
    Int,
    F32,
    F64,
}

/// Every scalar of a struct or a union, nested ones and arrays opened, each
/// at its offset; a union's overlap.
fn leaves_of(program: &Program, layouts: &mut Layouts, ty: Ty) -> Option<Vec<(u32, u32, Leaf)>> {
    let mut out = Vec::new();
    collect(program, layouts, ty, 0, &mut out)?;
    Some(out)
}

/// Adds `ty`'s scalars at `base`, each as its offset, its size and its
/// kind.
fn collect(
    program: &Program,
    layouts: &mut Layouts,
    ty: Ty,
    base: u32,
    out: &mut Vec<(u32, u32, Leaf)>,
) -> Option<()> {
    match program.types.kind(ty) {
        // A union's fields are all at 0, over the same bytes, so its scalars
        // overlap, which is what both ABIs classify: System V merges what
        // shares an eightbyte, and AAPCS64 asks whether every member is of
        // one floating type.
        TyKind::Struct(..) => {
            for (i, field) in program.field_tys(ty).into_iter().enumerate() {
                let offset = layouts.field_offset(program, ty, i as u32);
                collect(program, layouts, field, base + offset, out)?;
            }
        }
        TyKind::Array(elem, len) => {
            let stride = layouts.of(program, elem).stride();
            for i in 0..len as u32 {
                collect(program, layouts, elem, base + i * stride, out)?;
            }
        }
        TyKind::Float(FloatTy::F32) => out.push((base, 4, Leaf::F32)),
        TyKind::Float(_) => out.push((base, 8, Leaf::F64)),
        TyKind::Unit => {}
        _ => {
            let size = layouts.of(program, ty).size;
            if size == 0 && ty != Types::UNIT {
                return None;
            }
            out.push((base, size, Leaf::Int));
        }
    }
    Some(())
}

/// The registers a struct of `size` bytes travels in, or `None` where it
/// goes through memory: over sixteen bytes on either processor, unless on
/// arm64 it is a homogeneous floating aggregate.
fn pieces(arch: Arch, size: u32, leaves: &[(u32, u32, Leaf)]) -> Option<Vec<Part>> {
    match arch {
        Arch::Arm64 | Arch::AppleArm64 => {
            // A homogeneous floating aggregate: every scalar in it `f32`, or
            // every one `f64`, and one to four of that type in its size,
            // each in a floating register of its own (AAPCS64 B.3 and C.2).
            // It is counted by size rather than by member, which for a
            // struct is the same thing — one of a single floating type has
            // no padding — and for a union is the rule: its members overlap,
            // and it holds as many as fit.
            let first = leaves.first()?.2;
            let (reg, base) = match first {
                Leaf::F32 => (Reg::F32, 4),
                Leaf::F64 => (Reg::F64, 8),
                Leaf::Int => (Reg::I64, 0),
            };
            let homogeneous = base != 0
                && size.is_multiple_of(base)
                && (1..=4).contains(&(size / base))
                && leaves.iter().all(|&(_, _, leaf)| leaf == first);
            if homogeneous {
                return Some(
                    (0..size / base)
                        .map(|i| Part {
                            offset: i * base,
                            reg,
                        })
                        .collect(),
                );
            }
            // Anything else of sixteen bytes or fewer is its bytes, in one
            // or two general registers (C.10).
            if size > 16 {
                return None;
            }
            Some(
                (0..size.div_ceil(8))
                    .map(|i| Part {
                        offset: i * 8,
                        reg: Reg::I64,
                    })
                    .collect(),
            )
        }
        Arch::X86_64 => {
            // Over sixteen bytes is MEMORY (System V 3.2.3, rule 1).
            if size > 16 {
                return None;
            }
            // Each eightbyte is INTEGER if anything in it is, and SSE if
            // everything in it is floating.
            let parts = (0..size.div_ceil(8))
                .map(|i| {
                    let (start, end) = (i * 8, i * 8 + 8);
                    let inside: Vec<_> = leaves
                        .iter()
                        .filter(|&&(offset, _, _)| offset >= start && offset < end)
                        .collect();
                    let reg = if inside.iter().any(|&&(_, _, leaf)| leaf == Leaf::Int) {
                        Reg::I64
                    } else {
                        // An SSE eightbyte whose data ends in its low half
                        // is a `float`; otherwise the register's low 64 bits
                        // carry it — a `double`, or two `float`s side by
                        // side.
                        let used = inside
                            .iter()
                            .map(|&&(offset, size, _)| offset + size - start)
                            .max()
                            .unwrap_or(0);
                        if used <= 4 { Reg::F32 } else { Reg::F64 }
                    };
                    Part { offset: start, reg }
                })
                .collect();
            Some(parts)
        }
    }
}

/// The `@export("C")` functions this target cannot give C's signature: a struct
/// by value on a target whose rules are not written here, and one taken as a
/// value, which Wip would call the way Wip calls. A call *into* C has a wrapper
/// to fall back on; a call from C has none. Either backend builds C's side of
/// the rest.
pub fn refused_exports(
    program: &Program,
    interner: &Interner,
    arch: Option<Arch>,
) -> Vec<Diagnostic> {
    let mut layouts = Layouts::new();
    let mut refused = Vec::new();
    let mut as_values = FxHashSet::default();
    for (_, def) in program.fns.iter() {
        let Some(body) = &def.body else { continue };
        for (_, expr) in body.exprs.iter() {
            if let ExprKind::FnRef { id, .. } = expr.kind {
                as_values.insert(id);
            }
        }
    }
    for (id, def) in program.fns.iter() {
        if !def.exports_c || !by_value(program, def) {
            continue;
        }
        let name = interner.resolve(def.name);
        let classified = arch.and_then(|arch| classify(program, &mut layouts, def, arch));
        if classified.is_none() {
            refused.push(
                Diagnostic::error(
                    codes::UNSUPPORTED,
                    format!("C cannot call `{name}` on this target"),
                    def.name_span,
                    "a struct by value the compiler cannot pass as C does here",
                )
                .with_note(
                    "how C passes a struct by value is written for Apple arm64, Linux arm64 and Linux x86-64 only; a call into C goes through a wrapper in C elsewhere, but a call from C has none",
                )
                .with_help("take the struct by reference, `&T`, which C sees as a pointer"),
            );
        } else if as_values.contains(&id) {
            refused.push(
                Diagnostic::error(
                    codes::UNSUPPORTED,
                    format!("`{name}` is called by C, and cannot also be a Wip function value"),
                    def.name_span,
                    "takes or answers a struct by value",
                )
                .with_note(
                    "C passes the struct in registers and Wip passes its address, so one function cannot be both",
                )
                .with_help("wrap it in a function of Wip's own, and take that as the value"),
            );
        }
    }
    refused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(fields: &[(u32, u32, Leaf)]) -> Vec<(u32, u32, Leaf)> {
        fields.to_vec()
    }

    const F: Leaf = Leaf::F32;
    const D: Leaf = Leaf::F64;
    const I: Leaf = Leaf::Int;

    fn part(offset: u32, reg: Reg) -> Part {
        Part { offset, reg }
    }

    #[test]
    fn vector2_is_two_floats_on_arm64_and_one_sse_register_on_x86_64() {
        let v2 = leaves(&[(0, 4, F), (4, 4, F)]);
        assert_eq!(
            pieces(Arch::Arm64, 8, &v2),
            Some(vec![part(0, Reg::F32), part(4, Reg::F32)])
        );
        assert_eq!(pieces(Arch::X86_64, 8, &v2), Some(vec![part(0, Reg::F64)]));
    }

    #[test]
    fn vector3_splits_on_x86_64_into_a_pair_and_a_float() {
        let v3 = leaves(&[(0, 4, F), (4, 4, F), (8, 4, F)]);
        assert_eq!(
            pieces(Arch::X86_64, 12, &v3),
            Some(vec![part(0, Reg::F64), part(8, Reg::F32)])
        );
        assert_eq!(pieces(Arch::Arm64, 12, &v3).map(|p| p.len()), Some(3));
    }

    #[test]
    fn color_is_one_integer_register() {
        let color = leaves(&[(0, 1, I), (1, 1, I), (2, 1, I), (3, 1, I)]);
        assert_eq!(
            pieces(Arch::Arm64, 4, &color),
            Some(vec![part(0, Reg::I64)])
        );
        assert_eq!(
            pieces(Arch::X86_64, 4, &color),
            Some(vec![part(0, Reg::I64)])
        );
    }

    #[test]
    fn a_mixed_eightbyte_is_integer() {
        let mixed = leaves(&[(0, 4, F), (4, 4, I), (8, 8, D)]);
        assert_eq!(
            pieces(Arch::X86_64, 16, &mixed),
            Some(vec![part(0, Reg::I64), part(8, Reg::F64)])
        );
        // Not homogeneous, so its bytes, in two general registers.
        assert_eq!(
            pieces(Arch::Arm64, 16, &mixed),
            Some(vec![part(0, Reg::I64), part(8, Reg::I64)])
        );
    }

    #[test]
    fn four_doubles_are_registers_on_arm64_and_memory_on_x86_64() {
        let v4d = leaves(&[(0, 8, D), (8, 8, D), (16, 8, D), (24, 8, D)]);
        assert_eq!(pieces(Arch::Arm64, 32, &v4d).map(|p| p.len()), Some(4));
        assert_eq!(pieces(Arch::X86_64, 32, &v4d), None);
    }

    #[test]
    fn five_floats_are_not_homogeneous() {
        let five = leaves(&[(0, 4, F), (4, 4, F), (8, 4, F), (12, 4, F), (16, 4, F)]);
        assert_eq!(pieces(Arch::Arm64, 20, &five), None);
    }

    #[test]
    fn mixed_floats_are_not_homogeneous() {
        let mixed = leaves(&[(0, 4, F), (8, 8, D)]);
        assert_eq!(
            pieces(Arch::Arm64, 16, &mixed),
            Some(vec![part(0, Reg::I64), part(8, Reg::I64)])
        );
        assert_eq!(
            pieces(Arch::X86_64, 16, &mixed),
            Some(vec![part(0, Reg::F32), part(8, Reg::F64)])
        );
    }

    #[test]
    fn a_spilled_floating_struct_is_its_members_on_apple_and_its_bytes_on_linux() {
        let v3 = vec![part(0, Reg::F32), part(4, Reg::F32), part(8, Reg::F32)];
        assert_eq!(spilled(Arch::AppleArm64, 12, v3.clone()), v3);
        assert_eq!(
            spilled(Arch::Arm64, 12, v3),
            vec![part(0, Reg::F64), part(8, Reg::F64)]
        );
    }

    #[test]
    fn a_spilled_integer_struct_is_its_eight_byte_pieces_everywhere() {
        let pair = vec![part(0, Reg::I64), part(8, Reg::I64)];
        assert_eq!(spilled(Arch::AppleArm64, 16, pair.clone()), pair);
        assert_eq!(spilled(Arch::Arm64, 16, pair.clone()), pair);
    }

    #[test]
    fn a_union_of_floats_is_counted_by_its_size() {
        // union { float a; float b[2]; }: two floats' worth, not three.
        let overlapping = leaves(&[(0, 4, F), (0, 4, F), (4, 4, F)]);
        assert_eq!(
            pieces(Arch::Arm64, 8, &overlapping),
            Some(vec![part(0, Reg::F32), part(4, Reg::F32)])
        );
        assert_eq!(
            pieces(Arch::X86_64, 8, &overlapping),
            Some(vec![part(0, Reg::F64)])
        );
    }

    #[test]
    fn a_union_of_two_floating_types_is_not_homogeneous() {
        // union { float f; double d; }
        let mixed = leaves(&[(0, 4, F), (0, 8, D)]);
        assert_eq!(
            pieces(Arch::Arm64, 8, &mixed),
            Some(vec![part(0, Reg::I64)])
        );
        assert_eq!(
            pieces(Arch::X86_64, 8, &mixed),
            Some(vec![part(0, Reg::F64)])
        );
    }

    #[test]
    fn an_integer_over_a_float_makes_the_eightbyte_integer() {
        // union { float f; int32_t i; }
        let merged = leaves(&[(0, 4, F), (0, 4, I)]);
        assert_eq!(
            pieces(Arch::X86_64, 4, &merged),
            Some(vec![part(0, Reg::I64)])
        );
        assert_eq!(
            pieces(Arch::Arm64, 4, &merged),
            Some(vec![part(0, Reg::I64)])
        );
    }
}
