//! The `wip` driver: reads source files, runs the pipeline, links with `cc`
//! and renders diagnostics. The only crate that produces text for humans.

mod backend;
mod build;
/// The compiler's version, with the commit it was built from where there
/// is one: `0.1.0 (1d16797a2b3c 2026-09-29)`.
pub const VERSION: &str = env!("WIP_VERSION");

pub mod c_build;
mod check;
pub mod debugger;
pub mod doc;
mod load;
pub mod lsp;
mod packages;
pub mod render;
mod sha256;
mod std_library;
pub mod targets;
mod temp_dir;
mod timings;
pub mod tools;

use std::collections::HashMap;
use std::ffi::OsString;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use wip_hir::Program;
use wip_syntax::ast::{Ast, Item};
use wip_syntax::{Diagnostic, Interner, Lexed, Parsed, Span, Symbol, lex_at, parallel, parse_at};

pub use backend::{Backend, choose_backend};
pub use build::*;
pub use check::*;
pub use load::*;
pub use packages::*;
pub use std_library::*;
pub use temp_dir::*;
pub use timings::*;
