//! Sizes, alignments and field offsets, computed once per program and shared by
//! every backend. Struct layouts are C-like.

use rustc_hash::FxHashMap;
use wip_hir::{FloatTy, Program, Ty, TyKind};

/// The size and alignment of a type, in bytes.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub size: u32,
    pub align: u32,
}

struct StructLayout {
    layout: Layout,
    offsets: Vec<u32>,
}

/// Struct layouts are C-like: fields in declaration order, each at the next
/// offset that satisfies its alignment. Each instance of a generic struct or
/// enum has a layout of its own, so layouts are kept by type,
/// and computed when first asked for.
#[derive(Default)]
pub struct Layouts {
    structs: FxHashMap<Ty, StructLayout>,
    enums: FxHashMap<Ty, EnumLayout>,
}

/// Where an enum records which variant it holds.
#[derive(Clone, Copy, Debug)]
pub enum Tag {
    /// The variant's index, in the first `size` bytes.
    Byte { size: u32 },
    /// No tag: the variant `payload` holds an `own` at `offset`, and a null
    /// pointer there means the variant `empty` instead. Nothing else in
    /// `payload` owns memory, since taking the `own` out leaves a null.
    Niche {
        payload: u32,
        empty: u32,
        offset: u32,
    },
}

/// An enum's layout: its tag, and the offsets of each variant's fields from
/// the start of the value.
struct EnumLayout {
    layout: Layout,
    tag: Tag,
    offsets: Vec<Vec<u32>>,
}

impl Layout {
    /// Distance between consecutive array elements.
    pub fn stride(self) -> u32 {
        self.size.next_multiple_of(self.align)
    }
}

impl Layouts {
    pub fn new() -> Layouts {
        Layouts::default()
    }

    pub fn struct_layout(&mut self, program: &Program, ty: Ty) -> Layout {
        if let Some(s) = self.structs.get(&ty) {
            return s.layout;
        }
        // A union's fields are all at 0, over the same bytes, and it is as
        // large as the largest of them.
        let union =
            matches!(program.types.kind(ty), TyKind::Struct(id, _) if program.structs[id].is_union);
        let (mut size, mut align) = (0u32, 1u32);
        let mut offsets = Vec::new();
        for field in program.field_tys(ty) {
            let field = self.of(program, field);
            if union {
                offsets.push(0);
                size = size.max(field.size);
            } else {
                size = size.next_multiple_of(field.align);
                offsets.push(size);
                size += field.size;
            }
            align = align.max(field.align);
        }
        let layout = Layout {
            size: size.next_multiple_of(align),
            align,
        };
        self.structs.insert(ty, StructLayout { layout, offsets });
        layout
    }

    pub fn of(&mut self, program: &Program, ty: Ty) -> Layout {
        match program.types.kind(ty) {
            TyKind::Unit | TyKind::Never | TyKind::Error => Layout { size: 0, align: 1 },
            // Integers are aligned to their size: `i128` to 16 bytes, as C
            // compilers align `__int128`.
            TyKind::Int(t) => {
                let size = t.bits() / 8;
                Layout { size, align: size }
            }
            TyKind::Float(FloatTy::F32) => Layout { size: 4, align: 4 },
            // A slice is known by a pointer and a length, and so are a
            // reference to one and a buffer. A `&dyn` is a pointer and a table
            // of methods.
            TyKind::Slice(_) | TyKind::Dyn(..) => Layout { size: 16, align: 8 },
            // A closure lent for one call is what it captured and its code.
            TyKind::Ref(inner, _) | TyKind::Own(inner)
                if matches!(
                    program.types.kind(inner),
                    TyKind::Slice(_) | TyKind::Dyn(..) | TyKind::Fn(..)
                ) =>
            {
                Layout { size: 16, align: 8 }
            }
            TyKind::Float(FloatTy::F64)
            | TyKind::Cstring
            | TyKind::Fn(..)
            | TyKind::Own(_)
            // A C pointer is one word, like every other pointer.
            | TyKind::Ptr(_)
            | TyKind::Ref(..) => Layout { size: 8, align: 8 },
            // No value of an opaque C type exists; only `ptr<T>` of one.
            TyKind::Opaque(_) => Layout { size: 0, align: 1 },
            // A block of slots is a pointer and how many there are.
            TyKind::Slots(_) => Layout { size: 16, align: 8 },
            // A pointer and a length.
            TyKind::Str => Layout { size: 16, align: 8 },
            TyKind::Bool => Layout { size: 1, align: 1 },
            // A Unicode scalar value, in 32 bits.
            TyKind::Char => Layout { size: 4, align: 4 },
            TyKind::Struct(..) => self.struct_layout(program, ty),
            TyKind::Array(elem, len) => {
                let elem = self.of(program, elem);
                let size = u64::from(elem.stride()) * len;
                let size = u32::try_from(size).expect("arrays larger than 4 GiB are not supported");
                Layout {
                    size,
                    align: elem.align,
                }
            }
            TyKind::Enum(..) => self.enum_layout(program, ty),
            TyKind::Param(_) => unreachable!("only instances of generic items are laid out"),
        }
    }

    pub fn field_offset(&mut self, program: &Program, ty: Ty, index: u32) -> u32 {
        self.struct_layout(program, ty);
        self.structs[&ty].offsets[index as usize]
    }

    /// An enum's layout: a tag and then each variant's fields
    /// over the same bytes, or no tag at all when a null `own`, or a null
    /// function, can stand for the only variant without
    /// fields.
    pub fn enum_layout(&mut self, program: &Program, ty: Ty) -> Layout {
        if let Some(e) = self.enums.get(&ty) {
            return e.layout;
        }
        let variants = program.variant_field_tys(ty);
        let fields: Vec<Vec<Layout>> = variants
            .iter()
            .map(|v| v.iter().map(|&f| self.of(program, f)).collect())
            .collect();
        let niche = if variants.len() == 2 {
            variants
                .iter()
                .position(|v| v.is_empty())
                .and_then(|empty| {
                    let payload = 1 - empty;
                    let fields = &variants[payload];
                    let field = fields.iter().position(|&f| {
                        matches!(program.types.kind(f), TyKind::Own(_) | TyKind::Fn(..))
                    })?;
                    // Taking the `own` out leaves a null, which reads as the
                    // empty variant, so nothing else in the variant may still
                    // need dropping then.
                    let rest_owns = fields
                        .iter()
                        .enumerate()
                        .any(|(i, &f)| i != field && program.owns_memory(f));
                    (!rest_owns).then_some((payload, empty, field))
                })
        } else {
            None
        };
        let enum_layout = match niche {
            Some((payload, empty, field)) => {
                let (offsets, end) = sequential(&fields[payload], 0);
                let align = fields[payload].iter().map(|f| f.align).max().unwrap_or(1);
                let offset = offsets[field];
                let mut all = vec![Vec::new(); 2];
                all[payload] = offsets;
                EnumLayout {
                    layout: Layout {
                        size: end.next_multiple_of(align),
                        align,
                    },
                    tag: Tag::Niche {
                        payload: payload as u32,
                        empty: empty as u32,
                        offset,
                    },
                    offsets: all,
                }
            }
            None => {
                let tag_size: u32 = if variants.len() <= 256 { 1 } else { 4 };
                let payload_align = fields.iter().flatten().map(|f| f.align).max().unwrap_or(1);
                let start = tag_size.next_multiple_of(payload_align);
                let mut size = tag_size;
                let mut offsets = Vec::new();
                for variant in &fields {
                    let (variant_offsets, end) = sequential(variant, start);
                    size = size.max(end);
                    offsets.push(variant_offsets);
                }
                let align = tag_size.max(payload_align);
                EnumLayout {
                    layout: Layout {
                        size: size.next_multiple_of(align),
                        align,
                    },
                    tag: Tag::Byte { size: tag_size },
                    offsets,
                }
            }
        };
        let layout = enum_layout.layout;
        self.enums.insert(ty, enum_layout);
        layout
    }

    pub fn enum_tag(&mut self, program: &Program, ty: Ty) -> Tag {
        self.enum_layout(program, ty);
        self.enums[&ty].tag
    }

    pub fn variant_offset(&mut self, program: &Program, ty: Ty, variant: u32, field: u32) -> u32 {
        self.enum_layout(program, ty);
        self.enums[&ty].offsets[variant as usize][field as usize]
    }
}

/// Offsets for fields laid out one after another from `start`, each at the
/// next multiple of its alignment, and where the last one ends.
fn sequential(fields: &[Layout], start: u32) -> (Vec<u32>, u32) {
    let mut end = start;
    let offsets = fields
        .iter()
        .map(|f| {
            let offset = end.next_multiple_of(f.align);
            end = offset + f.size;
            offset
        })
        .collect();
    (offsets, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wip_syntax::Interner;

    /// The program `src` declares, and the interner that reads its names.
    fn lowered(src: &str) -> (Program, Interner) {
        let mut interner = Interner::new();
        let lexed = wip_syntax::lex(src, &mut interner);
        let parsed = wip_syntax::parse(src, &lexed);
        let lowered = wip_hir::lower_file(&parsed.ast, &interner);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        (lowered.program, interner)
    }

    /// The type of the struct or enum `src` declares as `name`.
    fn named(program: &Program, interner: &Interner, name: &str) -> Ty {
        let kind = match program
            .structs
            .iter()
            .find(|(_, s)| interner.resolve(s.name) == name)
        {
            Some((id, _)) => TyKind::Struct(id, wip_hir::TyList::EMPTY),
            None => {
                let (id, _) = program
                    .enums
                    .iter()
                    .find(|(_, e)| interner.resolve(e.name) == name)
                    .expect("declared");
                TyKind::Enum(id, wip_hir::TyList::EMPTY)
            }
        };
        program.types.find(kind).expect("its type is interned")
    }

    /// The size of each enum in `src`, by name.
    fn enum_sizes(src: &str) -> Vec<(String, u32)> {
        let (program, interner) = lowered(src);
        let mut layouts = Layouts::new();
        program
            .enums
            .iter()
            .map(|(id, e)| {
                let ty = program
                    .types
                    .find(TyKind::Enum(id, wip_hir::TyList::EMPTY))
                    .expect("each enum's type is interned");
                let size = layouts.enum_layout(&program, ty).size;
                (interner.resolve(e.name).to_string(), size)
            })
            .collect()
    }

    /// An optional `own` costs one pointer.
    #[test]
    fn a_null_own_stands_for_the_empty_variant() {
        let sizes = enum_sizes(
            "struct Node { value: i64 }
             enum Link { Some(node: own<Node>), None }
             enum Cell { Full(value: i64, next: own<Node>), Empty }
             enum Light { Red, Yellow, Green }
             enum Shape { Circle(r: f64), Rect(w: f64, h: f64), Dot }
             enum Pair { Both(left: own<Node>, right: own<Node>), Neither }",
        );
        // `Pair` keeps its tag: taking `left` out must not make the value
        // read as `Neither` while `right` still needs dropping.
        let expected = [
            ("Link", 8),
            ("Cell", 16),
            ("Light", 1),
            ("Shape", 24),
            ("Pair", 24),
        ];
        let expected: Vec<(String, u32)> = expected
            .iter()
            .map(|&(name, size)| (name.to_string(), size))
            .collect();
        assert_eq!(sizes, expected);
    }

    /// An instance of a generic enum has a layout of its own: with an `own`
    /// payload, a null pointer stands for the empty variant, as for an enum
    /// written out by hand.
    #[test]
    fn instances_have_layouts_of_their_own() {
        let (program, interner) = lowered(
            "struct Node { value: i64 }
             enum Maybe<T> { Some(value: T), None }
             struct Holder { owned: Maybe<own<Node>>, plain: Maybe<i64> }",
        );
        let holder = named(&program, &interner, "Holder");
        let mut layouts = Layouts::new();
        let [owned, plain] = program.field_tys(holder)[..] else {
            panic!("two fields")
        };
        assert_eq!(layouts.of(&program, owned).size, 8);
        assert_eq!(layouts.of(&program, plain).size, 16);
        assert_eq!(layouts.of(&program, holder).size, 24);
    }

    /// A struct is laid out as C lays it out: each field at the next
    /// offset its alignment allows, and the whole rounded up to the
    /// largest alignment, so an array of them keeps each aligned.
    #[test]
    fn a_struct_is_laid_out_as_c_lays_it_out() {
        let (program, interner) = lowered("struct Mixed { small: u8, big: i64, middle: u16 }");
        let mixed = named(&program, &interner, "Mixed");
        let mut layouts = Layouts::new();
        let offsets: Vec<u32> = (0..3)
            .map(|i| layouts.field_offset(&program, mixed, i))
            .collect();
        assert_eq!(offsets, vec![0, 8, 16]);
        let layout = layouts.of(&program, mixed);
        assert_eq!((layout.size, layout.align), (24, 8));
        assert_eq!(layout.stride(), 24);
    }

    /// A union's fields all start at 0, over the same bytes, and it is as
    /// large as the largest of them.
    #[test]
    fn a_union_is_as_large_as_its_largest_field() {
        let (program, interner) = lowered(
            "extern \"C\" {\n    union Either {\n        small: u8\n        big: i64\n    }\n}\n",
        );
        let either = named(&program, &interner, "Either");
        let mut layouts = Layouts::new();
        assert_eq!(layouts.field_offset(&program, either, 0), 0);
        assert_eq!(layouts.field_offset(&program, either, 1), 0);
        let layout = layouts.of(&program, either);
        assert_eq!((layout.size, layout.align), (8, 8));
    }

    /// A variant's fields start after the tag, where the largest alignment
    /// of any variant's fields allows, so every variant's are aligned.
    #[test]
    fn a_variants_fields_start_after_the_tag() {
        let (program, interner) = lowered("enum Shape { Dot, Small(b: u8), Circle(r: f64) }");
        let shape = named(&program, &interner, "Shape");
        let mut layouts = Layouts::new();
        assert!(matches!(
            layouts.enum_tag(&program, shape),
            Tag::Byte { size: 1 }
        ));
        assert_eq!(layouts.variant_offset(&program, shape, 1, 0), 8);
        assert_eq!(layouts.variant_offset(&program, shape, 2, 0), 8);
        assert_eq!(layouts.of(&program, shape).size, 16);
    }

    /// More than 256 variants do not fit a byte, and the tag takes four.
    #[test]
    fn a_tag_grows_past_a_byte_of_variants() {
        let variants: Vec<String> = (0..300).map(|i| format!("V{i}")).collect();
        let (program, interner) = lowered(&format!("enum Many {{ {} }}", variants.join(", ")));
        let many = named(&program, &interner, "Many");
        let mut layouts = Layouts::new();
        assert!(matches!(
            layouts.enum_tag(&program, many),
            Tag::Byte { size: 4 }
        ));
        assert_eq!(layouts.of(&program, many).size, 4);
    }
}
