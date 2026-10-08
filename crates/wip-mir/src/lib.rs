//! The mid-level IR.
//!
//! A function body becomes a control-flow graph: typed locals, and blocks of
//! statements that each end in a terminator. Everything the typed IR leaves
//! implicit is explicit here: evaluation order, drops, `defer`, temporaries,
//! moves and bounds checks. A backend translates the statements one by one,
//! and never decides when anything is dropped.

mod bodies;
mod build;
pub mod c_abi;
mod c_bools;
mod dump;
mod eval;
mod frame;
mod initialized;
mod inline;
mod layout;
mod scalars;
mod shims;
mod simplify;
mod table;
#[cfg(test)]
mod tests;
mod validate;

pub use bodies::function_body;
pub use build::{generator_frames, lower_drop_fn, lower_drop_in_place_fn, lower_fn};
pub use c_bools::convert_c_bools;
pub use dump::dump;
pub use eval::{Locate, evaluate_constants};
pub use initialized::reads_before_writes;
pub use inline::inline;
pub use layout::{Layout, Layouts, Tag};
pub use scalars::scalars;
pub use shims::{shim_name, write_header, write_shims};
pub use simplify::simplify;
pub use table::{TableBytes, table_bytes};
pub use validate::validate;

use wip_hir::{BinaryOp, ConstId, FnId, InterfaceId, Program, Ty, TyKind, Types, UnaryOp};
use wip_syntax::{Span, Symbol};

/// A local of a body: a parameter, a variable, a temporary or the result.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Local(pub u32);

/// A block of a body. Block 0 is the entry, and nothing jumps to it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockId(pub u32);

#[derive(Clone, Debug)]
pub struct Body {
    pub locals: Vec<LocalDecl>,
    /// One local per parameter, in order.
    pub params: Vec<Local>,
    /// The result, or `None` for a body that returns nothing. For an
    /// aggregate result, a backend makes this local the memory the caller
    /// supplied.
    pub ret: Option<Local>,
    pub blocks: Vec<Block>,
}

impl Body {
    pub fn local(&self, local: Local) -> &LocalDecl {
        &self.locals[local.0 as usize]
    }

    pub fn block(&self, block: BlockId) -> &Block {
        &self.blocks[block.0 as usize]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LocalDecl {
    pub ty: Ty,
    pub kind: LocalKind,
    /// The variable or parameter of the source it is, if it is one.
    pub source: Option<Source>,
}

/// A variable or parameter as the source has it, for a debugger to show.
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub name: Symbol,
    /// The name where it is declared.
    pub at: Span,
    /// Where the code that can see it ends: its block's, its `match` arm's
    /// or its loop's end.
    pub seen_until: u32,
    /// Whether the local holds the address of what the name stands for,
    /// as a binding that aliases the value it matched does.
    pub by_address: bool,
}

impl LocalDecl {
    /// A local the source does not name.
    pub fn unnamed(ty: Ty, kind: LocalKind) -> LocalDecl {
        LocalDecl {
            ty,
            kind,
            source: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocalKind {
    Param,
    /// A variable of the source, including a `match` binding.
    Var,
    Temp,
    Return,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

/// A location: a local, and a path of projections into it.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub local: Local,
    pub projections: Vec<Projection>,
}

impl Place {
    pub fn local(local: Local) -> Place {
        Place {
            local,
            projections: Vec::new(),
        }
    }

    /// This place, followed by `projection`.
    pub fn project(&self, projection: Projection) -> Place {
        let mut place = self.clone();
        place.projections.push(projection);
        place
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Projection {
    /// Through an `own<T>`, `&T` or `&var T`.
    Deref,
    /// A struct field. On a `str`, a slice, a reference to a slice or a
    /// buffer, 0 is the pointer to the first byte or element and 1 is the
    /// length.
    Field(u32),
    /// A field of an enum's payload.
    VariantField { variant: u32, field: u32 },
    /// The element whose index a local holds. Not checked: a `Check` before
    /// it compares the index with the length.
    Index(Local),
    /// The element at a constant index.
    ConstIndex(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// The value a place holds. An aggregate operand is passed by address.
    Copy(Place),
    Const(Const),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Const {
    /// The bits of an integer, truncated to its type; a pointer's null is
    /// `0` with the pointer's type.
    Int {
        bits: u128,
        ty: Ty,
    },
    Float {
        value: f64,
        ty: Ty,
    },
    Bool(bool),
    /// The address of a string's NUL-terminated bytes.
    CStr(Symbol),
    /// The address of a constant table, which the program keeps once, in
    /// read-only data; `ty` is the `&T` it is.
    Table {
        id: ConstId,
        ty: Ty,
    },
    /// The address of a function, of the function type `ty`.
    Fn {
        id: FnId,
        ty: Ty,
    },
    /// The address of a type's table of methods for an interface, which a
    /// `&dyn` carries beside the value.
    VTable {
        interface: InterfaceId,
        ty: Ty,
    },
    /// The address of the drop function of `own<ty>`, which an owned
    /// closure's environment carries so that the closure can be dropped
    /// without knowing which lambda made it.
    DropFn(Ty),
    /// The address of the block of zeroed words the program has once,
    /// where the runtime written in Wip keeps its counters.
    RuntimeWords,
    /// The address of the list of the modules' tables of functions and
    /// lines, which the prelude's object holds.
    FrameTables,
    /// The size of `of` as C's `sizeof` answers it — the distance between
    /// two in an array — and its alignment, as an integer of type `ty`;
    /// the layout decides both, when the program is laid out.
    SizeOf {
        of: Ty,
        ty: Ty,
    },
    AlignOf {
        of: Ty,
        ty: Ty,
    },
}

/// What a call calls.
#[derive(Clone, Debug, PartialEq)]
pub enum Callee {
    Fn(FnId),
    /// The runtime's comparison of two strings' bytes.
    StrCmp,
    /// A value of a function type.
    Value(Operand),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rvalue {
    Use(Operand),
    Unary(UnaryOp, Operand),
    /// Never `&&` or `||`, which are branches.
    Binary(BinaryOp, Operand, Operand),
    Cast(Operand, Ty),
    AddressOf(Place),
    /// Which variant the enum at a place holds, as an `i32`.
    Variant(Place),
    /// The length of a C string: the bytes before its NUL, found by walking
    /// it.
    CstrLen(Operand),
    /// A float operation the machine does exactly.
    Float(FloatOp, Operand),
    /// The first times the second, plus the third, rounded once: IEEE
    /// 754's fused multiply-add.
    MulAdd(Operand, Operand, Operand),
    /// What the machine counts or reorders of an integer's bits.
    /// The answer is the operand's type.
    Integer(IntegerOp, Operand),
    /// An integer's bits turned by an amount of its own type, taken modulo
    /// its width.
    Rotate(Turn, Operand, Operand),
    /// Whether `lhs op rhs` — `+`, `-` or `*` on integers of up to 64 bits,
    /// signed by their type — does not fit that type: a `bool`, from the
    /// instruction a checked `+` uses, which panics on it instead.
    Overflows(BinaryOp, Operand, Operand),
    /// A scalar's bits, as the type of the place it is assigned: a float's
    /// as the unsigned integer of its width, and back.
    Bits(Operand),
    /// The function address at `index` of a table of function addresses: a
    /// type's methods for an interface, or the header of an
    /// owned closure's environment, whose first word is its drop function.
    VTableFn {
        table: Operand,
        index: u32,
        /// A type's table of methods, which is constant and never freed,
        /// where an environment's header is memory freed and used again.
        methods: bool,
    },
}

/// What [`Rvalue::Float`] does: each exact, so every machine and the
/// compiler give one answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatOp {
    /// The square root, correctly rounded, as IEEE 754 has it.
    Sqrt,
    Floor,
    Ceil,
    Trunc,
}

/// What [`Rvalue::Integer`] does to an integer's bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegerOp {
    /// How many bits are one.
    CountOnes,
    /// How many zeros come before the highest one bit: the width, for 0.
    LeadingZeros,
    /// How many zeros come after the lowest one bit: the width, for 0.
    TrailingZeros,
    /// The bytes in the other order.
    SwapBytes,
    /// The bits in the other order.
    ReverseBits,
}

/// Which way [`Rvalue::Rotate`] turns the bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Assign(Place, Rvalue),
    /// Writes the enum's tag, or the null that stands for the empty variant
    /// of a niche. The fields are assigned separately.
    SetVariant(Place, u32),
    /// Fills a place with zeros: what a move out of it leaves behind.
    Zero(Place),
    /// Fills a place with a pattern no value holds: what a move out of it
    /// leaves behind, in place of zeros, in a build that checks moves,
    /// for a type whose `destroy` would run on zeros.
    Poison(Place),
    /// Panics where the place holds what [`Statement::Poison`] left: a
    /// value about to be dropped after it was moved away.
    CheckMoved {
        place: Place,
        at: Span,
    },
    Call {
        callee: Callee,
        args: Vec<Operand>,
        dest: Option<Place>,
    },
    /// Heap memory for one value of `ty`; its address goes to `dest`.
    Alloc {
        dest: Place,
        ty: Ty,
    },
    /// Heap memory for `count` elements of type `elem`, which must not be
    /// negative. The pointer and `count` go to `dest`, an `own<[elem]>`. The
    /// program stops if the size in bytes does not fit in an `i64`.
    AllocBuffer {
        dest: Place,
        elem: Ty,
        count: Operand,
    },
    Free(Operand),
    /// Calls the drop function of an `own<ty>` with a pointer, which may be
    /// null ([`lower_drop_fn`]).
    DropFn {
        ty: Ty,
        ptr: Operand,
    },
    /// Calls the function that drops a value of `ty` where it lies, given
    /// its address, and frees nothing ([`lower_drop_in_place_fn`]): a
    /// buffer's element, which may hold a buffer of its own type.
    DropInPlace {
        ty: Ty,
        ptr: Operand,
    },
    /// `dest = lhs op rhs` for `+`, `-` and `*` on integers, which panics
    /// if the answer does not fit. It is one statement
    /// because the answer and whether it fits come from one instruction.
    Arith {
        dest: Place,
        op: BinaryOp,
        lhs: Operand,
        rhs: Operand,
        at: Span,
    },
    /// A value of `ty` — `bool` or an integer of up to 64 bits — at `address`,
    /// read, written or changed as one step that no other thread sees half of,
    /// sequentially consistent: what the runtime's counters, a thread's
    /// "finished" and `std::sync::Atomic` need. What changes it answers what
    /// was there before, in `dest`.
    Atomic {
        op: AtomicOp,
        ty: Ty,
        address: Operand,
        value: Option<Operand>,
        /// What a compare-and-swap expects to find.
        expected: Option<Operand>,
        dest: Option<Place>,
    },
    /// Panics if `fails` is true, with the message its kind gives and the
    /// place `at` names.
    Check {
        fails: Operand,
        kind: CheckKind,
        at: Span,
    },
    /// What follows was written at the span: the source position the code
    /// generator gives the instructions after it, which debug information
    /// maps to a line. It does nothing.
    At(Span),
}

/// Which atomic operation a [`Statement::Atomic`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicOp {
    Add,
    Subtract,
    Swap,
    CompareSwap,
    Load,
    Store,
}

/// What a check reports, with what the runtime formats into its message.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckKind {
    /// An index or a range outside the elements.
    Bounds { index: Operand, length: Operand },
    /// A negative length for a buffer.
    Length { length: Operand },
    /// A division or remainder by zero.
    Division,
    /// An answer that does not fit the type it is written in.
    /// The operator is what the message names.
    Overflow(BinaryOp),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Terminator {
    Goto(BlockId),
    /// A panic: the message, and where it was written. A
    /// message built when the program runs is `note`, a `str`, and what is
    /// written in the program follows it.
    Panic {
        note: Option<Operand>,
        message: Option<Symbol>,
        at: Span,
    },
    Branch {
        cond: Operand,
        then: BlockId,
        otherwise: BlockId,
    },
    /// Goes to the block of the case equal to `value`, an `i32`, or to
    /// `otherwise`.
    Switch {
        value: Operand,
        cases: Vec<(u32, BlockId)>,
        otherwise: BlockId,
    },
    Return,
    /// Control never gets here: after a `match` that covered every variant,
    /// or in a block that nothing reaches.
    Unreachable,
}

impl Terminator {
    /// The blocks control goes to next, once per edge.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Terminator::Goto(target) => vec![*target],
            Terminator::Branch {
                then, otherwise, ..
            } => vec![*then, *otherwise],
            Terminator::Switch {
                cases, otherwise, ..
            } => cases
                .iter()
                .map(|&(_, target)| target)
                .chain([*otherwise])
                .collect(),
            Terminator::Return | Terminator::Unreachable | Terminator::Panic { .. } => Vec::new(),
        }
    }
}

/// Whether values of type `ty` live in memory and are passed by address:
/// structs, enums, arrays, `str`, slices, references to slices and buffers.
pub fn is_aggregate(program: &Program, ty: Ty) -> bool {
    matches!(
        program.types.kind(ty),
        TyKind::Struct(..) | TyKind::Enum(..) | TyKind::Array(..) | TyKind::Str | TyKind::Slice(_)
    ) || is_slice_pointer(program, ty)
        || is_dyn_pointer(program, ty)
        || is_closure_pointer(program, ty)
}

/// Whether a value of type `ty` is one machine value: an integer, a float, a
/// `bool`, a `cstring`, or an `own` or reference to anything but a slice.
pub fn is_scalar(program: &Program, ty: Ty) -> bool {
    matches!(
        program.types.kind(ty),
        TyKind::Int(_)
            | TyKind::Float(_)
            | TyKind::Bool
            | TyKind::Char
            | TyKind::Cstring
            | TyKind::Fn(..)
            | TyKind::Own(_)
            // A C pointer is a scalar: one word, copied.
            | TyKind::Ptr(_)
            | TyKind::Ref(..)
    ) && !is_slice_pointer(program, ty)
        && !is_dyn_pointer(program, ty)
        && !is_closure_pointer(program, ty)
}

/// A reference to a `dyn Interface`: a pointer to the value, and the address
/// of its type's table of methods.
pub fn is_dyn_pointer(program: &Program, ty: Ty) -> bool {
    matches!(program.types.kind(ty), TyKind::Ref(inner, _) if matches!(program.types.kind(inner), TyKind::Dyn(..)))
}

/// A closure lent for one call: what it captured, and the address of its
/// code.
pub fn is_closure_pointer(program: &Program, ty: Ty) -> bool {
    matches!(program.types.kind(ty), TyKind::Ref(inner, _) | TyKind::Own(inner) if matches!(program.types.kind(inner), TyKind::Fn(..)))
}

/// A reference to a slice or a buffer: a pointer to the elements, and their
/// number. A block of slots is the same two words.
pub fn is_slice_pointer(program: &Program, ty: Ty) -> bool {
    matches!(program.types.kind(ty), TyKind::Slots(_))
        || matches!(program.types.kind(ty), TyKind::Ref(inner, _) | TyKind::Own(inner) if matches!(program.types.kind(inner), TyKind::Slice(_)))
}

/// The element type of an array, a slice, a reference to a slice, or a
/// buffer.
pub fn element_ty(program: &Program, ty: Ty) -> Option<Ty> {
    match program.types.kind(ty) {
        TyKind::Array(elem, _) | TyKind::Slice(elem) | TyKind::Slots(elem) => Some(elem),
        // A `str` is bytes.
        TyKind::Str => Some(Types::U8),
        // A C pointer points at the first of however many there are.
        TyKind::Ptr(elem) => Some(elem),
        TyKind::Ref(inner, _) | TyKind::Own(inner) => match program.types.kind(inner) {
            TyKind::Slice(elem) => Some(elem),
            _ => None,
        },
        _ => None,
    }
}

/// The type of what `projection` reaches in a place of type `ty`.
pub fn project_ty(program: &Program, ty: Ty, projection: Projection) -> Ty {
    let words = matches!(program.types.kind(ty), TyKind::Str | TyKind::Slice(_))
        || is_slice_pointer(program, ty)
        || is_dyn_pointer(program, ty)
        || is_closure_pointer(program, ty);
    match (projection, program.types.kind(ty)) {
        (Projection::Deref, TyKind::Own(inner) | TyKind::Ref(inner, _) | TyKind::Ptr(inner)) => {
            inner
        }
        (Projection::Field(i), TyKind::Struct(..)) => program.field_ty(ty, i),
        // A pointer, then a length; a `&dyn`'s second word is its table of
        // methods, and a closure's are its code and its captures, all
        // addresses.
        (Projection::Field(0), _) if words => Types::PTR_U8,
        (Projection::Field(1), _)
            if is_dyn_pointer(program, ty) || is_closure_pointer(program, ty) =>
        {
            Types::PTR_U8
        }
        (Projection::Field(1), _) if words => Types::I64,
        (Projection::VariantField { variant, field }, TyKind::Enum(..)) => {
            program.variant_field_ty(ty, variant, field)
        }
        (Projection::Index(_) | Projection::ConstIndex(_), _) => element_ty(program, ty)
            .unwrap_or_else(|| panic!("{projection:?} of a value that has no elements")),
        (projection, kind) => panic!("{projection:?} does not apply to {kind:?}"),
    }
}

/// The type of a place of `body`.
pub fn place_ty(program: &Program, body: &Body, place: &Place) -> Ty {
    place
        .projections
        .iter()
        .fold(body.local(place.local).ty, |ty, &projection| {
            project_ty(program, ty, projection)
        })
}

/// The type of an operand of `body`.
pub fn operand_ty(program: &Program, body: &Body, operand: &Operand) -> Ty {
    match operand {
        Operand::Copy(place) => place_ty(program, body, place),
        Operand::Const(Const::Int { ty, .. } | Const::Float { ty, .. }) => *ty,
        Operand::Const(Const::Bool(_)) => Types::BOOL,
        Operand::Const(Const::CStr(_)) => Types::CSTRING,
        Operand::Const(
            Const::Fn { ty, .. }
            | Const::Table { ty, .. }
            | Const::SizeOf { ty, .. }
            | Const::AlignOf { ty, .. },
        ) => *ty,
        // A table of methods is an address.
        Operand::Const(
            Const::VTable { .. } | Const::DropFn(_) | Const::RuntimeWords | Const::FrameTables,
        ) => Types::PTR_U8,
    }
}
