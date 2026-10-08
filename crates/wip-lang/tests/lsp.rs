//! `wip lsp` as an editor sees it: a session over standard
//! input and output, with a program of two modules on disk, beside a
//! file that is not part of it, and an edit that is never saved.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};
use wip_lang::TempDir;

struct Session {
    child: Child,
    input: ChildStdin,
    messages: mpsc::Receiver<Value>,
    next_id: u64,
}

impl Session {
    fn start() -> Session {
        Session::spawn(Command::new(env!("CARGO_BIN_EXE_wip")))
    }

    /// A server whose cache is `cache`, so that what it writes there can
    /// be looked at.
    fn start_with_cache(cache: &Path) -> Session {
        let mut command = Command::new(env!("CARGO_BIN_EXE_wip"));
        command.env("WIP_CACHE_DIR", cache);
        Session::spawn(command)
    }

    fn spawn(mut command: Command) -> Session {
        let mut child = command
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("`wip lsp` runs");
        let input = child.stdin.take().expect("stdin");
        let mut output = BufReader::new(child.stdout.take().expect("stdout"));
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            loop {
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if output.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(n) = line.strip_prefix("Content-Length:") {
                        length = n.trim().parse().expect("a length");
                    }
                }
                let mut body = vec![0; length];
                output.read_exact(&mut body).expect("a body");
                let message = serde_json::from_slice(&body).expect("JSON");
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        Session {
            child,
            input,
            messages,
            next_id: 1,
        }
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{body}", body.len()).expect("written");
        self.input.flush().expect("flushed");
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Asks, and waits for the answer.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let answer = self.wait(|m| m["id"] == json!(id));
        assert!(answer.get("error").is_none(), "{answer}");
        answer["result"].clone()
    }

    /// The next message the server sends that `wanted` accepts.
    fn wait(&mut self, wanted: impl Fn(&Value) -> bool) -> Value {
        loop {
            let message = self
                .messages
                .recv_timeout(Duration::from_secs(20))
                .expect("the server answers");
            if wanted(&message) {
                return message;
            }
        }
    }

    /// The diagnostics shown next for `uri`.
    fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        let published = self.wait(|m| {
            m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri
        });
        published["params"]["diagnostics"]
            .as_array()
            .expect("a list")
            .clone()
    }
}

fn uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// A definition in the standard library opens the library the compiler
/// carries, written out once into the cache and read-only there: an
/// installed `wip` has no checkout to send an editor to.
#[test]
fn a_definition_in_the_standard_library_opens_what_the_compiler_carries() {
    let dir = TempDir::new().expect("a temporary directory");
    let root = std::fs::canonicalize(dir.path()).expect("canonical");
    let cache = root.join("cache");
    let program = root.join("main.wip");
    let text = "fn main() = {\n\tval o: Option<i64> = .None\n}\n";
    std::fs::write(&program, text).expect("written");
    let main = uri(&program);

    let mut session = Session::start_with_cache(&cache);
    session.request("initialize", json!({ "capabilities": {} }));
    session.notify("initialized", json!({}));
    session.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": main, "languageId": "wip", "version": 1, "text": text } }),
    );
    let found = session.request(
        "textDocument/definition",
        json!({ "textDocument": { "uri": main }, "position": { "line": 1, "character": 9 } }),
    );
    let target = found["uri"].as_str().unwrap_or_default();
    let cache = std::fs::canonicalize(&cache).expect("the cache was written");
    assert!(target.starts_with(&uri(&cache)), "{found}");
    assert!(target.ends_with("std/prelude/option.wip"), "{found}");
    let written = Path::new(target.trim_start_matches("file://"));
    let carried = include_str!("../../../std/prelude/option.wip");
    assert_eq!(std::fs::read_to_string(written).expect("readable"), carried);
    let permissions = std::fs::metadata(written).expect("there").permissions();
    assert!(permissions.readonly(), "the copy is not to be edited");
}

#[test]
fn an_editor_sees_what_is_wrong_and_how_to_fix_it() {
    let dir = TempDir::new().expect("a temporary directory");
    // The program is in a directory beside loose files of its own, which
    // are not part of it: its root is where it reaches the file edited.
    let above = std::fs::canonicalize(dir.path()).expect("canonical");
    std::fs::write(above.join("loose.wip"), "fn main() = 0\n").expect("written");
    let root = above.join("prog");
    std::fs::create_dir(&root).expect("created");
    std::fs::write(
        root.join("main.wip"),
        "import shapes\n\nfn main() = {\n\tval x = shapes::area()\n}\n",
    )
    .expect("written");
    std::fs::create_dir(root.join("shapes")).expect("created");
    let shapes = root.join("shapes/area.wip");
    let wrong = "pub fn area(): i64 = {\n\tval total = 1\n\ttotal = \"two\"\n\ttotal\n}\n";
    std::fs::write(&shapes, "pub fn area(): i64 = 1\n").expect("written");
    let main = uri(&root.join("main.wip"));
    let area = uri(&shapes);

    let mut session = Session::start();
    let initialized = session.request("initialize", json!({ "capabilities": {} }));
    assert_eq!(
        initialized["capabilities"]["documentFormattingProvider"],
        json!(true)
    );
    session.notify("initialized", json!({}));

    // The file on disk is right; the one in the editor is not, and the
    // mistakes are shown in it though the program starts in another.
    session.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": area, "languageId": "wip", "version": 1, "text": wrong } }),
    );
    let shown = session.diagnostics(&area);
    let codes: Vec<&str> = shown.iter().filter_map(|d| d["code"].as_str()).collect();
    assert!(!shown.is_empty(), "no diagnostics");
    let assigned = shown
        .iter()
        .find(|d| d["code"] == "E0304")
        .unwrap_or_else(|| panic!("no diagnostic about the `val`: {codes:?} {shown:?}"));
    assert_eq!(assigned["range"]["start"]["line"], json!(2), "{assigned}");

    // Its fix: declare it with `var`.
    let actions = session.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": area },
            "range": assigned["range"],
            "context": { "diagnostics": [assigned] },
        }),
    );
    let edits = actions
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|a| a["title"].as_str().is_some_and(|t| t.contains("var")))
        })
        .map(|a| a["edit"]["changes"][&area].clone())
        .unwrap_or_else(|| panic!("no fix offered: {actions}"));
    assert_eq!(edits[0]["newText"], json!("var"), "{edits}");
    assert_eq!(
        edits[0]["range"]["start"],
        json!({ "line": 1, "character": 1 }),
        "{edits}"
    );

    // The outline.
    let symbols = session.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": area } }),
    );
    assert_eq!(symbols[0]["name"], json!("area"), "{symbols}");
    assert_eq!(symbols[0]["kind"], json!(12), "{symbols}");

    // Put right, but not saved: the diagnostics go.
    let right = "/// The area of the one shape.\npub fn area(): i64 = {\n\tvar total = 1\n\ttotal = 2\n\ttotal\n}\n";
    session.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": area, "version": 2 },
            "contentChanges": [{ "text": right }],
        }),
    );
    assert_eq!(session.diagnostics(&area), Vec::<Value>::new());

    // A struct of two fields built without their names: the fix names the
    // field a variable is named for, and gives the other value the field
    // left.
    let built = "pub struct File {\n\tname: String\n\tfileNo: i64\n}\n\npub fn open(name: str, fileNo: i64): File = File(fileNo, String::of(name))\n";
    session.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": area, "version": 3 },
            "contentChanges": [{ "text": built }],
        }),
    );
    let shown = session.diagnostics(&area);
    let unnamed = shown
        .iter()
        .find(|d| d["code"] == "E0331")
        .unwrap_or_else(|| panic!("no diagnostic about the names: {shown:?}"));
    let actions = session.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": area },
            "range": unnamed["range"],
            "context": { "diagnostics": [unnamed] },
        }),
    );
    let edits = actions
        .as_array()
        .and_then(|a| {
            a.iter().find(|a| {
                a["title"]
                    .as_str()
                    .is_some_and(|t| t.contains("name the fields"))
            })
        })
        .map(|a| a["edit"]["changes"][&area].clone())
        .unwrap_or_else(|| panic!("no fix offered: {actions}"));
    let line = built.lines().nth(5).expect("the function");
    let at = |text: &str| line.find(text).expect("in the line") as u64;
    let inserted: Vec<(u64, &str)> = edits
        .as_array()
        .expect("a list")
        .iter()
        .map(|e| {
            assert_eq!(e["range"]["start"]["line"], json!(5), "{edits}");
            let column = e["range"]["start"]["character"]
                .as_u64()
                .unwrap_or_default();
            (column, e["newText"].as_str().unwrap_or_default())
        })
        .collect();
    assert_eq!(
        inserted,
        [
            (at("fileNo, String"), "fileNo: "),
            (at("String::of(name))"), "name: ")
        ],
        "{edits}"
    );
    // Arguments in each other's places: the fix names them.
    let swapped = "fn draw(width: i64, height: i64) = {}\n\nfn frame(width: i64, height: i64) = draw(height, width)\n";
    session.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": area, "version": 4 },
            "contentChanges": [{ "text": swapped }],
        }),
    );
    let shown = session.diagnostics(&area);
    let warned = shown
        .iter()
        .find(|d| d["code"] == "E0361")
        .unwrap_or_else(|| panic!("no warning about the order: {shown:?}"));
    assert_eq!(warned["severity"], json!(2), "{warned}");
    let actions = session.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": area },
            "range": warned["range"],
            "context": { "diagnostics": [warned] },
        }),
    );
    let edits = actions
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|a| a["title"].as_str().is_some_and(|t| t.contains("name them")))
        })
        .map(|a| a["edit"]["changes"][&area].clone())
        .unwrap_or_else(|| panic!("no fix offered: {actions}"));
    let line = swapped.lines().nth(2).expect("the call");
    let at = |text: &str| line.find(text).expect("in the line") as u64;
    let inserted: Vec<(u64, &str)> = edits
        .as_array()
        .expect("a list")
        .iter()
        .map(|e| {
            let column = e["range"]["start"]["character"]
                .as_u64()
                .unwrap_or_default();
            (column, e["newText"].as_str().unwrap_or_default())
        })
        .collect();
    assert_eq!(
        inserted,
        [
            (at("height, width)"), "width: "),
            (at("width)"), "height: ")
        ],
        "{edits}"
    );
    session.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": area, "version": 5 },
            "contentChanges": [{ "text": right }],
        }),
    );
    assert_eq!(session.diagnostics(&area), Vec::<Value>::new());

    // Formatting lays the file out as `wip fmt` does.
    session.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": main, "languageId": "wip", "version": 1,
            "text": "import shapes\n\nfn main() = {\nval x = shapes::area()\n}\n" } }),
    );
    let formatted = session.request(
        "textDocument/formatting",
        json!({ "textDocument": { "uri": main }, "options": { "tabSize": 4, "insertSpaces": false } }),
    );
    assert_eq!(
        formatted[0]["newText"],
        json!("import shapes\n\nfn main() = {\n\tval x = shapes::area()\n}\n"),
        "{formatted}"
    );
    // A file whose leading comments ask to be left as written is given no
    // edits.
    let kept = uri(&root.join("kept.wip"));
    session.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": kept, "languageId": "wip", "version": 1,
            "text": "// wip fmt: off\nval  KEPT = [1,2]\n" } }),
    );
    let untouched = session.request(
        "textDocument/formatting",
        json!({ "textDocument": { "uri": kept }, "options": { "tabSize": 4, "insertSpaces": false } }),
    );
    assert_eq!(untouched, json!([]), "{untouched}");

    // What names mean: hover shows what a name is and what its
    // declaration's comment says, and definition goes there, in another
    // module or in the standard library's sources.
    let text = "import shapes\n\nfn main() = {\n\tval x = shapes::area()\n\tval o: Option<i64> = .None\n}\n";
    session.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": main, "version": 2 }, "contentChanges": [{ "text": text }] }),
    );
    let at = |line: u32, character: u32| json!({ "textDocument": { "uri": main }, "position": { "line": line, "character": character } });
    let hover = session.request("textDocument/hover", at(3, 18));
    let shown = hover["contents"]["value"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(shown.contains("fn area(): i64"), "{hover}");
    assert!(shown.contains("The area of the one shape."), "{hover}");
    let declared = session.request("textDocument/definition", at(3, 18));
    assert_eq!(declared["uri"], json!(area), "{declared}");
    assert_eq!(declared["range"]["start"]["line"], json!(1), "{declared}");

    let local = session.request("textDocument/hover", at(3, 5));
    assert!(
        local["contents"]["value"]
            .as_str()
            .is_some_and(|v| v.contains("val x: i64")),
        "{local}"
    );

    let option = session.request("textDocument/definition", at(4, 9));
    assert!(
        option["uri"]
            .as_str()
            .is_some_and(|u| u.ends_with("std/prelude/option.wip")),
        "{option}"
    );

    // Completion, where the text does not parse yet: a module's items
    // after `::`, a value's methods after `.`, and the names in scope.
    let mut complete = |line: &str| {
        let text =
            format!("import shapes\n\nfn main() = {{\n\tval x = shapes::area()\n{line}\n}}\n");
        session.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": main, "version": 3 }, "contentChanges": [{ "text": text }] }),
        );
        let at = json!({ "line": 4, "character": line.len() });
        let list = session.request(
            "textDocument/completion",
            json!({ "textDocument": { "uri": main }, "position": at }),
        );
        let labels: Vec<String> = list["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i["label"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        labels
    };
    let module = complete("\tval y = shapes::");
    assert!(module.contains(&"area".to_string()), "{module:?}");
    let methods = complete("\tval y = x.");
    assert!(methods.contains(&"compare".to_string()), "{methods:?}");
    // What an interface's extensions give, where the condition holds: a
    // character is ordered, and does not add up.
    let chars = complete("\tval y = \"ab\".chars().");
    assert!(chars.contains(&"max".to_string()), "{chars:?}");
    assert!(!chars.contains(&"sum".to_string()), "{chars:?}");
    let scope = complete("\tval y = ");
    for name in ["x", "main", "shapes", "Option", "while"] {
        assert!(
            scope.contains(&name.to_string()),
            "{name} is not in {scope:?}"
        );
    }

    // References and rename: `area` is declared in one module and called
    // in the other.
    let text = "import shapes\n\nfn main() = {\n\tval x = shapes::area()\n\tval o: Option<i64> = .None\n}\n";
    session.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": main, "version": 4 }, "contentChanges": [{ "text": text }] }),
    );
    let place = |line: u32, character: u32| json!({ "textDocument": { "uri": main }, "position": { "line": line, "character": character } });
    let mut with = place(3, 18);
    with["context"] = json!({ "includeDeclaration": true });
    let found = session.request("textDocument/references", with);
    let mut places: Vec<(String, Value)> = found
        .as_array()
        .expect("a list")
        .iter()
        .map(|l| {
            (
                l["uri"].as_str().unwrap_or_default().to_string(),
                l["range"].clone(),
            )
        })
        .collect();
    places.sort_by_key(|(uri, _)| uri.clone());
    assert_eq!(places.len(), 2, "{found}");
    assert!(places.contains(&(
        main.clone(),
        json!({ "start": { "line": 3, "character": 17 }, "end": { "line": 3, "character": 21 } })
    )), "{found}");
    assert!(
        places.contains(&(
            area.clone(),
            json!({ "start": { "line": 1, "character": 7 }, "end": { "line": 1, "character": 11 } })
        )),
        "{found}"
    );

    let mut renaming = place(3, 18);
    renaming["newName"] = json!("size");
    let edit = session.request("textDocument/rename", renaming);
    assert_eq!(
        edit["changes"][&main][0]["newText"],
        json!("size"),
        "{edit}"
    );
    assert_eq!(
        edit["changes"][&area][0]["newText"],
        json!("size"),
        "{edit}"
    );

    // What the standard library declares is not renamed, nor is a name
    // given that is not one.
    for (at, name) in [(place(4, 9), "Maybe"), (place(3, 18), "while")] {
        let mut asked = at;
        asked["newName"] = json!(name);
        let id = 900;
        session.send(
            json!({ "jsonrpc": "2.0", "id": id, "method": "textDocument/rename", "params": asked }),
        );
        let refused = session.wait(|m| m["id"] == json!(id));
        assert!(refused.get("error").is_some(), "{refused}");
    }

    // What `@derive` writes is placed at its argument, and an editor
    // passes over it: a field is named where the program names it, and
    // nowhere in the annotation.
    let text = "@derive(Clone)\nstruct Point {\n\tone: i64\n}\n\nfn main() = {\n\tval p = Point { one: 1 }.clone()\n\tval n = p.one\n}\n";
    session.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": main, "version": 5 }, "contentChanges": [{ "text": text }] }),
    );
    let mut with = place(2, 2);
    with["context"] = json!({ "includeDeclaration": true });
    let found = session.request("textDocument/references", with);
    let lines: Vec<u64> = found
        .as_array()
        .expect("a list")
        .iter()
        .map(|l| l["range"]["start"]["line"].as_u64().unwrap_or_default())
        .collect();
    assert_eq!(lines, [2, 7], "{found}");
    let hover = session.request("textDocument/hover", place(0, 10));
    assert_eq!(hover, Value::Null, "{hover}");

    // A call's signature, at its second argument, before it is closed.
    let text = "fn main() = {\n\tval z = scale(1, \n}\n\n/// Times.\nfn scale(by: i64, of: i64): i64 = by * of\n";
    session.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": main, "version": 6 }, "contentChanges": [{ "text": text }] }),
    );
    let help = session.request(
        "textDocument/signatureHelp",
        json!({ "textDocument": { "uri": main }, "position": { "line": 1, "character": 17 } }),
    );
    assert_eq!(
        help["signatures"][0]["label"],
        json!("scale(by: i64, of: i64): i64"),
        "{help}"
    );
    assert_eq!(help["activeParameter"], json!(1), "{help}");
    assert_eq!(
        help["signatures"][0]["documentation"]["value"],
        json!("Times."),
        "{help}"
    );

    session.request("shutdown", Value::Null);
    session.notify("exit", Value::Null);
    let status = session.child.wait().expect("it ends");
    assert!(status.success(), "{status}");
}
