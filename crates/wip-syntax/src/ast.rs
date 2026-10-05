//! The syntax tree.
//!
//! Expressions, statements and types live in index arenas owned by [`Ast`] and
//! refer to each other by [`ExprId`], [`StmtId`] and [`TypeId`]. Every node
//! carries its source span. A parse error leaves an `Error` node where the
//! malformed construct was, so later phases can keep going.

mod dump;

pub use dump::{dump, dump_at};
use la_arena::{Arena, Idx};

use crate::{Span, Symbol};

pub type ExprId = Idx<Expr>;
pub type StmtId = Idx<Stmt>;
pub type TypeId = Idx<Type>;

#[derive(Debug, Default)]
pub struct Ast {
    /// Top-level items in source order.
    pub items: Vec<Item>,
    pub exprs: Arena<Expr>,
    pub stmts: Arena<Stmt>,
    pub types: Arena<Type>,
}

/// An identifier together with where it was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Name {
    pub sym: Symbol,
    pub span: Span,
}

// ---- items ----

/// `@name` or `@name(a, b = 1)` before a declaration. The
/// set of names is closed and the checker knows it; the parser only reads
/// the form.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub name: Name,
    pub args: Vec<AnnotationArg>,
    /// The `(` and `)`, where they were written: an annotation given an
    /// empty list is not the same as one given none.
    pub parens: Option<Span>,
    pub span: Span,
}

/// One argument of an annotation: a literal or a name, or `name = literal`.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationArg {
    pub name: Option<Name>,
    pub value: AnnotationValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationValue {
    Int(u128),
    Str(Symbol),
    Bool(bool),
    /// A name: the interfaces `@derive(Eq, Hash)` asks the compiler to
    /// write.
    Name(Symbol),
    /// A token that is none of those: reported, and kept so that the rest of
    /// the declaration is still parsed.
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Import(ImportDecl),
    /// `val NAME: T = value` at the top level: a constant, whose value is
    /// known where it is written.
    Val(ValDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Fn(FnDecl),
    Extern(ExternBlock),
    /// `extend Type { … }`: methods of a type, outside its declaration.
    Extend(ExtendBlock),
    /// `interface Name { … }`.
    Interface(InterfaceDecl),
    /// `type Name<T> = …`: another name for a type.
    Type(TypeAlias),
    /// `assert(condition, "note")` at the top level: checked while the
    /// program is compiled.
    Assert(AssertDecl),
}

/// A top-level `assert`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssertDecl {
    /// The annotations written before it: `@target` alone.
    pub annotations: Vec<Annotation>,
    /// The `assert(…)` itself, an [`ExprKind::Assert`].
    pub assert: ExprId,
    pub span: Span,
}

/// `type Id = i64`, `type Pairs<T> = Vec<(T, T)>`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeAlias {
    pub is_pub: bool,
    pub name: Name,
    pub generics: Vec<GenericParam>,
    pub ty: TypeId,
    pub span: Span,
}

/// `val NAME: T = value` at the top level. Its value is
/// worked out where it is written, so nothing runs before `main`.
#[derive(Debug, Clone, PartialEq)]
pub struct ValDecl {
    /// The annotations written before it.
    pub annotations: Vec<Annotation>,
    pub name: Name,
    pub ty: Option<TypeId>,
    pub value: ExprId,
    /// Exported from its module.
    pub is_pub: bool,
    pub span: Span,
}

/// `interface Name { … }`: a set of methods a type may implement.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDecl {
    /// The annotations written before it.
    pub annotations: Vec<Annotation>,
    pub name: Name,
    /// `interface From<T>`: the types it is about, which its methods may
    /// use.
    pub generics: Vec<GenericParam>,
    pub methods: Vec<InterfaceMethod>,
    /// Exported from its module.
    pub is_pub: bool,
    pub span: Span,
}

/// A method of an interface: a signature, with a default body where one is
/// written.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceMethod {
    pub sig: FnSig,
    /// How it takes its receiver, and the span of the word that says so.
    pub receiver: (Receiver, Span),
    /// The expression after `=`, where the method has a default.
    pub default: Option<ExprId>,
    pub span: Span,
}

/// `extend Type { … }`, or `extend Type<T> { … }`, whose type arguments bind
/// the type's parameters under the names written.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtendBlock {
    /// The annotations written before it.
    pub annotations: Vec<Annotation>,
    /// `extend [T]`: methods of a slice, with the element's name here.
    /// `path` then holds that name, so spans still work.
    pub slice_of: Option<Name>,
    /// The interface implemented: `extend Rect: Shape`.
    pub interface: Option<Name>,
    /// `extend ConfigError: From<io::IoError>`: what the interface's own
    /// type parameters are here.
    pub interface_args: Option<TypeArgs>,
    /// The type: one name, or a path when another module's type was named,
    /// which the type checker reports.
    pub path: Vec<Name>,
    pub generics: Vec<GenericParam>,
    pub methods: Vec<FnDecl>,
    /// Written by `@derive`, not by the programmer: the argument that asked
    /// for it, which every node of the block is placed at.
    pub derived: Option<Span>,
    pub span: Span,
}

/// `import a::b::c`, `import a::b::c as name`, or
/// `import a::b::{self, Item, other as name}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    /// The module.
    pub path: Vec<Name>,
    /// `as name` after the path: the name the module is bound to.
    pub alias: Option<Name>,
    /// The names in braces after the path, if there are braces. Only these
    /// are bound, and `self` among them binds the module.
    pub items: Option<Vec<ImportItem>>,
    pub span: Span,
}

/// One name in the braces of an import.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportItem {
    /// An item of the module; `None` for `self`, the module itself.
    pub name: Option<Name>,
    /// The name as written, `self` included.
    pub span: Span,
    /// `as name`: the name it is bound to instead.
    pub alias: Option<Name>,
}

/// A type parameter, `T` or `T: copy`, of a generic item.
#[derive(Debug, Clone, PartialEq)]
pub struct GenericParam {
    pub name: Name,
    /// The constraints after `:`, separated by `+`: `copy`, and the
    /// interfaces the type must implement.
    pub bounds: Vec<Bound>,
    /// `K = T`: the type it is where a use leaves it out, which a struct's
    /// or an enum's parameter may have.
    pub default: Option<TypeId>,
    pub span: Span,
}

/// One constraint: `copy`, or an interface with the types it takes where it
/// takes any — `T: From<i64>`, `C: Items<T>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    pub name: Name,
    pub args: Vec<TypeId>,
    pub span: Span,
}

/// One parameter of a lambda: a name, and a type where one is written.
#[derive(Debug, Clone, PartialEq)]
pub struct LambdaParam {
    pub name: Name,
    pub ty: Option<TypeId>,
    pub span: Span,
}

/// `<A, B>` in an expression, written after the segment `after` of its
/// path.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeArgs {
    pub after: usize,
    pub args: Vec<TypeId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    /// The annotations written before it.
    pub annotations: Vec<Annotation>,
    /// `extern struct`: C's layout, promised now and later.
    pub is_extern: bool,
    /// `extern union`: every field over the same bytes.
    pub is_union: bool,
    /// `view struct`: may hold a `str` or a `&` reference, and is kept
    /// where a `str` is.
    pub is_view: bool,
    pub name: Name,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<Field>,
    /// Methods and static functions, in the order they are written among the
    /// fields.
    pub methods: Vec<FnDecl>,
    /// Exported from its module.
    pub is_pub: bool,
    pub span: Span,
}

/// `name: type` — a struct field, a variant field or a parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// `pub` on a struct's field, which exports it from the module.
    /// A parameter never has one.
    pub is_pub: bool,
    /// `pub var`: another module may write it too, not only read it.
    pub is_var: bool,
    pub name: Name,
    pub ty: TypeId,
    /// `= value` after a parameter's type: its default.
    pub default: Option<ExprId>,
    pub span: Span,
}

pub type Param = Field;

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    /// The annotations written before it.
    pub annotations: Vec<Annotation>,
    pub name: Name,
    pub generics: Vec<GenericParam>,
    pub variants: Vec<Variant>,
    /// Methods and static functions, in the order they are written among the
    /// variants.
    pub methods: Vec<FnDecl>,
    /// Exported from its module.
    pub is_pub: bool,
    /// `view enum`: its variants may borrow, as a `view struct`'s fields
    /// may.
    pub is_view: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: Name,
    /// Empty for a unit variant.
    pub fields: Vec<Field>,
    pub span: Span,
}

/// Everything in a function declaration before its body.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSig {
    pub name: Name,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    /// `...` after the parameters: a C function that takes more, where it
    /// was written.
    pub variadic: Option<Span>,
    /// `None` means the function returns nothing.
    pub ret: Option<TypeId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    /// The annotations written before it.
    pub annotations: Vec<Annotation>,
    pub sig: FnSig,
    /// The expression after `=`. An `@intrinsic` declaration has none: the
    /// compiler writes its body.
    pub body: Option<ExprId>,
    /// Exported from its module, and where the word was
    /// written, so that one that says nothing can be reported with a fix.
    pub is_pub: bool,
    pub pub_span: Option<Span>,
    /// How a method takes its receiver, and the span of the word that says
    /// so. `None` for a function that is not declared in a type.
    pub receiver: Option<(Receiver, Span)>,
    pub span: Span,
}

/// What a method does with the value it is called on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receiver {
    /// `fn`: `self` is `&Self`.
    Read,
    /// `var fn`: `self` is `&var Self`.
    Var,
    /// `move fn`: `self` is `Self`, by value.
    Move,
    /// `static fn`: no receiver.
    Static,
    /// `lend fn`: a lending pair declared once, a reading half with `self`
    /// a `&Self` and a writing half with `self` a `&var Self`, from one body.
    Lend,
}

impl Receiver {
    pub fn text(self) -> &'static str {
        match self {
            Receiver::Read => "fn",
            Receiver::Var => "var fn",
            Receiver::Move => "move fn",
            Receiver::Static => "static fn",
            Receiver::Lend => "lend fn",
        }
    }
}

/// A C function a module declares. `pub` exports it, so that a module of
/// bindings is the unit people share.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternFn {
    pub sig: FnSig,
    /// `@symbol("sqlite3_open")`: what C calls it.
    pub annotations: Vec<Annotation>,
    pub is_pub: bool,
}

/// `val optind: c_int` or `var optind: c_int` in an extern block: a
/// variable C owns.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternGlobal {
    pub name: Name,
    /// `@symbol("__stderrp")`: what C calls it.
    pub annotations: Vec<Annotation>,
    pub ty: TypeId,
    /// `var`: Wip may write it too. A `val` is read-only.
    pub is_mut: bool,
    pub is_pub: bool,
    pub span: Span,
}

/// `type name` in an extern block, and whether it is exported.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternType {
    pub name: Name,
    pub annotations: Vec<Annotation>,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternBlock {
    /// The annotations written before it: `@link("sqlite3")` names the
    /// library its declarations come from.
    pub annotations: Vec<Annotation>,
    /// The ABI string, quotes included. Empty if it was missing.
    pub abi: Span,
    pub fns: Vec<ExternFn>,
    /// `type name`: C types whose contents Wip does not know.
    pub types: Vec<ExternType>,
    /// `val` and `var`: the variables C owns.
    pub globals: Vec<ExternGlobal>,
    /// `struct` and `union` written inside the block: C's layout, and the
    /// block's `@header`. They are the same declarations
    /// as `extern struct` at the top level.
    pub structs: Vec<StructDecl>,
    pub span: Span,
}

// ---- types ----

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

/// An array's length: a literal, or a top-level `val`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArrayLen {
    Int(u64),
    Name(Name),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// A type name, including built-ins such as `i64`, with its type
    /// arguments, if any: `Pair<i64, bool>`.
    Named {
        name: Symbol,
        args: Vec<TypeId>,
    },
    /// A type from another module, `a::b::Point`.
    Path {
        segments: Vec<Name>,
        args: Vec<TypeId>,
    },
    Own(TypeId),
    /// `&T`, or `&var T` when `var` is set.
    Ref {
        var: bool,
        inner: TypeId,
    },
    Array {
        elem: TypeId,
        len: ArrayLen,
    },
    /// `[T]`, a slice: only behind `&`.
    Slice(TypeId),
    /// `dyn Name`, and `dyn Name<A>` where the interface takes types: some
    /// type that implements it, only behind `&`.
    Dyn(Name, Vec<TypeId>),
    /// `(a: A, b: B) => R`, a function type. The parameters'
    /// names are required, and only document the type.
    Fn {
        params: Vec<Field>,
        ret: TypeId,
    },
    Error,
}

// ---- statements ----

/// `{ … }`. Its value is its last statement, if that is an expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<StmtId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `val` (immutable) or `var` (mutable).
    Let {
        mutable: bool,
        name: Name,
        ty: Option<TypeId>,
        init: ExprId,
    },
    /// `val pattern = value else { … }`: the pattern's
    /// bindings for the rest of the block, or the `else` block, which leaves.
    Guard {
        /// `var`: each name is a variable of its own, holding its part.
        mutable: bool,
        pattern: Pattern,
        value: ExprId,
        /// `else { … }`, which a refutable pattern needs and an
        /// irrefutable one has no use for.
        else_block: Option<Block>,
    },
    Defer(ExprId),
    Return(Option<ExprId>),
    /// `yield value`: hands the value to the list being built, `own [ … ]`
    /// or `own for`.
    Yield(ExprId),
    While {
        /// `outer: while …`, which `break outer` leaves.
        label: Option<Name>,
        cond: ExprId,
        body: Block,
    },
    /// `for name in source { … }`, with `None` for `_`.
    For {
        /// `outer: for …`, which `break outer` leaves.
        label: Option<Name>,
        /// What each element is bound to: a name, `_`, or a struct taken
        /// apart.
        binding: Pattern,
        source: ForSource,
        body: Block,
    },
    /// `break`, or `break outer`: which loop it leaves.
    Break(Option<Name>),
    Continue(Option<Name>),
    Expr(ExprId),
}

/// What a `for` walks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ForSource {
    /// The elements of an array, a slice or a buffer.
    Elements(ExprId),
    /// The integers from `lo` up to `hi`: not including it, or, written
    /// `..=`, including it.
    Range {
        lo: ExprId,
        hi: ExprId,
        inclusive: bool,
    },
}

// ---- expressions ----

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// The literal's magnitude. A leading `-` is a separate [`UnaryOp::Neg`],
    /// so the range check belongs to the type checker.
    Int(u128),
    Float(f64),
    /// The contents, with escapes applied.
    Str(Symbol),
    /// `'x'`: a Unicode scalar value.
    Char(u32),
    /// `b'x'`: the byte of an ASCII character.
    Byte(u8),
    Bool(bool),
    /// `null`: a C pointer that points at nothing.
    Null,
    Name(Symbol),
    /// `self`: a method's receiver.
    SelfRef,
    Paren(ExprId),
    Block(Block),
    /// `else if` is an `else` whose expression is another `If`.
    If {
        cond: ExprId,
        then_block: Block,
        else_branch: Option<ExprId>,
    },
    /// A qualified name: `Enum::Variant`, `pkg::item`, `a::b::c::item`, or
    /// `.Variant` with a leading dot, which takes the expected enum.
    /// What it names is decided when it is resolved; the arguments, if any,
    /// belong to the `Call` around it.
    Path {
        leading_dot: bool,
        segments: Vec<Name>,
        type_args: Option<TypeArgs>,
    },
    /// `assert(condition)` and `assert(condition, "note")`: the condition
    /// holds, or the program says so and ends.
    Assert {
        cond: ExprId,
        /// The note the call gives, if any: any text, built when the
        /// assert fails.
        note: Option<ExprId>,
        /// What a failure prints, which the lexer makes from the
        /// condition's own text; `None` where the lexer could not read the
        /// call, which the parser reports.
        message: Option<Symbol>,
    },
    /// `yield place`, in a projection.
    /// `lend place`.
    Lend(ExprId),
    Array(Vec<ExprId>),
    /// `for x in xs { … }` where a value stands: among a list's elements,
    /// or after `own`, a loop whose `yield`s the list collects.
    /// Anywhere else it is refused.
    ForElement {
        binding: Pattern,
        source: ForSource,
        body: Block,
    },
    ArrayRepeat {
        elem: ExprId,
        count: RepeatCount,
    },
    Match {
        scrutinee: ExprId,
        arms: Vec<Arm>,
    },
    Unary {
        op: UnaryOp,
        op_span: Span,
        operand: ExprId,
    },
    Binary {
        op: BinaryOp,
        op_span: Span,
        lhs: ExprId,
        rhs: ExprId,
        /// Written `+%`, `-%` or `*%`: the answer wraps rather than the
        /// program panicking.
        wrapping: bool,
    },
    /// `value is pattern`: whether the value is the pattern's variant;
    /// `value !is pattern` where `negated`, which binds
    /// nothing.
    Is {
        scrutinee: ExprId,
        pattern: Pattern,
        negated: bool,
    },
    /// `expr?`: the value of `.Ok` or `.Some`, or an early return of the
    /// other variant.
    Try(ExprId),
    /// `(a, b: B): R => body`: a lambda. A parameter's type
    /// may be left out where the expected type gives it, and so may the
    /// result's.
    Lambda {
        params: Vec<LambdaParam>,
        ret: Option<TypeId>,
        body: ExprId,
    },
    /// `expr as T`.
    Cast {
        expr: ExprId,
        ty: TypeId,
    },
    /// `target = value`, or `target op= value` with `op`,
    /// which with `wrapping` is `+%=`, `-%=` or `*%=`.
    Assign {
        target: ExprId,
        op: Option<(BinaryOp, Span)>,
        wrapping: bool,
        value: ExprId,
    },
    Field {
        base: ExprId,
        name: Name,
    },
    /// A call. `names[i]` is the name `args[i]` was given, if any:
    /// `f(x, width: 2)`. `rest` is a last argument
    /// `..base`, which a call that builds a struct takes: the fields it
    /// does not name are `base`'s.
    Call {
        callee: ExprId,
        args: Vec<ExprId>,
        names: Vec<Option<Name>>,
        rest: Option<ExprId>,
    },
    Index {
        base: ExprId,
        index: ExprId,
    },
    /// `base[lo..hi]`, either bound optional, or
    /// `base[lo..=hi]`, which takes `hi` too.
    SubSlice {
        base: ExprId,
        lo: Option<ExprId>,
        hi: Option<ExprId>,
        inclusive: bool,
    },
    Error,
}

/// `pattern => body`. A block arm is a [`ExprKind::Block`] body.
#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pattern: Pattern,
    /// `if …` between the pattern and `=>`: the arm matches only where it
    /// is true.
    pub guard: Option<ExprId>,
    pub body: ExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    Wildcard,
    /// A bare name. Always a new binding, never a variant.
    Binding(Symbol),
    /// A number the value must equal: `0 => …`. `negative` is for the `-`
    /// before it.
    Int {
        negative: bool,
        magnitude: u128,
    },
    /// `true` or `false`.
    Bool(bool),
    /// Text the value must equal, by its bytes.
    Str(Symbol),
    /// A character the value must equal.
    Char(u32),
    /// A byte the value must equal: `b'/'`.
    Byte(u8),
    /// `.Round(..) | .Square(..)`: any one of them.
    Any(Vec<Pattern>),
    /// `'a'..='z'`, `0..10`, `..0`, `10..`: the values from `lo` to `hi`,
    /// `hi` included where the range is written `..=`; an end left off
    /// is the type's least or greatest value.
    Range {
        lo: Option<RangeBound>,
        hi: Option<RangeBound>,
        inclusive: bool,
    },
    Variant {
        /// Written with a leading `.`, so the enum is the one being matched.
        leading_dot: bool,
        /// The path, whose last segment is the variant's name.
        segments: Vec<Name>,
        binders: Option<Vec<Binder>>,
        /// The pattern ended with `..`, so fields it does not name are
        /// ignored.
        rest: bool,
    },
    /// `[first, ..rest]`, `[a, b]`, `[.., last]`: the elements of an array
    /// or a slice, from either end. Which `..` is the rest
    /// is the checker's to say, since `..MAX` is an element below a
    /// constant.
    Slice(Vec<SliceElement>),
    Error,
}

/// One element of a slice pattern, in the order written.
#[derive(Debug, Clone, PartialEq)]
pub enum SliceElement {
    Pattern(Pattern),
    /// `..`, with the name the elements between bind, where it has one.
    Rest {
        name: Option<Name>,
        span: Span,
    },
}

/// An end of a range pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeBound {
    pub kind: RangeBoundKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RangeBoundKind {
    /// A number, with the `-` before it where it has one.
    Int {
        negative: bool,
        magnitude: u128,
    },
    Char(u32),
    Byte(u8),
    /// A constant, by its name or its path: `MIN`, `limits::MAX`.
    Const(Vec<Name>),
}

/// One binder of a pattern: the field it takes, and what to do with it — bind
/// it under another name, or take it apart in turn. `None` binds it under the
/// field's own name.
#[derive(Debug, Clone, PartialEq)]
pub struct Binder {
    pub field: Name,
    pub pattern: Option<Pattern>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    Ref,
    /// `&var`.
    RefVar,
    Move,
    Own,
}

impl UnaryOp {
    pub fn text(self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
            UnaryOp::Ref => "&",
            UnaryOp::RefVar => "&var",
            UnaryOp::Move => "move",
            UnaryOp::Own => "own",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    /// `&`, `|`, `^`, `<<` and `>>` on integers.
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinaryOp {
    pub fn text(self) -> &'static str {
        match self {
            BinaryOp::Or => "||",
            BinaryOp::And => "&&",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "^",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
        }
    }
}

/// The length of `[x; n]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepeatCount {
    /// An integer literal, which is part of the array's type.
    Literal(u64),
    /// Any other expression: a length known only when the program runs,
    /// which only a buffer on the heap can have.
    Expr(ExprId),
}
