//! Checking on several threads gives exactly what checking on one does:
//! the same typed program, down to how its types are
//! numbered, and the same diagnostics in the same order.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use wip_hir::ModuleAst;
use wip_lang::LoadedModule;
use wip_syntax::{Diagnostic, Interner};

fn cases(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cases")
        .join(dir)
}

/// The typed program and every diagnostic, as text, checked on `threads`
/// threads. The move checker runs only on a program without type errors, as
/// in the driver.
fn checked(modules: &[ModuleAst<'_>], interner: &Interner, threads: usize) -> String {
    let lowered = wip_hir::lower_with_threads(modules, interner, threads);
    let moves = if lowered.diagnostics.iter().any(Diagnostic::is_error) {
        Vec::new()
    } else {
        wip_analysis::check_with_threads(&lowered.program, interner, threads)
    };
    format!(
        "{:#?}\n{:#?}\n{:#?}",
        lowered.program, lowered.diagnostics, moves
    )
}

/// Every case's `main.wip`: one directory per case.
fn case_entries() -> Vec<PathBuf> {
    let mut mains = Vec::new();
    for dir in ["ok", "err"] {
        for entry in std::fs::read_dir(cases(dir))
            .expect("the test cases")
            .flatten()
        {
            let main = entry.path().join("main.wip");
            if main.exists() {
                mains.push(main);
            }
        }
    }
    mains.sort();
    mains
}

/// The thread counts a case that checks differently on, against one.
fn differs(main: &Path) -> Vec<usize> {
    let loaded = wip_lang::load(main).expect("a loadable program");
    let modules: Vec<ModuleAst<'_>> = loaded.modules.iter().map(LoadedModule::syntax).collect();
    let one = checked(&modules, &loaded.interner, 1);
    [2, 3, 8]
        .into_iter()
        .filter(|&threads| one != checked(&modules, &loaded.interner, threads))
        .collect()
}

#[test]
fn cases_check_the_same_on_any_number_of_threads() {
    let mains = case_entries();
    assert!(mains.len() > 100, "found only {} cases", mains.len());
    // The cases are independent, so they are checked side by side, each
    // taken by whichever thread is free; each is still checked on one
    // thread and on several, and compared.
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut different: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut found = Vec::new();
                    while let Some(main) = mains.get(next.fetch_add(1, Ordering::Relaxed)) {
                        for threads in differs(main) {
                            found.push(format!(
                                "`{}` checks differently on {threads} threads",
                                main.display()
                            ));
                        }
                    }
                    found
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    different.sort();
    assert!(different.is_empty(), "{}", different.join("\n"));
}
