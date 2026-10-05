//! The bytes of a constant table, which a program keeps once, in read-only
//! data: laid out as a value of its type is anywhere else,
//! by the layouts every backend shares, with the strings in it left for the
//! backend to point at.

use wip_hir::{ConstValue, FloatTy, Program, Ty, TyKind};
use wip_syntax::{Interner, Symbol};

use crate::{Layouts, Tag};

/// A table's bytes, and where in them each string's address goes: a `str`
/// is its bytes' address and their length, a `cstring` the address of bytes
/// that end in a NUL, and only the backend knows where
/// those bytes are.
pub struct TableBytes {
    pub bytes: Vec<u8>,
    pub strings: Vec<(u32, Symbol)>,
    /// Where the address of each file's bytes goes, and the bytes, which
    /// the backend keeps too.
    pub blobs: Vec<(u32, std::sync::Arc<[u8]>)>,
}

/// The bytes of `value`, a constant of type `ty`, for a target whose words
/// are little-endian or not.
pub fn table_bytes(
    program: &Program,
    interner: &Interner,
    layouts: &mut Layouts,
    value: &ConstValue,
    ty: Ty,
    little_endian: bool,
) -> TableBytes {
    let size = layouts.of(program, ty).size;
    let mut writer = Writer {
        program,
        interner,
        layouts,
        little_endian,
        table: TableBytes {
            bytes: vec![0; size as usize],
            strings: Vec::new(),
            blobs: Vec::new(),
        },
    };
    writer.write(value, ty, 0);
    writer.table
}

struct Writer<'a> {
    program: &'a Program,
    interner: &'a Interner,
    layouts: &'a mut Layouts,
    little_endian: bool,
    table: TableBytes,
}

impl Writer<'_> {
    /// The low `size` bytes of `bits`, at `at`, in the target's order.
    fn put(&mut self, at: u32, size: u32, bits: u128) {
        let (at, size) = (at as usize, size as usize);
        let bytes = if self.little_endian {
            bits.to_le_bytes()
        } else {
            bits.to_be_bytes()
        };
        let word = if self.little_endian {
            &bytes[..size]
        } else {
            &bytes[bytes.len() - size..]
        };
        self.table.bytes[at..at + size].copy_from_slice(word);
    }

    /// Writes `value`, of type `ty`, at `offset`.
    fn write(&mut self, value: &ConstValue, ty: Ty, offset: u32) {
        let program = self.program;
        match value {
            ConstValue::Int(bits) => {
                let size = self.layouts.of(program, ty).size;
                self.put(offset, size, *bits);
            }
            ConstValue::Bool(value) => self.put(offset, 1, u128::from(*value)),
            ConstValue::Float(value) => match program.types.kind(ty) {
                TyKind::Float(FloatTy::F32) => {
                    self.put(offset, 4, u128::from((*value as f32).to_bits()))
                }
                _ => self.put(offset, 8, u128::from(value.to_bits())),
            },
            ConstValue::Str(sym) => {
                self.table.strings.push((offset, *sym));
                if program.types.kind(ty) == TyKind::Str {
                    let len = self.interner.resolve(*sym).len() as u128;
                    self.put(offset + 8, 8, len);
                }
            }
            // A file's bytes: their address, left to the backend, and how
            // many there are.
            ConstValue::Bytes(bytes) => {
                self.table.blobs.push((offset, bytes.clone()));
                self.put(offset + 8, 8, bytes.len() as u128);
            }
            ConstValue::Array(values) => {
                let TyKind::Array(elem, _) = program.types.kind(ty) else {
                    unreachable!("an array constant has an array's type")
                };
                let stride = self.layouts.of(program, elem).stride();
                for (i, part) in values.iter().enumerate() {
                    self.write(part, elem, offset + i as u32 * stride);
                }
            }
            ConstValue::Struct(values) => {
                let fields = program.field_tys(ty);
                for (i, part) in values.iter().enumerate() {
                    let at = self.layouts.field_offset(program, ty, i as u32);
                    self.write(part, fields[i], offset + at);
                }
            }
            ConstValue::Variant { variant, fields } => {
                match self.layouts.enum_tag(program, ty) {
                    Tag::Byte { size } => self.put(offset, size, u128::from(*variant)),
                    Tag::Niche { .. } => {
                        unreachable!("a constant holds no `own`, which a niche is")
                    }
                }
                let tys = program.variant_field_tys(ty);
                for (i, part) in fields.iter().enumerate() {
                    let at = self.layouts.variant_offset(program, ty, *variant, i as u32);
                    self.write(part, tys[*variant as usize][i], offset + at);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes of the constant `name` that `src` declares, and the
    /// program's interner.
    fn bytes_of(src: &str, name: &str, little_endian: bool) -> (TableBytes, Interner) {
        let mut interner = Interner::new();
        let lexed = wip_syntax::lex(src, &mut interner);
        let parsed = wip_syntax::parse(src, &lexed);
        let lowered = wip_hir::lower_file(&parsed.ast, &interner);
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        let program = lowered.program;
        let (_, def) = program
            .consts
            .iter()
            .find(|(_, def)| interner.resolve(def.name) == name)
            .expect("declared");
        let value = def.value.as_ref().expect("worked out");
        let mut layouts = Layouts::new();
        let table = table_bytes(
            &program,
            &interner,
            &mut layouts,
            value,
            def.ty,
            little_endian,
        );
        (table, interner)
    }

    /// Each field where the struct's layout puts it, with the padding
    /// between them zero, in the target's order of bytes.
    #[test]
    fn a_struct_is_written_where_its_layout_puts_each_field() {
        let src = "struct Mixed { small: u8, big: i64, middle: u16 }\n\
                   val M: Mixed = Mixed { small: 1, big: 2, middle: 3 }\n";
        let (little, _) = bytes_of(src, "M", true);
        let mut expected = vec![0u8; 24];
        expected[0] = 1;
        expected[8] = 2;
        expected[16] = 3;
        assert_eq!(little.bytes, expected);
        let (big, _) = bytes_of(src, "M", false);
        let mut expected = vec![0u8; 24];
        expected[0] = 1;
        expected[15] = 2;
        expected[17] = 3;
        assert_eq!(big.bytes, expected);
    }

    /// A variant's number in its tag, and its fields where the enum's
    /// layout puts them.
    #[test]
    fn a_variant_is_its_tag_and_its_fields() {
        let src = "enum Shape { Dot, Circle(r: i64) }\nval S: Shape = .Circle(r: 5)\n";
        let (table, _) = bytes_of(src, "S", true);
        let mut expected = vec![0u8; 16];
        expected[0] = 1;
        expected[8] = 5;
        assert_eq!(table.bytes, expected);
    }

    /// An array's elements a stride apart, and floats and a `bool` in their
    /// own widths.
    #[test]
    fn an_array_is_its_elements_a_stride_apart() {
        let src = "struct Pair { ratio: f32, on: bool }\n\
                   val P: [Pair; 2] = [Pair { ratio: 1.5, on: true }, Pair { ratio: -2.0, on: false }]\n";
        let (table, _) = bytes_of(src, "P", true);
        let mut expected = Vec::new();
        expected.extend_from_slice(&1.5f32.to_le_bytes());
        expected.extend_from_slice(&[1, 0, 0, 0]);
        expected.extend_from_slice(&(-2.0f32).to_le_bytes());
        expected.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(table.bytes, expected);
    }

    /// A `str` leaves its address to the backend and writes its length; a
    /// `cstring` is its address alone.
    #[test]
    fn a_string_is_left_for_its_address() {
        let src = "val WORDS: [str; 2] = [\"a\", \"hello\"]\nval C: [cstring; 1] = [\"x\"]\n";
        let (table, interner) = bytes_of(src, "WORDS", true);
        let offsets: Vec<(u32, &str)> = table
            .strings
            .iter()
            .map(|&(offset, sym)| (offset, interner.resolve(sym)))
            .collect();
        assert_eq!(offsets, vec![(0, "a"), (16, "hello")]);
        assert_eq!(table.bytes[8..16], 1u64.to_le_bytes());
        assert_eq!(table.bytes[24..32], 5u64.to_le_bytes());
        let (table, _) = bytes_of(src, "C", true);
        assert_eq!(table.bytes, vec![0u8; 8]);
        assert_eq!(table.strings.len(), 1);
    }
}
