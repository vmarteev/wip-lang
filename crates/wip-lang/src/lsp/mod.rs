//! `wip lsp`: a language server, speaking the Language Server Protocol on
//! standard input and output.
//!
//! Every edit checks again the whole program each open file belongs to,
//! with what the editor has not saved in place of the disk; a program of
//! thousands of lines is checked in milliseconds, so nothing is kept from
//! one check to the next. Messages that arrive together are answered
//! together, and the program is checked once after them.

mod complete;
mod features;
pub(crate) mod navigate;
mod protocol;
mod signature;

use std::collections::{HashMap, HashSet};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;

use serde_json::{Value, json};

use crate::Overlay;
use features::Fix;

/// Runs the server until the editor says `exit`.
pub fn serve() -> ExitCode {
    let (sender, messages) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        while let Ok(Some(message)) = protocol::read(&mut input) {
            if sender.send(message).is_err() {
                break;
            }
        }
    });
    let mut server = Server::default();
    while let Ok(message) = messages.recv() {
        let mut next = Some(message);
        while let Some(message) = next {
            if let Some(code) = server.handle(message) {
                return code;
            }
            next = messages.try_recv().ok();
        }
        if server.dirty {
            server.recheck();
        }
    }
    // The editor went away without saying `exit`.
    ExitCode::from(1)
}

/// A program checked with a placeholder at the cursor, and where the
/// file asked about begins in it.
struct Patched {
    loaded: crate::Loaded,
    program: wip_hir::Program,
    base: u32,
}

/// A file the editor has open.
struct Document {
    path: PathBuf,
    text: String,
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, Document>,
    overlay: Overlay,
    /// The files that have diagnostics shown, to clear them once they have
    /// none.
    published: HashSet<String>,
    /// The fixes of the diagnostics shown, by file.
    fixes: HashMap<String, Vec<Fix>>,
    /// Each program last checked, to answer what its names mean.
    programs: Vec<(crate::Loaded, wip_hir::Program)>,
    /// The program each directory's files belong to, found once.
    roots: HashMap<PathBuf, PathBuf>,
    /// Whether an edit has not been checked yet.
    dirty: bool,
    shut_down: bool,
}

impl Server {
    /// Handles one message; `Some` is the exit code once the editor says
    /// `exit`.
    fn handle(&mut self, message: Value) -> Option<ExitCode> {
        let method = message["method"].as_str().unwrap_or_default().to_string();
        let params = &message["params"];
        let Some(id) = message.get("id").cloned() else {
            return self.notified(&method, params);
        };
        if method.is_empty() {
            // A response to a request of ours; the server makes none.
            return None;
        }
        // A request is answered from what the editor has sent so far.
        if self.dirty {
            self.recheck();
        }
        let answer = match method.as_str() {
            "initialize" => Ok(json!({
                "capabilities": {
                    "textDocumentSync": { "openClose": true, "change": 1, "save": true },
                    "documentFormattingProvider": true,
                    "documentSymbolProvider": true,
                    "hoverProvider": true,
                    "completionProvider": { "triggerCharacters": [".", ":"] },
                    "signatureHelpProvider": {
                        "triggerCharacters": ["(", ","],
                        "retriggerCharacters": [","],
                    },
                    "definitionProvider": true,
                    "referencesProvider": true,
                    "renameProvider": true,
                    "codeActionProvider": { "codeActionKinds": ["quickfix"] },
                },
                "serverInfo": { "name": "wip", "version": crate::VERSION },
            })),
            "shutdown" => {
                self.shut_down = true;
                Ok(Value::Null)
            }
            "textDocument/formatting" => Ok(self.with_document(params, features::format)),
            "textDocument/documentSymbol" => Ok(self.with_document(params, features::symbols)),
            "textDocument/codeAction" => Ok(self.code_actions(params)),
            "textDocument/hover" => Ok(self.at_position(params, features::hover)),
            "textDocument/completion" => Ok(self.complete(params)),
            "textDocument/signatureHelp" => Ok(self.signature_help(params)),
            "textDocument/references" => {
                let declaration = params["context"]["includeDeclaration"]
                    .as_bool()
                    .unwrap_or(true);
                Ok(
                    self.with_position(params, |programs, open, file, position| {
                        features::references(programs, open, file, position, declaration)
                    }),
                )
            }
            "textDocument/rename" => {
                let new = params["newName"].as_str().unwrap_or_default().to_string();
                let renamed = self.with_position(params, |programs, open, file, position| {
                    match features::rename(programs, open, file, position, &new) {
                        Ok(edit) => json!({ "ok": edit }),
                        Err(why) => json!({ "error": why }),
                    }
                });
                match renamed.get("ok") {
                    Some(edit) => Ok(edit.clone()),
                    None => Err((
                        -32803,
                        renamed["error"]
                            .as_str()
                            .unwrap_or("cannot rename")
                            .to_string(),
                    )),
                }
            }
            "textDocument/definition" => Ok(self.at_position(params, features::definition)),
            _ => Err((-32601, format!("`wip lsp` does not answer `{method}`"))),
        };
        let response = match answer {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": code, "message": message },
            }),
        };
        send(&response);
        None
    }

    fn notified(&mut self, method: &str, params: &Value) -> Option<ExitCode> {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        match method {
            "exit" => {
                return Some(if self.shut_down {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                });
            }
            "textDocument/didOpen" => {
                let text = params["textDocument"]["text"].as_str().unwrap_or_default();
                self.opened(uri, text.to_string());
            }
            "textDocument/didChange" => {
                // The whole text, as `initialize` asked for.
                let changes = params["contentChanges"].as_array();
                if let Some(text) = changes
                    .and_then(|c| c.last())
                    .and_then(|c| c["text"].as_str())
                {
                    self.opened(uri, text.to_string());
                }
            }
            "textDocument/didClose" => {
                if let Some(document) = self.documents.remove(uri) {
                    self.overlay.remove(&document.path);
                }
                self.dirty = true;
            }
            // A save, or a file changed on disk, can change what another
            // file means, and which program a file is in.
            "textDocument/didSave" | "workspace/didChangeWatchedFiles" => {
                self.roots.clear();
                self.dirty = true;
            }
            _ => {}
        }
        None
    }

    fn opened(&mut self, uri: &str, text: String) {
        let Some(path) = protocol::path_of(uri) else {
            return;
        };
        self.overlay.set(&path, text.clone());
        self.documents
            .insert(uri.to_string(), Document { path, text });
        self.dirty = true;
    }

    /// Runs `answer` on the document a request names, or answers `null`
    /// for one that is not open.
    fn with_document(&self, params: &Value, answer: fn(&Path, &str) -> Value) -> Value {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        match self.documents.get(uri) {
            Some(document) => answer(&document.path, &document.text),
            None => Value::Null,
        }
    }

    /// What could be written at the position a request names: the program
    /// is checked with a placeholder written there, so that it parses
    /// (see `complete`).
    fn complete(&mut self, params: &Value) -> Value {
        let empty = json!({ "isIncomplete": false, "items": [] });
        let Some((path, text, cursor)) = self.request_at(params) else {
            return empty;
        };
        let Some(patched) = self.patched(&path, &text, cursor, "") else {
            return empty;
        };
        complete::items(
            &patched.loaded,
            &patched.program,
            (&path, &text),
            patched.base,
            cursor,
        )
    }

    /// The signature of the call the cursor is in the arguments of, and
    /// which argument it is at.
    fn signature_help(&mut self, params: &Value) -> Value {
        let Some((path, text, cursor)) = self.request_at(params) else {
            return Value::Null;
        };
        let Some(call) = signature::call_at(&text, cursor) else {
            return Value::Null;
        };
        // The call may not be closed yet: then it is closed after the
        // placeholder.
        for close in ["", ")"] {
            if let Some(patched) = self.patched(&path, &text, cursor, close)
                && let Some(help) =
                    signature::help(&patched.loaded, &patched.program, patched.base, &call)
            {
                return help;
            }
        }
        Value::Null
    }

    /// The file a request names, its text, and the byte offset of the
    /// position it names; nothing for a file of the standard library.
    fn request_at(&self, params: &Value) -> Option<(PathBuf, String, usize)> {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        let document = self.documents.get(uri)?;
        if features::in_std(&document.path) {
            return None;
        }
        let cursor = protocol::Lines::new(&document.text).offset(&params["position"]);
        Some((document.path.clone(), document.text.clone(), cursor))
    }

    /// The program `path` is in, checked with the placeholder name, and
    /// `close` after it, written at `cursor`, so that the text there
    /// parses (see `complete`).
    fn patched(&mut self, path: &Path, text: &str, cursor: usize, close: &str) -> Option<Patched> {
        let mut patched = text.to_string();
        patched.insert_str(cursor, &format!("{}{close}", complete::PLACEHOLDER));
        let mut overlay = self.overlay.clone();
        overlay.set(path, patched);
        let root = features::program_root(path, &overlay, &mut self.roots);
        let mut loaded = crate::load_overlaid(&root, crate::Mode::Tests, &overlay).ok()?;
        let checked = crate::check_loaded(&mut loaded, &mut crate::Timings::default());
        let file = crate::canonical_file(path);
        let base = loaded.sources.entries().find_map(|(name, _, base)| {
            (Path::new(name).is_absolute() && crate::canonical_file(Path::new(name)) == file)
                .then_some(base)
        })?;
        Some(Patched {
            loaded,
            program: checked.program,
            base,
        })
    }

    /// Runs `answer` on the program that holds the document a request
    /// names, at the position it names.
    fn at_position(&self, params: &Value, answer: features::AtPosition) -> Value {
        self.with_position(params, answer)
    }

    /// Like [`Server::at_position`], for an answer that takes more than the
    /// position.
    fn with_position(
        &self,
        params: &Value,
        answer: impl Fn(
            &[(crate::Loaded, wip_hir::Program)],
            &[(&str, &Path, &str)],
            (&Path, &str),
            &Value,
        ) -> Value,
    ) -> Value {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        let Some(document) = self.documents.get(uri) else {
            return Value::Null;
        };
        let open: Vec<(&str, &Path, &str)> = self
            .documents
            .iter()
            .map(|(uri, d)| (uri.as_str(), d.path.as_path(), d.text.as_str()))
            .collect();
        let file = (document.path.as_path(), document.text.as_str());
        answer(&self.programs, &open, file, &params["position"])
    }

    /// Checks every program an open file belongs to, and shows what is
    /// wrong in each file, clearing what no longer is.
    fn recheck(&mut self) {
        self.dirty = false;
        let open: Vec<(&str, &Path, &str)> = self
            .documents
            .iter()
            .map(|(uri, d)| (uri.as_str(), d.path.as_path(), d.text.as_str()))
            .collect();
        let found = features::diagnostics(&open, &self.overlay, &mut self.roots);
        for (uri, diagnostics) in &found.diagnostics {
            send(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": { "uri": uri, "diagnostics": diagnostics },
            }));
        }
        // An open file with nothing wrong is told so, and one that had
        // something wrong is cleared.
        let clean = self.published.iter().chain(self.documents.keys());
        for uri in clean.collect::<HashSet<_>>() {
            if !found.diagnostics.contains_key(uri) {
                send(&json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": { "uri": uri, "diagnostics": [] },
                }));
            }
        }
        self.published = found.diagnostics.into_keys().collect();
        self.fixes = found.fixes;
        self.programs = found.programs;
    }

    /// The fixes of the diagnostics in the range asked about.
    fn code_actions(&self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        let Some(fixes) = self.fixes.get(uri) else {
            return json!([]);
        };
        let start = &params["range"]["start"];
        let end = &params["range"]["end"];
        let before = |a: &Value, b: &Value| {
            (a["line"].as_u64(), a["character"].as_u64())
                <= (b["line"].as_u64(), b["character"].as_u64())
        };
        let actions: Vec<Value> = fixes
            .iter()
            .filter(|fix| {
                before(&fix.diagnostic["range"]["start"], end)
                    && before(start, &fix.diagnostic["range"]["end"])
            })
            .map(|fix| {
                json!({
                    "title": fix.title,
                    "kind": "quickfix",
                    "diagnostics": [fix.diagnostic],
                    "isPreferred": true,
                    "edit": { "changes": fix.changes },
                })
            })
            .collect();
        json!(actions)
    }
}

fn send(message: &Value) {
    // Standard output is the editor's; if it has gone, so will the server.
    let _ = protocol::write(&mut io::stdout().lock(), message);
}
