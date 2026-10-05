//! `wip debug`: the debugger, with the scripts that show
//! Wip's values as they are meant — a `Vec` as its elements, an enum as the
//! variant it holds. The scripts are part of the compiler, as the standard
//! library is, so they read the layouts of the compiler that built the
//! program; they are written into the cache when asked for.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::c_build;

const LLDB: &str = include_str!("../debugger/wip_lldb.py");
const GDB: &str = include_str!("../debugger/wip_gdb.py");

/// A debugger the scripts are written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Debugger {
    Lldb,
    Gdb,
}

impl Debugger {
    /// The one a system's own compiler comes with: lldb on a Mac, gdb
    /// elsewhere.
    pub fn native() -> Debugger {
        match cfg!(target_vendor = "apple") {
            true => Debugger::Lldb,
            false => Debugger::Gdb,
        }
    }
}

/// Where the scripts are, once written.
pub struct Scripts {
    pub lldb: PathBuf,
    pub gdb: PathBuf,
}

/// The scripts, written into the cache where they are not there already.
pub fn scripts() -> Result<Scripts, String> {
    let dir = c_build::generated(
        &c_build::cache_dir(),
        &[("wip_lldb.py", LLDB), ("wip_gdb.py", GDB)],
    )?;
    Ok(Scripts {
        lldb: dir.join("wip_lldb.py"),
        gdb: dir.join("wip_gdb.py"),
    })
}

/// The command that starts `debugger` on `program`, which is to be given
/// `args`, with the scripts loaded.
pub fn command(debugger: Debugger, program: &Path, args: &[OsString]) -> Result<Command, String> {
    let scripts = scripts()?;
    let mut command = match debugger {
        Debugger::Lldb => {
            let mut import = OsString::from("command script import ");
            import.push(&scripts.lldb);
            let mut command = Command::new("lldb");
            command.arg("-o").arg(import).arg("--").arg(program);
            command
        }
        Debugger::Gdb => {
            let mut command = Command::new("gdb");
            command
                .arg("-x")
                .arg(&scripts.gdb)
                .arg("--args")
                .arg(program);
            command
        }
    };
    command.args(args);
    Ok(command)
}
