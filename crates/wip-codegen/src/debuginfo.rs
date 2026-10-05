//! Debug information: where each function's code was
//! written, as DWARF's line table, and an entry for each function, written
//! into the module's object and relocated against the functions' symbols.
//! A debugger stops at `file:line`, steps by line, and names every frame's
//! function and line; a profiler gives time to lines.

use cranelift_module::FuncId;
use cranelift_object::ObjectProduct;
use gimli::write::{
    Address, AttributeValue, DwarfUnit, EndianVec, Expression, FileId, LineProgram, LineString,
    Range, RangeList, RelocateWriter, Relocation, RelocationTarget, Sections, UnitEntryId,
};
use gimli::{Encoding, Format, LineEncoding, RunTimeEndian, SectionId};
use object::write::{Object, Relocation as ObjectRelocation};
use object::{BinaryFormat, RelocationEncoding, RelocationFlags, RelocationKind, SectionKind};
use rustc_hash::FxHashMap;
use wip_syntax::Span;

use crate::Locations;
use crate::debugtypes::DebugTypes;

/// What a function's code tells the debugger: its name, where it was
/// declared, how long its code is, and where each run of it was written.
/// Code the compiler writes on its own — a drop function, the entry — was
/// declared nowhere and has no lines.
pub(crate) struct DebugFn {
    pub func: FuncId,
    pub name: String,
    pub at: Option<Span>,
    /// Whether the compiler wrote it on its own, which a panic's calls
    /// pass over.
    pub glue: bool,
    pub size: u32,
    /// Where a run of code begins in the function, and the span it was
    /// written at, in the order of the code.
    pub rows: Vec<(u32, u32)>,
    /// The variables and parameters of the source it keeps where a
    /// debugger finds them: a debug build's.
    pub variables: Vec<DebugVar>,
    /// How to unwind its frame, where Cranelift can say.
    pub unwind: Option<cranelift_codegen::isa::unwind::UnwindInfo>,
}

/// A variable or parameter, and where the function keeps it.
pub(crate) struct DebugVar {
    pub source: wip_mir::Source,
    pub ty: wip_hir::Ty,
    pub param: bool,
    /// How far from the frame pointer its slot is.
    pub offset: i64,
    /// Whether the slot holds its address rather than it.
    pub indirect: bool,
}

/// A section being written, and the relocations it needs.
#[derive(Clone)]
pub(crate) struct DebugSection {
    pub data: EndianVec<RunTimeEndian>,
    pub relocations: Vec<Relocation>,
}

impl DebugSection {
    pub fn new(data: EndianVec<RunTimeEndian>) -> DebugSection {
        DebugSection {
            data,
            relocations: Vec::new(),
        }
    }
}

impl RelocateWriter for DebugSection {
    type Writer = EndianVec<RunTimeEndian>;

    fn writer(&self) -> &Self::Writer {
        &self.data
    }

    fn writer_mut(&mut self) -> &mut Self::Writer {
        &mut self.data
    }

    fn relocate(&mut self, relocation: Relocation) {
        self.relocations.push(relocation);
    }
}

/// The files the line table names, each once.
struct Files {
    ids: FxHashMap<String, FileId>,
}

impl Files {
    fn of(&mut self, program: &mut LineProgram, name: &str) -> FileId {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let dir = program.default_directory();
        let id = program.add_file(LineString::String(path_of(name).into_bytes()), dir, None);
        self.ids.insert(name.to_string(), id);
        id
    }
}

/// A file as the debugger is to find it: where it is on this machine, if it
/// is there, and as it is named otherwise — the standard library's files
/// live in the compiler.
fn path_of(name: &str) -> String {
    std::fs::canonicalize(name).map_or_else(|_| name.to_string(), |path| path.display().to_string())
}

/// Writes the module's debug information into its object: one compile unit
/// named `unit`, with a line table and an entry for each function in `fns`,
/// holding its variables and parameters, and the types they are.
pub(crate) fn write(
    product: &mut ObjectProduct,
    fns: &[DebugFn],
    locations: Locations<'_>,
    unit: &str,
    types: &mut DebugTypes<'_>,
) {
    if fns.is_empty() {
        return;
    }
    let encoding = Encoding {
        format: Format::Dwarf32,
        version: 4,
        address_size: 8,
    };
    let comp_dir = std::env::current_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let mut dwarf = DwarfUnit::new(encoding);
    let mut program = LineProgram::new(
        encoding,
        LineEncoding::default(),
        LineString::String(comp_dir.clone().into_bytes()),
        None,
        LineString::String(unit.as_bytes().to_vec()),
        None,
    );
    let mut files = Files {
        ids: FxHashMap::default(),
    };
    let root = dwarf.unit.root();
    let mut ranges = Vec::new();
    for (index, function) in fns.iter().enumerate() {
        let start = Address::Symbol {
            symbol: index,
            addend: 0,
        };
        program.begin_sequence(Some(start));
        // The code before the first statement's — the frame, the parameters
        // put in their slots — is the declaration's, as C compilers write
        // it; the first statement's run ends the prologue, which is where a
        // debugger stops at a function named, with its parameters in place.
        let first = function.rows.first().map(|&(offset, _)| offset);
        if let Some(at) = function.at
            && first.is_some_and(|offset| offset > 0)
        {
            let (name, line, column) = locations(at);
            let file = files.of(&mut program, &name);
            let row = program.row();
            row.address_offset = 0;
            row.file = file;
            row.line = u64::from(line);
            row.column = u64::from(column);
            program.generate_row();
        }
        let mut last = None;
        for &(offset, lo) in &function.rows {
            let (name, line, column) = locations(Span::at(lo));
            // A place the sources do not know says nothing of a line.
            if line == 0 || last == Some((lo, line, column)) {
                continue;
            }
            let file = files.of(&mut program, &name);
            let row = program.row();
            row.address_offset = u64::from(offset);
            row.file = file;
            row.line = u64::from(line);
            row.column = u64::from(column);
            row.prologue_end = last.is_none();
            last = Some((lo, line, column));
            program.generate_row();
        }
        program.end_sequence(u64::from(function.size));
        ranges.push(Range::StartLength {
            begin: start,
            length: u64::from(function.size),
        });

        let entry_id = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        let entry = dwarf.unit.get_mut(entry_id);
        entry.set(
            gimli::DW_AT_name,
            AttributeValue::String(function.name.clone().into_bytes()),
        );
        entry.set(gimli::DW_AT_low_pc, AttributeValue::Address(start));
        entry.set(
            gimli::DW_AT_high_pc,
            AttributeValue::Udata(u64::from(function.size)),
        );
        if let Some(at) = function.at {
            let (name, line, _) = locations(at);
            let file = files.of(&mut program, &name);
            entry.set(
                gimli::DW_AT_decl_file,
                AttributeValue::FileIndex(Some(file)),
            );
            entry.set(
                gimli::DW_AT_decl_line,
                AttributeValue::Udata(u64::from(line)),
            );
        }
        if function.glue {
            entry.set(gimli::DW_AT_artificial, AttributeValue::Flag(true));
        }
        entry.set(gimli::DW_AT_external, AttributeValue::Flag(true));
        if !function.variables.is_empty() {
            let mut base = Expression::new();
            base.op_reg(FRAME_POINTER);
            entry.set(gimli::DW_AT_frame_base, AttributeValue::Exprloc(base));
            let subprogram = entry_id;
            write_variables(
                &mut dwarf,
                &mut program,
                &mut files,
                types,
                (index, function, subprogram),
                locations,
            );
        }
    }
    dwarf.unit.line_program = program;
    let ranges = dwarf.unit.ranges.add(RangeList(ranges));
    let root = dwarf.unit.get_mut(root);
    root.set(
        gimli::DW_AT_producer,
        AttributeValue::String(b"wip".to_vec()),
    );
    // Wip has no language number of its own; C's is what debuggers take
    // for a language they do not know, and names and lines need no more.
    root.set(
        gimli::DW_AT_language,
        AttributeValue::Language(gimli::DW_LANG_C99),
    );
    root.set(
        gimli::DW_AT_name,
        AttributeValue::String(unit.as_bytes().to_vec()),
    );
    root.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(comp_dir.into_bytes()),
    );
    root.set(
        gimli::DW_AT_low_pc,
        AttributeValue::Address(Address::Constant(0)),
    );
    root.set(gimli::DW_AT_ranges, AttributeValue::RangeListRef(ranges));

    let mut sections = Sections::new(DebugSection::new(EndianVec::new(RunTimeEndian::Little)));
    dwarf
        .write(&mut sections)
        .expect("debug information can be written to memory");
    add_sections(product, &sections, fns);
}

/// The register a function's frame pointer is in, which its variables are
/// found from.
#[cfg(target_arch = "aarch64")]
const FRAME_POINTER: gimli::Register = gimli::AArch64::X29;
#[cfg(target_arch = "x86_64")]
const FRAME_POINTER: gimli::Register = gimli::X86_64::RBP;

/// A function's variables and parameters, each in its slot. A parameter is
/// seen by the whole function; a variable by the code written between its
/// name and the end of its scope, as a lexical block of that code, nested
/// in the block of the variable whose scope holds its own, so that the
/// innermost of two of one name is the one a debugger shows.
fn write_variables(
    dwarf: &mut DwarfUnit,
    program: &mut LineProgram,
    files: &mut Files,
    types: &mut DebugTypes<'_>,
    (index, function, subprogram): (usize, &DebugFn, UnitEntryId),
    locations: Locations<'_>,
) {
    let mut variables: Vec<&DebugVar> = function.variables.iter().collect();
    variables.sort_by_key(|var| {
        (
            !var.param,
            var.source.at.lo,
            std::cmp::Reverse(var.source.seen_until),
        )
    });
    // The scopes open around the variable being written: where each ends,
    // and its block.
    let mut open: Vec<(u32, UnitEntryId)> = Vec::new();
    for var in variables {
        let parent = match var.param {
            true => subprogram,
            false => {
                let ranges = code_between(function, var.source.at.lo, var.source.seen_until);
                // Code no run of which was written in its scope never sees
                // it.
                if ranges.is_empty() {
                    continue;
                }
                while open
                    .last()
                    .is_some_and(|&(end, _)| end < var.source.seen_until)
                {
                    open.pop();
                }
                let outer = open.last().map_or(subprogram, |&(_, block)| block);
                let block = dwarf.unit.add(outer, gimli::DW_TAG_lexical_block);
                let list = RangeList(
                    ranges
                        .into_iter()
                        .map(|(start, end)| Range::StartLength {
                            begin: Address::Symbol {
                                symbol: index,
                                addend: i64::from(start),
                            },
                            length: u64::from(end - start),
                        })
                        .collect(),
                );
                let list = dwarf.unit.ranges.add(list);
                dwarf
                    .unit
                    .get_mut(block)
                    .set(gimli::DW_AT_ranges, AttributeValue::RangeListRef(list));
                open.push((var.source.seen_until, block));
                block
            }
        };
        let tag = match var.param {
            true => gimli::DW_TAG_formal_parameter,
            false => gimli::DW_TAG_variable,
        };
        // A binding that aliases what it matched is a reference, and is
        // shown as what it refers to.
        let ty = match (var.source.by_address, types.program().types.kind(var.ty)) {
            (true, wip_hir::TyKind::Ref(inner, _)) => inner,
            _ => var.ty,
        };
        let ty = types.of(&mut dwarf.unit, ty);
        let (file, line, _) = locations(var.source.at);
        let file = files.of(program, &file);
        let mut location = Expression::new();
        location.op_fbreg(var.offset);
        if var.indirect {
            location.op_deref();
        }
        let name = types.name(var.source.name);
        let id = dwarf.unit.add(parent, tag);
        let entry = dwarf.unit.get_mut(id);
        entry.set(gimli::DW_AT_name, AttributeValue::String(name.into_bytes()));
        if let Some(ty) = ty {
            entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(ty));
        }
        entry.set(
            gimli::DW_AT_decl_file,
            AttributeValue::FileIndex(Some(file)),
        );
        entry.set(
            gimli::DW_AT_decl_line,
            AttributeValue::Udata(u64::from(line)),
        );
        entry.set(gimli::DW_AT_location, AttributeValue::Exprloc(location));
    }
}

/// The runs of a function's code written from `lo` up to `until`, as
/// ranges of offsets into the code, adjacent runs joined.
fn code_between(function: &DebugFn, lo: u32, until: u32) -> Vec<(u32, u32)> {
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for (i, &(start, at)) in function.rows.iter().enumerate() {
        if at < lo || at >= until {
            continue;
        }
        let end = function
            .rows
            .get(i + 1)
            .map_or(function.size, |&(next, _)| next);
        if start >= end {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if last.1 == start => last.1 = end,
            _ => ranges.push((start, end)),
        }
    }
    ranges
}

/// Puts the written sections into the object, with the relocations that
/// give each address its function and each offset its section.
fn add_sections(product: &mut ObjectProduct, sections: &Sections<DebugSection>, fns: &[DebugFn]) {
    let macho = product.object.format() == BinaryFormat::MachO;
    let mut written: Vec<(SectionId, &DebugSection)> = Vec::new();
    sections
        .for_each(|id, section| -> Result<(), ()> {
            if !section.data.slice().is_empty() {
                written.push((id, section));
            }
            Ok(())
        })
        .expect("nothing fails");
    let ids: FxHashMap<SectionId, object::write::SectionId> = written
        .iter()
        .map(|&(id, _)| (id, add_section(&mut product.object, id, macho)))
        .collect();
    for (id, section) in written {
        let mut data = section.data.slice().to_vec();
        let mut relocations = Vec::new();
        for relocation in &section.relocations {
            let mut addend = relocation.addend;
            let symbol = match relocation.target {
                // Apple's tools find a function's address through its
                // section: a relocation against a symbol the object keeps to
                // itself is read as the start of the code, so it is made
                // against the section, at the function's offset in it, as
                // clang writes it.
                RelocationTarget::Symbol(index) if macho => {
                    let symbol = product
                        .object
                        .symbol(product.function_symbol(fns[index].func));
                    let object::write::SymbolSection::Section(section) = symbol.section else {
                        unreachable!("a defined function is in a section")
                    };
                    addend += symbol.value as i64;
                    product.object.section_symbol(section)
                }
                RelocationTarget::Symbol(index) => product.function_symbol(fns[index].func),
                // Apple's tools read an offset into another section as it is
                // written, and ELF's linker moves it with its section.
                RelocationTarget::Section(_) if macho => {
                    let at = relocation.offset;
                    let size = usize::from(relocation.size);
                    let value = relocation.addend.to_le_bytes();
                    data[at..at + size].copy_from_slice(&value[..size]);
                    continue;
                }
                RelocationTarget::Section(target) => product.object.section_symbol(ids[&target]),
            };
            relocations.push(ObjectRelocation {
                offset: relocation.offset as u64,
                symbol,
                addend,
                flags: RelocationFlags::Generic {
                    kind: RelocationKind::Absolute,
                    encoding: RelocationEncoding::Generic,
                    size: relocation.size * 8,
                },
            });
        }
        let section = ids[&id];
        product.object.set_section_data(section, data, 1);
        for relocation in relocations {
            product
                .object
                .add_relocation(section, relocation)
                .expect("a debug section's relocation is one the object takes");
        }
    }
}

/// Mach-O keeps the value of a relocation against a section in place, as
/// an address in the object, the function's section's address and all —
/// which is what `dsymutil` reads, for a relocation against a symbol too.
/// The `object` crate places the sections only as it writes the object,
/// and writes the offset into the section; so once it has, each such value
/// in the debug sections is given its section's address, as an
/// assembler's last pass gives it. Without this a function's lines were
/// right only in an object whose code came first.
pub(crate) fn place_macho_addresses(bytes: &mut [u8]) {
    use object::read::macho::MachOFile64;
    use object::{Endianness, Object, ObjectSection};

    let mut places: Vec<(usize, u8, u64)> = Vec::new();
    {
        let file = MachOFile64::<Endianness>::parse(&*bytes)
            .expect("the object just written is a Mach-O object");
        for section in file.sections() {
            if section.segment_name() != Ok(Some("__DWARF")) {
                continue;
            }
            let Some((start, _)) = section.file_range() else {
                continue;
            };
            for (offset, relocation) in section.relocations() {
                let object::RelocationTarget::Section(target) = relocation.target() else {
                    continue;
                };
                let address = file
                    .section_by_index(target)
                    .expect("a relocation is against a section of the object")
                    .address();
                places.push(((start + offset) as usize, relocation.size(), address));
            }
        }
    }
    for (at, bits, address) in places {
        let size = usize::from(bits / 8);
        let mut value = [0u8; 8];
        value[..size].copy_from_slice(&bytes[at..at + size]);
        let placed = u64::from_le_bytes(value).wrapping_add(address);
        bytes[at..at + size].copy_from_slice(&placed.to_le_bytes()[..size]);
    }
}

/// A debug section of the object: `.debug_line` in ELF, `__debug_line` in
/// Mach-O's `__DWARF` segment.
fn add_section(object: &mut Object<'_>, id: SectionId, macho: bool) -> object::write::SectionId {
    let name = id.name();
    let (segment, name) = match macho {
        true => (
            b"__DWARF".to_vec(),
            format!("__{}", &name[1..]).into_bytes(),
        ),
        false => (Vec::new(), name.as_bytes().to_vec()),
    };
    object.add_section(segment, name, SectionKind::Debug)
}
