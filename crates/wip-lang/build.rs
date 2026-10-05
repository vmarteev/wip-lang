//! The compiler's version, with the commit it was built from where it was
//! built from a repository: `0.1.0 (1d16797a2b3c 2026-09-29)`, as `rustc
//! --version` names its own. A copy of the source without its history is
//! the version alone.

use std::path::Path;
use std::process::Command;

fn main() {
    let version = env!("CARGO_PKG_VERSION");
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo says where the crate is");
    let described = match commit(Path::new(&dir)) {
        Some((hash, date)) => format!("{version} ({hash} {date})"),
        None => version.to_string(),
    };
    println!("cargo:rustc-env=WIP_VERSION={described}");
}

/// The commit checked out, and the day it was made; and, for cargo, the
/// files that change when another is checked out or made.
fn commit(dir: &Path) -> Option<(String, String)> {
    let git_dir = git(dir, &["rev-parse", "--absolute-git-dir"])?;
    let git_dir = Path::new(&git_dir);
    // A file that is not there would have cargo run this every time.
    let watch = |path: &Path| {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    };
    watch(&git_dir.join("HEAD"));
    if let Some(branch) = git(dir, &["symbolic-ref", "-q", "HEAD"]) {
        watch(&git_dir.join(branch));
    }
    watch(&git_dir.join("packed-refs"));
    let hash = git(dir, &["rev-parse", "--short=12", "HEAD"])?;
    let date = git(dir, &["log", "-1", "--format=%cs", "HEAD"])?;
    Some((hash, date))
}

/// What `git args` prints, where git is there and succeeds.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.trim().to_string())
}
