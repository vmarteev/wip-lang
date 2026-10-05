//! Which backend compiles Wip's code, and building with LLVM through
//! `clang`.
//!
//! A debug build is Cranelift's. A release build is LLVM's where there is a
//! `clang` that reads the IR the LLVM backend writes — version 15 or newer,
//! found by asking it to compile a module of one function — and
//! Cranelift's, with a note, where there is not. `--backend` says which,
//! where it matters.
//!
//! What `clang` makes of a program is kept in the cache, as the C a module
//! carries is, under the hash of everything it depends on:
//! this compiler, the `clang`, the flags and the IR.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use crate::c_build;
use crate::sha256::{Sha256, hex};
use crate::{BuildError, Profile, TempDir};

/// What compiles Wip's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cranelift,
    Llvm,
}

/// The backend a build of `profile` uses, where `asked` is what
/// `--backend` said, if anything; and, for a release build that has to
/// fall back to Cranelift, the note that says so.
pub fn choose_backend(
    profile: Profile,
    asked: Option<Backend>,
) -> Result<(Backend, Option<String>), BuildError> {
    match (asked, profile) {
        (Some(Backend::Cranelift), _) | (None, Profile::Debug) => Ok((Backend::Cranelift, None)),
        (Some(Backend::Llvm), Profile::Debug) => Err(BuildError::Link(
            "`--backend llvm` builds a release build, `--release`: LLVM's code describes no \
             variables, and is no debug build"
                .to_string(),
        )),
        (Some(Backend::Llvm), Profile::Release) => match clang() {
            Some(_) => Ok((Backend::Llvm, None)),
            None => Err(BuildError::Link(format!(
                "the LLVM backend needs a clang 15 or newer: {}",
                missing()
            ))),
        },
        (None, Profile::Release) => match clang() {
            Some(_) => Ok((Backend::Llvm, None)),
            None => Ok((
                Backend::Cranelift,
                Some(format!("built by Cranelift: {}", missing())),
            )),
        },
    }
}

/// Why there is no `clang` for LLVM, and what to do.
fn missing() -> String {
    match std::env::var_os("WIP_CLANG") {
        Some(clang) => format!(
            "`{}`, which WIP_CLANG names, is not a clang 15 or newer, or is not there",
            clang.to_string_lossy()
        ),
        None => {
            "`clang` is not a clang 15 or newer, or is not there; WIP_CLANG names one".to_string()
        }
    }
}

/// The `clang` that compiles the LLVM backend's IR: the one `WIP_CLANG`
/// names, or `clang`, if it reads the IR. Asked once for each `clang`, and
/// the answer kept in the cache.
fn clang() -> Option<&'static OsString> {
    static FOUND: OnceLock<Option<OsString>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let clang = std::env::var_os("WIP_CLANG").unwrap_or_else(|| OsString::from("clang"));
            reads_the_ir(&clang).then_some(clang)
        })
        .as_ref()
}

/// Whether `clang` compiles a module written as the LLVM backend writes
/// one: with pointers that name no type, which `clang` 15 reads by
/// default and 14 does not. Apple counts its versions as LLVM does not, so
/// the module answers rather than the number.
fn reads_the_ir(clang: &OsString) -> bool {
    let identity = c_build::identity(clang);
    let mut key = Sha256::default();
    key.field(b"reads the LLVM backend's IR");
    key.field(&identity);
    let answer = c_build::cache_dir().join("llvm").join(hex(&key.finish()));
    if let Ok(known) = std::fs::read(&answer) {
        return known == b"yes";
    }
    let Ok(dir) = TempDir::new() else {
        return false;
    };
    let module = dir.path().join("probe.ll");
    let reads = std::fs::write(&module, "define ptr @probe(ptr %p) {\n  ret ptr %p\n}\n").is_ok()
        && Command::new(clang)
            .args(["-c", "-x", "ir", "-Wno-override-module"])
            .arg(&module)
            .arg("-o")
            .arg(dir.path().join("probe.o"))
            .output()
            .is_ok_and(|out| out.status.success());
    // Only an answer from a `clang` that was there, and said its version,
    // is kept: one installed later is asked again.
    let there = identity.len() > clang.as_encoded_bytes().len();
    if there {
        let _ = std::fs::create_dir_all(answer.parent().expect("a directory"));
        let _ = std::fs::write(&answer, if reads { "yes" } else { "no" });
    }
    reads
}

/// Whether `clang` is Apple's.
fn is_apple_clang(clang: &OsString) -> bool {
    String::from_utf8_lossy(&c_build::identity(clang))
        .lines()
        .any(|line| line.contains("Apple clang"))
}

/// The object `clang -O2` makes of the LLVM backend's IR, with the table a
/// panic reads its calls from in it, which its DWARF says the contents of. Kept
/// in the cache under what it depends on, and taken from there when nothing has
/// changed. `WIP_LLVM_IR` names a file to keep the IR in, to read.
pub(crate) fn llvm_object(ir: &wip_llvm::Ir, cpu: wip_codegen::Cpu) -> Result<Vec<u8>, BuildError> {
    if let Some(keep) = std::env::var_os("WIP_LLVM_IR") {
        std::fs::write(&keep, &ir.text)
            .map_err(|err| BuildError::Link(format!("cannot keep the LLVM IR: {err}")))?;
    }
    let clang = clang().ok_or_else(|| {
        BuildError::Link(format!(
            "the LLVM backend needs a clang 15 or newer: {}",
            missing()
        ))
    })?;
    // Code that goes where a position-independent program is loaded, as
    // Cranelift's does, for the processor the program's C
    // is built for.
    let mut target: Vec<OsString> = Vec::new();
    if !cfg!(target_vendor = "apple") {
        target.push("-fPIC".into());
    }
    target.push(c_build::cpu_flag(cpu).into());
    // `WIP_LLVM_OPT` sets the level, to tell what optimising does from what
    // the IR says.
    let level = std::env::var("WIP_LLVM_OPT").unwrap_or_else(|_| "2".to_string());
    let mut compile: Vec<OsString> = vec![
        format!("-O{level}").into(),
        "-x".into(),
        "ir".into(),
        "-Wno-override-module".into(),
    ];
    // A function's cold code stays in the function: moved to one of its
    // own, it would be a call a panic's calls show, of a function the
    // program does not have. Apple's clang moves it unless
    // told not to; LLVM's own does not unless told to, and knows no such
    // flag.
    if is_apple_clang(clang) {
        compile.extend(["-Xclang".into(), "-fno-split-cold-code".into()]);
    }
    // An ELF object is written again with the table in it, by a writer
    // that knows no table of the functions whose addresses are taken,
    // which only a linker folding identical code reads.
    if !cfg!(target_vendor = "apple") {
        compile.push("-fno-addrsig".into());
    }
    compile.extend(target.iter().cloned());

    let mut key = Sha256::default();
    key.field(b"an LLVM object");
    key.field(&this_compiler());
    key.field(&c_build::identity(clang));
    for arg in &compile {
        key.field(arg.as_encoded_bytes());
    }
    key.field(ir.text.as_bytes());
    let kept = c_build::cache_dir()
        .join("llvm")
        .join(format!("{}.o", hex(&key.finish())));
    if let Ok(object) = std::fs::read(&kept) {
        return Ok(object);
    }

    let dir = TempDir::new()
        .map_err(|err| BuildError::Link(format!("cannot create a temporary directory: {err}")))?;
    let source = dir.path().join("program.ll");
    let object = dir.path().join("program.o");
    std::fs::write(&source, &ir.text)
        .map_err(|err| BuildError::Link(format!("cannot write the LLVM IR: {err}")))?;
    let read = |path: &Path| {
        std::fs::read(path)
            .map_err(|err| BuildError::Link(format!("cannot read the object clang wrote: {err}")))
    };
    let dwarf = |err: String| BuildError::Link(format!("cannot read the DWARF clang wrote: {err}"));
    let built = if cfg!(target_vendor = "apple") {
        // Mach-O, which `object` cannot write back: the table follows the
        // program's assembly.
        let assembly = dir.path().join("program.s");
        let first = dir.path().join("first.o");
        let mut args = compile;
        args.extend([
            "-S".into(),
            source.into_os_string(),
            "-o".into(),
            assembly.clone().into_os_string(),
        ]);
        crate::build::run(clang, &args)?;
        let assemble = |output: &Path| -> Result<(), BuildError> {
            let mut args: Vec<OsString> = vec!["-c".into(), "-x".into(), "assembler".into()];
            args.extend(target.iter().cloned());
            args.extend([
                assembly.clone().into_os_string(),
                "-o".into(),
                output.as_os_str().to_owned(),
            ]);
            crate::build::run(clang, &args)
        };
        assemble(&first)?;
        let tables = wip_llvm::frame_table_assembly(&read(&first)?, &ir.files).map_err(dwarf)?;
        // Bytes, not text: what the program's strings and names hold is
        // written as it is.
        let mut text = std::fs::read(&assembly).map_err(|err| {
            BuildError::Link(format!("cannot read the assembly clang wrote: {err}"))
        })?;
        text.extend_from_slice(tables.as_bytes());
        std::fs::write(&assembly, text)
            .map_err(|err| BuildError::Link(format!("cannot write the assembly: {err}")))?;
        assemble(&object)?;
        read(&object)?
    } else {
        // ELF: the object, written again with the table in it, with no
        // assembly between, whose names an assembler may read as registers.
        let mut args = compile;
        args.extend([
            "-c".into(),
            source.into_os_string(),
            "-o".into(),
            object.clone().into_os_string(),
        ]);
        crate::build::run(clang, &args)?;
        let built = wip_llvm::with_frame_tables(&read(&object)?, &ir.files).map_err(dwarf)?;
        std::fs::write(&object, &built)
            .map_err(|err| BuildError::Link(format!("cannot write the object: {err}")))?;
        built
    };
    keep(&kept, &object);
    Ok(built)
}

/// Puts an object in the cache, whole or not at all: written beside where
/// it goes under a name no other writer uses — builds run at once on the
/// threads of one process, the case suite's among them — then renamed
/// there. A cache that cannot be written is a build that is not kept, and
/// nothing worse.
fn keep(kept: &Path, object: &Path) {
    let (Some(dir), Some(name)) = (kept.parent(), kept.file_name()) else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let partial = dir.join(format!(
        "{}.{}.partial",
        name.to_string_lossy(),
        c_build::unique_suffix()
    ));
    if std::fs::copy(object, &partial).is_ok() && std::fs::rename(&partial, kept).is_err() {
        let _ = std::fs::remove_file(&partial);
    }
}

/// This compiler, as far as an object it made depends on it: the table of
/// calls is written by its code, which the IR does not say. Its path, size
/// and time, which a new build of it changes.
fn this_compiler() -> Vec<u8> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let mut id = exe.as_os_str().as_encoded_bytes().to_vec();
    if let Ok(meta) = std::fs::metadata(&exe) {
        id.extend_from_slice(&meta.len().to_le_bytes());
        if let Ok(modified) = meta.modified()
            && let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            id.extend_from_slice(&since.as_nanos().to_le_bytes());
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Objects kept at once by the threads of one process are each kept
    /// under their own key: the case suite builds so, and a name shared by
    /// the writes once put one program's object under another's.
    #[test]
    fn objects_kept_at_once_stay_their_own() {
        let dir = TempDir::new().expect("a temporary directory");
        let cache = dir.path().join("llvm");
        std::thread::scope(|scope| {
            for i in 0..32 {
                let (dir, cache) = (dir.path(), &cache);
                scope.spawn(move || {
                    let object = dir.join(format!("made{i}.o"));
                    std::fs::write(&object, vec![i as u8; 64 * 1024]).expect("written");
                    keep(&cache.join(format!("{i}.o")), &object);
                });
            }
        });
        for i in 0..32 {
            let kept = std::fs::read(cache.join(format!("{i}.o"))).expect("kept");
            assert!(
                kept.iter().all(|&byte| byte == i as u8),
                "{i}.o holds another's"
            );
        }
    }
}
