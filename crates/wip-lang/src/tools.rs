//! The tools written in Wip that `wip` carries and runs.
//! Their source is compiled into the compiler, as the standard library is;
//! the first time one is asked for it is built into the cache, keyed by
//! what it says and by the compiler that builds it, and it is run from
//! there after.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::{BuildError, BuildOptions, Mode, Timings, c_build, render, sha256};

/// `wip bindgen`'s source: each file's path in the tool's directory, and its
/// text. Its tests are not here: they are run from the repository, by `wip test
/// tools/bindgen/main.wip`.
const BINDGEN: &[(&str, &str)] = &[
    ("main.wip", include_str!("../../../tools/bindgen/main.wip")),
    (
        "types/types.wip",
        include_str!("../../../tools/bindgen/types/types.wip"),
    ),
    (
        "header/header.wip",
        include_str!("../../../tools/bindgen/header/header.wip"),
    ),
    (
        "render/render.wip",
        include_str!("../../../tools/bindgen/render/render.wip"),
    ),
    (
        "declared/declared.wip",
        include_str!("../../../tools/bindgen/declared/declared.wip"),
    ),
    (
        "probe/probe.wip",
        include_str!("../../../tools/bindgen/probe/probe.wip"),
    ),
    (
        "clang/clang.wip",
        include_str!("../../../tools/bindgen/clang/clang.wip"),
    ),
];

/// The binding generator, built by this compiler: where its executable is,
/// or why it could not be built.
pub fn bindgen(color: bool) -> Result<PathBuf, String> {
    built("bindgen", BINDGEN, color)
}

/// The tool `name` of `files`, built into `<cache>/tools/<name>/<key>`
/// the first time it is asked for, and found there after.
fn built(name: &str, files: &[(&str, &str)], color: bool) -> Result<PathBuf, String> {
    let mut hasher = sha256::Sha256::default();
    hasher.field(&compiler_identity());
    for (path, text) in files {
        hasher.field(path.as_bytes());
        hasher.field(text.as_bytes());
    }
    let dir = c_build::cache_dir()
        .join("tools")
        .join(name)
        .join(sha256::hex(&hasher.finish()));
    let exe = dir.join(name);
    if exe.is_file() {
        return Ok(exe);
    }
    // Built beside where it goes and moved into place, so that two runs at
    // once never find half of one.
    let partial = dir.with_extension(format!("partial{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&partial);
    if let Err(message) = build_into(&partial, name, files, color) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(message);
    }
    // Where another run moved its own into place first, that one is kept.
    if std::fs::rename(&partial, &dir).is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    match exe.is_file() {
        true => Ok(exe),
        false => Err(format!("cannot keep `wip {name}` in `{}`", dir.display())),
    }
}

/// Writes `files` into `dir/src` and builds them into `dir/<name>`.
fn build_into(dir: &Path, name: &str, files: &[(&str, &str)], color: bool) -> Result<(), String> {
    let src = dir.join("src");
    for (path, text) in files {
        let file = src.join(path);
        let parent = file.parent().expect("a file has a directory");
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("cannot create `{}`: {err}", parent.display()))?;
        std::fs::write(&file, text)
            .map_err(|err| format!("cannot write `{}`: {err}", file.display()))?;
    }
    let mut timings = Timings::default();
    let mut loaded = crate::load_mode(&src.join("main.wip"), Mode::Program, &mut timings)?;
    let exe = dir.join(name);
    // A tool is built to be used, once, and run many times.
    let options = BuildOptions {
        profile: crate::Profile::Release,
        ..BuildOptions::default()
    };
    match crate::build_loaded(&mut loaded, &exe, &options, &mut timings) {
        Ok(_) => Ok(()),
        Err(BuildError::Diagnostics(diagnostics)) => Err(format!(
            "`wip {name}` does not compile with this compiler:\n{}",
            render::render_program(&diagnostics, &loaded.sources, color)
        )),
        Err(BuildError::Link(message)) => Err(message),
    }
}

/// Which compiler this is, as far as what it builds depends on it: its
/// version, and its executable, by where it is, how large it is and when
/// it was written. A compiler built again is another compiler.
fn compiler_identity() -> Vec<u8> {
    let mut id = crate::VERSION.as_bytes().to_vec();
    let Ok(exe) = std::env::current_exe() else {
        return id;
    };
    id.extend_from_slice(exe.as_os_str().as_encoded_bytes());
    if let Ok(meta) = std::fs::metadata(&exe) {
        id.extend_from_slice(&meta.len().to_le_bytes());
        if let Ok(since) = meta.modified().map(|m| m.duration_since(UNIX_EPOCH)) {
            let nanos = since.map(|d| d.as_nanos()).unwrap_or_default();
            id.extend_from_slice(&nanos.to_le_bytes());
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every file of the tool but its tests is carried: one left out would
    /// be missing only where the tool is built from what `wip` carries.
    #[test]
    fn bindgen_carries_every_file_of_the_tool() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/bindgen");
        let mut on_disk = Vec::new();
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("the tool's directory is there") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "wip")
                    && !path.to_string_lossy().ends_with(".test.wip")
                {
                    let relative = path.strip_prefix(&root).expect("under the root");
                    on_disk.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        on_disk.sort();
        let mut carried: Vec<String> = BINDGEN.iter().map(|(p, _)| p.to_string()).collect();
        carried.sort();
        assert_eq!(on_disk, carried);
    }
}
