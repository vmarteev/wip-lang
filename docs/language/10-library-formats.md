# 10.3. Connections and formats

Part of [the standard library](10-library.md): connections, HTTP and
TLS over them, and the formats data is written in — compressed, JSON,
bytes, base64, hex and URLs, HTML and XML — with regular expressions,
secrets and digests.

## Connections: `std::net`

A `Listener` listens on a port or a Unix-domain socket and accepts
connections; a `Stream` is one connection, accepted or made; an `Address`
says where one is, and prints as `127.0.0.1:8080` or `[::1]:8080`. Each
owns its descriptor and closes it where it ends, and what fails is an
`io::IoError`, as a file's failures are.

```wip,run
import std::future
import std::net::{Address, Listener, Stream}

fn main() = {
    // Port 0: the system chooses one, and `localAddress` says which.
    val listener = Listener::tcp("127.0.0.1", 0).unwrap()
    val .Ok(.V4(port: port, ..)) = listener.localAddress() else return
    val (_, answer) = future::both(
        () => {
            var (stream, from) = listener.accept().unwrap()
            var request: [u8; 64] = [0; 64]
            val count = stream.read(&var request).unwrap()
            stream.writeText("got \(count) bytes from \(from.host())").unwrap()
        },
        () => {
            var stream = Stream::tcp("127.0.0.1", port).unwrap()
            stream.writeText("hello").unwrap()
            var reply: [u8; 64] = [0; 64]
            val count = stream.read(&var reply).unwrap()
            return String::of(str::fromBytes(&reply[0..count]))
        },
    )
    assert(answer == "got 5 bytes from 127.0.0.1")
}
```

`Listener::tcp(.None, port)` listens on the loopback, and `"0.0.0.0"` or
`"::"` on every address. `Stream::tcp` waits as long as the system does
for a host to answer, and `Stream::tcpWithin(host, port, duration)` at
most that long for each of its addresses, answering `.TimedOut`. A
`Stream` is an `io::Read` and an `io::Write`: `read` answers `0` once the
other end has closed, and `writeAll` and `writeText` write all they are
given, each through `var`, as a file's do; `write` may write fewer
bytes, and takes `&self`. A write to a connection the other end has
closed answers `.BrokenPipe`, and no signal ends the program.
`setBlocking(false)` makes `accept`, `read` and `write` answer
`.WouldBlock` where they would wait, and `descriptor()` is the number an
event queue watches. `accept` takes `&self`, so several threads may
accept from one listener.

An `io::EventQueue` serves many connections on one thread: it is given
each descriptor a program waits on — a listener, its connections, a
watcher's — with what it waits for, `.Read`, `.Write` or `.Both`, and a
`token`, a `u64` of the program's own, such as a slot; `wait(timeout,
&var events)` sleeps until one is ready, or the timeout passes — `.None`
waits as long as it takes, and no time only looks — and fills `events`
with each that is: its `token`, whether it is `readable` and `writable`,
and whether it is `closed`, the other end gone or the descriptor failed.
A descriptor is told of while it is ready, every wait, until it is read
or written to `.WouldBlock`, so one read a wait is never left unread.
`change` waits on one for something else, and `remove` no longer. It is
kqueue on a Mac and on iOS, epoll on Linux, and `WSAPoll` on Windows,
which waits so on sockets alone, a listener's and its connections':

```wip,run
import std::io
import std::net::{Listener, Stream}
import std::time::{Duration}

fn main() = {
    val listener = Listener::tcp("127.0.0.1", 0).unwrap()
    val .Ok(.V4(port: port, ..)) = listener.localAddress() else return
    var queue = io::EventQueue::new().unwrap()
    assert(queue.add(listener.descriptor(), .Read, 0).isOk())
    var events: Vec<io::Event> = Vec()

    var client = Stream::tcp("127.0.0.1", port).unwrap()
    assert(queue.wait(Duration::seconds(5), &var events).isOk())
    assert(events[0].token == 0 && events[0].readable) // a connection came
    val (served, _) = listener.accept().unwrap()
    assert(queue.add(served.descriptor(), .Read, 1).isOk())

    client.writeText("hi").unwrap()
    assert(queue.wait(Duration::seconds(5), &var events).isOk())
    assert(events.len() == 1 && events[0].token == 1 && events[0].readable)
    assert(queue.wait(Duration::millis(0), &var events).isOk())
    assert(events.len() == 1) // told again, while it is unread
}
```

## HTTP: `std::http`

`http::get(url)` fetches a URL and answers its `Response`: its `status`,
its `headers`, its `body`, `text()` and `json<T>()`, and `header(name)`,
which finds a header in any case. A status is an answer, not an error:
a `404` is a `Response`, and an `HttpError` is what kept an answer from
coming — a URL it cannot send to, the connection, TLS, a timeout, too
many redirects, or an answer that is not HTTP.

Every request is sent to a `Url` (`std::url`, below), lent. A literal is
one where it is expected, `http::get("https://example.com/")`, its holes
encoded by their places; text from outside is read first, with
`Url::parse(text)?`. What only HTTP refuses of a URL is refused as it is
sent: a scheme other than `http` and `https`, credentials, a port that is
not a number.

A `Client` sends any `Request` — `Request::get(url)`, `post`, `put`,
`patch`, `delete`, `head`, each given `.header(name, value)` and a body of
`.body(bytes)`, `.text(text)`, `.json(&value)` or `.file(path)` — and keeps
a connection to each host between requests, follows redirects up to its
`redirects`, ten unless set, and stops waiting after its `timeout`, where
one is set. It asks for gzip, and undoes a body sent so; on Windows, which
has no zlib, it asks for the body as it is. `https` is the system's TLS,
which refuses a server whose certificate it does not trust;
`Client::overUnix(path)` sends every request over one Unix-domain socket.
`client.open(request)` answers once the status and the headers have come,
with the body an `io::Read` that comes as it is read, so that a body larger
than memory is never held whole; `client.upload(request, &var source)`
sends a body read from any `io::Read` as it is sent:

```wip,run
import std::future
import std::http::{Client, Request}
import std::net::{Listener}
import std::url::{Url}

fn main() = {
    val listener = Listener::tcp("127.0.0.1", 0).unwrap()
    val .Ok(.V4(port: port, ..)) = listener.localAddress() else return
    val (_, response) = future::both(
        () => {
            var (stream, _) = listener.accept().unwrap()
            var room = [0 as u8; 4096]
            stream.read(&var room).unwrap()
            val reply =
                "HTTP/1.1 201 Created\r\nContent-Length: 7\r\n\r\n[1,2,3]"
            stream.writeText(reply).unwrap()
        },
        () => {
            var client = Client()
            val item = [1, 2, 3]
            val url: Url = "http://127.0.0.1:\(port)/items"
            return client.send(Request::post(&url).json(&item)).unwrap()
        },
    )
    assert(response.status == 201 && response.isSuccess())
    val numbers: Vec<i64> = response.json().unwrap()
    assert(numbers == [1, 2, 3])
}
```

`std::tls` is the connection under `https`, for any protocol: a
`TlsStream::connect(host, port)` is an `io::Read` and an `io::Write`, the
server's certificate checked against the system's roots and the host's
name, and a `TlsError` says what refused it — the connection, the
certificate, or the handshake. It is OpenSSL on Linux, Network.framework
on a Mac and iOS, and SChannel on Windows:

```wip
import std::io
import std::tls::{TlsStream}

fn firstLine(host: str): String = {
    val .Ok(stream) = TlsStream::connect(host, 443) else return String()
    var secure = stream
    val request = "HEAD / HTTP/1.1\r\nHost: \(host)\r\nConnection: close\r\n\r\n"
    if secure.writeAll(&request).isErr() then
        return String()

    var room = [0 as u8; 64]
    val got = secure.read(&var room).unwrapOr(0)
    return str::fromBytes(&room[0..got])
}

fn main() = io::println(firstLine("example.com"))
```

## Compression: `std::compress`

`compress::gzip(bytes)` compresses as a `.gz` file holds, and
`gunzip(bytes)` undoes it; `zlib` and `unzlib` are zlib's wrapping, which
a PNG and HTTP's `deflate` hold, and `deflate` and `inflate` none. What
cannot be undone answers a `CompressError`: cut short, or not the format.
An `Inflater` and a `Deflater` take bytes in pieces, appending what each
makes to a `Vec<u8>`, so that nothing larger than a piece is held. It is
the system's zlib, which Windows does not have: there the module declares
nothing, and its names are refused as a Unix's:

```wip,run,unix
import std::compress

fn main() = {
    val text = "the same words, the same words, the same words"
    val packed = compress::gzip(&text)
    assert(packed[0] == 31 && packed[1] == 139)
    assert(str::fromBytes(compress::gunzip(&packed).unwrap()) == text)
    assert(compress::gunzip(&packed[0..10]).isErr())
}
```

## JSON: `std::json`

`json::parse(text)` reads a whole document into a `Json`, and a `Json`
interpolated is its JSON, compact.

```wip,run
import std::json
import std::json::{Json}

fn main() = {
    val .Ok(doc) = json::parse(
        "{\"name\": \"wip\", \"tags\": [1, 2.5, null]}",
    ) else {
        return
    }
    assert("\(doc)" == "{\"name\":\"wip\",\"tags\":[1,2.5,null]}")
    val .Object(fields) = doc else return
    assert(fields[0].0 == "name" && fields[0].1 == .Text("wip"))
    assert(fields[1].0 == "tags" && "\(fields[1].1)" == "[1,2.5,null]")

    val .Err(error) = json::parse("[1,\n 2,]") else return
    assert("\(error)" == "expected a value, at line 2, column 4")

    // A field and an element, by reference, and what a value is.
    assert(doc.get("name").flatMap((name) => name.asText()) == "wip")
    val second = doc.get("tags").flatMap((tags) => tags.item(1))
    assert(second.flatMap((tag) => tag.asFloat()) == 2.5)
    assert(doc.get("size").isNone())
}
```

`json::encode(&value)` writes any value that implements `Encode` as JSON's
text, and `json::decode<T>(text)` reads one that implements `Decode`,
answering a `json::DecodeError` that says what was wrong and at which
field ([page 8](08-interfaces.md)); `toJson` and `fromJson` do the same
with a `Json` tree. A record is an object of its fields, an `Option` that
is nothing a field left out, a map an object of its keys as text, and a
variant its name, or an object of its name and its fields.

A number is an `Int` where it is written without a fraction or an
exponent and fits an `i64`, and a `Float` otherwise; a `Float` is written
with a `.0` where it would read back as an `Int`, and one that is not a
number or is infinite is written `null`. An object keeps its fields in
the order written, a name written twice included. What is not JSON is a
`JsonError`, which says what, and where by line and column; arrays and
objects nested deeper than `json::MOST_NESTED`, 512, are one.

What is there is read with `get(name)`, a field of an object, and
`item(index)`, an element of an array, each by reference or nothing, and
with `asText`, `asInt`, `asFloat`, `asBool`, `asArray` and `isNull`, which
answer where the value is of that kind. A name written twice answers the
one written last.

## Bytes: `std::bytes`

A model's file, a texture's header, a save and a packet are numbers laid
out in bytes in a known order, little-endian (`le`) or big-endian (`be`).
`bytes::u32le(&header, 4)` reads the number at a place — `u16le`, `i16be`,
`u32be`, `f32le` and the rest — and `bytes::putU32le(&var header, 4, size)`
writes one there; each panics where its bytes are not all there, as an
index past the end does, and checks once, where an index for each byte
would check each. A `bytes::Reader` reads them from a slice in turn, `u8`
and the same numbers, each answering nothing past the end, and `seek`s; a
`bytes::Writer` appends them to a `Vec<u8>`, which `take()` hands over.

`bytes::encode(&value)` writes any value that implements `Encode` in a
binary format of its own, and `bytes::decode<T>(&bytes)` reads it back,
answering a `BinaryError` that says what was wrong and at which byte. It
holds what JSON does — a record keeps its fields' names, read in any
order, so a save written by one version of a program loads in the next,
which has a field more or one less — in less room, with whole numbers and
lengths in as few bytes as they need, each field's and each variant's name
spelled once and said after by where it was, and with no text to parse.
Bytes written before names were spelled once are read as they were:

```wip,run
import std::bytes
import std::bytes::{Reader, Writer}

@derive(Encode)
struct SaveBefore {
    name: String
    lives: i64
}

@derive(Decode)
struct Save {
    name: String
    level: i64 = 1
}

fn main() = {
    var out = Writer()
    out.u32le(0x46424C47)
    out.f32le(0.5)
    out.u16be(300)
    val written = out.take()
    var read = Reader::of(&written.items())
    assert(read.u32le() == 0x46424C47 && read.f32le() == 0.5)
    assert(read.u16be() == 300 && read.u8().isNone())

    var header = [0 as u8; 8]
    bytes::putU16be(&var header, 2, 300)
    bytes::putF32le(&var header, 4, 0.5)
    assert(header[2] == 1 && header[3] == 44)
    assert(bytes::u16be(&header, 2) == 300 && bytes::f32le(&header, 4) == 0.5)
    assert(bytes::u32le(&written, 0) == 0x46424C47)

    val old = bytes::encode(SaveBefore(name: "ann", lives: 3))
    val loaded = bytes::decode<Save>(&old.items()).unwrap()
    assert(loaded.name == "ann" && loaded.level == 1)
    assert(bytes::decode<Save>(&old.items()[0..4]).isErr())
}
```

## Bytes as text: `std::base64`, `std::hex` and `std::url`

Bytes that must travel as text — in JSON, a header, a URL, a line a
person copies — are written in one of three ways. `base64::encode(bytes)`
writes each three bytes as four characters, as RFC 4648 has it, with
`alphabet: .Url` for the alphabet a URL and a file name hold, and
`padded: false` where a format leaves out the `=`; `base64::decode(text)`
reads it, with its padding or without. `hex::encode(bytes)` writes two
digits a byte, in lower case unless `upper: true`, and `hex::decode`
reads either case. `url::encode(text)` writes every byte of `text` but
letters, digits, `-`, `.`, `_` and `~` as `%` and two digits, so that it
may stand in any part of a URL; what is passed as `keep` is left, a
path's `/`. `url::decode` undoes it, and leaves a `+` as it is. Each
decoding answers an error that says where the text went wrong:

```wip,run
import std::base64
import std::base64::{Base64Error}
import std::hex
import std::url

fn main() = {
    val token = base64::encode("wip:secret")
    assert(token == "d2lwOnNlY3JldA==")
    val back = base64::decode(&token).unwrap()
    assert(str::fromBytes(&back) == "wip:secret")
    val ends = [251 as u8, 255]
    assert(base64::encode(&ends, alphabet: .Url, padded: false) == "-_8")
    assert(base64::decode("d2lw OnNl") == .Err(Base64Error::NotInAlphabet(4)))

    assert(hex::encode([0 as u8, 15, 255]) == "000fff")
    assert(hex::decode("CAFE").unwrap() == [202 as u8, 254])

    val query = "q=\(url::encode("crème brûlée & tea"))"
    assert(query == "q=cr%C3%A8me%20br%C3%BBl%C3%A9e%20%26%20tea")
    assert(url::encode("/a b/c", keep: "/") == "/a%20b/c")
    assert(url::decode("cr%C3%A8me") == String::of("crème"))
}
```

`Url` is a URL's text and its parts: `scheme`, `host`, without an IPv6
address's brackets, `port`, `path`, `query` and `fragment`, each as
written, and `userInfo`. It may be relative, as a link is.
`Url::parse(text)` reads one, or answers the byte where the text is not
a URL: a scheme that is not a letter and then letters, digits, `+`, `-`
or `.`, a port that is not a number, a `%` without its two digits, a `[`
with no `]`, a space.

`Url` is a template: where one is expected, a literal is a URL, checked
while the program is compiled, and each hole's value is percent-encoded
as the part it stands in needs, so that it is always that part. In a path
it cannot add a segment, in a query it cannot add a parameter, and in a
host it cannot become a path; a port's value keeps only its digits as
they are. A hole in the scheme or inside a `%` and its digits is refused.
A `Url` in a hole is joined only at the start, as the URL the rest goes
on from, and adds to whatever part it ended in:

```wip,run
import std::url::{Url}

fn main() = {
    val api: Url = "https://api.example.com/v1"
    val user = "../admin"
    val tag = "a&admin=true"
    val posts: Url = "\(api)/users/\(user)/posts?tag=\(tag)"
    assert(
        posts.toStr() ==
            "https://api.example.com/v1/users/..%2Fadmin/posts?tag=a%26admin%3Dtrue",
    )
    assert(
        posts.host() == "api.example.com" &&
            posts.query() == "tag=a%26admin%3Dtrue",
    )

    val .Ok(read) = Url::parse("http://[::1]:8080/a?b=1#c") else return
    assert(read.host() == "::1" && read.port() == .Some(8080 as u16))
    assert(Url::parse("http://example.com:80a/").isErr())
}
```

## HTML and XML: `std::markup`

`Html` and `Xml` are templates: where one is expected, an interpolated
literal builds it, its own text as written and each value escaped as its
place in the page needs. A value of `Html` in a hole is joined as it is,
so a page is built from functions that answer `Html`, and a title, a name
or a number is text, escaped with no call to say so:

```wip,run
import std::markup::{Html}

fn row(id: i64, name: str): Html = {
    val href = "/users/\(id)"
    return "<li><a href=\"\(href)\">\(name)</a></li>"
}

fn page(title: str, rows: Html): Html = {
    return "<title>\(title)</title><ul>\(rows)</ul>"
}

fn main() = {
    var rows = Html()
    rows.push(row(1, "Ada & Bob"))
    rows.push(row(2, "<script>"))
    val written = page("Users", rows)
    assert(
        written.toStr() ==
            "<title>Users</title><ul><li><a href=\"/users/1\">Ada &amp; Bob</a></li><li><a href=\"/users/2\">&lt;script&gt;</a></li></ul>",
    )
}
```

`Html` reads where each hole is from the text written before it, as a
browser's tokenizer does, tag and attribute names in any case. Between
tags, in `<title>` and `<textarea>`, and in a quoted attribute's value, a
value's `&`, `<`, `>`, `"` and `'` are written as entities. In a link's
value — `href`, `src`, `action`, `formaction`, `cite`, `poster`, `data` and
`manifest` — a `:` a value writes before the link's scheme is known is
written `%3A` unless the scheme is `http`, `https` or `mailto`, so that
`javascript:` from outside is a relative path, which runs nothing and
still names the file it spells.

Anywhere else a value is not written safely by entities, and a hole there
is refused: inside `<script>` or `<style>`; in `style`, `srcset`, `srcdoc`
and any attribute whose name starts `on`; in an attribute's value with no
quotes; in a tag, between its attributes or in its name; and in a comment
or a doctype. Where a hole is follows from the literal's own text, so
`Html`'s `refusal` reads each literal as it reads a page, and the compiler
asks it of every literal that builds `Html`: a refused hole is an error
where it is written, whether or not the program ever writes the page.

```wip,error=E0486
import std::markup::{Html}

fn greeting(name: str): Html = "<script>const user = \"\(name)\";</script>"

fn main() = {
    val page = greeting("Ada")
}
```

A script is given its data in an attribute, escaped, and an attribute that
is there or not is two literals:

```wip,run
import std::markup::{Html}

fn link(href: str): Html = "<a href=\"\(href)\">link</a>"

fn checkbox(done: bool): Html =
    if done then "<input type=\"checkbox\" checked>"
    else "<input type=\"checkbox\">"

fn main() = {
    assert(
        link("javascript:go()").toStr() ==
            "<a href=\"javascript%3Ago()\">link</a>",
    )
    assert(
        link("https://example.com").toStr() ==
            "<a href=\"https://example.com\">link</a>",
    )
    val user = "Ada"
    val data: Html = "<main data-user=\"\(user)\"><script src=\"app.js\"></script>"
    assert(data.toStr().contains("data-user=\"Ada\""))
    assert(checkbox(true).toStr().contains("checked"))
}
```

`Html` is joined only between tags, and only `Html` that ends there. A hole
of `Html` anywhere else in a literal is refused with the rest, while the
program is compiled; a piece left inside a tag, a comment or a `<script>`
is refused as the page is written, with a panic where it is joined, since
where a piece ends depends on how it was built. So a function's piece
cannot leave its caller's page in a place it does not know of. `push` adds
`Html` at the end, `Html::joined(&pieces, ", ")` puts a separator between
pieces, and `Html::trusted(text)` is HTML another writer made, a renderer's
or a sanitizer's, which the caller vouches for. Written out, through
`Text`, `Html` is its text as it is, and a `String` put back into a hole is
text again.

`Xml` is the same without what is HTML's alone: a value between tags or in
a quoted attribute's value is escaped, and a hole in a tag, in an
unquoted value, a comment, CDATA or a processing instruction is refused.
A `<script>` in XML is an element like any other. A value given to
`pushValue` by name, not in a literal, is refused as the page is written,
with a panic at the call.

## Regular expressions: `std::regex`

A pattern is compiled once and then matched, searched and replaced with.
Matching takes time in proportion to the text times the pattern, whatever
either holds: no pattern backtracks without end.

`Regex` is a template: where one is expected, a literal is its pattern,
checked while the program is compiled and compiled where the literal
runs. A hole's value is matched as the text it is, each character that
means something in a pattern escaped, so `tar.gz` from outside matches
`tar.gz` and not `tarXgz`; and a hole of a `Regex` is its pattern, joined
as a group that does not capture, its groups counted after those before
it. `Regex::new(text)` compiles a pattern that comes from outside, and
answers what is wrong with it.

```wip,run
import std::regex::{Regex}

fn main() = {
    val numbered: Regex = "(\\d+)"
    assert(numbered.isMatch("img12.jpg"))
    assert(numbered.replaceAll("img12-3.jpg", "<$1>") == "img<12>-<3>.jpg")
    val .Some(found) = numbered.find("a1b22") else return
    assert(found.start == 1 && found.group(1) == .Some((1, 2)))

    val extension = "tar.gz"
    val ending: Regex = "\\.\(extension)$"
    assert(ending.isMatch("logs.tar.gz") && !ending.isMatch("logs.tarXgz"))
    val pair: Regex = "^\(numbered)-\(numbered)$"
    assert(pair.groups() == 2 && pair.isMatch("12-3"))

    assert(Regex::new("(unclosed").isErr())
}
```

A literal that is not a pattern is an error at the byte that stops it,
and a hole is refused where a value cannot be text: inside a class,
`[…]`, inside a repetition's braces, `{…}`, and after a `\`.

```wip,error=E0486
import std::regex::{Regex}

fn main() = {
    val line: Regex = "// \\[((java|c) line (\\d+)\\] (Error.*)"
}
```

The syntax is Rust's, in part: `.`, classes such as `[a-z]`, `[^0-9]`,
`\d`, `\w` and `\s`, the anchors `^` and `$`, groups `(…)` and `(?:…)`,
`|`, and `*`, `+`, `?` and `{n,m}`, with a `?` after them for as few as can
be. A replacement names what matched as `$0` and a group as `$1` or
`${1}`. What is not a pattern is a `RegexError`, which says what and at
which byte. `Regex::anyCase(pattern)` compiles one that matches ASCII
letters in either case, as POSIX's `REG_ICASE` does, so a host name
matches however it is written; a pattern cannot say so, so one is not
joined into another. `regex::escape(text)` is what a hole writes, for a
pattern built another way.

## Secrets and signatures: `std::secret` and `std::digest`

What an attacker must not guess — a key, a session's token, a salt — is
made by `std::secret`, from the system's generator for secrets:
`secret::bytes(count)` is that many bytes, and `secret::fill(&var key)`
fills an array. `Random` is never the answer, however it is seeded.

A signature is checked with `secret::equal(a, b)`, never `==`: `==` stops
at the first byte that differs, so how long it takes says how many were
right, and a signature can be found a byte at a time. `equal` looks at
every byte whatever they hold, in a time that depends on their lengths
alone, and the optimiser is kept from ending it early.

`std::digest` has SHA-256, SHA-512 and SHA-1, each a function of bytes in
hand and a type written in pieces, and HMAC over each, a digest under a
key: `hmacSha256(key, message)`, `hmacSha512` and `hmacSha1`. SHA-1 is
broken for signatures, since two inputs with one digest are made at will,
and is there for the formats that name data by it, as git and WebSocket's
handshake do; HMAC-SHA1, a one-time code's, is not broken by it. A webhook
signed as GitHub signs one, with the HMAC in hex after `sha256=`:

```wip,run
import std::digest
import std::hex
import std::secret

/// Whether `body` was signed with `key`, as the header says.
fn signed(body: &[u8], header: str, key: &[u8]): bool = {
    val .Some(("sha256", given)) = header.splitOnce("=") else return false
    val .Ok(sent) = hex::decode(given) else return false
    return secret::equal(&sent, digest::hmacSha256(&key, &body))
}

fn main() = {
    val key = secret::bytes(32)
    val body = "{\"action\": \"opened\"}"
    val header = "sha256=\(hex::encode(digest::hmacSha256(&key, &body)))"
    assert(signed(&body, header.toStr(), &key))
    assert(!signed(&body, "sha256=00", &key))
    val token = hex::encode(secret::bytes(16))
    assert(token.len() == 32)
}
```
