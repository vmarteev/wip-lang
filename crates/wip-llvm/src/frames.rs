//! The table a panic reads to say the calls that led to it,
//! made from the DWARF LLVM wrote.
//!
//! What the table holds — each function's code, and where each run of it
//! was written — is known only once LLVM has compiled the program, and
//! LLVM has inlined much of it. The table names each function by its
//! symbol, which only the program's own object may do of a function it
//! keeps to itself, so the table goes into that object:
//!
//! - **ELF:** the object `clang` wrote is read, and written again with the
//!   table, its relocations and its symbol added (`with_frame_tables`).
//! - **Mach-O,** which `object` cannot write back: the program is compiled
//!   to assembly, assembled once to read its DWARF, and assembled again
//!   with the table after it (`frame_table_assembly`); the code is the
//!   same both times, since the table adds only data. Every symbol there
//!   begins with `_`, so none reads as a register, which on Linux's arm64
//!   `v0` or `x1` would.
//!
//! The table has the form Cranelift's code writes (`wip-codegen`'s
//! `frames.rs`), with what LLVM's inlining adds: a row may say that its
//! code is a call inlined, and the list of those says what was called, and
//! from which line of what.

use std::fmt::Write as _;

use gimli::{AttributeValue, Reader as _, RunTimeEndian};
use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
use rustc_hash::FxHashMap;

/// The runtime's own file: its functions are how a panic is told, not
/// where it happened.
const RUNTIME: &str = "std/prelude/runtime.wip";

/// What a debug section's relocations put at each offset: an addend, and
/// whether it adds to what is written there.
#[derive(Debug, Default)]
struct Relocations(FxHashMap<u64, (bool, u64)>);

impl Relocations {
    fn of(file: &object::File<'_>, section: &object::Section<'_, '_>) -> Result<Self, String> {
        let mut map = FxHashMap::default();
        for (offset, relocation) in section.relocations() {
            if relocation.kind() != object::RelocationKind::Absolute {
                return Err(format!("a relocation of {:?} in DWARF", relocation.kind()));
            }
            let target = match relocation.target() {
                object::RelocationTarget::Symbol(index) => {
                    let symbol = file.symbol_by_index(index).map_err(|err| err.to_string())?;
                    address(file, symbol.section_index(), symbol.address())
                }
                object::RelocationTarget::Section(index) => address(file, Some(index), 0),
                _ => return Err("a relocation in DWARF to nothing it names".to_string()),
            };
            let addend = target.wrapping_add(relocation.addend() as u64);
            map.insert(offset, (relocation.has_implicit_addend(), addend));
        }
        Ok(Relocations(map))
    }

    fn relocate(&self, offset: usize, value: u64) -> u64 {
        match self.0.get(&(offset as u64)) {
            Some(&(true, addend)) => value.wrapping_add(addend),
            Some(&(false, addend)) => addend,
            None => value,
        }
    }
}

/// An address in `section` of the object, where every section of code has
/// a place of its own: a Mach-O object's sections have theirs, and an ELF
/// object's each begin at 0, so each is given a base of its own — LLVM may
/// put a cold function in a section other than the rest. An address in any
/// other section is a distance into it, as DWARF reads one in its own
/// sections; DWARF names no other code or data.
fn address(file: &object::File<'_>, section: Option<object::SectionIndex>, at: u64) -> u64 {
    let Some(index) = section else {
        return at;
    };
    let Ok(section) = file.section_by_index(index) else {
        return at;
    };
    if section.kind() != object::SectionKind::Text {
        return at;
    }
    match file.format() {
        object::BinaryFormat::Elf => ((index.0 as u64) << 32) + at,
        _ => section.address() + at,
    }
}

impl gimli::read::Relocate for &Relocations {
    fn relocate_address(&self, offset: usize, value: u64) -> gimli::Result<u64> {
        Ok(self.relocate(offset, value))
    }

    fn relocate_offset(&self, offset: usize, value: usize) -> gimli::Result<usize> {
        <usize as gimli::ReaderOffset>::from_u64(self.relocate(offset, value as u64))
    }
}

type Reader<'a> = gimli::RelocateReader<gimli::EndianSlice<'a, RunTimeEndian>, &'a Relocations>;

/// A function of the program as the DWARF describes it.
struct Function {
    symbol: String,
    name: String,
    low: u64,
    high: u64,
    pass_over: bool,
}

/// A call inlined: what was called, and from where in what. Two calls
/// inlined that say the same are one in the table.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Inlined {
    name: String,
    pass_over: bool,
    file: u32,
    line: u32,
    /// The call inlined this one was inlined into, counting from 1; 0 for
    /// the function itself.
    parent: u32,
}

/// A call inlined into the function being read: its record in the table,
/// and the code it is.
struct Call {
    record: u32,
    ranges: Vec<(u64, u64)>,
}

/// A row of a function's table: from `offset`, code written at `file` and
/// `line`, in the call inlined `inlined` (from 1), or in the function.
#[derive(Clone, Copy, PartialEq)]
struct Row {
    offset: u32,
    file: u32,
    line: u32,
    inlined: u32,
}

/// The program's table and the list of tables, as assembly to follow the
/// program's: read from `object`, the program assembled, whose DWARF names
/// files as `files` says a panic names them. For Mach-O.
pub fn frame_table_assembly(
    object: &[u8],
    files: &FxHashMap<String, String>,
) -> Result<String, String> {
    let file = object::File::parse(object).map_err(|err| err.to_string())?;
    let macho = file.format() == object::BinaryFormat::MachO;
    let table = read(&file, files)?;
    Ok(assembly(&table.layout(), macho))
}

/// `object`, the program as `clang` compiled it, with its table and the
/// list of tables added: read from its DWARF, which names files as `files`
/// says a panic names them. For ELF.
pub fn with_frame_tables(
    object: &[u8],
    files: &FxHashMap<String, String>,
) -> Result<Vec<u8>, String> {
    let file = object::File::parse(object).map_err(|err| err.to_string())?;
    let table = read(&file, files)?;
    elf_with(object, &table.layout())
}

/// The table an object's DWARF and symbols describe.
fn read(file: &object::File<'_>, files: &FxHashMap<String, String>) -> Result<Table, String> {
    let endian = match file.is_little_endian() {
        true => RunTimeEndian::Little,
        false => RunTimeEndian::Big,
    };
    let macho = file.format() == object::BinaryFormat::MachO;
    let sections = gimli::DwarfSections::load(|id| -> Result<(&[u8], Relocations), String> {
        match file.section_by_name(id.name()) {
            Some(section) => Ok((
                section.data().map_err(|err| err.to_string())?,
                Relocations::of(file, &section)?,
            )),
            None => Ok((&[], Relocations::default())),
        }
    })?;
    let dwarf = sections.borrow(|(data, relocations)| {
        gimli::RelocateReader::new(gimli::EndianSlice::new(data, endian), relocations)
    });
    // A function's symbol, by the address of its code: the program's own,
    // not an assembler's label at the same place.
    let mut symbols: FxHashMap<u64, String> = FxHashMap::default();
    for symbol in file.symbols() {
        if symbol.kind() != object::SymbolKind::Text || symbol.is_undefined() {
            continue;
        }
        let Ok(name) = symbol.name() else { continue };
        let temporary = name.is_empty()
            || name.starts_with(".L")
            || (macho && (name.starts_with('l') || name.starts_with('L')));
        if !temporary {
            let at = address(file, symbol.section_index(), symbol.address());
            symbols.entry(at).or_insert_with(|| name.to_string());
        }
    }

    let mut table = Table::default();
    let mut units = dwarf.units();
    while let Some(header) = units.next().map_err(|err| err.to_string())? {
        let unit = dwarf.unit(header).map_err(|err| err.to_string())?;
        read_unit(&dwarf, &unit, files, &symbols, &mut table).map_err(|err| err.to_string())?;
    }
    Ok(table)
}

#[derive(Default)]
struct Table {
    files: Vec<String>,
    file_index: FxHashMap<String, u32>,
    functions: Vec<(Function, Vec<Row>)>,
    inlined: Vec<Inlined>,
    inlined_index: FxHashMap<Inlined, u32>,
}

impl Table {
    /// A call inlined's record, counting from 1.
    fn inlined(&mut self, call: Inlined) -> u32 {
        if let Some(&index) = self.inlined_index.get(&call) {
            return index;
        }
        self.inlined.push(call.clone());
        let index = self.inlined.len() as u32;
        self.inlined_index.insert(call, index);
        index
    }

    fn file(&mut self, name: String) -> u32 {
        if let Some(&index) = self.file_index.get(&name) {
            return index;
        }
        self.files.push(name.clone());
        let index = (self.files.len() - 1) as u32;
        self.file_index.insert(name, index);
        index
    }
}

fn read_unit<'a>(
    dwarf: &gimli::Dwarf<Reader<'a>>,
    unit: &gimli::Unit<Reader<'a>>,
    files: &FxHashMap<String, String>,
    symbols: &FxHashMap<u64, String>,
    table: &mut Table,
) -> gimli::Result<()> {
    // The unit's files, as a panic names them, in the table's list.
    let mut unit_files: FxHashMap<u64, u32> = FxHashMap::default();
    let mut lines: Vec<(u64, u64, u32)> = Vec::new();
    if let Some(program) = unit.line_program.clone() {
        let header = program.header().clone();
        let mut index = 0;
        loop {
            // DWARF 4 counts files from 1, and 5 from 0.
            if let Some(entry) = header.file(index) {
                let name = file_name(dwarf, unit, &header, entry, files)?;
                let at = table.file(name);
                unit_files.insert(index, at);
            } else if index > 0 {
                break;
            }
            index += 1;
        }
        let mut rows = program.rows();
        while let Some((_, row)) = rows.next_row()? {
            let line = row.line().map_or(0, |line| line.get() as u32);
            // The end of a sequence is the end of the code before it, and
            // a row of no line is code the compiler wrote on its own: the
            // row before it goes on, as Cranelift's table has it.
            if row.end_sequence() || line == 0 {
                continue;
            }
            lines.push((row.address(), row.file_index(), line));
        }
    }
    lines.sort_by_key(|&(address, ..)| address);
    let file_of = |index: u64| unit_files.get(&index).copied().unwrap_or(0);

    let mut entries = unit.entries();
    // The inlined calls open at each depth, by their index in the table's
    // list, and the function open, if one is.
    let mut open: Vec<(isize, u32)> = Vec::new();
    let mut function: Option<(isize, Function, Vec<Call>)> = None;
    while let Some(entry) = entries.next_dfs()? {
        let depth = entry.depth();
        open.retain(|&(at, _)| at < depth);
        if function.as_ref().is_some_and(|(at, ..)| *at >= depth) {
            let (_, done, calls) = function.take().expect("a function is open");
            finish(table, done, &calls, &lines, &file_of);
        }
        match entry.tag() {
            gimli::DW_TAG_subprogram => {
                let Some(AttributeValue::Addr(_)) = entry.attr_value(gimli::DW_AT_low_pc) else {
                    continue;
                };
                let mut ranges = dwarf.die_ranges(unit, entry)?;
                let Some(range) = ranges.next()? else {
                    continue;
                };
                let Some(symbol) = symbols.get(&range.begin) else {
                    // No symbol at the code's start: not a function the
                    // program can name.
                    continue;
                };
                let (name, pass_over) = described(dwarf, unit, entry, &file_of, table)?;
                function = Some((
                    depth,
                    Function {
                        symbol: symbol.clone(),
                        name,
                        low: range.begin,
                        high: range.end,
                        pass_over,
                    },
                    Vec::new(),
                ));
            }
            gimli::DW_TAG_inlined_subroutine if function.is_some() => {
                let (name, pass_over) = described(dwarf, unit, entry, &file_of, table)?;
                let mut ranges = Vec::new();
                let mut iter = dwarf.die_ranges(unit, entry)?;
                while let Some(range) = iter.next()? {
                    ranges.push((range.begin, range.end));
                }
                let call_file = file_index(entry, gimli::DW_AT_call_file).map_or(0, &file_of);
                let call_line = entry
                    .attr_value(gimli::DW_AT_call_line)
                    .and_then(|value| value.udata_value())
                    .unwrap_or(0) as u32;
                let record = table.inlined(Inlined {
                    name,
                    pass_over,
                    file: call_file,
                    line: call_line,
                    parent: open.last().map_or(0, |&(_, index)| index),
                });
                open.push((depth, record));
                if let Some((_, _, calls)) = function.as_mut() {
                    calls.push(Call { record, ranges });
                }
            }
            _ => {}
        }
    }
    if let Some((_, done, calls)) = function.take() {
        finish(table, done, &calls, &lines, &file_of);
    }
    Ok(())
}

/// A file's name as a panic gives it, from the line program's entry.
fn file_name<'a>(
    dwarf: &gimli::Dwarf<Reader<'a>>,
    unit: &gimli::Unit<Reader<'a>>,
    header: &gimli::LineProgramHeader<Reader<'a>>,
    entry: &gimli::FileEntry<Reader<'a>>,
    files: &FxHashMap<String, String>,
) -> gimli::Result<String> {
    let name = dwarf.attr_string(unit, entry.path_name())?;
    let name = name.to_string_lossy()?.into_owned();
    if let Some(shown) = files.get(&name) {
        return Ok(shown.clone());
    }
    let directory = match entry.directory(header) {
        Some(directory) => dwarf
            .attr_string(unit, directory)?
            .to_string_lossy()?
            .into_owned(),
        None => String::new(),
    };
    let path = format!("{directory}/{name}");
    Ok(files.get(&path).cloned().unwrap_or(name))
}

/// A subprogram's name, and whether a panic's calls pass over it: code the
/// compiler wrote on its own, or the runtime's, which tells the panic. An
/// entry that is a call inlined, or a function also inlined elsewhere, is
/// described by the one it is an instance of.
fn described<'a>(
    dwarf: &gimli::Dwarf<Reader<'a>>,
    unit: &gimli::Unit<Reader<'a>>,
    entry: &gimli::DebuggingInformationEntry<Reader<'a>>,
    file_of: &impl Fn(u64) -> u32,
    table: &Table,
) -> gimli::Result<(String, bool)> {
    let mut entry = entry.clone();
    loop {
        let origin = entry
            .attr_value(gimli::DW_AT_abstract_origin)
            .or_else(|| entry.attr_value(gimli::DW_AT_specification));
        match origin {
            Some(AttributeValue::UnitRef(offset)) => entry = unit.entry(offset)?,
            _ => break,
        }
    }
    let name = match entry.attr_value(gimli::DW_AT_name) {
        Some(value) => dwarf
            .attr_string(unit, value)?
            .to_string_lossy()?
            .into_owned(),
        None => String::new(),
    };
    let artificial = matches!(
        entry.attr_value(gimli::DW_AT_artificial),
        Some(AttributeValue::Flag(true))
    );
    let runtime = file_index(&entry, gimli::DW_AT_decl_file)
        .is_some_and(|index| table.files[file_of(index) as usize] == RUNTIME);
    Ok((name, artificial || runtime))
}

/// The line program's index of the file an attribute names.
fn file_index(
    entry: &gimli::DebuggingInformationEntry<Reader<'_>>,
    at: gimli::DwAt,
) -> Option<u64> {
    match entry.attr_value(at)? {
        AttributeValue::FileIndex(index) => Some(index),
        value => value.udata_value(),
    }
}

/// A function's rows: where the line or the call inlined changes, the
/// line, and the innermost call inlined there.
fn finish(
    table: &mut Table,
    function: Function,
    calls: &[Call],
    lines: &[(u64, u64, u32)],
    file_of: &impl Fn(u64) -> u32,
) {
    let (low, high) = (function.low, function.high);
    let mut points: Vec<u64> = vec![low];
    let first = lines.partition_point(|&(address, ..)| address < low);
    points.extend(
        lines[first..]
            .iter()
            .map(|&(address, ..)| address)
            .take_while(|&address| address < high),
    );
    for call in calls {
        for &(begin, end) in &call.ranges {
            points.extend(
                [begin, end]
                    .into_iter()
                    .filter(|&at| at >= low && at < high),
            );
        }
    }
    points.sort_unstable();
    points.dedup();
    let mut rows: Vec<Row> = Vec::new();
    for at in points {
        let line = lines.partition_point(|&(address, ..)| address <= at);
        if line == 0 || lines[line - 1].0 < low {
            continue;
        }
        let (_, file, line) = lines[line - 1];
        // The innermost: a call inlined comes after the one it is in.
        let inlined = calls
            .iter()
            .rev()
            .find(|call| {
                call.ranges
                    .iter()
                    .any(|&(begin, end)| begin <= at && at < end)
            })
            .map_or(0, |call| call.record);
        let row = Row {
            offset: (at - low) as u32,
            file: file_of(file),
            line,
            inlined,
        };
        if rows.last().is_some_and(|last| {
            (last.file, last.line, last.inlined) == (row.file, row.line, row.inlined)
        }) {
            continue;
        }
        rows.push(row);
    }
    table.functions.push((function, rows));
}

/// The name of the list of the tables, which the runtime reads.
const TABLES: &str = "wip.frame_tables";

/// What the list of tables and this table are, in order: words, addresses,
/// rows' `u32`s and names' bytes.
enum Item {
    Word(u64),
    Address(Target),
    U32(u32),
    Text(Vec<u8>),
}

/// What an address is of.
enum Target {
    /// A function, by its symbol.
    Function(String),
    /// A place in the list and the table, by its distance from their start.
    Here(u64),
}

impl Table {
    /// The list of tables, `wip.frame_tables`, which holds this one, then
    /// this one: its functions, files, calls inlined, rows and names.
    fn layout(&self) -> Vec<Item> {
        // Where each part begins: the list, two words; the table, three
        // words and seven for each function; two words for each file, six
        // for each call inlined, sixteen bytes for each row, and the names.
        let table = 16;
        let files = table + 24 + 56 * self.functions.len() as u64;
        let inlined = files + 16 * self.files.len() as u64;
        let rows = inlined + 48 * self.inlined.len() as u64;
        let row_count: usize = self.functions.iter().map(|(_, rows)| rows.len()).sum();
        let names_at = rows + 16 * row_count as u64;
        // Each name once, however many say it.
        let mut names: Vec<u8> = Vec::new();
        let mut named: FxHashMap<String, u64> = FxHashMap::default();
        let mut name = |text: &str| {
            let at = *named.entry(text.to_string()).or_insert_with(|| {
                names.extend_from_slice(text.as_bytes());
                names_at + (names.len() - text.len()) as u64
            });
            Item::Address(Target::Here(at))
        };

        let mut items = vec![
            Item::Word(1),
            Item::Address(Target::Here(table)),
            Item::Word(self.functions.len() as u64),
            Item::Address(Target::Here(files)),
            Item::Address(Target::Here(inlined)),
        ];
        let mut row_at = 0u64;
        for (function, function_rows) in &self.functions {
            items.push(Item::Address(Target::Function(function.symbol.clone())));
            items.push(Item::Word(function.high - function.low));
            items.push(name(&function.name));
            items.push(Item::Word(function.name.len() as u64));
            items.push(Item::Address(Target::Here(rows + row_at * 16)));
            items.push(Item::Word(function_rows.len() as u64));
            items.push(Item::Word(u64::from(function.pass_over)));
            row_at += function_rows.len() as u64;
        }
        for file in &self.files {
            items.push(name(file));
            items.push(Item::Word(file.len() as u64));
        }
        // A call inlined: what was called, its name's length, the file and
        // line of the call, the call it is in, and whether it is passed
        // over.
        for call in &self.inlined {
            items.push(name(&call.name));
            items.push(Item::Word(call.name.len() as u64));
            items.push(Item::Word(u64::from(call.file)));
            items.push(Item::Word(u64::from(call.line)));
            items.push(Item::Word(u64::from(call.parent)));
            items.push(Item::Word(u64::from(call.pass_over)));
        }
        for (_, function_rows) in &self.functions {
            for row in function_rows {
                items.extend([row.offset, row.file, row.line, row.inlined].map(Item::U32));
            }
        }
        items.push(Item::Text(names));
        items
    }
}

/// The table as assembly, in the program's data.
fn assembly(items: &[Item], macho: bool) -> String {
    let symbol = |name: &str| match macho {
        true => format!("\"_{name}\""),
        false => format!("\"{name}\""),
    };
    let tables = symbol(TABLES);
    let mut out = String::from("\n");
    match macho {
        true => {
            out.push_str("\t.section\t__DATA,__const\n");
            let _ = writeln!(out, "\t.private_extern\t{tables}");
        }
        false => {
            out.push_str("\t.section\t.data.rel.ro,\"aw\",@progbits\n");
            let _ = writeln!(out, "\t.hidden\t{tables}");
        }
    }
    let _ = writeln!(out, "\t.globl\t{tables}\n\t.p2align\t3\n{tables}:");
    for item in items {
        let _ = match item {
            Item::Word(word) => writeln!(out, "\t.quad\t{word}"),
            // A function's symbol is as the object names it, `_` and all.
            Item::Address(Target::Function(name)) => writeln!(out, "\t.quad\t\"{name}\""),
            Item::Address(Target::Here(at)) => writeln!(out, "\t.quad\t{tables}+{at}"),
            Item::U32(word) => writeln!(out, "\t.long\t{word}"),
            Item::Text(text) => writeln!(out, "\t.ascii\t\"{}\"", ascii(text)),
        };
    }
    out
}

/// `object` with the table added: a section of its own, relocated, and the
/// symbol of the list of tables, kept to the program.
fn elf_with(object: &[u8], items: &[Item]) -> Result<Vec<u8>, String> {
    use object::build::elf::{Builder, Relocation, SectionData};
    use object::elf;

    let mut builder = Builder::read(object).map_err(|err| err.to_string())?;
    let absolute = match builder.header.e_machine {
        elf::EM_AARCH64 => elf::R_AARCH64_ABS64,
        elf::EM_X86_64 => elf::R_X86_64_64,
        machine => return Err(format!("no relocation for an address on machine {machine}")),
    };
    // LLVM's objects keep the sections' names with the symbols' in
    // `.strtab`, and the builder writes them in a table of their own.
    if !builder
        .sections
        .iter()
        .any(|section| matches!(section.data, SectionData::SectionString))
    {
        let names = builder.sections.add();
        names.name = b".shstrtab".as_slice().into();
        names.sh_type = elf::SHT_STRTAB;
        names.sh_addralign = 1;
        names.data = SectionData::SectionString;
    }
    let symtab = builder
        .sections
        .iter()
        .find(|section| matches!(section.data, SectionData::Symbol))
        .map(|section| section.id())
        .ok_or("an object with no symbol table")?;
    let functions: FxHashMap<Vec<u8>, _> = builder
        .symbols
        .iter()
        .map(|symbol| (symbol.name.to_vec(), symbol.id()))
        .collect();

    let mut bytes: Vec<u8> = Vec::new();
    let mut addresses: Vec<(u64, &Target)> = Vec::new();
    for item in items {
        match item {
            Item::Word(word) => bytes.extend_from_slice(&word.to_le_bytes()),
            Item::Address(target) => {
                addresses.push((bytes.len() as u64, target));
                bytes.extend_from_slice(&0u64.to_le_bytes());
            }
            Item::U32(word) => bytes.extend_from_slice(&word.to_le_bytes()),
            Item::Text(text) => bytes.extend_from_slice(text),
        }
    }
    let size = bytes.len() as u64;

    let section = builder.sections.add();
    section.name = b".data.rel.ro.wip.frames".as_slice().into();
    section.sh_type = elf::SHT_PROGBITS;
    section.sh_flags = u64::from(elf::SHF_ALLOC | elf::SHF_WRITE);
    section.sh_addralign = 8;
    section.data = SectionData::Data(bytes.into());
    let table = section.id();

    let symbol = builder.symbols.add();
    symbol.name = TABLES.as_bytes().into();
    symbol.section = Some(table);
    symbol.set_st_info(elf::STB_GLOBAL, elf::STT_OBJECT);
    symbol.st_other = elf::STV_HIDDEN;
    symbol.st_size = size;
    let tables = symbol.id();

    let mut relocations = Vec::with_capacity(addresses.len());
    for (offset, target) in addresses {
        let (symbol, addend) = match target {
            Target::Function(name) => (
                *functions
                    .get(name.as_bytes())
                    .ok_or_else(|| format!("no symbol `{name}` in the object"))?,
                0,
            ),
            Target::Here(at) => (tables, *at as i64),
        };
        relocations.push(Relocation {
            r_offset: offset,
            symbol: Some(symbol),
            r_type: absolute,
            r_addend: addend,
        });
    }
    let rela = builder.sections.add();
    rela.name = b".rela.data.rel.ro.wip.frames".as_slice().into();
    rela.sh_type = elf::SHT_RELA;
    rela.sh_flags = u64::from(elf::SHF_INFO_LINK);
    rela.sh_link_section = Some(symtab);
    rela.sh_info_section = Some(table);
    rela.sh_addralign = 8;
    rela.sh_entsize = 24;
    rela.data = SectionData::Relocation(relocations);

    let mut out = Vec::new();
    builder.write(&mut out).map_err(|err| err.to_string())?;
    Ok(out)
}

/// Bytes as an assembler's `.ascii` takes them.
fn ascii(text: &[u8]) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text {
        match byte {
            b'"' | b'\\' => {
                out.push('\\');
                out.push(byte as char);
            }
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\{byte:03o}");
            }
        }
    }
    out
}
