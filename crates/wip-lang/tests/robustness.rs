//! The front end must survive any input: no panics, no infinite loops, and
//! every diagnostic span inside the source on character boundaries.
//!
//! For every file in the corpus, this parses each prefix that ends at a token
//! boundary and each variant with one token deleted — the two most common
//! shapes of half-written code. The examples are checked whole, types and
//! all, with each line cut short, as an editor checks them while a line is
//! typed.

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use wip_lang::{
    Mode, Overlay, SourceFile, Timings, check_loaded, load_overlaid, parse_file, render,
};
use wip_syntax::{Diagnostic, Interner, Span, lex};

fn check_span(src: &str, span: Span, what: &str) {
    let (lo, hi) = (span.lo as usize, span.hi as usize);
    assert!(
        lo <= hi && hi <= src.len(),
        "{what} span {span:?} out of bounds in {src:?}"
    );
    assert!(
        src.is_char_boundary(lo) && src.is_char_boundary(hi),
        "{what} span {span:?} splits a character in {src:?}"
    );
}

fn check_diagnostics(src: &str, diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        check_span(src, d.primary.span, "primary");
        for label in &d.secondary {
            check_span(src, label.span, "secondary");
        }
        for edit in d.fix.iter().flat_map(|f| &f.edits) {
            check_span(src, edit.span, "fix");
        }
    }
}

fn mutate_and_parse(src: &str) {
    let mut interner = Interner::new();
    let tokens = lex(src, &mut interner).tokens;
    for token in &tokens {
        let (lo, hi) = (token.span.lo as usize, token.span.hi as usize);
        for text in [
            src[..lo].to_string(),
            format!("{}{}", &src[..lo], &src[hi..]),
        ] {
            let file = SourceFile {
                name: "mutated.wip".to_string(),
                text,
            };
            let parsed = parse_file(&file);
            check_diagnostics(&file.text, &parsed.diagnostics);
            // Rendering must not panic either.
            render::render_all(&parsed.diagnostics, &file.name, &file.text, false);
        }
    }
}

#[test]
fn truncated_and_mutilated_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cases");
    let mut files = 0;
    // One directory per case, whose files may lie in directories below it.
    for dir in ["ok", "err"] {
        let mut dirs = vec![root.join(dir)];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "wip") {
                    mutate_and_parse(&fs::read_to_string(&path).unwrap());
                    files += 1;
                }
            }
        }
    }
    assert!(files >= 40, "found only {files} corpus files");
}

/// The program an editor checks `file` in: the package around it, or its
/// directory.
fn program_of(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap();
    dir.ancestors()
        .find(|dir| dir.join("package.wip").is_file())
        .unwrap_or(dir)
        .to_path_buf()
}

/// Where `src`'s lines are cut: before every token with `every`, and
/// otherwise before one token of each line, a different one from line to
/// line, which keeps the check quick enough to run on every commit.
fn cuts(src: &str, every: bool) -> Vec<usize> {
    let mut interner = Interner::new();
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut line_of = None;
    for token in lex(src, &mut interner).tokens {
        let lo = token.span.lo as usize;
        let line = src[..lo].matches('\n').count();
        if line_of != Some(line) {
            lines.push(Vec::new());
            line_of = Some(line);
        }
        lines.last_mut().unwrap().push(lo);
    }
    if every {
        return lines.concat();
    }
    lines
        .iter()
        .enumerate()
        .map(|(i, starts)| starts[i * 7 % starts.len()])
        .collect()
}

/// `file` with the line at `lo` cut there, the rest of the file as it is,
/// and its program checked: the line as cut, and why, where it panics.
fn check_cut(file: &Path, src: &str, lo: usize) -> Option<String> {
    let line_start = src[..lo].rfind('\n').map_or(0, |i| i + 1);
    let line_end = src[lo..].find('\n').map_or(src.len(), |i| lo + i);
    let mut overlay = Overlay::default();
    overlay.set(file, format!("{}{}", &src[..lo], &src[line_end..]));
    let root = program_of(file);
    let checked = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut loaded) = load_overlaid(&root, Mode::Tests, &overlay) {
            check_loaded(&mut loaded, &mut Timings::default());
        }
    }));
    let payload = checked.err()?;
    let why = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("?");
    let line = src[..lo].matches('\n').count() + 1;
    Some(format!(
        "{}:{line}: `{}`: {why}",
        file.display(),
        &src[line_start..lo]
    ))
}

/// An editor checks a program at every keystroke, so the checker must
/// survive a line half typed. Each line of the examples is cut short, the
/// rest of the file left as it is, and the program checked. With
/// `WIP_EVERY_CUT=1` each line is cut before every one of its tokens,
/// which takes some minutes.
#[test]
fn examples_checked_while_a_line_is_typed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let every = std::env::var_os("WIP_EVERY_CUT").is_some();
    let mut work: Vec<(PathBuf, Arc<String>, usize)> = Vec::new();
    let mut dirs = vec![root];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "wip") {
                let src = Arc::new(fs::read_to_string(&path).unwrap());
                for lo in cuts(&src, every) {
                    work.push((path.clone(), Arc::clone(&src), lo));
                }
            }
        }
    }
    assert!(work.len() >= 300, "found only {} lines to cut", work.len());
    // Each cut is a whole check, taken by whichever thread is free.
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let panicked: Vec<String> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut found = Vec::new();
                    while let Some((file, src, lo)) = work.get(next.fetch_add(1, Ordering::Relaxed))
                    {
                        found.extend(check_cut(file, src, *lo));
                    }
                    found
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    assert!(
        panicked.is_empty(),
        "the checker panicked on {} half-typed lines:\n{}",
        panicked.len(),
        panicked.join("\n")
    );
}
