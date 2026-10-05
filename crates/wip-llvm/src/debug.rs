//! Debug information as LLVM's metadata: a compile unit,
//! a file for each file the program's code was written in, a subprogram
//! for each function, and for each instruction the place it was written
//! at. LLVM writes it as DWARF — the lines and functions Cranelift's code
//! carries too — and keeps it through its inlining, so that the
//! DWARF says which calls were inlined where. The table a panic reads its
//! calls from is made from that DWARF (`frames.rs`).
//!
//! The module's other metadata — a branch's weights, and what a load of a
//! reference may assume — is numbered here too, since all
//! of it is numbered as one.

use std::fmt::Write as _;

use rustc_hash::FxHashMap;
use wip_syntax::Span;

use crate::{Locations, escape};

/// A function's subprogram, and the file it was written in.
#[derive(Clone, Copy)]
pub(crate) struct Subprogram {
    pub node: usize,
    pub file: usize,
}

pub(crate) struct Debug<'p> {
    locations: Locations<'p>,
    comp_dir: String,
    /// The metadata, node `i` being `!i`.
    nodes: Vec<String>,
    unit: usize,
    unit_file: usize,
    subroutine: usize,
    flags: [usize; 2],
    files: FxHashMap<String, usize>,
    /// Each file as the DWARF names it, with the name a panic gives it.
    pub paths: FxHashMap<String, String>,
    /// A subprogram's places in a file other than its own.
    scopes: FxHashMap<(usize, usize), usize>,
    places: FxHashMap<(usize, u32, u32), usize>,
    constants: FxHashMap<String, usize>,
}

impl<'p> Debug<'p> {
    pub(crate) fn new(locations: Locations<'p>) -> Self {
        let comp_dir = std::env::current_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default();
        let mut debug = Debug {
            locations,
            comp_dir,
            nodes: Vec::new(),
            unit: 0,
            unit_file: 0,
            subroutine: 0,
            flags: [0; 2],
            files: FxHashMap::default(),
            paths: FxHashMap::default(),
            scopes: FxHashMap::default(),
            places: FxHashMap::default(),
            constants: FxHashMap::default(),
        };
        debug.unit_file = debug.file("wip");
        debug.unit = debug.node(format!(
            "distinct !DICompileUnit(language: DW_LANG_C99, file: !{}, producer: \"wip\", \
             isOptimized: true, runtimeVersion: 0, emissionKind: FullDebug)",
            debug.unit_file
        ));
        debug.subroutine = debug.node("!DISubroutineType(types: !{})".to_string());
        // DWARF 4, as Cranelift's code has.
        debug.flags = [
            debug.node("!{i32 7, !\"Dwarf Version\", i32 4}".to_string()),
            debug.node("!{i32 2, !\"Debug Info Version\", i32 3}".to_string()),
        ];
        debug
    }

    fn node(&mut self, text: String) -> usize {
        self.nodes.push(text);
        self.nodes.len() - 1
    }

    /// A file by the name a panic gives it, named in the DWARF as the file
    /// system names it, as Cranelift's DWARF names it.
    fn file(&mut self, name: &str) -> usize {
        if let Some(&node) = self.files.get(name) {
            return node;
        }
        let path = path_of(name);
        let node = self.node(format!(
            "!DIFile(filename: \"{}\", directory: \"{}\")",
            escape(path.as_bytes()),
            escape(self.comp_dir.as_bytes())
        ));
        self.paths.insert(path, name.to_string());
        self.files.insert(name.to_string(), node);
        node
    }

    /// The subprogram of a function named `name` in a panic's calls and the
    /// debugger, declared at `at`; code the compiler writes on its own has
    /// no place, and is marked artificial, as a panic's calls pass over it.
    /// No linkage name: a debugger would show the symbol for the name, and
    /// the table finds each function's symbol in the object.
    pub(crate) fn subprogram(&mut self, name: &str, at: Option<Span>) -> Subprogram {
        let (file, line, flags) = match at {
            Some(at) => {
                let (file, line, _) = (self.locations)(at);
                (self.file(&file), line, "")
            }
            None => (self.unit_file, 0, "flags: DIFlagArtificial, "),
        };
        let node = self.node(format!(
            "distinct !DISubprogram(name: \"{}\", scope: !{file}, file: !{file}, \
             line: {line}, type: !{}, scopeLine: {line}, {flags}\
             spFlags: DISPFlagDefinition | DISPFlagOptimized, unit: !{})",
            escape(name.as_bytes()),
            self.subroutine,
            self.unit
        ));
        Subprogram { node, file }
    }

    /// The location of code written at `span` in `subprogram`, in `block`
    /// where it is in the function's own file and a variable's scope holds
    /// it; nothing when the span says no line.
    pub(crate) fn location(
        &mut self,
        subprogram: Subprogram,
        span: Span,
        block: Option<usize>,
    ) -> Option<usize> {
        let (file, line, column) = (self.locations)(span);
        if line == 0 {
            return None;
        }
        let file = self.file(&file);
        // Code spliced from a function `@inline` names is
        // the function's own, at the line it was written at, which may be
        // in another file.
        let scope = match file == subprogram.file {
            true => block.unwrap_or(subprogram.node),
            false => match self.scopes.get(&(subprogram.node, file)) {
                Some(&scope) => scope,
                None => {
                    let scope = self.node(format!(
                        "!DILexicalBlockFile(scope: !{}, file: !{file}, discriminator: 0)",
                        subprogram.node
                    ));
                    self.scopes.insert((subprogram.node, file), scope);
                    scope
                }
            },
        };
        Some(self.place(scope, line, column))
    }

    /// The location that says no line, in `subprogram`: code the compiler
    /// wrote on its own.
    pub(crate) fn no_line(&mut self, subprogram: Subprogram) -> usize {
        self.place(subprogram.node, 0, 0)
    }

    fn place(&mut self, scope: usize, line: u32, column: u32) -> usize {
        if let Some(&node) = self.places.get(&(scope, line, column)) {
            return node;
        }
        let node = self.node(format!(
            "!DILocation(line: {line}, column: {column}, scope: !{scope})"
        ));
        self.places.insert((scope, line, column), node);
        node
    }

    /// The weights of a branch to a panic: taken once in two thousand,
    /// which keeps the panic's code out of the way of the rest.
    pub(crate) fn unlikely(&mut self) -> usize {
        self.constant("!{!\"branch_weights\", i32 1, i32 2000}")
    }

    /// A node of its own, numbered now and written by `fill`, for what
    /// refers to itself: a struct whose field points back to it.
    pub(crate) fn reserve(&mut self) -> usize {
        self.node(String::new())
    }

    /// What a node `reserve` numbered says.
    pub(crate) fn fill(&mut self, node: usize, text: String) {
        self.nodes[node] = text;
    }

    /// A node of its own, written now.
    pub(crate) fn add(&mut self, text: String) -> usize {
        self.node(text)
    }

    /// Where a span was written: its file's node, its line and its column.
    pub(crate) fn at(&mut self, span: Span) -> (usize, u32, u32) {
        let (file, line, column) = (self.locations)(span);
        (self.file(&file), line, column)
    }

    /// A node that is only what it says, as `!{i64 8}`, once however often
    /// it is asked for.
    pub(crate) fn constant(&mut self, text: &str) -> usize {
        if let Some(&node) = self.constants.get(text) {
            return node;
        }
        let node = self.node(text.to_string());
        self.constants.insert(text.to_string(), node);
        node
    }

    /// The module's metadata, for the end of its text.
    pub(crate) fn write(&self, out: &mut String) {
        let _ = writeln!(out, "!llvm.dbg.cu = !{{!{}}}", self.unit);
        let _ = writeln!(
            out,
            "!llvm.module.flags = !{{!{}, !{}}}",
            self.flags[0], self.flags[1]
        );
        for (i, node) in self.nodes.iter().enumerate() {
            let _ = writeln!(out, "!{i} = {node}");
        }
    }
}

/// A file as the debugger is to find it: where it is on this machine, if it
/// is there, and as it is named otherwise — the standard library's files
/// live in the compiler.
fn path_of(name: &str) -> String {
    std::fs::canonicalize(name).map_or_else(|_| name.to_string(), |path| path.display().to_string())
}
