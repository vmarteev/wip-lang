//! The program's table of functions and lines, which a panic reads to say
//! the calls that led to it. Each module's object
//! holds its own table, and the prelude's the list of all of them, so the
//! runtime finds every module's functions without reading DWARF.
//!
//! A module's table is words: how many functions, where its files' names
//! are, where its calls inlined are, and then for each function its code,
//! the length of its code, its name and the name's length, its rows and how
//! many, and whether a panic's calls pass over it: the runtime's own
//! functions, which tell the panic, and the ones the compiler writes on its
//! own. A row is four `u32`s: where in the function a run of code begins,
//! which file, the line, and the call inlined the code is in, counting from
//! 1, or 0; a run on the same line as the one before it is the same row. A
//! file is its name and the name's length.
//!
//! Cranelift's code has no calls inlined but the MIR's, which are the
//! function they were spliced into, so its rows say 0 and
//! its list of them is empty. The LLVM backend's table has them.

use super::*;
use wip_syntax::Span;

/// A module's table, by the module's index in the program.
fn table_name(module: usize) -> String {
    format!("wip_frames_{module}")
}

/// The list of every module's table, which the prelude's object holds.
const TABLES: &str = "wip_frame_tables";

/// The words a function takes in its module's table.
const ENTRY_WORDS: usize = 7;

/// The words before the functions in a module's table.
const HEADER_WORDS: usize = 3;

const ROW_BYTES: usize = 16;

/// The runtime's own file: its functions are how a panic is told, not
/// where it happened.
const RUNTIME: &str = "std/prelude/runtime.wip";

impl Codegen<'_> {
    /// Writes this module's table from the functions defined so far, and
    /// the list of all of them where this is the prelude's object.
    pub(super) fn define_frame_tables(&mut self) {
        self.define_frame_table();
        if self.program.modules[self.this_module as usize] == wip_hir::PRELUDE {
            self.define_table_list();
        }
    }

    fn define_frame_table(&mut self) {
        let locations = self.locations;
        let mut files: Vec<String> = Vec::new();
        let mut file_index: FxHashMap<String, u32> = FxHashMap::default();
        let mut rows: Vec<u8> = Vec::new();
        // Where each function's rows start in `rows`, and how many it has.
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for function in &self.described {
            let start = rows.len();
            let mut last = None;
            for &(offset, lo) in &function.rows {
                let (file, line, _) = locations(Span::at(lo));
                if line == 0 || last.as_ref() == Some(&(file.clone(), line)) {
                    continue;
                }
                let index = *file_index.entry(file.clone()).or_insert_with(|| {
                    files.push(file.clone());
                    (files.len() - 1) as u32
                });
                for word in [offset, index, line, 0] {
                    rows.extend_from_slice(&word.to_le_bytes());
                }
                last = Some((file, line));
            }
            runs.push((start, (rows.len() - start) / ROW_BYTES));
        }

        let rows_id = self.local_data("wip.frames.rows", rows);
        let mut bytes = vec![0u8; files.len().max(1) * 16];
        for (i, file) in files.iter().enumerate() {
            put(&mut bytes, i * 16 + 8, file.len() as u64);
        }
        let mut names = DataDescription::new();
        names.define(bytes.into_boxed_slice());
        for (i, file) in files.iter().enumerate() {
            let text = self.text_data(file);
            let text = self.module.declare_data_in_data(text, &mut names);
            names.write_data_addr((i * 16) as u32, text, 0);
        }
        let files_id = self.define_local("wip.frames.files", names);

        let described = std::mem::take(&mut self.described);
        let count = described.len();
        // No calls inlined: the list's address is null.
        let mut bytes = vec![0u8; (HEADER_WORDS + count * ENTRY_WORDS) * 8];
        put(&mut bytes, 0, count as u64);
        for (i, (function, &(_, rows_count))) in described.iter().zip(&runs).enumerate() {
            let at = (HEADER_WORDS + i * ENTRY_WORDS) * 8;
            put(&mut bytes, at + 8, u64::from(function.size));
            put(&mut bytes, at + 24, function.name.len() as u64);
            put(&mut bytes, at + 40, rows_count as u64);
            let runtime = function.at.is_some_and(|at| locations(at).0 == RUNTIME);
            put(&mut bytes, at + 48, u64::from(function.glue || runtime));
        }
        let mut table = DataDescription::new();
        table.define(bytes.into_boxed_slice());
        let files_gv = self.module.declare_data_in_data(files_id, &mut table);
        table.write_data_addr(8, files_gv, 0);
        let rows_gv = self.module.declare_data_in_data(rows_id, &mut table);
        for (i, (function, &(start, _))) in described.iter().zip(&runs).enumerate() {
            let at = (HEADER_WORDS + i * ENTRY_WORDS) * 8;
            let func = self.module.declare_func_in_data(function.func, &mut table);
            table.write_function_addr(at as u32, func);
            let name = self.text_data(&function.name);
            let name = self.module.declare_data_in_data(name, &mut table);
            table.write_data_addr((at + 16) as u32, name, 0);
            table.write_data_addr((at + 32) as u32, rows_gv, start as i64);
        }
        self.described = described;
        table.set_align(8);
        let id = self
            .module
            .declare_data(
                &table_name(self.this_module as usize),
                Linkage::Export,
                false,
                false,
            )
            .expect("a module's table is declared once");
        self.module
            .define_data(id, &table)
            .expect("a module's table is defined once");
    }

    /// The list of every module's table, in the prelude's object: how many,
    /// and the address of each.
    fn define_table_list(&mut self) {
        let modules = self.program.modules.len();
        let mut bytes = vec![0u8; (1 + modules) * 8];
        put(&mut bytes, 0, modules as u64);
        let mut list = DataDescription::new();
        list.define(bytes.into_boxed_slice());
        for module in 0..modules {
            // This module's own table is declared already, as the others'
            // are declared here: each is a symbol the linker joins.
            let linkage = match module == self.this_module as usize {
                true => Linkage::Export,
                false => Linkage::Import,
            };
            let table = self
                .module
                .declare_data(&table_name(module), linkage, false, false)
                .expect("a module's table has one name");
            let table = self.module.declare_data_in_data(table, &mut list);
            list.write_data_addr(((1 + module) * 8) as u32, table, 0);
        }
        list.set_align(8);
        let id = self.frame_tables();
        self.module
            .define_data(id, &list)
            .expect("the list of tables is defined once");
    }

    /// The list of every module's table: defined in the prelude's object,
    /// where the runtime that reads it is, and taken from there by any
    /// other that uses it.
    pub(super) fn frame_tables(&mut self) -> DataId {
        if let Some(id) = self.frame_tables {
            return id;
        }
        let defines = self.program.modules[self.this_module as usize] == wip_hir::PRELUDE;
        let linkage = match defines {
            true => Linkage::Export,
            false => Linkage::Import,
        };
        let id = self
            .module
            .declare_data(TABLES, linkage, false, false)
            .expect("the list of tables is declared once");
        self.frame_tables = Some(id);
        id
    }

    /// A read-only object of this module holding `bytes`.
    fn local_data(&mut self, name: &str, bytes: Vec<u8>) -> DataId {
        let mut data = DataDescription::new();
        // An object holds a byte at least.
        let bytes = match bytes.is_empty() {
            true => vec![0u8; 4],
            false => bytes,
        };
        data.define(bytes.into_boxed_slice());
        data.set_align(8);
        self.define_local(name, data)
    }

    fn define_local(&mut self, name: &str, mut data: DataDescription) -> DataId {
        data.set_align(8);
        let id = self
            .module
            .declare_data(name, Linkage::Local, false, false)
            .expect("a table's part is declared once");
        self.module
            .define_data(id, &data)
            .expect("a table's part is defined once");
        id
    }
}

/// Writes a word into bytes being prepared for an object.
fn put(bytes: &mut [u8], at: usize, word: u64) {
    bytes[at..at + 8].copy_from_slice(&word.to_le_bytes());
}
