//! The standard library the compiler carries: its files, compiled in, and
//! written out for an editor to open.

use super::*;

/// The standard library, compiled into the compiler: a program finds it by
/// path, not in its own directory.
const STD: &[(&str, &str, &str)] = &[
    // The prelude is one module of several files, one per type.
    (
        "std::prelude",
        "std/prelude/option.wip",
        include_str!("../../../std/prelude/option.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/result.wip",
        include_str!("../../../std/prelude/result.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/tuple.wip",
        include_str!("../../../std/prelude/tuple.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/arithmetic.wip",
        include_str!("../../../std/prelude/arithmetic.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/index.wip",
        include_str!("../../../std/prelude/index.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/parse.wip",
        include_str!("../../../std/prelude/parse.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/search.wip",
        include_str!("../../../std/prelude/search.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/sort.wip",
        include_str!("../../../std/prelude/sort.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/compare.wip",
        include_str!("../../../std/prelude/compare.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/convert.wip",
        include_str!("../../../std/prelude/convert.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/hash.wip",
        include_str!("../../../std/prelude/hash.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/destroy.wip",
        include_str!("../../../std/prelude/destroy.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/integer.wip",
        include_str!("../../../std/prelude/integer.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/overflow.wip",
        include_str!("../../../std/prelude/overflow.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/str.wip",
        include_str!("../../../std/prelude/str.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/items.wip",
        include_str!("../../../std/prelude/items.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/sequence.wip",
        include_str!("../../../std/prelude/sequence.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/iterator.wip",
        include_str!("../../../std/prelude/iterator.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/char.wip",
        include_str!("../../../std/prelude/char.wip"),
    ),
    // Whether a character is a letter or a number.
    (
        "std::prelude",
        "std/prelude/category.wip",
        include_str!("../../../std/prelude/category.wip"),
    ),
    // How many columns a character takes.
    (
        "std::prelude",
        "std/prelude/width.wip",
        include_str!("../../../std/prelude/width.wip"),
    ),
    // A float to a precision, an integer in a radix.
    (
        "std::prelude",
        "std/prelude/written.wip",
        include_str!("../../../std/prelude/written.wip"),
    ),
    // A copy of a value.
    (
        "std::prelude",
        "std/prelude/clone.wip",
        include_str!("../../../std/prelude/clone.wip"),
    ),
    // A character's case.
    (
        "std::prelude",
        "std/prelude/case.wip",
        include_str!("../../../std/prelude/case.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/split.wip",
        include_str!("../../../std/prelude/split.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/float.wip",
        include_str!("../../../std/prelude/float.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/bits.wip",
        include_str!("../../../std/prelude/bits.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/float_words.wip",
        include_str!("../../../std/prelude/float_words.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/trigonometry.wip",
        include_str!("../../../std/prelude/trigonometry.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/inverse_trigonometry.wip",
        include_str!("../../../std/prelude/inverse_trigonometry.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/exponential.wip",
        include_str!("../../../std/prelude/exponential.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/logarithm.wip",
        include_str!("../../../std/prelude/logarithm.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/logarithm_table.wip",
        include_str!("../../../std/prelude/logarithm_table.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/logarithm2.wip",
        include_str!("../../../std/prelude/logarithm2.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/logarithm2_table.wip",
        include_str!("../../../std/prelude/logarithm2_table.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/logarithm10.wip",
        include_str!("../../../std/prelude/logarithm10.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/power.wip",
        include_str!("../../../std/prelude/power.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/power_table.wip",
        include_str!("../../../std/prelude/power_table.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/exponential_table.wip",
        include_str!("../../../std/prelude/exponential_table.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/slice.wip",
        include_str!("../../../std/prelude/slice.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/transform.wip",
        include_str!("../../../std/prelude/transform.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/vec.wip",
        include_str!("../../../std/prelude/vec.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/error.wip",
        include_str!("../../../std/prelude/error.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/string.wip",
        include_str!("../../../std/prelude/string.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/text.wip",
        include_str!("../../../std/prelude/text.wip"),
    ),
    // What the code generator calls by name: a panic, and what `wip test`
    // prints.
    (
        "std::prelude",
        "std/prelude/runtime.wip",
        include_str!("../../../std/prelude/runtime.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/allocator.wip",
        include_str!("../../../std/prelude/allocator.wip"),
    ),
    (
        "std::prelude",
        "std/prelude/int128.wip",
        include_str!("../../../std/prelude/int128.wip"),
    ),
    // What the prelude's methods answer, named by import.
    (
        "std::iter",
        "std/iter/adapters.wip",
        include_str!("../../../std/iter/adapters.wip"),
    ),
    (
        "std::iter",
        "std/iter/walk.wip",
        include_str!("../../../std/iter/walk.wip"),
    ),
    (
        "std::iter",
        "std/iter/forwards.wip",
        include_str!("../../../std/iter/forwards.wip"),
    ),
    (
        "std::iter",
        "std/iter/backwards.wip",
        include_str!("../../../std/iter/backwards.wip"),
    ),
    (
        "std::iter",
        "std/iter/taking.wip",
        include_str!("../../../std/iter/taking.wip"),
    ),
    (
        "std::text",
        "std/text/chars.wip",
        include_str!("../../../std/text/chars.wip"),
    ),
    (
        "std::text",
        "std/text/split.wip",
        include_str!("../../../std/text/split.wip"),
    ),
    (
        "std::text",
        "std/text/parse_error.wip",
        include_str!("../../../std/text/parse_error.wip"),
    ),
    // Containers beyond `Vec`, by import: their names are the program's to
    // use otherwise.
    (
        "std::collections",
        "std/collections/map.wip",
        include_str!("../../../std/collections/map.wip"),
    ),
    (
        "std::collections",
        "std/collections/set.wip",
        include_str!("../../../std/collections/set.wip"),
    ),
    (
        "std::collections",
        "std/collections/table.wip",
        include_str!("../../../std/collections/table.wip"),
    ),
    (
        "std::collections",
        "std/collections/deque.wip",
        include_str!("../../../std/collections/deque.wip"),
    ),
    (
        "std::collections",
        "std/collections/arena.wip",
        include_str!("../../../std/collections/arena.wip"),
    ),
    (
        "std::shared",
        "std/shared/shared.wip",
        include_str!("../../../std/shared/shared.wip"),
    ),
    (
        "std::net",
        "std/net/net.wip",
        include_str!("../../../std/net/net.wip"),
    ),
    // The C library, as std calls it.
    (
        "std::libc",
        "std/libc/libc.wip",
        include_str!("../../../std/libc/libc.wip"),
    ),
    (
        "std::io",
        "std/io/io.wip",
        include_str!("../../../std/io/io.wip"),
    ),
    (
        "std::fs",
        "std/fs/fs.wip",
        include_str!("../../../std/fs/fs.wip"),
    ),
    (
        "std::time",
        "std/time/time.wip",
        include_str!("../../../std/time/time.wip"),
    ),
    (
        "std::math",
        "std/math/math.wip",
        include_str!("../../../std/math/math.wip"),
    ),
    // Work on another thread.
    (
        "std::future",
        "std/future/future.wip",
        include_str!("../../../std/future/future.wip"),
    ),
    // A program's own arguments, read as options.
    (
        "std::args",
        "std/args/args.wip",
        include_str!("../../../std/args/args.wip"),
    ),
    // Random numbers, for what is not cryptography.
    (
        "std::random",
        "std/random/random.wip",
        include_str!("../../../std/random/random.wip"),
    ),
    // Starting another program.
    (
        "std::process",
        "std/process/process.wip",
        include_str!("../../../std/process/process.wip"),
    ),
    // Regular expressions, for what a program renames or searches by
    // pattern.
    (
        "std::regex",
        "std/regex/regex.wip",
        include_str!("../../../std/regex/regex.wip"),
    ),
    // JSON, read into a tree and written back.
    (
        "std::json",
        "std/json/json.wip",
        include_str!("../../../std/json/json.wip"),
    ),
    (
        "std::path",
        "std/path/path.wip",
        include_str!("../../../std/path/path.wip"),
    ),
    (
        "std::mem",
        "std/mem/mem.wip",
        include_str!("../../../std/mem/mem.wip"),
    ),
    (
        "std::digest",
        "std/digest/digest.wip",
        include_str!("../../../std/digest/digest.wip"),
    ),
    (
        "std::c",
        "std/c/c.wip",
        include_str!("../../../std/c/c.wip"),
    ),
    (
        "std::embed",
        "std/embed/embed.wip",
        include_str!("../../../std/embed/embed.wip"),
    ),
    (
        "std::sync",
        "std/sync/sync.wip",
        include_str!("../../../std/sync/sync.wip"),
    ),
    (
        "std::json",
        "std/json/parse.wip",
        include_str!("../../../std/json/parse.wip"),
    ),
    (
        "std::json",
        "std/json/lookup.wip",
        include_str!("../../../std/json/lookup.wip"),
    ),
];

/// Every file of the standard library, as the compiler reads it for this
/// machine: its path from the root of a checkout, and its text.
fn all_std_files() -> Vec<(String, String)> {
    modules()
        .into_iter()
        .flat_map(|module| std_files(module, targets::Target::host()))
        .collect()
}

/// The standard library's modules, by path, `std::prelude` among them.
pub fn modules() -> Vec<&'static str> {
    let mut modules: Vec<&str> = STD.iter().map(|(module, ..)| *module).collect();
    modules.sort_unstable();
    modules.dedup();
    modules
}

/// A directory holding the standard library the compiler carries, as
/// files, for an editor to open: `<cache>/std/<hash>/std`, named by what
/// the files hold, so that two compilers never share one. It is written
/// once, and its files are read-only, since changing them changes
/// nothing the compiler reads.
pub fn std_on_disk() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let files = all_std_files();
        let mut hasher = sha256::Sha256::default();
        for (name, text) in &files {
            hasher.field(name.as_bytes());
            hasher.field(text.as_bytes());
        }
        let root = c_build::cache_dir()
            .join("std")
            .join(sha256::hex(&hasher.finish()));
        let dir = root.join("std");
        if dir.is_dir() {
            return Some(dir);
        }
        // Written beside it and moved into place, so that two servers
        // starting at once never see half of it.
        let partial = root.with_extension(format!("partial{}", std::process::id()));
        let written = files.iter().try_for_each(|(name, text)| {
            let file = partial.join(name);
            std::fs::create_dir_all(file.parent().expect("a file has a directory"))?;
            std::fs::write(&file, text)?;
            let mut permissions = std::fs::metadata(&file)?.permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&file, permissions)
        });
        if written.is_err() || std::fs::rename(&partial, &root).is_err() {
            let _ = std::fs::remove_dir_all(&partial);
        }
        dir.is_dir().then_some(dir)
    })
    .clone()
}

/// The files of one of the standard library's modules, which are compiled
/// into the compiler, as `target` reads them.
pub(crate) fn std_files(path: &str, target: targets::Target) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = STD
        .iter()
        .filter(|(module, ..)| *module == path)
        .map(|(_, name, text)| ((*name).to_string(), (*text).to_string()))
        .collect();
    // The prelude says what the target is, which only the compiler
    // knows, so that file is written here.
    if path == wip_hir::PRELUDE {
        files.push((
            "std/prelude/target.wip".to_string(),
            targets::prelude(target),
        ));
    }
    files
}
