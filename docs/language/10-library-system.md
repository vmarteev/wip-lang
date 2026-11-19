# 10.2. The system

Part of [the standard library](10-library.md): what a program is given
and what it reaches for on the machine it runs on — its arguments, its
log, the clock, random numbers, other programs, files and directories.

## A program's arguments: `std::args`

`main(args: &[cstring])` is given the program's arguments, and
`Arguments::of(&args)` reads them one at a time, from the one after the
program's name: each is a short option, a long one, or a positional
argument, and the program says what each means in a `match`, asking for an
option's value where it takes one:

```wip,run
import std::args::{ArgumentError, Arguments}

// What the options say; it holds names from the arguments, so it is a view.
view struct Options {
    count: i64 = 10
    verbose: bool = false
    files: Vec<str> = Vec()
}

fn options(args: &[cstring]): Result<Options, ArgumentError> = {
    var options = Options()
    var arguments = Arguments::of(&args)
    while arguments.next()? is .Some(argument) {
        match argument {
            .Short('n') | .Long("lines") => options.count = arguments.int()?
            .Short('v') | .Long("verbose") => options.verbose = true
            .Positional(file) => options.files.push(file)
            _ => return .Err(argument.unexpected())
        }
    }
    return options
}

fn main() = {
    // What `main` would be given for `lines -vn 5 a.txt --lines=3`.
    val given: [cstring; 5] = ["lines", "-vn", "5", "a.txt", "--lines=3"]
    val .Ok(read) = options(&given) else return
    assert(read.count == 3 && read.verbose && read.files.len() == 1)
    val wrong: [cstring; 2] = ["lines", "-x"]
    assert(options(&wrong) is .Err(.UnknownOption(..)))
}
```

It reads them as `getopt_long` does: `-abc` is `-a`, `-b` and `-c`;
`-ofile` and `-o file` are one option with its value, as are
`--output=file` and `--output file`; `--` ends the options, and `-` alone
is positional; options and positional arguments come in any order. A
value is the next argument whatever it looks like, so `-o -` and `-n -5`
are values.

`value()` answers an option's value, and `int()` and `float()` read it as
a number; `optionalValue()` takes only a value written onto its option,
`--color=always`; `rest()` answers every argument not read yet, as it is,
for a program that hands them on; `program()` is the name it was run by.
An `ArgumentError` says what is wrong — "`-n` needs a value", "`--verbose`
takes no value, and was given `yes`" — and `main` that answers one prints
it after `error: ` and exits with 1. A subcommand is a positional argument,
after which the same `Arguments` reads its own options. The usage message
is the program's own, written as text and printed where it matches `-h`.

## Logs: `std::log`

A `Log` is the least level a program keeps, `Level`'s `.Debug`, `.Info`,
`.Warn` or `.Error`, and where what it keeps goes. A function that logs
takes the program's log as an implicit parameter named `log`, `implicit
log: &Log` ([page 3](03-functions.md)), and is given it by every call that
has one, so its signature says that it logs, and a library and a program
that both name it `log` meet with nothing to set up. There is no global
log. A log is lent shared, so one serves every thread it is lent to.

`debug`, `info`, `warn` and `error` each make a `Record`, its level, its
text, the place of the call and the time, where the level is kept, and
give it to where the log's records go:

- **`Log::toStderr(least)`** writes each as a line on standard error, its
  time in UTC, its level and its text, in one write, so two threads' lines
  do not mix.
- **`Log::toSystem(least)`** gives each to the system's log, under the
  program's name: through `syslog` on a Unix, which Console shows on a
  Mac and `journalctl` on Linux, and to the Event Log on Windows, which
  the Event Viewer shows under Windows Logs, Application.
- **`Log::to(least, write)`** gives each to `write`, a closure that may be
  called from several threads at once: a line of the program's own, JSON,
  or what a test keeps.
- **`Log::none()`** keeps nothing, for a caller that needs no log.

```wip,run
import std::log::{Log}

fn served(path: str, implicit log: &Log) = {
    log.info("served \(path)")
    log.debug("not kept: below the least level")
}

fn main() = {
    implicit val log = Log::toStderr(least: .Info)
    served("/index.html")
    assert(log.enabled(.Warn) && !log.enabled(.Debug))
}
```

A message is made before the call, so `log.debug("\(tree)")` writes the
tree's text even where `.Debug` is not kept. Where that costs, ask first:
`if log.enabled(.Debug) then log.debug("\(tree)")`.

The library's own modules log nothing. They answer what went wrong, and
what of it to log is the program's to decide.

## Time: `std::time`

`std::time` measures and waits: a `Duration` is made in any unit and read
back in any, adds and compares, and is written in the largest unit it
fills; an `Instant` is a moment on a clock that only goes forward, which
means nothing alone and something against another:

```wip,run
import std::time::{Duration, Instant, sleep}

fn main() = {
    val start = Instant::now()
    sleep(Duration::millis(5))
    assert(start.elapsed() >= Duration::millis(5))
    assert("\(Duration::millis(1500))" == "1.5s")
}
```

A game updates its world at a steady rate, whatever rate it draws at, so
that its physics is the same on a fast machine and a slow one. A
`FixedStep` adds up real time with `advance()` once a frame; `due()`
answers whether another update fits in what it has added up, and takes it;
and `alpha()` is how far the frame is into the next update, from 0 to 1,
which the drawing goes between the last two by. A frame adds at most a
quarter of a second, so a stall — a breakpoint, a window dragged — runs
no long catch-up after it. `advanceBy(duration)` adds a given time, as a
test or a replay does:

```wip,run
import std::time::{Duration, FixedStep}

fn main() = {
    var step = FixedStep::of(60)
    var height = 10.0
    var updates = 0
    step.advanceBy(Duration::millis(40))
    while step.due() {
        height -= 9.8 * step.seconds() * step.seconds()
        updates += 1
    }
    assert(updates == 2 && step.alpha() > 0.39 && step.alpha() < 0.41)
    step.advanceBy(Duration::seconds(3))
    var caught = 0
    while step.due() do
        caught += 1

    assert(caught == 15, "a quarter of a second, and no more")
}
```

The date it is is another clock: `time::now()` answers the seconds since
1970 as the system says, which may be set back, and a `Utc` is such a
moment as a calendar has it — year, month, day, hour, minute, second and
weekday, in UTC, with no time zone and no leap second. `Utc::of` makes
one from its seconds, and `Utc::at` from its date and time of day, or
answers nothing for a day the month does not have, the 30th of February,
or an hour past 23. It is written as HTTP and as ISO 8601 write it, and
both are read back: `parseIso` reads RFC 3339's form, the one `iso()`
writes, with `Z` or any offset, and a fraction of a second left out:

```wip,run
import std::time
import std::time::{Utc}

fn main() = {
    assert(time::now() > 0)
    val moment = Utc::at(
        year: 1994,
        month: 11,
        day: 6,
        hour: 8,
        minute: 49,
        second: 37,
    ).unwrap()
    assert(moment.seconds() == 784111777 && moment.weekday() == 0)
    assert(Utc::at(year: 2100, month: 2, day: 29).isNone(), "not a leap year")
    assert(moment.httpDate() == "Sun, 06 Nov 1994 08:49:37 GMT")
    assert(moment.iso() == "1994-11-06T08:49:37Z")
    assert(Utc::parseHttpDate("Sun, 06 Nov 1994 08:49:37 GMT") == moment)
    assert(Utc::parseIso("1994-11-06T01:49:37.250-07:00") == moment)
    assert(Utc::parseIso("1994-11-06").isNone(), "a date alone")
}
```

A `Utc` is moved and measured by a `Duration`, as an `Instant` is, with
`after`, `before` and `since`, in whole seconds. A calendar's arithmetic
is its own: `addDays` keeps the time of day, `addMonths` keeps the day
of the month, or takes the month's last where it is shorter, and
`daysSince` counts the dates between two moments, not their hours:

```wip,run
import std::time::{Duration, Utc}

fn main() = {
    val due = Utc::at(year: 2026, month: 1, day: 31, hour: 9).unwrap()
    val next = due.addMonths(1)
    assert(next.month() == 2 && next.day() == 28 && next.hour() == 9)
    assert(due.addDays(1).month() == 2 && due.addDays(1).day() == 1)
    val later = due.after(Duration::seconds(90))
    assert(later.since(&due) == Duration::seconds(90))
    assert(next.daysSince(&due) == 28)
}
```

A `Local` is a moment in the time zone the program runs in, as the
system's database of zones says, `TZ` naming the zone where it is set: the
moment, the zone's offset from UTC then, in seconds and negative west of
Greenwich, and the zone's name then, `PDT` or `CET`, which says whether
summer time is kept. It reads as a `Utc` does, what the zone's clocks read
at that moment, and is written as ISO 8601 writes a moment with its offset,
`2026-10-10T11:26:03-07:00`. A date as a person reads it is written with
interpolation's widths:

```wip,run
import std::time
import std::time::{Local, Utc}

fn main() = {
    val now = time::now()
    val here = Local::of(now)
    assert(here.utc() == Utc::of(now))
    // What the zone's clocks read is UTC's, moved by the offset.
    val clock = Utc::of(now + here.offset())
    assert(here.day() == clock.day() && here.hour() == clock.hour())
    val day = "\(here.year())-\(here.month(), width: 2, fill: '0')-\(here.day(), width: 2, fill: '0')"
    assert(day.len() == 10 && "\(here)".startsWith(day.toStr()))
}
```

## Random numbers: `std::random`

`Random` is a generator of numbers for what is not cryptography — a game's
dice, a test's input, a shuffled deck. What it answers next follows from
what it has answered, so it is not fit to make a key or a password.
`Random::seeded(seed)` answers the same numbers on every machine and every
run, for a test or a game played again; `Random::fromEntropy()` is seeded
by the system, and differs on every run:

```wip,run
import std::random::{Random}

fn main() = {
    var rng = Random::seeded(42)
    val roll = rng.below(6) + 1
    assert(roll >= 1 && roll <= 6)
    val point = rng.inRange(-10, 10)
    assert(point >= -10 && point < 10)
    val chance = rng.float()
    assert(chance >= 0.0 && chance < 1.0)
    var deck = [1, 2, 3, 4, 5]
    rng.shuffle(&var deck)
    var again = Random::seeded(42)
    assert(again.below(6) + 1 == roll, "the same seed, the same numbers")
}
```

`below(count)` is a number from 0 up to the count, and `inRange(low, high)`
one from `low` up to `high`, leaving `high` out as a range does; each
number is as likely as any other, and an empty range panics. `next()` is
64 random bits, `float()` is from 0.0 up to 1.0, `bool()` is either, and
`shuffle` puts a slice's elements in any order, each as likely. The
generator is xoshiro256\*\*, with a seed spread through SplitMix64.

What a game draws is there too: `floatIn(low, high)`, `f32()` and
`f32In(low, high)`; `onCircle()` and `inDisk()` as `std::math`'s `Vec2`,
and `onSphere()` and `inSphere()` as its `Vec3`, each point as likely as
any other; `pick(&items)`, one of them, `.None` where there are none;
`weighted(&weights)`, an index chosen as often as its weight is of their
sum; and `gaussian(mean, deviation)`, a number from the bell curve:

```wip,run
import std::random::{Random}

fn main() = {
    var rng = Random::seeded(7)
    val spark = rng.onSphere()
    assert((spark.length() - 1.0).abs() < 0.0001, "a direction")
    val drop = rng.f32In(-0.5, 0.5)
    assert(drop >= -0.5 && drop < 0.5)
    val loot = ["coin", "gem", "map"]
    val .Some(found) = rng.pick(&loot) else panic("there is loot")
    assert(found.len() >= 3)
    val rare = rng.weighted([90.0, 9.0, 1.0]).unwrap()
    assert(rare >= 0 && rare < 3)
    val height = rng.gaussian(mean: 170.0, deviation: 8.0)
    assert(height > 100.0 && height < 240.0)
}
```

## Other programs: `std::process`

A `Command` names a program and its arguments, which no shell reads: each
is one argument, whatever it holds. `run()` gives it this program's
terminal and waits for it, `output()` keeps what it printed, `start()`
keeps it running as a `Child`, which is waited for where it is dropped,
and `launch()` lets it go:

```wip,run,unix
import std::process::{Command}

fn main() = {
    val .Ok(said) = Command::of("printf").args([
        "%s",
        "a;$(rm x) b",
    ]).output() else {
        assert(false, "printf is there")
        return
    }
    assert(said.stdout == "a;$(rm x) b")
    assert(said.exit.success())

    assert(Command::of("false").run() == .Code(1))
    assert(Command::of("no-such-program").run() is .Err(.NotFound))
}
```

An app on iOS may not start another program: there, `Command` answers the
error the system gives, as it answers any refusal; and iOS has none of the
programs a shell has, `printf` and `false` among them, even in the
simulator, which allows a start.

A program that ends with a code other than 0 has answered, not failed:
`exit.success()` asks. Failing to start is an `IoError`.

It is given this program's environment and directory unless its command
says otherwise: `env(name, value)` sets a variable, `unset(name)` leaves
one out, each in the order written, and `directory(path)` is where it
starts. It is given none of the files, sockets and pipes this program has
open, only its standard streams:

```wip,run,unix
import std::process::{Command}

fn main() = {
    val script = "echo $GREETING ${HOME-nowhere}"
    val command = Command::of("sh").env("GREETING", "hello").unset("HOME")
    val .Ok(said) = command.args(["-c", script]).output() else {
        assert(false, "sh is there")
        return
    }
    assert(said.stdout == "hello nowhere\n")
}
```

The program's own environment is `std::io`'s. `io::env(name)` reads one
variable, copied, and `io::environment()` lists every one, as name and
value. `io::setEnv(name, value)` and `io::unsetEnv(name)` change it for
what reads it from then on: this program, the C it links, which is told
some things only by a variable, `TZ` among them, and every program it
starts. They panic where the process runs a second thread, since C's
library does not make a change safe beside another thread's read, and a
thread a C library started reads too. So the environment is changed
before the first thread starts, and a program started is given a
variable of its own by its `Command`, which changes nothing here:

```wip,run
import std::io

fn main() = {
    io::setEnv("GREETING", "hello")
    assert(io::env("GREETING") is .Some(said) && said == "hello")
    var seen = 0
    for (name, value) in io::environment() do
        if name == "GREETING" && value == "hello" then
            seen += 1

    assert(seen == 1)
    io::unsetEnv("GREETING")
    assert(io::env("GREETING").isNone())
}
```

`connect()` starts it to be talked to as it runs: what this program writes
with `writeText`, it reads on its standard input, and what it writes this
program reads through `output()`, an `io::Reader`, by lines, by length or
to the end. Its standard error is this program's. `closeInput()` ends its
input, which is how many programs know to finish, and `wait()` closes it
and waits; dropped, it is closed and waited for too. Writing to a program
that has stopped reading answers `.BrokenPipe`. A `Reader` — this one, a
file's or standard input — may be told how long a read waits for
something to come, `setTimeout(.Some(Duration::seconds(5)))`, after which
it answers `.WouldBlock`; what it had read is appended where it was asked
for, so that asking again with the same `String` goes on from there:

```wip,run,unix
import std::io
import std::process::{Command}
import std::time::{Duration}

fn main() = {
    var echo = Command::of("cat").connect().unwrap()
    var line = String()
    echo.writeText("one\n").unwrap()
    echo.output().readLine(&var line).unwrap()
    echo.writeText("\(line), two\n").unwrap() // the next, from what came back
    line.clear()
    echo.output().readLine(&var line).unwrap()
    assert(line == "one, two")

    echo.output().setTimeout(Duration::millis(50))
    assert(echo.output().readLine(&var line) == .Err(io::IoError::WouldBlock))
    assert(echo.wait().success())
}
```

`input(text)` gives it text to read on its standard input, which
`output()` writes while it reads what the program prints and `run()`
writes before it waits. A program that stops reading ends the writing,
and the rest is dropped:

```wip,run,unix
import std::process::{Command}

fn main() = {
    val .Ok(sorted) = Command::of("sort").input("b\na\n").output() else {
        assert(false, "sort is there")
        return
    }
    assert(sorted.stdout == "a\nb\n")
}
```

A program catches signals with `Signals::catch`: Ctrl-C at its terminal,
`.Interrupt`; a request to end, `.Terminate`, as `kill` sends; its
terminal gone, `.Hangup`; Ctrl-\\, `.Quit`; and a program it started
ending, `.Child`. A signal caught ends nothing: the system keeps it, and
`received()` answers what came since it was last asked, without waiting,
as a program in a terminal asks each turn of its loop, to save before it
quits, and `wait()` waits for the next. `number()` is what the system
numbers one, which `libc::kill` sends to pass it on. `descriptor()` is
ready while one waits, for an event queue. No handler runs at a moment
of the system's choosing, so what a program does about a signal is
ordinary code. Where the `Signals` ends, each is as it was; a program
started meanwhile handles them as it would have. On Linux a signal is
held back in the thread that catches it and those it starts after, so a
program catches what it wants as it begins:

```wip,run,unix
import std::libc
import std::process
import std::process::{Signal, Signals}

fn main() = {
    var signals = Signals::catch([Signal::Interrupt, .Terminate]).unwrap()
    assert(signals.received().isEmpty())
    libc::kill(process::id() as c_int, libc::SIGINT) // as Ctrl-C would
    assert(signals.wait() == .Interrupt) // and the program goes on
}
```

A server that starts as root takes what it needs of root's and then gives
it up, and `std::process` has each step. `userId()` and `groupId()` are
whom the program acts as, 0 for root; `std::users` finds a `User` or a
`Group` by name or by number, `User::named("www")`, with its `id`, its
group, its home and its shell, or nothing where the system has none.
`fs::setOwner(path, user, group)` gives a file to them, and `fs::meta`
says whose a file is, `meta.user` and `meta.group`; `fs::setMode(path,
0o644)` sets its permission bits. `limit(resource)` and
`setLimit(resource, limit)` read and set how much the program may hold —
`.OpenFiles`, `.Processes`, `.CoreFile`, `.Stack` — as a `Limit` of what
it holds to now, `current`, and the most it may raise that to, `most`,
each nothing for no limit; any program may lower either, and only root
raise the most. Users and groups by number, the limits, `fork` and the
rest of this are a Unix's: Windows names its users otherwise, and a
program built for it that uses them is told they are a Unix's.

`fork()` copies the program: it answers `.Parent(child)` here, the copy
a `Child` as a program started is, and `.Child` in the copy, which goes
on from the same place. Only the thread that asks goes on in the copy,
so a program forks before it starts any. `newGroup()` makes the program
a group of its own, and `sendToGroup(signal)` passes a signal on to every
process in it. Then, in the copy that will serve, `changeRoot(path)`
makes a directory the program's `/`, moving into it, and
`becomeUser(user, group)` goes on as that user of that group alone, for
good, in the order that works; each is root's to do, and is refused
otherwise with `.Denied`:

```wip,run,unix
import std::fs
import std::libc
import std::process
import std::process::{Resource}
import std::users::{User}

fn main() = {
    val root = User::named("root").unwrap()
    assert(root.id == 0 && User::withId(0).unwrap().name == "root")
    val me = User::withId(process::userId()).unwrap()

    val dir = fs::TempDir::new().unwrap()
    val note = dir.join("note")
    assert(fs::writeFile(&note, "x").isOk())
    assert(fs::meta(&note).unwrap().user == me.id)
    assert(fs::setMode(&note, 0o600).isOk())
    assert(fs::meta(&note).unwrap().mode == 0o600)

    val files = process::limit(Resource::OpenFiles).unwrap()
    assert(process::setLimit(Resource::OpenFiles, files.clone()).isOk())

    match process::fork().unwrap() {
        .Parent(copy) => assert(copy.wait() == .Code(3))
        .Child => libc::exitAtOnce(3)
    }
    if me.id != 0 then
        assert(process::becomeUser(0, 0) == .Err(.Denied))
}
```

## Files and directories

`std::fs` reads and writes a whole file, lists a directory, and answers
what the filesystem says about a path. `struct stat` and `struct dirent`
are laid out differently on every system, so neither reaches Wip: what
comes back is a `Meta` and a `Vec<Entry>`.

```wip,run
import std::fs

fn main() = {
    val .Ok(entries) = fs::readDir("std/collections") else {
        // Run from somewhere else, there is nothing to list.
        return
    }
    assert(entries.len() >= 2)
    for entry in entries.items() do
        assert(entry.kind == .File || entry.isDirectory())

    val .Ok(about) = fs::meta("std/prelude/option.wip") else return
    assert(about.isFile())
    assert(about.size > 0)
    assert(about.modified > 0) // seconds since 1970
    assert(about.modifiedNanos < 1000000000) // and the nanoseconds past them
    assert(about.mode > 0) // the permission bits, as `chmod` writes them
    assert(about.inode > 0) // with `device`, which file it is, whatever its name
}
```

A `Meta`'s and an `Entry`'s `kind` is a `File`, a `Directory`, a `Link`,
or one of the special files — a `Pipe`, a `Socket`, a `CharDevice`, a
`BlockDevice` — or `Other`; `kind.isSpecial()` says whether it is none of
the first three. `meta` follows a symbolic link and `linkMeta` looks at
the link itself; `exists` asks whether there is anything there;
`readBytes` reads a whole file that need not be text, as a `Vec<u8>`;
`removeFile` removes a file and refuses a directory; `makeDir` makes a
directory and fails where one already is, and `File::createNew` makes a
file the same way, where `File::create` would empty one that is there. An
open `File` reads by lines, whole, or exactly so many bytes,
`readExact(count, &var into)`, which a message whose header gave its
length is read by, and `readAt(offset, into)` reads from where it is asked
without moving where the next read starts, as C's `pread` does; `size()`
says how long the file is. A `Meta`'s `device` and `inode` say which file
a path names, so a program can tell, before it acts, that a name it listed
still means what it did. Entries come in the order the filesystem gives
them, without `.` and `..`, and sorting is the caller's. `realPath`
answers where a path really is: absolute, with no `.`, `..` or link on the
way.

`fs::walk(dir)` is every entry under a directory, at any depth, each an
`Entry` whose `name` is its path from `dir`, `lang/main.wip`: depth first,
a directory before what it holds. A link is an entry and is not
followed. Its second argument says which directories it enters, and one
it does not enter is still an entry. It fails only where `dir` cannot be
read; a directory under it that cannot be is an entry with nothing under
it:

```wip,run
import std::fs
import std::path

/// A directory to go into: one that is not hidden.
fn shown(entry: &fs::Entry): bool = !path::name(entry.name).startsWith(".")

fn main() = {
    // The library's sources, but not what is in a hidden directory.
    val .Ok(walked) = fs::walk("std", shown) else {
        // Run from somewhere else, there is nothing to walk.
        return
    }
    val sources: Vec<String> = [
        for entry in walked do
            if entry.kind == .File && path::matches("**/*.wip", entry.name) then
                yield path::join("std", entry.name)
    ]
    assert(sources.any((file) => file == "std/path/path.wip"))
    assert(walked.any((entry) => entry.name == "prelude"))
}
```

Bytes as they come are `std::io`'s two interfaces. `io::Read` is what bytes
are read from: an open `File`, standard input and a program's output
(`io::Reader`), a connection (`net::Stream`); its `read(&var into)` puts
what comes next at the start of `into` and answers how many bytes, 0 only
at the end. A `File` and an `io::Reader` read bytes from the same buffer
they read lines from, so the two may follow each other. `io::Write` is what
bytes are written to — a `File`, a connection, a program's input, and
standard output and standard error as `io::stdout()` and `io::stderr()`
give them, which write through the buffer `print` writes through, in order
with it — and its `writeAll(bytes)` writes all of them, carrying on where a
write was cut short. A function that reads or writes without knowing which
takes a `&var dyn io::Read` or a `&var dyn io::Write`, and `io::copy(&var
from, &var to)` writes everything one reads to the other, as it comes, and
answers how many bytes:

```wip,run
import std::fs
import std::io

/// How many bytes are left to read, whatever they are read from.
fn counted(from: &var dyn io::Read): i64 = {
    var room = [0 as u8; 4]
    var total = 0
    while true {
        val got = from.read(&var room).unwrap()
        if got == 0 then
            break

        total += got
    }
    return total
}

fn main() = {
    val dir = fs::TempDir::new().unwrap()
    var first = fs::File::create(dir.join("first")).unwrap()
    assert(first.writeAll([1 as u8, 2, 3, 4, 5, 6, 7]).isOk())
    var from = fs::File::open(dir.join("first")).unwrap()
    var to = fs::File::create(dir.join("second")).unwrap()
    assert(io::copy(&var from, &var to) == 7)
    var again = fs::File::open(dir.join("second")).unwrap()
    assert(counted(&var again) == 7)
    var out = io::stdout()
    assert(out.writeAll("done\n").isOk())
}
```

`makeDirs` makes a directory and every one above it that is missing, with
none an error where it is there already; `removeDir` removes an empty
directory, and `removeAll` a directory and everything in it, or a file;
`rename` moves a path within its filesystem; `copy` writes a file's bytes
and its permission bits to another, and answers how many bytes;
`makeLink(target, at)` makes a symbolic link, which `removeAll` removes
without what it names.

`writeFile` writes text as the whole of a file and `writeBytes` bytes
that need not be text, each making the file or emptying it first. A file
that must never be found half written — a save, a document — is replaced
with `replaceFile` or `replaceBytes` instead: the new file is written
beside it, handed to the disk, and renamed into its place at once, so that
whoever opens it, and a program started after a crash, finds the old one
or the new one; it keeps the old one's permission bits, and where the path
is a symbolic link, the file the link names is replaced and the link
kept. A `TempDir` is a new directory of the system's temporary files,
named for the program and readable by its owner alone, removed with
everything in it where the value ends; `temporaryRoot()` is where such
directories are made, `$TMPDIR` or `/tmp` on a Unix, and on Windows the
directory `GetTempPath` answers, `%TEMP%`'s:

```wip,run
import std::fs

fn main() = {
    val dir = fs::TempDir::new().unwrap()
    val picture = dir.join("picture")
    assert(fs::writeBytes(picture, [0 as u8, 255, 0]).isOk())
    assert(fs::readBytes(picture).unwrap().len() == 3)

    val save = dir.join("save.json")
    assert(fs::replaceFile(save, "{\"coins\": 1}").isOk())
    assert(fs::replaceFile(save, "{\"coins\": 2}").isOk())
    assert(fs::readFile(save) == String::of("{\"coins\": 2}"))
    assert(fs::readDir(dir.path).unwrap().len() == 2) // nothing left beside
    assert(dir.path.startsWith(fs::temporaryRoot().trimEndMatches("/")))
}
```

`std::process` answers the program's own `currentDir()`, `executable()` —
the absolute path of the file it was started from — `besideProgram(path)`,
where `path` is beside that file, which is where the files a package names
with `@files` are, each package's code finding its own with
`package::besideProgram` ([page 12](12-tools.md)); this one is for what is
beside the program that no package names — and `id()`, and `std::io`
whether a stream is a terminal, `io::isTerminal(.Output)`, which is what
decides whether to colour what is written:

```wip,run
import std::fs
import std::io
import std::path
import std::process

fn main() = {
    val root = "target/docs-\(process::id())"
    assert(fs::makeDirs("\(root)/a/b").isOk())
    assert(fs::writeFile("\(root)/a/b/note", "hi").isOk())
    assert(fs::copy("\(root)/a/b/note", "\(root)/copy") == 2)
    assert(fs::rename("\(root)/copy", "\(root)/moved").isOk())
    assert(fs::makeLink("a", "\(root)/link").isOk())
    assert(fs::readFile("\(root)/link/b/note") == String::of("hi"))
    assert(fs::removeAll(root).isOk() && !fs::exists(root))
    assert(process::executable().isOk())
    assert(path::name(process::besideProgram("levels")) == "levels")
    val colour = io::isTerminal(.Output)
    assert(colour || !colour)
}
```

`std::path` takes a path apart and puts one together, as text: a `str`
of names between `/`s, which is what `std::fs` takes. Nothing in it reads
the disk. `clean` writes a path plainly, its `.`, `..` and doubled `/`s
worked out by the text alone, so a `..` after a link is taken as the
name's parent, where `fs::realPath` would follow the link. `relative`
answers a path from a directory it is within, and nothing for one it is
not. `withExtension` replaces a name's extension, or adds one. On Windows a
path's names are between `\`s, and a `/` is read as one too; `root` is what
a path begins at — `/` on a Unix, a drive's `C:\` or a share's
`\\server\share\` on Windows — and nothing of a relative path; `join` puts
the system's separator between, and `joinAsWritten` the one the directory
is written with, so that a path a person gave as `game/levels` is echoed
back as they wrote it:

```wip,run,unix
import std::path

fn main() = {
    assert(path::join("/a", "b") == "/a/b")
    assert(path::joinAsWritten("a/b", "c") == path::join("a/b", "c"))
    assert(path::root("/a/b") == "/" && path::root("a/b").isEmpty())
    assert(path::name("/a/b/c.tar.gz") == "c.tar.gz")
    assert(path::parent("/a/b/c.tar.gz") == "/a/b")
    assert(path::parent("c") == ".")
    assert(path::extension("c.tar.gz") == "gz")
    assert(path::extension(".bashrc").isNone())
    assert(path::stem("c.tar.gz") == "c.tar")
    assert(path::isWithin("/a/b", "/a") && !path::isWithin("/ab", "/a"))

    assert(path::clean("/a/./b//c/../d/") == "/a/b/d")
    assert(path::clean("../a/../../b") == "../../b")
    assert(path::relative("/a/b/c.txt", "/a") == "b/c.txt")
    assert(path::relative("/ab/c", "/a").isNone())
    assert(path::withExtension("a/b.wip", "o") == "a/b.o")
}
```

`path::matches(pattern, path)` says whether a whole path matches a glob,
as git matches the patterns of a `.gitignore`: `*` is any run of what is
not `/`, `?` one character but `/`, `[…]` one character of a set, with
ranges, and `[!…]` one outside it, and `**`, alone between `/`s or at an
end, any number of whole directories, none among them. `\` takes the
character after it as it is:

```wip,run
import std::path

fn main() = {
    assert(path::matches("*.wip", "main.wip"))
    assert(!path::matches("*.wip", "lang/main.wip"))
    assert(path::matches("**/*.wip", "lang/main.wip"))
    assert(path::matches("**/*.wip", "main.wip"))
    assert(path::matches("src/**/test_?.c", "src/a/b/test_1.c"))
    assert(path::matches("[!.]*", "shown"))
    assert(!path::matches("[!.]*", ".hidden"))
}
```

## Watching files: `std::watch`

A `Watcher` is told by the system when what it watches changes, and answers
which, when it is asked: `watch(path)` watches a directory for what is
made, written, renamed or removed in it — not what is deeper — or a file
for itself, written or replaced under its name, as an editor saves;
`changed()` answers each watched path that changed since it was last asked,
once, as it was given, and never waits; `pending()` says whether it would
answer anything; `unwatch(path)` lets one go. Writing a file is many
changes, and they are told once or twice, not once each; what changed a
moment before a path was watched may be told of too. `descriptor()` is
ready to read while a change waits, for an event queue. It is FSEvents on a
Mac, inotify on Linux and `ReadDirectoryChangesW` on Windows; on iOS, which
lets an app watch nothing, `Watcher::new()` answers `.Unsupported`:

```wip,run
import std::fs
import std::time::{Duration, Instant, sleep}
import std::watch::{Watcher}

fn main() = {
    val .Ok(var watcher) = Watcher::new() else return // iOS
    val dir = fs::TempDir::new().unwrap()
    assert(watcher.watch(dir.path).isOk())
    sleep(Duration::millis(200))
    watcher.changed() // what came just before
    assert(fs::writeFile(dir.join("level.txt"), "o..o").isOk())
    val began = Instant::now()
    while !watcher.pending() && began.elapsed() < Duration::seconds(5) do
        sleep(Duration::millis(10))

    assert(watcher.changed() == [dir.path.clone()].toVec())
}
```

## Files embedded: `std::embed`

`embed::bytes(path)` and `embed::text(path)` read a file when the program
is compiled, and its contents are a constant of the program, kept once in
memory nothing writes: a `&[u8]`, or a `str`, which must be UTF-8. The path
is a string literal, relative to the directory of the module that names it,
and may not leave the package — `..` past its root, a whole path, or a link
that leads out of it is refused. They are written where a constant's value
is worked out, a top-level `val` or `assert`, and a function reads the file
through that constant, which stands for the bytes as a reference parameter
does: `FONT.len()`, `FONT[0]`, `FONT.toVec()`. A program built this way
carries its assets in itself:

```wip,ignore
import std::embed

val FONT: &[u8] = embed::bytes("assets/font.png")
val HELP: str = embed::text("help.txt")

@comptime
val LEVELS: [Level; 12] = parseLevels(embed::text("levels.txt"))

assert(FONT.len() > 0)
```

`embed::textOr(path, otherwise)` is the text of a file that may not be
there: what a build writes beside a module, a commit or a date, which a
copy of the source does not have. Where the file is not there it is
`otherwise`, a string literal too; anything else wrong with the path is
refused as `embed::text` refuses it:

```wip,ignore
import std::embed

val BUILT: str = embed::textOr("built.txt", "")
```

## What the target is

Four constants the compiler writes say what machine the program is being
compiled for: `TARGET_OS` is `macos`, `linux`, `windows` or `ios`, which
the simulator is too; `TARGET_ARCH` is `arm64` or `x86_64`; `TARGET_VENDOR`
is `apple` on a Mac and on iOS, and empty elsewhere; and `TARGET_FAMILY` is
`windows` on Windows and `unix` on every other system.

```wip,run
fn main() = {
    val separator = if TARGET_FAMILY == "windows" then "\\" else "/"
    assert(separator == "/" || TARGET_OS == "windows")
    assert(TARGET_VENDOR == "apple" || TARGET_OS != "macos")
    assert(TARGET_FAMILY == "unix" || TARGET_OS == "windows")
}
```

They are for the places where splitting an item with `@target` is more than
the difference is worth. The branch that cannot be taken is folded away,
but it still has to compile — which is the opposite trade to `@target`, and
the reason both exist.
