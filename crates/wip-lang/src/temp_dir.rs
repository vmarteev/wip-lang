//! A directory that is removed when it is dropped.

use super::*;

/// A fresh directory under the system temporary directory, removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> std::io::Result<TempDir> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("wip-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(TempDir(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
