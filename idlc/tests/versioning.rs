//! `silica-idlc check-compat` accepts exactly the additive changes of
//! `spec/IDL.md` section 4 (as refined by `spec/IDL_RATIONALE.md` section 3)
//! and rejects every other change with file, line and column; generated
//! clients and servers of two versions of one protocol interoperate in both
//! directions.

use std::path::{Path, PathBuf};
use std::process::Command;

use silica_idl_rt::{Array, CallError, CapList, Capability, Transport, TransportError};

mod v1 {
    include!(concat!(env!("OUT_DIR"), "/versions/v1/skew_counter.rs"));
}

mod v2 {
    include!(concat!(env!("OUT_DIR"), "/versions/v2/skew_counter.rs"));
}

// ---------------------------------------------------------------------------
// additive_only

/// Version 1 of a protocol using every construct the rules distinguish.
const BASE: &str = "\
/// Base.
protocol t.Proto version 1 {
    /// A color.
    enum Color : u8 {
        red = 0,
        green = 1,
    }

    enum Err : u32 {
        bad = 1,
    }

    enum NoZero : u16 {
        one = 1,
    }

    struct Point {
        x: i32,
        y: i32,
    }

    struct Holder {
        c: cap<Frame>,
    }

    /// Looks a key up.
    get @1 (key: string, at: Point) -> (value: bytes, color: Color) error Err;

    put @2 (key: string) -> ();

    give @3 (h: Holder) -> ();

    reserved @9;
}
";

/// `BASE` with each `(from, to)` replaced once; panics if `from` is absent,
/// so a typo cannot turn a case into a no-op.
fn edit(edits: &[(&str, &str)]) -> String {
    let mut s = BASE.to_string();
    for (from, to) in edits {
        assert!(s.contains(from), "edit target `{from}` not in BASE");
        s = s.replacen(from, to, 1);
    }
    s
}

fn parse(file: &str, src: &str) -> idlc::Protocol {
    idlc::parse(Path::new(file), src).unwrap_or_else(|e| {
        panic!(
            "{file} does not parse:\n{}\n{src}",
            e.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

fn compat(new: &str) -> Vec<idlc::Diagnostic> {
    idlc::compat::check_compat(&parse("old.idl", BASE), &parse("new.idl", new))
}

const V2: (&str, &str) = ("version 1", "version 2");

/// Changes `spec/IDL.md` section 4 allows.
fn accepted() -> Vec<(&'static str, String)> {
    vec![
        ("identical", BASE.to_string()),
        (
            "doc comments only, same version",
            edit(&[
                ("/// Looks a key up.", "/// Finds a key."),
                ("/// A color.", ""),
            ]),
        ),
        (
            "doc comments only, new version",
            edit(&[V2, ("/// Base.", "/// Base, revised.")]),
        ),
        (
            "version jumps by more than one",
            edit(&[("version 1", "version 3")]),
        ),
        (
            "method appended",
            edit(&[
                V2,
                (
                    "    reserved @9;",
                    "    reserved @9;\n    del @4 (key: string) -> ();",
                ),
            ]),
        ),
        (
            "method inserted before old items",
            edit(&[
                V2,
                (
                    "    /// A color.",
                    "    first @10 () -> (n: u64);\n    /// A color.",
                ),
            ]),
        ),
        (
            "struct and enum added",
            edit(&[
                V2,
                (
                    "    reserved @9;",
                    "    reserved @9;\n    struct Extra { n: u8 }\n    enum Shade : u8 { dark = 0 }",
                ),
            ]),
        ),
        (
            "parameters appended with every defaulted type",
            edit(&[
                V2,
                (
                    "put @2 (key: string)",
                    "put @2 (key: string, a: u8, b: u16, c: u32, d: u64, e: i8, f: i16, g: i32, h: i64, \
                     i: bool, j: string, k: bytes, l: array<u32>, m: option<u64>, n: Color, o: Point, \
                     p: array<Point>, q: option<Point>)",
                ),
            ]),
        ),
        (
            "result appended",
            edit(&[
                V2,
                ("color: Color) error", "color: Color, size: u64) error"),
            ]),
        ),
        (
            "result list grows from empty",
            edit(&[
                V2,
                (
                    "put @2 (key: string) -> ()",
                    "put @2 (key: string) -> (ok: bool)",
                ),
            ]),
        ),
        (
            "struct field appended",
            edit(&[
                V2,
                ("        y: i32,\n", "        y: i32,\n        z: i32,\n"),
            ]),
        ),
        (
            "struct field of a struct with defaults appended",
            edit(&[
                V2,
                (
                    "        y: i32,\n",
                    "        y: i32,\n        tint: Color,\n",
                ),
                ("put @2 (key: string)", "put @2 (key: string, spot: Point)"),
            ]),
        ),
        (
            "enum variant appended",
            edit(&[
                V2,
                (
                    "        green = 1,\n",
                    "        green = 1,\n        blue = 2,\n",
                ),
            ]),
        ),
        (
            "enum variant inserted",
            edit(&[
                V2,
                (
                    "        red = 0,\n",
                    "        red = 0,\n        black = 7,\n",
                ),
            ]),
        ),
        (
            "error enum variant appended",
            edit(&[
                V2,
                (
                    "        bad = 1,\n",
                    "        bad = 1,\n        worse = 2,\n",
                ),
            ]),
        ),
        (
            "new method using a new capability",
            edit(&[
                V2,
                (
                    "    reserved @9;",
                    "    reserved @9;\n    lend @5 (f: cap<Frame>) -> (h: Holder);",
                ),
            ]),
        ),
    ]
}

/// Changes the rules forbid, each with a fragment of the expected message.
fn rejected() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "method removed",
            edit(&[V2, ("    put @2 (key: string) -> ();\n", "")]),
            "method `put` removed",
        ),
        (
            "method renamed",
            edit(&[V2, ("put @2", "store @2")]),
            "method `put` removed",
        ),
        (
            "ordinal changed",
            edit(&[V2, ("put @2", "put @7")]),
            "ordinal of method `put` changed from 2 to 7",
        ),
        (
            "error changed",
            edit(&[V2, ("error Err;", "error Color;")]),
            "error of method `get` changed from `Err` to `Color`",
        ),
        (
            "error removed",
            edit(&[V2, (" error Err;", ";")]),
            "error of method `get` changed from `Err` to none",
        ),
        (
            "error added",
            edit(&[
                V2,
                (
                    "put @2 (key: string) -> ();",
                    "put @2 (key: string) -> () error Err;",
                ),
            ]),
            "error of method `put` changed from none to `Err`",
        ),
        (
            "parameter removed",
            edit(&[V2, ("(key: string, at: Point)", "(key: string)")]),
            "parameters of method `get`: field `at` removed",
        ),
        (
            "result removed",
            edit(&[V2, ("(value: bytes, color: Color)", "(value: bytes)")]),
            "results of method `get`: field `color` removed",
        ),
        (
            "parameter renamed",
            edit(&[V2, ("at: Point)", "point: Point)")]),
            "field `at` renamed to `point`",
        ),
        (
            "parameters reordered",
            edit(&[V2, ("(key: string, at: Point)", "(at: Point, key: string)")]),
            "parameters of method `get`: field `at` moved from position 2 to 1",
        ),
        (
            "parameter inserted before an old one",
            edit(&[
                V2,
                (
                    "(key: string, at: Point)",
                    "(key: string, n: u32, at: Point)",
                ),
            ]),
            "parameters of method `get`: field `at` moved from position 2 to 3",
        ),
        (
            "parameter retyped",
            edit(&[V2, ("put @2 (key: string)", "put @2 (key: bytes)")]),
            "type of field `key` changed from `string` to `bytes`",
        ),
        (
            "result retyped inside option",
            edit(&[V2, ("(value: bytes,", "(value: option<bytes>,")]),
            "type of field `value` changed from `bytes` to `option<bytes>`",
        ),
        (
            "capability type changed",
            edit(&[V2, ("c: cap<Frame>", "c: cap")]),
            "type of field `c` changed from `cap<Frame>` to `cap`",
        ),
        (
            "struct field removed",
            edit(&[V2, ("        y: i32,\n", "")]),
            "fields of struct `Point`: field `y` removed",
        ),
        (
            "struct field retyped",
            edit(&[V2, ("        y: i32,\n", "        y: i64,\n")]),
            "type of field `y` changed from `i32` to `i64`",
        ),
        (
            "struct removed",
            edit(&[
                V2,
                ("    struct Holder {\n        c: cap<Frame>,\n    }\n", ""),
                (
                    "    give @3 (h: Holder) -> ();\n",
                    "    give @3 (h: cap<Frame>) -> ();\n",
                ),
            ]),
            "struct `Holder` removed",
        ),
        (
            "struct turned into an enum",
            edit(&[
                V2,
                (
                    "    struct Holder {\n        c: cap<Frame>,\n    }\n",
                    "    enum Holder : u8 { none = 0 }\n",
                ),
            ]),
            "struct `Holder` changed to a enum",
        ),
        (
            "enum removed",
            edit(&[
                V2,
                ("    enum NoZero : u16 {\n        one = 1,\n    }\n", ""),
            ]),
            "enum `NoZero` removed",
        ),
        (
            "enum width changed",
            edit(&[V2, ("enum Color : u8", "enum Color : u16")]),
            "width of enum `Color` changed from u8 to u16",
        ),
        (
            "enum value changed",
            edit(&[V2, ("green = 1", "green = 2")]),
            "value of variant `green` of enum `Color` changed from 1 to 2",
        ),
        (
            "enum variant removed",
            edit(&[V2, ("        green = 1,\n", "")]),
            "variant `green` of enum `Color` removed",
        ),
        (
            "enum variant renamed",
            edit(&[V2, ("green = 1", "lime = 1")]),
            "variant `green` of enum `Color` removed",
        ),
        (
            "enum variants reordered",
            edit(&[
                V2,
                (
                    "        red = 0,\n        green = 1,\n",
                    "        green = 1,\n        red = 0,\n",
                ),
            ]),
            "variant `green` of enum `Color` moved before `red`",
        ),
        (
            "reserved ordinal removed",
            edit(&[V2, ("    reserved @9;\n", "")]),
            "reserved @9 removed",
        ),
        (
            "reserved ordinal reused",
            edit(&[V2, ("    reserved @9;", "    reuse @9 () -> ();")]),
            "reserved @9 removed",
        ),
        (
            "items reordered",
            edit(&[
                V2,
                (
                    "    put @2 (key: string) -> ();\n\n    give @3 (h: Holder) -> ();\n",
                    "    give @3 (h: Holder) -> ();\n\n    put @2 (key: string) -> ();\n",
                ),
            ]),
            "method `give` moved before method `put`",
        ),
        (
            "appended capability",
            edit(&[
                V2,
                (
                    "put @2 (key: string)",
                    "put @2 (key: string, f: cap<Frame>)",
                ),
            ]),
            "appended field `f` of type `cap<Frame>` can carry a capability",
        ),
        (
            "appended optional capability",
            edit(&[
                V2,
                (
                    "put @2 (key: string)",
                    "put @2 (key: string, f: option<cap>)",
                ),
            ]),
            "appended field `f` of type `option<cap>` can carry a capability",
        ),
        (
            "appended array of endpoints",
            edit(&[
                V2,
                (
                    "color: Color) error",
                    "color: Color, peers: array<endpoint<t.Proto>>) error",
                ),
            ]),
            "appended field `peers` of type `array<endpoint<t.Proto>>` can carry a capability",
        ),
        (
            "appended struct holding a capability",
            edit(&[
                V2,
                (
                    "put @2 (key: string)",
                    "put @2 (key: string, h: option<Holder>)",
                ),
            ]),
            "appended field `h` of type `option<Holder>` can carry a capability",
        ),
        (
            "appended enum without a zero variant",
            edit(&[
                V2,
                ("put @2 (key: string)", "put @2 (key: string, n: NoZero)"),
            ]),
            "appended field `n` of type `NoZero` has no default",
        ),
        (
            "appended struct without defaults",
            edit(&[
                V2,
                (
                    "    reserved @9;",
                    "    reserved @9;\n    struct Tagged { tag: NoZero }",
                ),
                ("        y: i32,\n", "        y: i32,\n        t: Tagged,\n"),
            ]),
            "appended field `t` of type `Tagged` has no default",
        ),
        (
            "addition without a version increment",
            edit(&[("    reserved @9;", "    reserved @9;\n    del @4 () -> ();")]),
            "additive change without a version increment (still 1)",
        ),
        (
            "protocol renamed",
            edit(&[V2, ("t.Proto", "t.Other")]),
            "protocol renamed from `t.Proto` to `t.Other`",
        ),
    ]
}

#[test]
fn additive_only() {
    for (what, new) in accepted() {
        let errs = compat(&new);
        assert!(
            errs.is_empty(),
            "{what}: rejected:\n{}",
            errs.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    for (what, new, want) in rejected() {
        let errs = compat(&new);
        let shown = errs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            errs.iter().any(|e| e.message.contains(want)),
            "{what}: want a diagnostic containing `{want}`, got:\n{shown}"
        );
        for e in &errs {
            assert!(
                e.span.line >= 1 && e.span.col >= 1,
                "{what}: diagnostic without a position: {e}"
            );
            let file = e.file.to_string_lossy();
            assert!(
                file == "old.idl" || file == "new.idl",
                "{what}: diagnostic names no input file: {e}"
            );
        }
    }

    // A decreased version, checked by swapping the roles of two versions.
    let errs = idlc::compat::check_compat(&parse("old.idl", &edit(&[V2])), &parse("new.idl", BASE));
    assert!(
        errs.iter()
            .any(|e| e.message == "version decreased from 2 to 1" && e.span.line == 2),
        "{errs:?}"
    );

    // Positions point at the offending text in the new file.
    let errs = compat(&edit(&[V2, ("put @2", "put @7")]));
    let e = &errs[0];
    assert_eq!(
        (e.file.as_path(), e.span.line, e.span.col),
        (Path::new("new.idl"), 29, 10),
        "{e}"
    );
    // ...and a removal at the removed text in the old file.
    let errs = compat(&edit(&[V2, ("    reserved @9;\n", "")]));
    let e = &errs[0];
    assert_eq!(
        (e.file.as_path(), e.span.line, e.span.col),
        (Path::new("old.idl"), 33, 15),
        "{e}"
    );

    // The version-skew fixture is a compatible evolution.
    let (old, new) = (fixture("v1"), fixture("v2"));
    idlc::check_compat_files(&old, &new).unwrap_or_else(|e| panic!("{e:?}"));
    // Swapped, it is not: it removes what version 2 added.
    assert!(idlc::check_compat_files(&new, &old).is_err());

    // The command line: exit 0 when compatible, 1 with `ERROR:` lines when
    // not, 2 on a usage error.
    let bin = env!("CARGO_BIN_EXE_silica-idlc");
    let out = Command::new(bin)
        .arg("check-compat")
        .arg(&old)
        .arg(&new)
        .output()
        .expect("run silica-idlc");
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("versioning");
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("old.idl"), dir.join("new.idl"));
    std::fs::write(&a, BASE).unwrap();
    std::fs::write(&b, edit(&[V2, ("put @2", "put @7")])).unwrap();
    let out = Command::new(bin)
        .arg("check-compat")
        .arg(&a)
        .arg(&b)
        .output()
        .expect("run silica-idlc");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 1, "{err}");
    assert!(lines[0].starts_with("ERROR: "), "{err}");
    assert!(
        lines[0].ends_with(":29:10: ordinal of method `put` changed from 2 to 7"),
        "{err}"
    );

    let out = Command::new(bin)
        .arg("check-compat")
        .arg(&a)
        .output()
        .expect("run silica-idlc");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

fn fixture(v: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("versions")
        .join(v)
        .join("skew")
        .join("counter.idl")
}

// ---------------------------------------------------------------------------
// version_skew

/// Stand-in capability.
#[derive(Clone, Debug, PartialEq)]
struct Cap(u32);

impl Capability for Cap {}

/// A version 1 provider that records what it was given.
#[derive(Default)]
struct OldServer {
    total: u64,
    steps: Vec<u32>,
    frames: Vec<Cap>,
}

impl v1::counter::Server<Cap> for OldServer {
    fn add(
        &mut self,
        step: v1::counter::Step,
    ) -> Result<v1::counter::AddReply, v1::counter::Fault> {
        self.steps.push(step.by);
        self.total += u64::from(step.by);
        Ok(v1::counter::AddReply { total: self.total })
    }

    fn status(&mut self) -> v1::counter::StatusReply {
        v1::counter::StatusReply {
            mode: v1::counter::Mode::Down,
            total: self.total,
        }
    }

    fn swap(&mut self, frame: Cap) -> v1::counter::SwapReply<Cap> {
        self.frames.push(frame.clone());
        v1::counter::SwapReply { frame }
    }
}

/// What a version 2 provider saw of one `add` request.
#[derive(Debug, PartialEq)]
struct SeenAdd {
    by: u32,
    label: String,
    mode: v2::counter::Mode,
    repeat: u32,
    window: Option<v2::counter::Window>,
    strict: bool,
}

/// A version 2 provider that records what it was given and fills every
/// field version 2 added.
#[derive(Default)]
struct NewServer {
    total: u64,
    history: Vec<u64>,
    adds: Vec<SeenAdd>,
    offsets: Vec<u64>,
    resets: Vec<u64>,
    fail: bool,
}

impl v2::counter::Server<Cap> for NewServer {
    fn add<'a>(
        &'a mut self,
        step: v2::counter::Step<'a>,
        repeat: u32,
        window: Option<v2::counter::Window>,
        strict: bool,
    ) -> Result<v2::counter::AddReply<'a, Cap>, v2::counter::Fault> {
        self.adds.push(SeenAdd {
            by: step.by,
            label: step.label.to_string(),
            mode: step.mode,
            repeat,
            window: window.clone(),
            strict,
        });
        if self.fail {
            return Err(v2::counter::Fault::Frozen);
        }
        for _ in 0..repeat.max(1) {
            self.total += u64::from(step.by);
            self.history.push(self.total);
        }
        Ok(v2::counter::AddReply {
            total: self.total,
            history: Array::new(&self.history),
            note: "added by version 2",
        })
    }

    fn status(&mut self) -> v2::counter::StatusReply {
        v2::counter::StatusReply {
            mode: v2::counter::Mode::Hold,
            total: self.total,
            window: v2::counter::Window { lo: 3, hi: 9 },
        }
    }

    fn swap(&mut self, frame: Cap, offset: u64) -> v2::counter::SwapReply<Cap> {
        self.offsets.push(offset);
        v2::counter::SwapReply {
            frame,
            length: 4096,
        }
    }

    fn reset(&mut self, to: u64) -> v2::counter::ResetReply {
        self.resets.push(to);
        self.total = to;
        v2::counter::ResetReply {}
    }
}

/// Hands each request to a version 1 provider's `dispatch`.
struct ToOld(OldServer);

impl Transport<Cap> for ToOld {
    fn call(
        &mut self,
        message: &mut [u8],
        len: usize,
        caps: CapList<Cap>,
        reply_caps: &mut CapList<Cap>,
    ) -> Result<usize, TransportError> {
        let request = message[..len].to_vec();
        let (request, reply) = (&request[..], message);
        Ok(v1::counter::dispatch(
            &mut self.0,
            request,
            caps,
            reply,
            reply_caps,
        ))
    }
}

/// Hands each request to a version 2 provider's `dispatch`.
struct ToNew(NewServer);

impl Transport<Cap> for ToNew {
    fn call(
        &mut self,
        message: &mut [u8],
        len: usize,
        caps: CapList<Cap>,
        reply_caps: &mut CapList<Cap>,
    ) -> Result<usize, TransportError> {
        let request = message[..len].to_vec();
        let (request, reply) = (&request[..], message);
        Ok(v2::counter::dispatch(
            &mut self.0,
            request,
            caps,
            reply,
            reply_caps,
        ))
    }
}

#[test]
fn version_skew() {
    assert_eq!((v1::counter::VERSION, v2::counter::VERSION), (1, 2));
    assert_eq!(v1::counter::NAME, v2::counter::NAME);

    // An old client and a new server.
    let mut old: v1::counter::Client<ToNew, Cap> = ToNew(NewServer::default()).into();
    // Absent appended parameters reach the new server as their defaults;
    // results the old client does not know are skipped.
    let r = old.add(v1::counter::Step { by: 5 }).unwrap();
    assert_eq!(r.total, 5);
    // New enum variants reach the old client as `Unknown`, not as an error.
    let r = old.status().unwrap();
    assert_eq!(r.mode, v1::counter::Mode::Unknown(2));
    assert_eq!(r.total, 5);
    // A new variant of the error enum too.
    old.core.transport.0.fail = true;
    assert_eq!(
        old.add(v1::counter::Step { by: 1 }).unwrap_err(),
        CallError::Error(v1::counter::Fault::Unknown(2))
    );
    old.core.transport.0.fail = false;
    // Capabilities still travel with appended plain fields around them.
    let r = old.swap(Cap(7)).unwrap();
    assert_eq!(r.frame, Cap(7));
    let seen = &old.core.transport.0;
    assert_eq!(
        seen.adds[0],
        SeenAdd {
            by: 5,
            label: String::new(),
            mode: v2::counter::Mode::Up,
            repeat: 0,
            window: None,
            strict: false,
        }
    );
    assert_eq!(seen.adds.len(), 2);
    assert_eq!(seen.offsets, [0]);

    // A new client and an old server.
    let mut new: v2::counter::Client<ToOld, Cap> = ToOld(OldServer::default()).into();
    // Appended parameters the old server does not know are skipped; appended
    // results it does not send take their defaults.
    let r = new
        .add(
            v2::counter::Step {
                by: 4,
                label: "four",
                mode: v2::counter::Mode::Hold,
            },
            3,
            Some(v2::counter::Window { lo: 1, hi: 2 }),
            true,
        )
        .unwrap();
    assert_eq!(r.total, 4);
    assert!(r.history.is_empty());
    assert_eq!(r.note, "");
    let r = new.status().unwrap();
    assert_eq!(r.mode, v2::counter::Mode::Down);
    assert_eq!(r.total, 4);
    assert_eq!(r.window, v2::counter::Window { lo: 0, hi: 0 });
    let r = new.swap(Cap(8), 512).unwrap();
    assert_eq!((r.frame, r.length), (Cap(8), 0));
    // A method the old server does not have.
    assert_eq!(
        new.reset(0).unwrap_err(),
        CallError::Transport(TransportError::UnknownMethod)
    );
    let seen = &new.core.transport.0;
    assert_eq!(seen.steps, [4]);
    assert_eq!(seen.frames, [Cap(8)]);

    // Same versions still agree on every field.
    let mut same: v2::counter::Client<ToNew, Cap> = ToNew(NewServer::default()).into();
    let r = same
        .add(
            v2::counter::Step {
                by: 2,
                label: "two",
                mode: v2::counter::Mode::Down,
            },
            2,
            None,
            true,
        )
        .unwrap();
    assert_eq!(r.total, 4);
    assert_eq!(r.history.iter().collect::<Vec<_>>(), [2, 4]);
    assert_eq!(r.note, "added by version 2");
    same.reset(1).unwrap();
    assert_eq!(same.core.transport.0.resets, [1]);
}
