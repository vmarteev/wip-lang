//! The types of the variables a debugger is told of, as LLVM's metadata, in the
//! shapes and names `wip-codegen`'s `debugtypes.rs` writes as DWARF, which the
//! debugger's scripts read (`wip debug`): a struct is a structure, a pointer a
//! pointer, an array an array; a slice and a `str` are a pointer and a length,
//! a `dyn` and a closure their two words, and an enum a structure of its tag,
//! an enumeration of the variants' names, and a member for each variant that
//! carries something. Sizes and offsets are in bits, as LLVM counts them.

use wip_hir::{FloatTy, Ty, TyKind, Types};
use wip_mir::Tag;

use crate::{Llvm, escape};

impl Llvm<'_> {
    /// The node of `ty`, written the first time it is asked for; `None`
    /// for a type with no value, which C calls `void`.
    pub(crate) fn di_type(&mut self, ty: Ty) -> Option<usize> {
        if let Some(&node) = self.di_types.get(&ty) {
            return node;
        }
        let name = self.program.ty_name(ty, self.interner);
        let node = match self.kind(ty) {
            TyKind::Unit | TyKind::Never | TyKind::Error | TyKind::Param(_) => None,
            TyKind::Int(t) => {
                let encoding = match t.signed() {
                    true => "DW_ATE_signed",
                    false => "DW_ATE_unsigned",
                };
                Some(self.di_base(t.name(), encoding, t.bits()))
            }
            TyKind::Float(FloatTy::F32) => Some(self.di_base("f32", "DW_ATE_float", 32)),
            TyKind::Float(FloatTy::F64) => Some(self.di_base("f64", "DW_ATE_float", 64)),
            TyKind::Bool => Some(self.di_base("bool", "DW_ATE_boolean", 8)),
            TyKind::Char => Some(self.di_base("char", "DW_ATE_UTF", 32)),
            // C's `char *`, which a debugger prints as the text it is.
            TyKind::Cstring => {
                let char = self.di_base("char", "DW_ATE_signed_char", 8);
                Some(self.di_pointer(Some(char)))
            }
            TyKind::Str => {
                let byte = self.di_base("u8", "DW_ATE_unsigned_char", 8);
                let bytes = self.di_pointer(Some(byte));
                Some(self.di_pair(&name, "ptr", bytes))
            }
            // Only a declaration: what is in it is C's to know.
            TyKind::Opaque(_) => Some(self.debug.add(format!(
                "!DICompositeType(tag: DW_TAG_structure_type, name: \"{}\", flags: DIFlagFwdDecl)",
                escape(name.as_bytes())
            ))),
            // A function's address, as C's `void *`.
            TyKind::Fn(..) => {
                let pointer = self.di_pointer(None);
                Some(self.debug.add(format!(
                    "!DIDerivedType(tag: DW_TAG_typedef, name: \"{}\", baseType: !{pointer})",
                    escape(name.as_bytes())
                )))
            }
            TyKind::Ref(inner, _) | TyKind::Own(inner) | TyKind::Ptr(inner) => {
                Some(self.di_pointer_to(ty, inner, &name))
            }
            TyKind::Slice(elem) | TyKind::Slots(elem) => {
                let elem = self.di_type(elem);
                let items = self.di_pointer(elem);
                Some(self.di_pair(&name, "ptr", items))
            }
            TyKind::Dyn(..) => Some(self.di_dyn(&name)),
            TyKind::Array(elem, count) => {
                // Numbered before its element, which may be itself behind a
                // pointer.
                let node = self.debug.reserve();
                self.di_types.insert(ty, Some(node));
                let base = match self.di_type(elem) {
                    Some(elem) => format!("!{elem}"),
                    None => "null".to_string(),
                };
                let size = self.layout(ty).size * 8;
                let range = self.debug.add(format!("!DISubrange(count: {count})"));
                let elements = self.debug.add(format!("!{{!{range}}}"));
                self.debug.fill(
                    node,
                    format!(
                        "distinct !DICompositeType(tag: DW_TAG_array_type, baseType: {base}, \
                         size: {size}, elements: !{elements})"
                    ),
                );
                Some(node)
            }
            TyKind::Struct(..) => Some(self.di_structure(ty, &name)),
            TyKind::Enum(..) => Some(self.di_enumeration(ty, &name)),
        };
        self.di_types.insert(ty, node);
        node
    }

    /// A pointer to `inner`; a reference to a slice, a `dyn` or a closure
    /// is a pair, as it is laid out.
    fn di_pointer_to(&mut self, ty: Ty, inner: Ty, name: &str) -> usize {
        match self.kind(inner) {
            TyKind::Slice(elem) => {
                let elem = self.di_type(elem);
                let items = self.di_pointer(elem);
                self.di_pair(name, "ptr", items)
            }
            TyKind::Dyn(..) => self.di_dyn(name),
            TyKind::Fn(..) if !matches!(self.kind(ty), TyKind::Ptr(_)) => {
                let data = self.di_pointer(None);
                self.di_struct(name, 128, &[("env", data, 64, 0), ("code", data, 64, 64)])
            }
            _ => {
                // Numbered before what it points to, which may point back.
                let node = self.debug.reserve();
                self.di_types.insert(ty, Some(node));
                let base = match self.di_type(inner) {
                    Some(target) => format!("!{target}"),
                    None => "null".to_string(),
                };
                self.debug.fill(
                    node,
                    format!("!DIDerivedType(tag: DW_TAG_pointer_type, baseType: {base}, size: 64)"),
                );
                node
            }
        }
    }

    /// A pointer and a length: a slice, a `str`, a block of slots.
    fn di_pair(&mut self, name: &str, first: &str, items: usize) -> usize {
        let length = self.di_type(Types::I64).expect("an integer has a type");
        self.di_struct(name, 128, &[(first, items, 64, 0), ("len", length, 64, 64)])
    }

    /// A `dyn`: the value's address, and the table of its methods.
    fn di_dyn(&mut self, name: &str) -> usize {
        let data = self.di_pointer(None);
        self.di_struct(
            name,
            128,
            &[("data", data, 64, 0), ("vtable", data, 64, 64)],
        )
    }

    fn di_structure(&mut self, ty: Ty, name: &str) -> usize {
        let TyKind::Struct(id, _) = self.kind(ty) else {
            unreachable!("a structure of what is not a struct")
        };
        let program = self.program;
        let def = &program.structs[id];
        let tag = match def.is_union {
            true => "DW_TAG_union_type",
            false => "DW_TAG_structure_type",
        };
        // Where C alone knows the fields, only the name is told.
        if def.is_opaque {
            return self.debug.add(format!(
                "!DICompositeType(tag: {tag}, name: \"{}\", flags: DIFlagFwdDecl)",
                escape(name.as_bytes())
            ));
        }
        let size = self.layout(ty).size * 8;
        // Numbered before its fields, which may point back to it.
        let node = self.debug.reserve();
        self.di_types.insert(ty, Some(node));
        let mut members = Vec::new();
        for (index, field) in program.field_tys(ty).into_iter().enumerate() {
            let field_name = match def.fields.get(index) {
                Some(declared) => self.interner.resolve(declared.name).to_string(),
                // A generator's frame, after its fields.
                None => format!("frame.{}", index - def.fields.len()),
            };
            let offset = self.layouts.field_offset(program, ty, index as u32) * 8;
            if let Some(field_type) = self.di_type(field) {
                let field_size = self.layout(field).size * 8;
                members.push(self.di_member(node, &field_name, field_type, field_size, offset));
            }
        }
        let elements = self.di_list(&members);
        self.debug.fill(
            node,
            format!(
                "distinct !DICompositeType(tag: {tag}, name: \"{}\", size: {size}, \
                 elements: !{elements})",
                escape(name.as_bytes())
            ),
        );
        node
    }

    /// An enum: its tag and its variants, as they are laid out in memory.
    fn di_enumeration(&mut self, ty: Ty, name: &str) -> usize {
        let TyKind::Enum(enum_id, _) = self.kind(ty) else {
            unreachable!("an enumeration of what is not an enum")
        };
        let program = self.program;
        let def = &program.enums[enum_id];
        let size = self.layout(ty).size * 8;
        let tag = self.layouts.enum_tag(program, ty);
        let variants = program.variant_field_tys(ty);
        let names: Vec<String> = def
            .variants
            .iter()
            .map(|variant| self.interner.resolve(variant.name).to_string())
            .collect();
        match tag {
            // The one variant that carries something, where a null in its
            // `own` is the one that does not: the variant that carries is a
            // member, and the other an empty structure where the null would
            // be, so that both keep their names.
            Tag::Niche {
                payload,
                empty,
                offset,
            } => {
                let node = self.debug.reserve();
                self.di_types.insert(ty, Some(node));
                let fields = &variants[payload as usize];
                let carried = self.di_variant(node, ty, payload, &names[payload as usize], fields);
                let empty_name = &names[empty as usize];
                let nothing = self.di_struct(&format!("{name}::{empty_name}"), 0, &[]);
                let other = self.di_member(node, empty_name, nothing, 0, offset * 8);
                let elements = self.di_list(&[carried, other]);
                self.debug.fill(
                    node,
                    format!(
                        "distinct !DICompositeType(tag: DW_TAG_structure_type, name: \"{}\", \
                         size: {size}, elements: !{elements})",
                        escape(name.as_bytes())
                    ),
                );
                node
            }
            Tag::Byte { size: tag_size } => {
                let enumerators: Vec<usize> = names
                    .iter()
                    .enumerate()
                    .map(|(value, variant)| {
                        self.debug.add(format!(
                            "!DIEnumerator(name: \"{}\", value: {value}, isUnsigned: true)",
                            escape(variant.as_bytes())
                        ))
                    })
                    .collect();
                let enumerators = self.di_list(&enumerators);
                // The tag is unsigned, which LLVM learns from the
                // enumeration's base type: without one, it writes each value
                // signed, in as few bits as it needs, and 1 is -1.
                let tag_type = match tag_size {
                    1 => Types::U8,
                    _ => Types::U32,
                };
                let base = self.di_type(tag_type).expect("an integer has a type");
                // Variants that carry nothing are the names alone.
                if variants.iter().all(Vec::is_empty) {
                    return self.debug.add(format!(
                        "distinct !DICompositeType(tag: DW_TAG_enumeration_type, name: \"{}\", \
                         baseType: !{base}, size: {}, elements: !{enumerators})",
                        escape(name.as_bytes()),
                        tag_size * 8
                    ));
                }
                let names_of = self.debug.add(format!(
                    "distinct !DICompositeType(tag: DW_TAG_enumeration_type, baseType: !{base}, \
                     size: {}, elements: !{enumerators})",
                    tag_size * 8
                ));
                let node = self.debug.reserve();
                self.di_types.insert(ty, Some(node));
                let mut members = vec![self.di_member(node, "variant", names_of, tag_size * 8, 0)];
                for (index, fields) in variants.iter().enumerate() {
                    if !fields.is_empty() {
                        members.push(self.di_variant(
                            node,
                            ty,
                            index as u32,
                            &names[index],
                            fields,
                        ));
                    }
                }
                let elements = self.di_list(&members);
                self.debug.fill(
                    node,
                    format!(
                        "distinct !DICompositeType(tag: DW_TAG_structure_type, name: \"{}\", \
                         size: {size}, elements: !{elements})",
                        escape(name.as_bytes())
                    ),
                );
                node
            }
        }
    }

    /// The member of an enum's structure for one variant: a structure of
    /// its fields, placed where the first of them is.
    fn di_variant(
        &mut self,
        parent: usize,
        ty: Ty,
        index: u32,
        name: &str,
        fields: &[Ty],
    ) -> usize {
        let program = self.program;
        let offsets: Vec<u32> = (0..fields.len() as u32)
            .map(|field| self.layouts.variant_offset(program, ty, index, field))
            .collect();
        let start = offsets.iter().copied().min().unwrap_or(0);
        let end = fields
            .iter()
            .zip(&offsets)
            .map(|(&field, &offset)| offset + self.layout(field).size)
            .max()
            .unwrap_or(start);
        let TyKind::Enum(enum_id, _) = self.kind(ty) else {
            unreachable!("a variant of what is not an enum")
        };
        let declared = &program.enums[enum_id].variants[index as usize].fields;
        let full = format!("{}::{name}", program.ty_name(ty, self.interner));
        let node = self.debug.reserve();
        let mut members = Vec::new();
        for ((&field, &offset), def) in fields.iter().zip(&offsets).zip(declared) {
            if let Some(field_type) = self.di_type(field) {
                let field_name = self.interner.resolve(def.name).to_string();
                let field_size = self.layout(field).size * 8;
                members.push(self.di_member(
                    node,
                    &field_name,
                    field_type,
                    field_size,
                    (offset - start) * 8,
                ));
            }
        }
        let elements = self.di_list(&members);
        self.debug.fill(
            node,
            format!(
                "distinct !DICompositeType(tag: DW_TAG_structure_type, name: \"{}\", \
                 size: {}, elements: !{elements})",
                escape(full.as_bytes()),
                (end - start) * 8
            ),
        );
        self.di_member(parent, name, node, (end - start) * 8, start * 8)
    }

    /// A structure of the members given, each a name, a type, its size and
    /// its offset, in bits.
    fn di_struct(&mut self, name: &str, size: u32, members: &[(&str, usize, u32, u32)]) -> usize {
        let node = self.debug.reserve();
        let members: Vec<usize> = members
            .iter()
            .map(|&(member, ty, member_size, offset)| {
                self.di_member(node, member, ty, member_size, offset)
            })
            .collect();
        let elements = self.di_list(&members);
        self.debug.fill(
            node,
            format!(
                "distinct !DICompositeType(tag: DW_TAG_structure_type, name: \"{}\", \
                 size: {size}, elements: !{elements})",
                escape(name.as_bytes())
            ),
        );
        node
    }

    fn di_member(&mut self, scope: usize, name: &str, ty: usize, size: u32, offset: u32) -> usize {
        self.debug.add(format!(
            "!DIDerivedType(tag: DW_TAG_member, name: \"{}\", scope: !{scope}, \
             baseType: !{ty}, size: {size}, offset: {offset})",
            escape(name.as_bytes())
        ))
    }

    fn di_list(&mut self, nodes: &[usize]) -> usize {
        let items: Vec<String> = nodes.iter().map(|node| format!("!{node}")).collect();
        self.debug.add(format!("!{{{}}}", items.join(", ")))
    }

    /// A pointer to `target`, or C's `void *` where there is none.
    fn di_pointer(&mut self, target: Option<usize>) -> usize {
        let base = match target {
            Some(target) => format!("!{target}"),
            None => "null".to_string(),
        };
        self.debug.constant(&format!(
            "!DIDerivedType(tag: DW_TAG_pointer_type, baseType: {base}, size: 64)"
        ))
    }

    /// A base type, once.
    fn di_base(&mut self, name: &str, encoding: &str, size: u32) -> usize {
        self.debug.constant(&format!(
            "!DIBasicType(name: \"{name}\", size: {size}, encoding: {encoding})"
        ))
    }
}
