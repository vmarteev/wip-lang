//! What `@derive` writes: for each struct and enum of a
//! module that asks, an `extend Type: Interface` whose method is ordinary
//! code, built as syntax-tree nodes after the module is parsed and checked
//! as the program's own code is. A field then compares or clones by
//! whatever implementation its type has, generic or not.
//!
//! Every node is placed at the `@derive` argument that asked for it, in the
//! programmer's own source, and the block says so ([`ExtendBlock::derived`]):
//! the checker reports what keeps a derive from applying where the field is
//! written, and an editor passes over what the programmer did not write.
//!
//! All five are written here: `Eq` and `Ord`, `Hash`,
//! `Text` and `Clone`.

use crate::ast::{
    Annotation, AnnotationValue, Arm, Ast, BinaryOp, Binder, Block, Bound, Expr, ExprId, ExprKind,
    ExtendBlock, Field, FnDecl, FnSig, GenericParam, Item, Name, Param, Pattern, PatternKind,
    Receiver, Stmt, StmtId, StmtKind, Type, TypeId, TypeKind, UnaryOp, Variant,
};
use crate::{Interner, Span, Symbol};

/// What `@derive` writes for the types of one module, whose files are
/// `files`: one more syntax tree of the module, with an `extend` block for
/// each interface a type asks for. Empty where no type asks.
pub fn expand(files: &[&Ast], interner: &mut Interner) -> Ast {
    let names = Names::new(interner);
    let mut out = Ast::default();
    for &file in files {
        for item in &file.items {
            let (annotations, name, generics, shape) = match item {
                // An `extern union`'s fields lie over the same bytes, so
                // none of them is the value to compare or to clone.
                Item::Struct(decl) if !decl.is_union => (
                    &decl.annotations,
                    decl.name.sym,
                    &decl.generics,
                    Shape::Struct(&decl.fields),
                ),
                Item::Enum(decl) => (
                    &decl.annotations,
                    decl.name.sym,
                    &decl.generics,
                    Shape::Enum(&decl.variants),
                ),
                _ => continue,
            };
            for derive in [
                Derive::Eq,
                Derive::Ord,
                Derive::Hash,
                Derive::Text,
                Derive::Clone,
            ] {
                // An enum with no variants has no value to clone from.
                if derive == Derive::Clone && matches!(shape, Shape::Enum([])) {
                    continue;
                }
                let Some(at) = asks(annotations, &names, derive.interface(&names)) else {
                    continue;
                };
                let mut builder = Builder {
                    out: &mut out,
                    interner,
                    names: &names,
                    at,
                };
                let block = builder.block(derive, name, generics, shape);
                out.items.push(Item::Extend(block));
            }
        }
    }
    out
}

/// What a type is made of: a struct's fields, or an enum's variants.
#[derive(Clone, Copy)]
enum Shape<'a> {
    Struct(&'a [Field]),
    Enum(&'a [Variant]),
}

/// The interfaces written here.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Derive {
    Eq,
    Ord,
    Hash,
    Text,
    Clone,
}

impl Derive {
    fn interface(self, names: &Names) -> Symbol {
        match self {
            Derive::Eq => names.eq,
            Derive::Ord => names.ord,
            Derive::Hash => names.hash,
            Derive::Text => names.text,
            Derive::Clone => names.clone_interface,
        }
    }
}

/// The names the expansion writes: the prelude's, which no program can
/// declare again (E0345), `derive` to find what asks, and the names its
/// methods bind, which hold a `#` no program can write.
struct Names {
    derive: Symbol,
    eq: Symbol,
    equals: Symbol,
    ord: Symbol,
    compare: Symbol,
    ordering: Symbol,
    less: Symbol,
    same: Symbol,
    more: Symbol,
    hash: Symbol,
    hash_into: Symbol,
    hasher: Symbol,
    write_int: Symbol,
    out: Symbol,
    text: Symbol,
    append_to: Symbol,
    string: Symbol,
    push: Symbol,
    clone_interface: Symbol,
    clone: Symbol,
    clone_of: Symbol,
    bool_type: Symbol,
    other: Symbol,
    order: Symbol,
    /// `Self`, which names the type without looking its name up: a type
    /// whose name was refused is still the one that asked.
    self_type: Symbol,
}

impl Names {
    fn new(interner: &mut Interner) -> Names {
        Names {
            derive: interner.intern("derive"),
            eq: interner.intern("Eq"),
            equals: Symbol::equals(),
            ord: interner.intern("Ord"),
            compare: Symbol::compare(),
            ordering: interner.intern("Ordering"),
            less: interner.intern("Less"),
            same: interner.intern("Same"),
            more: interner.intern("More"),
            hash: interner.intern("Hash"),
            hash_into: Symbol::hash_into(),
            hasher: interner.intern("Hasher"),
            write_int: interner.intern("writeInt"),
            out: interner.intern("out"),
            text: interner.intern("Text"),
            append_to: Symbol::append_to(),
            string: Symbol::string_type(),
            push: Symbol::push(),
            clone_interface: interner.intern("Clone"),
            clone: interner.intern("clone"),
            clone_of: interner.intern("cloneOf"),
            bool_type: interner.intern("bool"),
            other: interner.intern("other"),
            order: interner.intern("order#"),
            self_type: interner.self_type_symbol(),
        }
    }
}

/// Where `@derive` names `interface`, if it does: the argument's span.
fn asks(annotations: &[Annotation], names: &Names, interface: Symbol) -> Option<Span> {
    annotations
        .iter()
        .filter(|a| a.name.sym == names.derive)
        .flat_map(|a| a.args.iter())
        .find(|arg| arg.value == AnnotationValue::Name(interface))
        .map(|arg| arg.span)
}

/// A piece of what `Text` writes: text as it is, or a value that writes
/// itself.
enum Written {
    Text(String),
    Value(ExprId),
}

/// Which side of a comparison a variant's fields are bound for: `self`'s,
/// or the other value's.
#[derive(Clone, Copy)]
enum Side {
    This,
    Other,
}

/// Builds the nodes of one derived block into `out`, each placed at `at`.
struct Builder<'a> {
    out: &'a mut Ast,
    interner: &'a mut Interner,
    names: &'a Names,
    at: Span,
}

impl Builder<'_> {
    /// `extend Type<T: Interface>: Interface { fn method(…): … = body }`:
    /// the type's parameters are bound by the interface, so each field of a
    /// parameter's type has it, and keep what the type binds them by, as
    /// in any `extend` of it.
    fn block(
        &mut self,
        derive: Derive,
        ty: Symbol,
        generics: &[GenericParam],
        shape: Shape,
    ) -> ExtendBlock {
        let names = self.names;
        let (method, params, ret, body) = match derive {
            // `fn equals(other: &Self): bool`
            Derive::Eq => {
                let body = match shape {
                    Shape::Struct(fields) => {
                        let pairs = self.field_pairs(fields);
                        self.all_equal(pairs)
                    }
                    Shape::Enum(variants) => self.eq_variants(variants),
                };
                let ret = self.named_type(names.bool_type);
                (names.equals, vec![self.other_param()], Some(ret), body)
            }
            // `fn compare(other: &Self): Ordering`
            Derive::Ord => {
                let body = match shape {
                    Shape::Struct(fields) => {
                        let pairs = self.field_pairs(fields);
                        self.first_difference(pairs)
                    }
                    Shape::Enum(variants) => self.ord_variants(variants),
                };
                let ret = self.named_type(names.ordering);
                (names.compare, vec![self.other_param()], Some(ret), body)
            }
            // `fn hashInto(out: &var Hasher)`
            Derive::Hash => {
                let body = match shape {
                    Shape::Struct(fields) => {
                        let parts = fields
                            .iter()
                            .map(|field| {
                                let this = self.expr(ExprKind::SelfRef);
                                self.field(this, field.name.sym)
                            })
                            .collect();
                        self.hash_parts(None, parts)
                    }
                    Shape::Enum(variants) => self.hash_variants(variants),
                };
                let hasher = self.names.hasher;
                (names.hash_into, vec![self.out_param(hasher)], None, body)
            }
            // `fn appendTo(out: &var String)`
            Derive::Text => {
                let body = match shape {
                    Shape::Struct(fields) => self.text_struct(ty, fields),
                    Shape::Enum(variants) => self.text_variants(variants),
                };
                let string = self.names.string;
                (names.append_to, vec![self.out_param(string)], None, body)
            }
            // `fn clone(): Self`
            Derive::Clone => {
                let body = match shape {
                    Shape::Struct(fields) => self.clone_struct(fields),
                    Shape::Enum(variants) => self.clone_variants(variants),
                };
                let ret = self.named_type(names.self_type);
                (names.clone, Vec::new(), Some(ret), body)
            }
        };
        let interface = derive.interface(names);
        let generics: Vec<GenericParam> = generics
            .iter()
            .map(|param| GenericParam {
                decided: None,
                name: self.name(param.name.sym),
                bounds: vec![Bound {
                    name: self.name(interface),
                    args: Vec::new(),
                    span: self.at,
                }],
                default: None,
                span: self.at,
            })
            .collect();
        let method = FnDecl {
            annotations: Vec::new(),
            sig: FnSig {
                name: self.name(method),
                generics: Vec::new(),
                params,
                variadic: None,
                ret,
                lends_from: Vec::new(),
                span: self.at,
            },
            body: Some(body),
            is_pub: false,
            pub_span: None,
            receiver: Some((Receiver::Read, self.at)),
            span: self.at,
        };
        ExtendBlock {
            annotations: Vec::new(),
            slice_of: None,
            interface: Some(self.name(interface)),
            interface_args: None,
            path: vec![self.name(ty)],
            generics,
            methods: vec![method],
            derived: Some(self.at),
            span: self.at,
        }
    }

    // ---- Eq ----

    /// `a == c && b == d`, and `true` where there is nothing to compare.
    fn all_equal(&mut self, pairs: Vec<(ExprId, ExprId)>) -> ExprId {
        let mut all: Option<ExprId> = None;
        for (this, other) in pairs {
            let equal = self.binary(BinaryOp::Eq, this, other);
            all = Some(match all {
                Some(before) => self.binary(BinaryOp::And, before, equal),
                None => equal,
            });
        }
        all.unwrap_or_else(|| self.expr(ExprKind::Bool(true)))
    }

    /// `match self { .Circle(r: self#r) => match other { .Circle(r: other#r) => self#r == other#r  _ => false } … }`.
    fn eq_variants(&mut self, variants: &[Variant]) -> ExprId {
        let arms = (0..variants.len())
            .map(|i| {
                let variant = &variants[i];
                let (inner, pairs) = self.bound_pairs(variant);
                let same = self.all_equal(pairs);
                let mut arms = vec![self.arm(inner, same)];
                if variants.len() > 1 {
                    let differ = self.expr(ExprKind::Bool(false));
                    arms.push(self.arm(self.wildcard(), differ));
                }
                let other = self.other();
                let body = self.expr(ExprKind::Match {
                    scrutinee: other,
                    arms,
                });
                let outer = self.bound(variant, Side::This);
                self.arm(outer, body)
            })
            .collect();
        let this = self.expr(ExprKind::SelfRef);
        self.expr(ExprKind::Match {
            scrutinee: this,
            arms,
        })
    }

    // ---- Ord ----

    /// The first of `a.compare(&c)`, `b.compare(&d)`, … that is not
    /// `.Same`, and `.Same` where every one is:
    /// `match a.compare(&c) { .Same => b.compare(&d)  order# => order# }`.
    fn first_difference(&mut self, pairs: Vec<(ExprId, ExprId)>) -> ExprId {
        let mut pairs = pairs.into_iter().rev();
        let Some((this, other)) = pairs.next() else {
            return self.variant(self.names.same);
        };
        let mut rest = self.compare_call(this, other);
        for (this, other) in pairs {
            let order = self.compare_call(this, other);
            let same = self.variant_pattern(self.names.same, None);
            let differs = Pattern {
                kind: PatternKind::Binding(self.names.order),
                span: self.at,
            };
            let answer = self.expr(ExprKind::Name(self.names.order));
            let arms = vec![self.arm(same, rest), self.arm(differs, answer)];
            rest = self.expr(ExprKind::Match {
                scrutinee: order,
                arms,
            });
        }
        rest
    }

    /// `match self { .Dot => match other { .Dot => .Same  .Circle(..) => .More  _ => .Less } … }`:
    /// two values of one variant by their fields, and of two by the order
    /// the variants are written in.
    fn ord_variants(&mut self, variants: &[Variant]) -> ExprId {
        let arms = (0..variants.len())
            .map(|i| {
                let variant = &variants[i];
                let (inner, pairs) = self.bound_pairs(variant);
                let order = self.first_difference(pairs);
                let mut arms = vec![self.arm(inner, order)];
                // A variant written before this one: `self` comes after it.
                if i > 0 {
                    let before = variants[..i]
                        .iter()
                        .map(|v| self.any_of(v))
                        .collect::<Vec<_>>();
                    let before = match before.len() {
                        1 => before.into_iter().next().expect("one pattern"),
                        _ => Pattern {
                            kind: PatternKind::Any(before),
                            span: self.at,
                        },
                    };
                    let more = self.variant(self.names.more);
                    arms.push(self.arm(before, more));
                }
                // One written after it: `self` comes first.
                if i + 1 < variants.len() {
                    let less = self.variant(self.names.less);
                    arms.push(self.arm(self.wildcard(), less));
                }
                let other = self.other();
                let body = self.expr(ExprKind::Match {
                    scrutinee: other,
                    arms,
                });
                let outer = self.bound(variant, Side::This);
                self.arm(outer, body)
            })
            .collect();
        let this = self.expr(ExprKind::SelfRef);
        self.expr(ExprKind::Match {
            scrutinee: this,
            arms,
        })
    }

    /// `this.compare(&other)`.
    fn compare_call(&mut self, this: ExprId, other: ExprId) -> ExprId {
        let callee = self.field(this, self.names.compare);
        let lent = self.expr(ExprKind::Unary {
            op: UnaryOp::Ref,
            op_span: self.at,
            operand: other,
        });
        self.expr(ExprKind::Call {
            callee,
            args: vec![lent],
            names: vec![None],
            rest: None,
        })
    }

    // ---- Hash ----

    /// `{ out.writeInt(1)  self#x.hashInto(&var out)  … }`: a variant's
    /// number, where there is one, and then each part in turn.
    fn hash_parts(&mut self, number: Option<usize>, parts: Vec<ExprId>) -> ExprId {
        let mut stmts = Vec::new();
        if let Some(number) = number {
            let out = self.expr(ExprKind::Name(self.names.out));
            let callee = self.field(out, self.names.write_int);
            let number = self.expr(ExprKind::Int(number as u128));
            let written = self.expr(ExprKind::Call {
                callee,
                args: vec![number],
                names: vec![None],
                rest: None,
            });
            stmts.push(self.stmt(written));
        }
        for part in parts {
            let callee = self.field(part, self.names.hash_into);
            let lent = self.lent_out();
            let hashed = self.expr(ExprKind::Call {
                callee,
                args: vec![lent],
                names: vec![None],
                rest: None,
            });
            stmts.push(self.stmt(hashed));
        }
        self.expr(ExprKind::Block(Block {
            stmts,
            span: self.at,
        }))
    }

    /// `match self { .Circle(r: self#r) => { out.writeInt(0)  self#r.hashInto(&var out) } … }`:
    /// which variant it is, as the number it is written as, then what it
    /// holds.
    fn hash_variants(&mut self, variants: &[Variant]) -> ExprId {
        let arms = variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let parts = variant
                    .fields
                    .iter()
                    .map(|field| {
                        let bound = self.binding(field.name.sym, Side::This);
                        self.expr(ExprKind::Name(bound))
                    })
                    .collect();
                let body = self.hash_parts(Some(i), parts);
                let pattern = self.bound(variant, Side::This);
                self.arm(pattern, body)
            })
            .collect();
        let this = self.expr(ExprKind::SelfRef);
        self.expr(ExprKind::Match {
            scrutinee: this,
            arms,
        })
    }

    // ---- Text ----

    /// `{ out.push("Point(x: ")  self.x.appendTo(&var out)  out.push(")") }`:
    /// the call that would make the value.
    fn text_struct(&mut self, ty: Symbol, fields: &[Field]) -> ExprId {
        let name = self.interner.resolve(ty).to_string();
        if fields.is_empty() {
            return self.written(vec![Written::Text(format!("{name}()"))]);
        }
        let mut pieces = vec![Written::Text(format!("{name}("))];
        for (i, field) in fields.iter().enumerate() {
            let field_name = self.interner.resolve(field.name.sym).to_string();
            let before = if i == 0 { "" } else { ", " };
            pieces.push(Written::Text(format!("{before}{field_name}: ")));
            let this = self.expr(ExprKind::SelfRef);
            pieces.push(Written::Value(self.field(this, field.name.sym)));
        }
        pieces.push(Written::Text(")".to_string()));
        self.written(pieces)
    }

    /// `match self { .Circle(r: self#r) => { out.push(".Circle(r: ")  self#r.appendTo(&var out)  out.push(")") } … }`:
    /// the variant as a program writes it.
    fn text_variants(&mut self, variants: &[Variant]) -> ExprId {
        let arms = variants
            .iter()
            .map(|variant| {
                let name = self.interner.resolve(variant.name.sym).to_string();
                let mut pieces = Vec::new();
                if variant.fields.is_empty() {
                    pieces.push(Written::Text(format!(".{name}")));
                } else {
                    pieces.push(Written::Text(format!(".{name}(")));
                    for (i, field) in variant.fields.iter().enumerate() {
                        let field_name = self.interner.resolve(field.name.sym).to_string();
                        let before = if i == 0 { "" } else { ", " };
                        pieces.push(Written::Text(format!("{before}{field_name}: ")));
                        let bound = self.binding(field.name.sym, Side::This);
                        pieces.push(Written::Value(self.expr(ExprKind::Name(bound))));
                    }
                    pieces.push(Written::Text(")".to_string()));
                }
                let body = self.written(pieces);
                let pattern = self.bound(variant, Side::This);
                self.arm(pattern, body)
            })
            .collect();
        let this = self.expr(ExprKind::SelfRef);
        self.expr(ExprKind::Match {
            scrutinee: this,
            arms,
        })
    }

    /// `{ out.push("…")  value.appendTo(&var out)  … }`.
    fn written(&mut self, pieces: Vec<Written>) -> ExprId {
        let stmts = pieces
            .into_iter()
            .map(|piece| {
                let call = match piece {
                    Written::Text(text) => {
                        let out = self.expr(ExprKind::Name(self.names.out));
                        let callee = self.field(out, self.names.push);
                        let text = self.interner.intern(&text);
                        let text = self.expr(ExprKind::Str(text));
                        self.expr(ExprKind::Call {
                            callee,
                            args: vec![text],
                            names: vec![None],
                            rest: None,
                        })
                    }
                    Written::Value(value) => {
                        let callee = self.field(value, self.names.append_to);
                        let lent = self.lent_out();
                        self.expr(ExprKind::Call {
                            callee,
                            args: vec![lent],
                            names: vec![None],
                            rest: None,
                        })
                    }
                };
                self.stmt(call)
            })
            .collect();
        self.expr(ExprKind::Block(Block {
            stmts,
            span: self.at,
        }))
    }

    // ---- Clone ----

    /// `Self(x: self.x.clone(), …)`.
    fn clone_struct(&mut self, fields: &[Field]) -> ExprId {
        let (names, args) = fields
            .iter()
            .map(|field| {
                let this = self.expr(ExprKind::SelfRef);
                let place = self.field(this, field.name.sym);
                let value = self.cloned(place);
                (Some(self.name(field.name.sym)), value)
            })
            .unzip();
        let callee = self.expr(ExprKind::Name(self.names.self_type));
        self.expr(ExprKind::Call {
            callee,
            args,
            names,
            rest: None,
        })
    }

    /// `match self { .Circle(r: self#r) => .Circle(r: self#r.clone()) … }`.
    fn clone_variants(&mut self, variants: &[Variant]) -> ExprId {
        let arms = variants
            .iter()
            .map(|variant| {
                let path = self.expr(ExprKind::Path {
                    leading_dot: true,
                    segments: vec![self.name(variant.name.sym)],
                    type_args: None,
                });
                let body = if variant.fields.is_empty() {
                    path
                } else {
                    let args = variant
                        .fields
                        .iter()
                        .map(|field| {
                            let bound = self.binding(field.name.sym, Side::This);
                            let bound = self.expr(ExprKind::Name(bound));
                            self.cloned(bound)
                        })
                        .collect();
                    let names = variant
                        .fields
                        .iter()
                        .map(|field| Some(self.name(field.name.sym)))
                        .collect();
                    self.expr(ExprKind::Call {
                        callee: path,
                        args,
                        names,
                        rest: None,
                    })
                };
                let pattern = self.bound(variant, Side::This);
                self.arm(pattern, body)
            })
            .collect();
        let this = self.expr(ExprKind::SelfRef);
        self.expr(ExprKind::Match {
            scrutinee: this,
            arms,
        })
    }

    /// A field's value in the clone: `cloneOf(&value)`, the prelude's
    /// clone of a value of the field's own type. A method call,
    /// `value.clone()`, would look through a field that is a `&T` to what
    /// it refers to; this clones the `&T`, which is the reference copied.
    fn cloned(&mut self, value: ExprId) -> ExprId {
        let callee = self.expr(ExprKind::Name(self.names.clone_of));
        let lent = self.expr(ExprKind::Unary {
            op: UnaryOp::Ref,
            op_span: self.at,
            operand: value,
        });
        self.expr(ExprKind::Call {
            callee,
            args: vec![lent],
            names: vec![None],
            rest: None,
        })
    }

    // ---- what they share ----

    /// `other: &Self`.
    fn other_param(&mut self) -> Param {
        let inner = self.named_type(self.names.self_type);
        let ty = self.out.types.alloc(Type {
            kind: TypeKind::Ref { var: false, inner },
            span: self.at,
        });
        Field {
            is_pub: false,
            is_var: false,
            name: self.name(self.names.other),
            ty,
            default: None,
            span: self.at,
        }
    }

    /// `out: &var Hasher`, or `out: &var String`: what is written into.
    fn out_param(&mut self, into: Symbol) -> Param {
        let inner = self.named_type(into);
        let ty = self.out.types.alloc(Type {
            kind: TypeKind::Ref { var: true, inner },
            span: self.at,
        });
        Field {
            is_pub: false,
            is_var: false,
            name: self.name(self.names.out),
            ty,
            default: None,
            span: self.at,
        }
    }

    /// `&var out`.
    fn lent_out(&mut self) -> ExprId {
        let out = self.expr(ExprKind::Name(self.names.out));
        self.expr(ExprKind::Unary {
            op: UnaryOp::RefVar,
            op_span: self.at,
            operand: out,
        })
    }

    fn stmt(&mut self, expr: ExprId) -> StmtId {
        self.out.stmts.alloc(Stmt {
            kind: StmtKind::Expr(expr),
            span: self.at,
        })
    }

    fn other(&mut self) -> ExprId {
        self.expr(ExprKind::Name(self.names.other))
    }

    /// `(self.x, other.x)` for each field.
    fn field_pairs(&mut self, fields: &[Field]) -> Vec<(ExprId, ExprId)> {
        fields
            .iter()
            .map(|field| {
                let this = self.expr(ExprKind::SelfRef);
                let this = self.field(this, field.name.sym);
                let other = self.other();
                let other = self.field(other, field.name.sym);
                (this, other)
            })
            .collect()
    }

    /// The variant as `other` is matched against it, its fields bound, and
    /// each field of `self`'s beside the same field of `other`'s.
    fn bound_pairs(&mut self, variant: &Variant) -> (Pattern, Vec<(ExprId, ExprId)>) {
        let pattern = self.bound(variant, Side::Other);
        let pairs = variant
            .fields
            .iter()
            .map(|field| {
                let this = self.binding(field.name.sym, Side::This);
                let other = self.binding(field.name.sym, Side::Other);
                (
                    self.expr(ExprKind::Name(this)),
                    self.expr(ExprKind::Name(other)),
                )
            })
            .collect();
        (pattern, pairs)
    }

    /// `self#field` or `other#field`: a name no program can write, so it
    /// shadows nothing.
    fn binding(&mut self, field: Symbol, side: Side) -> Symbol {
        let prefix = match side {
            Side::This => "self",
            Side::Other => "other",
        };
        let text = format!("{prefix}#{}", self.interner.resolve(field));
        self.interner.intern(&text)
    }

    /// `.Circle(r: self#r)`, or `.Dot` for a variant that carries nothing.
    fn bound(&mut self, variant: &Variant, side: Side) -> Pattern {
        let binders = (!variant.fields.is_empty()).then(|| {
            variant
                .fields
                .iter()
                .map(|field| {
                    let bound = self.binding(field.name.sym, side);
                    Binder {
                        field: self.name(field.name.sym),
                        pattern: Some(Pattern {
                            kind: PatternKind::Binding(bound),
                            span: self.at,
                        }),
                        span: self.at,
                    }
                })
                .collect()
        });
        self.variant_pattern(variant.name.sym, binders)
    }

    /// `.Circle(..)`, or `.Dot`: the variant, whatever it holds.
    fn any_of(&mut self, variant: &Variant) -> Pattern {
        let rest = !variant.fields.is_empty();
        Pattern {
            kind: PatternKind::Variant {
                leading_dot: true,
                segments: vec![self.name(variant.name.sym)],
                binders: rest.then(Vec::new),
                rest,
            },
            span: self.at,
        }
    }

    fn variant_pattern(&mut self, name: Symbol, binders: Option<Vec<Binder>>) -> Pattern {
        Pattern {
            kind: PatternKind::Variant {
                leading_dot: true,
                segments: vec![self.name(name)],
                binders,
                rest: false,
            },
            span: self.at,
        }
    }

    fn wildcard(&self) -> Pattern {
        Pattern {
            kind: PatternKind::Wildcard,
            span: self.at,
        }
    }

    fn arm(&self, pattern: Pattern, body: ExprId) -> Arm {
        Arm {
            pattern,
            guard: None,
            body,
            span: self.at,
        }
    }

    /// `.Same`, of the type expected where it stands.
    fn variant(&mut self, name: Symbol) -> ExprId {
        self.expr(ExprKind::Path {
            leading_dot: true,
            segments: vec![self.name(name)],
            type_args: None,
        })
    }

    fn binary(&mut self, op: BinaryOp, lhs: ExprId, rhs: ExprId) -> ExprId {
        self.expr(ExprKind::Binary {
            op,
            op_span: self.at,
            lhs,
            rhs,
            wrapping: false,
        })
    }

    fn named_type(&mut self, name: Symbol) -> TypeId {
        self.out.types.alloc(Type {
            kind: TypeKind::Named {
                name,
                args: Vec::new(),
            },
            span: self.at,
        })
    }

    fn name(&self, sym: Symbol) -> Name {
        Name { sym, span: self.at }
    }

    fn expr(&mut self, kind: ExprKind) -> ExprId {
        self.out.exprs.alloc(Expr {
            kind,
            span: self.at,
        })
    }

    /// `base.name`.
    fn field(&mut self, base: ExprId, name: Symbol) -> ExprId {
        let name = self.name(name);
        self.expr(ExprKind::Field { base, name })
    }
}
