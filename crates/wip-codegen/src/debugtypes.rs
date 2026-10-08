//! The types of the variables a debugger is told of, as DWARF, in the
//! terms of C, the language the debug information
//! names: a struct is a structure, a pointer a pointer, an array an array.
//! What C has no word for is spelled in those: a slice and a `str` are a
//! structure of a pointer and a length, and an enum a structure of its
//! tag, an enumeration of the variants' names, and a member for each
//! variant that carries something.

use gimli::write::{AttributeValue, Unit, UnitEntryId};
use rustc_hash::FxHashMap;
use wip_hir::{FloatTy, Program, Ty, TyKind};
use wip_mir::{Layouts, Tag};
use wip_syntax::Interner;

/// Writes each type once, into one compile unit.
pub(crate) struct DebugTypes<'a> {
    program: &'a Program,
    interner: &'a Interner,
    layouts: &'a mut Layouts,
    written: FxHashMap<Ty, Option<UnitEntryId>>,
    /// Named base types that are not a Wip type of their own: `u8` for the
    /// bytes of a `str`, `char` for a `cstring`'s.
    bases: FxHashMap<&'static str, UnitEntryId>,
}

impl<'a> DebugTypes<'a> {
    pub fn new(program: &'a Program, interner: &'a Interner, layouts: &'a mut Layouts) -> Self {
        DebugTypes {
            program,
            interner,
            layouts,
            written: FxHashMap::default(),
            bases: FxHashMap::default(),
        }
    }

    pub fn program(&self) -> &'a Program {
        self.program
    }

    /// A name as the source spells it.
    pub fn name(&self, name: wip_syntax::Symbol) -> String {
        self.interner.resolve(name).to_string()
    }

    /// The entry of `ty`, written into `unit` the first time it is asked
    /// for; `None` for a type with no value, which C calls `void`.
    pub fn of(&mut self, unit: &mut Unit, ty: Ty) -> Option<UnitEntryId> {
        if let Some(&id) = self.written.get(&ty) {
            return id;
        }
        let root = unit.root();
        let name = self.program.ty_name(ty, self.interner);
        let id = match self.program.types.kind(ty) {
            TyKind::Unit | TyKind::Never | TyKind::Error | TyKind::Param(_) | TyKind::Assoc(..) => {
                None
            }
            TyKind::Int(t) => {
                let encoding = match t.signed() {
                    true => gimli::DW_ATE_signed,
                    false => gimli::DW_ATE_unsigned,
                };
                Some(base(unit, t.name(), encoding, t.bits() / 8))
            }
            TyKind::Float(FloatTy::F32) => Some(base(unit, "f32", gimli::DW_ATE_float, 4)),
            TyKind::Float(FloatTy::F64) => Some(base(unit, "f64", gimli::DW_ATE_float, 8)),
            TyKind::Bool => Some(base(unit, "bool", gimli::DW_ATE_boolean, 1)),
            TyKind::Char => Some(base(unit, "char", gimli::DW_ATE_UTF, 4)),
            // C's `char *`, which a debugger prints as the text it is.
            TyKind::Cstring => {
                let char = self.base(unit, "char", gimli::DW_ATE_signed_char, 1);
                Some(pointer(unit, Some(char)))
            }
            TyKind::Str => {
                let byte = self.base(unit, "u8", gimli::DW_ATE_unsigned_char, 1);
                let bytes = pointer(unit, Some(byte));
                Some(self.pair(unit, &name, "ptr", bytes))
            }
            // Only a declaration: what is in it is C's to know.
            TyKind::Opaque(_) => {
                let id = unit.add(root, gimli::DW_TAG_structure_type);
                let entry = unit.get_mut(id);
                entry.set(gimli::DW_AT_name, AttributeValue::String(name.into_bytes()));
                entry.set(gimli::DW_AT_declaration, AttributeValue::Flag(true));
                Some(id)
            }
            // A function's address, as C's `void *`.
            TyKind::Fn(..) => Some(typedef(unit, &name, None)),
            TyKind::Ref(inner, _) | TyKind::Own(inner) | TyKind::Ptr(inner) => {
                Some(self.pointer_to(unit, ty, inner, &name))
            }
            TyKind::Slice(elem) | TyKind::Slots(elem) => {
                let elem = self.of(unit, elem);
                let items = pointer(unit, elem);
                Some(self.pair(unit, &name, "ptr", items))
            }
            TyKind::Dyn(..) => Some(self.dyn_pair(unit, &name)),
            TyKind::Array(elem, count) => {
                // Written before its element, which may be itself behind a
                // pointer.
                let id = unit.add(root, gimli::DW_TAG_array_type);
                self.written.insert(ty, Some(id));
                let elem = self.of(unit, elem);
                let entry = unit.get_mut(id);
                if let Some(elem) = elem {
                    entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(elem));
                }
                let range = unit.add(id, gimli::DW_TAG_subrange_type);
                unit.get_mut(range)
                    .set(gimli::DW_AT_count, AttributeValue::Udata(count));
                Some(id)
            }
            TyKind::Struct(id, _) => Some(self.structure(unit, ty, id, &name)),
            TyKind::Enum(..) => Some(self.enumeration(unit, ty, &name)),
        };
        self.written.insert(ty, id);
        id
    }

    /// A pointer to `inner`; a reference to a slice, a `dyn` or a closure
    /// is a pair, as it is laid out.
    fn pointer_to(&mut self, unit: &mut Unit, ty: Ty, inner: Ty, name: &str) -> UnitEntryId {
        match self.program.types.kind(inner) {
            TyKind::Slice(elem) => {
                let elem = self.of(unit, elem);
                let items = pointer(unit, elem);
                self.pair(unit, name, "ptr", items)
            }
            TyKind::Dyn(..) => self.dyn_pair(unit, name),
            TyKind::Fn(..) if !matches!(self.program.types.kind(ty), TyKind::Ptr(_)) => {
                let data = pointer(unit, None);
                let id = structure(unit, gimli::DW_TAG_structure_type, name, 16);
                member(unit, id, "env", data, 0);
                member(unit, id, "code", data, 8);
                id
            }
            _ => {
                // Written before what it points to, which may point back.
                let id = unit.add(unit.root(), gimli::DW_TAG_pointer_type);
                unit.get_mut(id)
                    .set(gimli::DW_AT_byte_size, AttributeValue::Udata(8));
                self.written.insert(ty, Some(id));
                if let Some(target) = self.of(unit, inner) {
                    unit.get_mut(id)
                        .set(gimli::DW_AT_type, AttributeValue::UnitRef(target));
                }
                id
            }
        }
    }

    /// A pointer and a length: a slice, a `str`, a block of slots.
    fn pair(
        &mut self,
        unit: &mut Unit,
        name: &str,
        first: &str,
        items: UnitEntryId,
    ) -> UnitEntryId {
        let length = self
            .of(unit, wip_hir::Types::I64)
            .expect("an integer has a type entry");
        let id = structure(unit, gimli::DW_TAG_structure_type, name, 16);
        member(unit, id, first, items, 0);
        member(unit, id, "len", length, 8);
        id
    }

    /// A `dyn`: the value's address, and the table of its methods.
    fn dyn_pair(&mut self, unit: &mut Unit, name: &str) -> UnitEntryId {
        let data = pointer(unit, None);
        let id = structure(unit, gimli::DW_TAG_structure_type, name, 16);
        member(unit, id, "data", data, 0);
        member(unit, id, "vtable", data, 8);
        id
    }

    fn structure(
        &mut self,
        unit: &mut Unit,
        ty: Ty,
        id: wip_hir::StructId,
        name: &str,
    ) -> UnitEntryId {
        let program = self.program;
        let def = &program.structs[id];
        let tag = match def.is_union {
            true => gimli::DW_TAG_union_type,
            false => gimli::DW_TAG_structure_type,
        };
        // Where C alone knows the fields, only the name is told.
        if def.is_opaque {
            let entry_id = unit.add(unit.root(), tag);
            let entry = unit.get_mut(entry_id);
            entry.set(
                gimli::DW_AT_name,
                AttributeValue::String(name.as_bytes().to_vec()),
            );
            entry.set(gimli::DW_AT_declaration, AttributeValue::Flag(true));
            return entry_id;
        }
        let size = self.layouts.of(self.program, ty).size;
        let entry_id = structure(unit, tag, name, size);
        // Written before its fields, which may point back to it.
        self.written.insert(ty, Some(entry_id));
        let fields = self.program.field_tys(ty);
        for (index, &field) in fields.iter().enumerate() {
            let field_name = match def.fields.get(index) {
                Some(declared) => self.interner.resolve(declared.name).to_string(),
                // A generator's frame, after its fields.
                None => format!("frame.{}", index - def.fields.len()),
            };
            let offset = self.layouts.field_offset(self.program, ty, index as u32);
            if let Some(field) = self.of(unit, field) {
                member(unit, entry_id, &field_name, field, offset);
            }
        }
        entry_id
    }

    /// An enum: its tag and its variants, as they are laid out in memory.
    fn enumeration(&mut self, unit: &mut Unit, ty: Ty, name: &str) -> UnitEntryId {
        let TyKind::Enum(enum_id, _) = self.program.types.kind(ty) else {
            unreachable!("an enumeration of a type that is not an enum")
        };
        let program = self.program;
        let def = &program.enums[enum_id];
        let size = self.layouts.of(self.program, ty).size;
        let tag = self.layouts.enum_tag(self.program, ty);
        let variants = self.program.variant_field_tys(ty);
        let names: Vec<String> = def
            .variants
            .iter()
            .map(|variant| self.interner.resolve(variant.name).to_string())
            .collect();
        match tag {
            // The one variant that carries something, where a null in its
            // `own` is the one that does not: the variant that carries is a
            // member, and the other an empty structure where the null
            // would be, so that both keep their names.
            Tag::Niche {
                payload,
                empty,
                offset,
            } => {
                let id = structure(unit, gimli::DW_TAG_structure_type, name, size);
                self.written.insert(ty, Some(id));
                let fields = &variants[payload as usize];
                self.variant(unit, id, ty, payload, &names[payload as usize], fields);
                let empty_name = &names[empty as usize];
                let nothing = structure(
                    unit,
                    gimli::DW_TAG_structure_type,
                    &format!("{name}::{empty_name}"),
                    0,
                );
                member(unit, id, empty_name, nothing, offset);
                id
            }
            Tag::Byte { size: tag_size } => {
                let names_of = unit.add(unit.root(), gimli::DW_TAG_enumeration_type);
                let entry = unit.get_mut(names_of);
                entry.set(
                    gimli::DW_AT_byte_size,
                    AttributeValue::Udata(u64::from(tag_size)),
                );
                for (value, variant) in names.iter().enumerate() {
                    let enumerator = unit.add(names_of, gimli::DW_TAG_enumerator);
                    let entry = unit.get_mut(enumerator);
                    entry.set(
                        gimli::DW_AT_name,
                        AttributeValue::String(variant.clone().into_bytes()),
                    );
                    entry.set(
                        gimli::DW_AT_const_value,
                        AttributeValue::Udata(value as u64),
                    );
                }
                // Variants that carry nothing are the names alone.
                if variants.iter().all(Vec::is_empty) {
                    unit.get_mut(names_of).set(
                        gimli::DW_AT_name,
                        AttributeValue::String(name.as_bytes().to_vec()),
                    );
                    self.written.insert(ty, Some(names_of));
                    return names_of;
                }
                let id = structure(unit, gimli::DW_TAG_structure_type, name, size);
                self.written.insert(ty, Some(id));
                member(unit, id, "variant", names_of, 0);
                for (index, fields) in variants.iter().enumerate() {
                    if !fields.is_empty() {
                        self.variant(unit, id, ty, index as u32, &names[index], fields);
                    }
                }
                id
            }
        }
    }

    /// A member of an enum's structure for one variant: a structure of its
    /// fields, placed where the first of them is.
    fn variant(
        &mut self,
        unit: &mut Unit,
        parent: UnitEntryId,
        ty: Ty,
        index: u32,
        name: &str,
        fields: &[Ty],
    ) {
        let offsets: Vec<u32> = (0..fields.len() as u32)
            .map(|field| self.layouts.variant_offset(self.program, ty, index, field))
            .collect();
        let start = offsets.iter().copied().min().unwrap_or(0);
        let end = fields
            .iter()
            .zip(&offsets)
            .map(|(&field, &offset)| offset + self.layouts.of(self.program, field).size)
            .max()
            .unwrap_or(start);
        let TyKind::Enum(enum_id, _) = self.program.types.kind(ty) else {
            unreachable!("a variant of a type that is not an enum")
        };
        let program = self.program;
        let declared = &program.enums[enum_id].variants[index as usize].fields;
        let full = format!("{}::{name}", self.program.ty_name(ty, self.interner));
        let id = structure(unit, gimli::DW_TAG_structure_type, &full, end - start);
        for ((field, offset), def) in fields.iter().zip(&offsets).zip(declared) {
            if let Some(field) = self.of(unit, *field) {
                let field_name = self.interner.resolve(def.name).to_string();
                member(unit, id, &field_name, field, offset - start);
            }
        }
        member(unit, parent, name, id, start);
    }

    /// A base type that is not a Wip type of its own, written once.
    fn base(
        &mut self,
        unit: &mut Unit,
        name: &'static str,
        encoding: gimli::DwAte,
        size: u32,
    ) -> UnitEntryId {
        if let Some(&id) = self.bases.get(name) {
            return id;
        }
        let id = base(unit, name, encoding, size);
        self.bases.insert(name, id);
        id
    }
}

fn base(unit: &mut Unit, name: &str, encoding: gimli::DwAte, size: u32) -> UnitEntryId {
    let id = unit.add(unit.root(), gimli::DW_TAG_base_type);
    let entry = unit.get_mut(id);
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(name.as_bytes().to_vec()),
    );
    entry.set(gimli::DW_AT_encoding, AttributeValue::Encoding(encoding));
    entry.set(
        gimli::DW_AT_byte_size,
        AttributeValue::Udata(u64::from(size)),
    );
    id
}

/// A pointer to `target`, or C's `void *` where there is none.
fn pointer(unit: &mut Unit, target: Option<UnitEntryId>) -> UnitEntryId {
    let id = unit.add(unit.root(), gimli::DW_TAG_pointer_type);
    let entry = unit.get_mut(id);
    entry.set(gimli::DW_AT_byte_size, AttributeValue::Udata(8));
    if let Some(target) = target {
        entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(target));
    }
    id
}

/// Another name for `target`: `void *` where there is none.
fn typedef(unit: &mut Unit, name: &str, target: Option<UnitEntryId>) -> UnitEntryId {
    let target = match target {
        Some(target) => target,
        None => pointer(unit, None),
    };
    let id = unit.add(unit.root(), gimli::DW_TAG_typedef);
    let entry = unit.get_mut(id);
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(name.as_bytes().to_vec()),
    );
    entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(target));
    id
}

fn structure(unit: &mut Unit, tag: gimli::DwTag, name: &str, size: u32) -> UnitEntryId {
    let id = unit.add(unit.root(), tag);
    let entry = unit.get_mut(id);
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(name.as_bytes().to_vec()),
    );
    entry.set(
        gimli::DW_AT_byte_size,
        AttributeValue::Udata(u64::from(size)),
    );
    id
}

fn member(unit: &mut Unit, parent: UnitEntryId, name: &str, ty: UnitEntryId, offset: u32) {
    let id = unit.add(parent, gimli::DW_TAG_member);
    let entry = unit.get_mut(id);
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(name.as_bytes().to_vec()),
    );
    entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(ty));
    entry.set(
        gimli::DW_AT_data_member_location,
        AttributeValue::Udata(u64::from(offset)),
    );
}
