//! The Language Server Protocol's framing, and its way of naming places:
//! JSON-RPC messages behind a `Content-Length` header, files by `file://`
//! URI, and positions by line and UTF-16 column.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The next message, or `None` at the end of the input.
pub fn read(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length:") {
            length = n.trim().parse::<usize>().ok();
        }
    }
    let Some(length) = length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a message with no Content-Length",
        ));
    };
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn write(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = message.to_string();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}

/// Where each line of a text begins, to turn byte offsets into the
/// protocol's positions and back.
pub struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(text: &'a str) -> Lines<'a> {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { text, starts }
    }

    /// `{ line, character }` of a byte offset, the column counted in UTF-16
    /// code units as the protocol counts it.
    pub fn position(&self, offset: usize) -> Value {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        let character = self.text[self.starts[line]..offset].encode_utf16().count();
        json!({ "line": line, "character": character })
    }

    pub fn range(&self, lo: usize, hi: usize) -> Value {
        json!({ "start": self.position(lo), "end": self.position(hi) })
    }

    /// The byte offset of a `{ line, character }`; past the end of a line
    /// is its end, and past the last line is the end of the text.
    pub fn offset(&self, position: &Value) -> usize {
        let line = position["line"].as_u64().unwrap_or(0) as usize;
        let character = position["character"].as_u64().unwrap_or(0) as usize;
        let Some(&start) = self.starts.get(line) else {
            return self.text.len();
        };
        let mut units = 0;
        for (i, c) in self.text[start..].char_indices() {
            if units >= character || c == '\n' {
                return start + i;
            }
            units += c.len_utf16();
        }
        self.text.len()
    }

    /// The range of the whole text.
    pub fn whole(&self) -> Value {
        self.range(0, self.text.len())
    }
}

/// The file a `file://` URI names.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = rest
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            decoded.push(byte);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

/// The `file://` URI of a path, with what a URI may not hold escaped.
pub fn uri_of(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_is_counted_in_utf16() {
        let text = "val a = 1\nval é𝄞 = 2\n";
        let lines = Lines::new(text);
        let two = text.find('2').expect("a 2");
        assert_eq!(lines.position(two), json!({ "line": 1, "character": 10 }));
        assert_eq!(lines.offset(&json!({ "line": 1, "character": 10 })), two);
        assert_eq!(lines.offset(&json!({ "line": 0, "character": 99 })), 9);
        assert_eq!(
            lines.offset(&json!({ "line": 9, "character": 0 })),
            text.len()
        );
    }

    #[test]
    fn a_uri_names_a_path_and_back() {
        let path = Path::new("/My Projects/wip/main.wip");
        let uri = uri_of(path);
        assert_eq!(uri, "file:///My%20Projects/wip/main.wip");
        assert_eq!(path_of(&uri).as_deref(), Some(path));
    }
}
