//! Parsing and checking a file or a loaded program: every phase before
//! code is generated.

use super::*;

pub struct SourceFile {
    /// The name shown in diagnostics.
    pub name: String,
    pub text: String,
}

impl SourceFile {
    pub fn read(path: &Path) -> Result<SourceFile, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("cannot read `{}`: {err}", path.display()))?;
        // Spans are u32 byte offsets.
        if u32::try_from(text.len()).is_err() {
            return Err(format!("`{}` is larger than 4 GiB", path.display()));
        }
        Ok(SourceFile {
            name: path.display().to_string(),
            text,
        })
    }
}

pub struct ParsedFile {
    pub ast: Ast,
    pub interner: Interner,
    /// Lexer and parser diagnostics together, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_file(file: &SourceFile) -> ParsedFile {
    parse_timed(file, &mut Timings::default())
}

fn parse_timed(file: &SourceFile, timings: &mut Timings) -> ParsedFile {
    let mut interner = Interner::new();
    let lexed = timings.time("lex", || lex_at(&file.text, 0, &mut interner));
    let parsed = timings.time("parse", || parse_at(&file.text, 0, &lexed));
    let mut diagnostics = lexed.diagnostics;
    diagnostics.extend(parsed.diagnostics);
    diagnostics.sort_by_key(|d| d.primary.span.lo);
    ParsedFile {
        ast: parsed.ast,
        interner,
        diagnostics,
    }
}

pub struct CheckedFile {
    pub ast: Ast,
    pub interner: Interner,
    pub program: Program,
    /// Diagnostics from every phase, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes, parses and type-checks a file.
///
/// Type errors inside an item that also has a syntax error are dropped: they
/// are almost always consequences of how the parser recovered, not separate
/// mistakes.
pub fn check_file(file: &SourceFile) -> CheckedFile {
    check_timed(file, &mut Timings::default())
}

fn check_timed(file: &SourceFile, timings: &mut Timings) -> CheckedFile {
    let mut parsed = parse_timed(file, timings);
    // What this target does not take is dropped before it is checked, not
    // before it is parsed: `wip parse` shows the file as written.
    let mut diagnostics = parsed.diagnostics;
    diagnostics.extend(targets::keep(
        &mut parsed.ast,
        &parsed.interner,
        targets::Target::host(),
    ));
    let mut parsed = ParsedFile {
        diagnostics,
        ..parsed
    };
    let lowered = timings.time("type check", || {
        wip_hir::lower_file(&parsed.ast, &parsed.interner)
    });
    let inside = |span: Span, pos: u32| span.lo <= pos && pos <= span.hi;
    // An item the parser reported an error in is not checked further: what
    // it would say follows from the error. A warning breaks nothing, and the
    // item's own errors are its own (E0126 is a warning, and the code under
    // it may still be wrong).
    let broken: Vec<Span> = item_spans(&parsed.ast)
        .filter(|&item| {
            parsed
                .diagnostics
                .iter()
                .any(|d| d.is_error() && inside(item, d.primary.span.lo))
        })
        .collect();
    let mut diagnostics = parsed.diagnostics;
    let type_diags: Vec<_> = lowered
        .diagnostics
        .into_iter()
        .filter(|d| !broken.iter().any(|&item| inside(item, d.primary.span.lo)))
        .collect();
    diagnostics.extend(type_diags);

    let mut program = lowered.program;
    if !diagnostics.iter().any(Diagnostic::is_error) {
        let analysis_diags = timings.time("move check", || {
            wip_analysis::check(&program, &parsed.interner)
        });
        diagnostics.extend(analysis_diags);
    }
    if !diagnostics.iter().any(Diagnostic::is_error) {
        let instance_diags = timings.time("instances", || {
            wip_hir::instantiate(&mut program, &parsed.interner)
        });
        diagnostics.extend(instance_diags);
        // What each generator keeps between calls, now that every one
        // is known.
        wip_mir::generator_frames(&mut program, &parsed.interner);
        let sources = Sources::single(file.name.clone(), file.text.clone());
        let evaluated = timings.time("constants", || {
            wip_mir::evaluate_constants(&mut program, &mut parsed.interner, &|span| {
                sources.locate(span)
            })
        });
        diagnostics.extend(evaluated);
    }
    diagnostics.sort_by_key(|d| d.primary.span.lo);
    CheckedFile {
        ast: parsed.ast,
        interner: parsed.interner,
        program,
        diagnostics,
    }
}

fn item_spans(ast: &Ast) -> impl Iterator<Item = Span> + '_ {
    ast.items.iter().map(|item| match item {
        Item::Struct(s) => s.span,
        Item::Enum(e) => e.span,
        Item::Fn(f) => f.span,
        Item::Val(v) => v.span,
        Item::Assert(a) => a.span,
        Item::Extern(e) => e.span,
        Item::Import(i) => i.span,
        Item::Extend(b) => b.span,
        Item::Interface(i) => i.span,
        Item::Type(t) => t.span,
    })
}

/// Runs every phase implemented so far and returns all their diagnostics.
pub fn check(file: &SourceFile) -> Vec<Diagnostic> {
    check_file(file).diagnostics
}

/// A checked program: every module, and everything the phases reported.
pub struct Checked {
    pub program: Program,
    pub diagnostics: Vec<Diagnostic>,
}

/// Type-checks and move-checks a loaded program, and
/// works out the constants that are run, whose strings the
/// interner takes.
pub fn check_loaded(loaded: &mut Loaded, timings: &mut Timings) -> Checked {
    let modules: Vec<wip_hir::ModuleAst<'_>> =
        loaded.modules.iter().map(LoadedModule::syntax).collect();
    let lowered = timings.time("type check", || wip_hir::lower(&modules, &loaded.interner));
    // A type error inside an item that also has a syntax error is almost
    // always a consequence of the recovery, not a separate mistake.
    let inside = |span: Span, pos: u32| span.lo <= pos && pos <= span.hi;
    let broken: Vec<Span> = loaded
        .modules
        .iter()
        .flat_map(|module| module.files.iter())
        .flat_map(|file| item_spans(&file.ast))
        .filter(|&item| {
            loaded
                .diagnostics
                .iter()
                .any(|d| d.is_error() && inside(item, d.primary.span.lo))
        })
        .collect();
    let mut diagnostics = loaded.diagnostics.clone();
    diagnostics.extend(
        lowered
            .diagnostics
            .into_iter()
            .filter(|d| !broken.iter().any(|&item| inside(item, d.primary.span.lo))),
    );
    let mut program = lowered.program;
    if !diagnostics.iter().any(Diagnostic::is_error) {
        let analysis = timings.time("move check", || {
            wip_analysis::check(&program, &loaded.interner)
        });
        diagnostics.extend(analysis);
    }
    // Instances of generic functions, once they are known to be correct.
    if !diagnostics.iter().any(Diagnostic::is_error) {
        let instances = timings.time("instances", || {
            wip_hir::instantiate(&mut program, &loaded.interner)
        });
        diagnostics.extend(instances);
        // What each generator keeps between calls, now that every one
        // is known.
        wip_mir::generator_frames(&mut program, &loaded.interner);
        let Loaded {
            sources, interner, ..
        } = loaded;
        let evaluated = timings.time("constants", || {
            wip_mir::evaluate_constants(&mut program, interner, &|span| sources.locate(span))
        });
        diagnostics.extend(evaluated);
    }
    diagnostics.sort_by_key(|d| d.primary.span.lo);
    Checked {
        program,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every item the compiler knows the prelude by is declared there, by
    /// the name the compiler looks for: a renamed one would otherwise be
    /// found by nothing, and what it is for would stop working quietly.
    #[test]
    fn the_prelude_declares_what_the_compiler_knows_it_by() {
        let source = SourceFile {
            name: "main.wip".to_string(),
            text: "fn main() = {}\n".to_string(),
        };
        let mut loaded = load_source(&source, &mut Timings::default()).expect("loaded");
        let checked = check_loaded(&mut loaded, &mut Timings::default());
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let items = &checked.program.prelude_items;
        for &known in wip_hir::KnownInterface::ALL {
            assert!(
                items.interface(known).is_some(),
                "interface {}",
                known.name()
            );
        }
        for &known in wip_hir::KnownEnum::ALL {
            assert!(items.enumeration(known).is_some(), "enum {}", known.name());
        }
        for &known in wip_hir::KnownStruct::ALL {
            assert!(items.structure(known).is_some(), "struct {}", known.name());
        }
        for &known in wip_hir::KnownFn::ALL {
            assert!(items.function(known).is_some(), "function {}", known.name());
        }
    }
}
