//! Compiling the C a program carries: each file on its own, in parallel,
//! and kept, so that a build whose C has not changed compiles none of it.
//!
//! An object is kept under the hash of what made it: the compiler, the
//! flags, the source's path and contents. Beside it is the list of headers
//! the compiler read, each with its size, time and hash, since a header can
//! change without the file that includes it changing. An entry is used
//! when every header is as it was.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use crate::sha256::{Sha256, hex, hex_of};

/// What a file is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    C,
    ObjectiveC,
}

/// One file to compile.
#[derive(Debug, Clone)]
pub struct CFile {
    pub source: PathBuf,
    /// Objective-C for a `.m` file, or what `@source`
    /// said.
    pub language: Language,
    /// A header put before this file, which is how a module configures
    /// the C it vendors: `@prefix("build.h")`. The
    /// compiler is given `-include`, and what it then reads is a
    /// dependency like any other, so a change to it recompiles.
    pub prefix: Option<PathBuf>,
    /// What this file is compiled with defined, `NAME` or `NAME=value`:
    /// its module's `@define`.
    pub defines: Vec<String>,
}

impl CFile {
    pub fn c(source: PathBuf) -> CFile {
        CFile {
            source,
            language: Language::C,
            prefix: None,
            defines: Vec::new(),
        }
    }

    /// A module's file, by its extension.
    pub fn of(source: PathBuf) -> CFile {
        let language = match implied(&source) {
            Some(Language::ObjectiveC) => Language::ObjectiveC,
            _ => Language::C,
        };
        CFile {
            source,
            language,
            prefix: None,
            defines: Vec::new(),
        }
    }
}

/// What a C compiler takes a file to be written in from its extension:
/// nothing for a header, which it would make a precompiled header of.
fn implied(source: &Path) -> Option<Language> {
    match source.extension().and_then(|e| e.to_str()) {
        Some("c") => Some(Language::C),
        Some("m") => Some(Language::ObjectiveC),
        _ => None,
    }
}

/// What came of compiling: an object for each file, in the order the files
/// were given, and how many were compiled rather than found.
#[derive(Debug)]
pub struct Compiled {
    pub objects: Vec<PathBuf>,
    pub compiled: usize,
}

/// Where the cache is: `$WIP_CACHE_DIR`, or the platform's cache directory.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WIP_CACHE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if std::env::consts::OS == "macos"
        && let Some(home) = &home
    {
        return home.join("Library/Caches/wip");
    }
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("wip");
    }
    match home {
        Some(home) => home.join(".cache/wip"),
        None => std::env::temp_dir().join("wip-cache"),
    }
}

/// The C compiler: `$CC`, or `cc`.
pub fn compiler() -> OsString {
    std::env::var_os("CC").unwrap_or_else(|| OsString::from("cc"))
}

/// Each compiler asked, with what it said.
type Identities = std::sync::Mutex<Vec<(OsString, Vec<u8>)>>;

/// Who the compiler is, as far as an object depends on it: its name and
/// what it says its version is. Asked once for each compiler.
pub(crate) fn identity(cc: &OsStr) -> Vec<u8> {
    static SEEN: OnceLock<Identities> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, known)) = seen.iter().find(|(name, _)| name == cc) {
        return known.clone();
    }
    let mut id = cc.as_encoded_bytes().to_vec();
    if let Ok(output) = Command::new(cc).arg("--version").output() {
        id.extend_from_slice(&output.stdout);
    }
    seen.push((cc.to_os_string(), id.clone()));
    id
}

/// Writes generated C where its path depends only on what it says, so that
/// the same source is the same entry build after build. `files` are written
/// side by side, into one directory named by all of them.
pub fn generated(cache: &Path, files: &[(&str, &str)]) -> Result<PathBuf, String> {
    let mut hasher = Sha256::default();
    for (name, text) in files {
        hasher.field(name.as_bytes());
        hasher.field(text.as_bytes());
    }
    let dir = cache.join("generated").join(hex(&hasher.finish()));
    for (name, text) in files {
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|kept| kept == text.as_bytes()) {
            continue;
        }
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("cannot create `{}`: {err}", dir.display()))?;
        write_atomically(&path, text.as_bytes())?;
    }
    Ok(dir)
}

/// Compiles `files` with `includes` on the include path, using what the
/// cache under `cache` already has. `position_independent` is for a shared
/// library on a system whose compiler does not make all code so.
pub fn compile(
    files: &[CFile],
    includes: &[PathBuf],
    cache: &Path,
    position_independent: bool,
    optimize: bool,
    cpu: wip_codegen::Cpu,
) -> Result<Compiled, String> {
    // A program of Wip alone has no C, and the C compiler is not asked
    // anything, not even who it is.
    if files.is_empty() {
        return Ok(Compiled {
            objects: Vec::new(),
            compiled: 0,
        });
    }
    let cc = compiler();
    let objects_dir = cache.join("objects");
    std::fs::create_dir_all(&objects_dir)
        .map_err(|err| format!("cannot create `{}`: {err}", objects_dir.display()))?;
    let identity = identity(&cc);
    let next = AtomicUsize::new(0);
    let compiled = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(files.len().max(1));
    let mut results: Vec<Option<Result<PathBuf, String>>> = vec![None; files.len()];
    let done: Vec<Vec<(usize, Result<PathBuf, String>)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(file) = files.get(i) else {
                            return out;
                        };
                        let flags = flags(file, includes, position_independent, optimize, cpu);
                        let result = object_for(file, flags, &cc, &identity, &objects_dir).map(
                            |(path, fresh)| {
                                if fresh {
                                    compiled.fetch_add(1, Ordering::Relaxed);
                                }
                                path
                            },
                        );
                        out.push((i, result));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap_or_default())
            .collect()
    });
    for (i, result) in done.into_iter().flatten() {
        results[i] = Some(result);
    }
    // The first failure, in the order the files were given, so the message
    // does not depend on which thread got there first.
    let objects = results
        .into_iter()
        .map(|r| r.unwrap_or_else(|| Err("a C file was not compiled".to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Compiled {
        objects,
        compiled: compiled.into_inner(),
    })
}

/// What tells a C compiler the processor a program is for, as GCC and
/// Clang name it: a level by its own name, and the compiling machine as
/// `native`.
pub(crate) fn cpu_flag(cpu: wip_codegen::Cpu) -> String {
    use wip_codegen::Level;
    match cpu.level() {
        None if cfg!(target_arch = "aarch64") => "-mcpu=native".to_string(),
        None => "-march=native".to_string(),
        Some(Level::Armv8_0) => "-march=armv8-a".to_string(),
        Some(Level::Armv8_1) => "-march=armv8.1-a".to_string(),
        // GCC before 13 does not know Apple's processors by name, and Linux
        // on a Mac may be built with it: v8.4 has what the M1 level does
        // (LSE, pointer authentication, dot products), and the M1 has v8.4.
        Some(Level::AppleM1) if cfg!(target_vendor = "apple") => "-mcpu=apple-m1".to_string(),
        Some(Level::AppleM1) => "-march=armv8.4-a+fp16".to_string(),
        Some(level) => format!("-march={}", level.name()),
    }
}

/// The flags one file is compiled with, besides where it is and where its
/// object goes.
fn flags(
    file: &CFile,
    includes: &[PathBuf],
    position_independent: bool,
    optimize: bool,
    cpu: wip_codegen::Cpu,
) -> Vec<OsString> {
    // Optimised in a release build, and not in a debug build; with the
    // debug information a debugger reads in both, as Wip's own code has it.
    let mut args: Vec<OsString> = match optimize {
        true => vec!["-c".into(), "-O2".into(), "-g".into()],
        false => vec!["-c".into(), "-O0".into(), "-g".into()],
    };
    if position_independent {
        args.push("-fPIC".into());
    }
    // For the processor Wip's code is for, so that the C beside it runs
    // where it does, and no further.
    args.push(cpu_flag(cpu).into());
    // The language, where the extension does not already say it: a header
    // is then compiled as the implementation it holds.
    if implied(&file.source) != Some(file.language) || file.language == Language::ObjectiveC {
        let language = match file.language {
            Language::C => "c",
            Language::ObjectiveC => "objective-c",
        };
        args.extend(["-x".into(), language.into()]);
    }
    // Before the prefix header, which may test them.
    for define in &file.defines {
        args.push(format!("-D{define}").into());
    }
    if let Some(prefix) = &file.prefix {
        args.push("-include".into());
        args.push(prefix.into());
    }
    for dir in includes {
        let mut arg = OsString::from("-I");
        arg.push(dir);
        args.push(arg);
    }
    args
}

/// The object for `file`: the one kept, if what made it is unchanged, or a
/// new one. The flag says whether it was compiled now.
fn object_for(
    file: &CFile,
    flags: Vec<OsString>,
    cc: &OsStr,
    identity: &[u8],
    objects_dir: &Path,
) -> Result<(PathBuf, bool), String> {
    let source = std::fs::canonicalize(&file.source)
        .map_err(|err| format!("cannot read `{}`: {err}", file.source.display()))?;
    let text = std::fs::read(&source)
        .map_err(|err| format!("cannot read `{}`: {err}", source.display()))?;
    let mut key = Sha256::default();
    key.field(identity);
    for flag in &flags {
        key.field(flag.as_encoded_bytes());
    }
    key.field(source.as_os_str().as_encoded_bytes());
    key.field(&text);
    let key = hex(&key.finish());
    let object = objects_dir.join(format!("{key}.o"));
    let deps = objects_dir.join(format!("{key}.deps"));
    if object.is_file() && still_current(&deps) {
        return Ok((object, false));
    }

    // Compiled beside the entry under a name of its own, then renamed into
    // place: a build running at the same time sees the old entry or the
    // new one, never half of one.
    let unique = unique_suffix();
    let fresh = objects_dir.join(format!("{key}.{unique}.o"));
    let made = objects_dir.join(format!("{key}.{unique}.d"));
    let mut args = flags;
    args.push(source.clone().into());
    args.extend(["-o".into(), fresh.clone().into()]);
    args.extend(["-MD".into(), "-MF".into(), made.clone().into()]);
    let result = Command::new(cc)
        .args(&args)
        .output()
        .map_err(|err| format!("cannot run `{}`: {err}", cc.to_string_lossy()))?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&fresh);
        let _ = std::fs::remove_file(&made);
        return Err(format!(
            "`{}` failed:\n{}",
            cc.to_string_lossy(),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let headers = std::fs::read_to_string(&made)
        .map(|text| dependencies(&text))
        .unwrap_or_default();
    let _ = std::fs::remove_file(&made);
    let mut listing = String::new();
    for header in headers {
        let path = std::fs::canonicalize(&header).unwrap_or(header);
        let Some(stamp) = Stamp::of(&path) else {
            continue;
        };
        listing.push_str(&stamp.line(&path));
    }
    // The object first, then what says it is current: a reader that finds
    // the list finds the object it describes.
    std::fs::rename(&fresh, &object)
        .map_err(|err| format!("cannot keep `{}`: {err}", object.display()))?;
    write_atomically(&deps, listing.as_bytes())?;
    Ok((object, true))
}

/// What a header was when an object was made from it.
struct Stamp {
    size: u64,
    modified: u128,
    hash: String,
}

impl Stamp {
    fn of(path: &Path) -> Option<Stamp> {
        let metadata = std::fs::metadata(path).ok()?;
        let text = std::fs::read(path).ok()?;
        Some(Stamp {
            size: metadata.len(),
            modified: modified(&metadata),
            hash: hex_of(&text),
        })
    }

    /// One line of the list: the size, the time, the hash and the path,
    /// the path last since it may hold spaces.
    fn line(&self, path: &Path) -> String {
        format!(
            "{} {} {} {}\n",
            self.size,
            self.modified,
            self.hash,
            path.display()
        )
    }
}

fn modified(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos())
}

/// Whether every header an entry was made from is as it was: the same size
/// and time, or, where the time moved, the same contents.
fn still_current(deps: &Path) -> bool {
    let Ok(listing) = std::fs::read_to_string(deps) else {
        return false;
    };
    listing.lines().all(|line| {
        let mut fields = line.splitn(4, ' ');
        let (Some(size), Some(time), Some(hash), Some(path)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return false;
        };
        let path = Path::new(path);
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        if size.parse::<u64>() != Ok(metadata.len()) {
            return false;
        }
        if time.parse::<u128>() == Ok(modified(&metadata)) {
            return true;
        }
        std::fs::read(path).is_ok_and(|text| hex_of(&text) == hash)
    })
}

/// The files a Make rule written by `-MD` depends on: everything after the
/// first `: `, with `\` continuations joined and `\ ` read as a space.
fn dependencies(rule: &str) -> Vec<PathBuf> {
    let joined = rule.replace("\\\r\n", " ").replace("\\\n", " ");
    let Some((_, after)) = joined.split_once(": ") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = after.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&' ') => {
                current.push(' ');
                chars.next();
            }
            '$' if chars.peek() == Some(&'$') => {
                current.push('$');
                chars.next();
            }
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    out.push(PathBuf::from(std::mem::take(&mut current)));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(PathBuf::from(current));
    }
    out
}

/// Writes `bytes` to `path` by way of a name of its own, so a reader never
/// sees half of it.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("{}.tmp", unique_suffix()));
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|err| {
            let _ = std::fs::remove_file(&temporary);
            format!("cannot write `{}`: {err}", path.display())
        })
}

/// A suffix no other writer uses: this process, and a count within it.
pub(crate) fn unique_suffix() -> String {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_rule_make_would() {
        let rule = "out.o: a.c include/b.h \\\n  with\\ space.h \\\n  $$dollar.h\n";
        assert_eq!(
            dependencies(rule),
            [
                PathBuf::from("a.c"),
                PathBuf::from("include/b.h"),
                PathBuf::from("with space.h"),
                PathBuf::from("$dollar.h"),
            ]
        );
    }
}
