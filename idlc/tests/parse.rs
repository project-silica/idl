//! A06.01: the parser accepts the sample protocol and rejects every violation
//! of the rules of `spec/IDL.md` sections 1 and 2 with file, line and column.

use std::path::{Path, PathBuf};

use idlc::ast::{Item, ObjType, TypeKind, Width};

fn sample_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(idlc::PROTOCOL_DIR)
        .join("sample")
        .join("echo.idl")
}

#[test]
fn sample_echo_parses() {
    let path = sample_path();
    let ps = idlc::load(std::slice::from_ref(&path)).unwrap_or_else(|e| {
        panic!(
            "{}",
            e.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    assert_eq!(ps.len(), 1);
    let p = &ps[0];
    assert_eq!(p.name.to_string(), "sample.Echo");
    assert_eq!(p.version.value, 1);
    assert!(p.docs[0].starts_with("A request/reply protocol"));

    let kinds: Vec<&str> = p
        .items
        .iter()
        .map(|i| match i {
            Item::Method(_) => "method",
            Item::Reserved(_) => "reserved",
            Item::Struct(_) => "struct",
            Item::Enum(_) => "enum",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "enum", "enum", "struct", "method", "method", "method", "reserved"
        ]
    );

    let e = p.enums().next().unwrap();
    assert_eq!(e.name.name, "EchoError");
    assert_eq!(e.width, Width::U32);
    assert_eq!(e.variants.len(), 2);
    assert_eq!(
        (e.variants[1].name.name.as_str(), e.variants[1].value.value),
        ("too_many", 2)
    );
    assert_eq!(e.variants[0].docs, ["No record was given."]);

    let rec = p.structs().next().unwrap();
    assert_eq!(rec.fields.len(), 14);
    assert!(matches!(&rec.fields[0].ty.kind, TypeKind::Named(id) if id.name == "Kind"));
    assert!(matches!(&rec.fields[12].ty.kind, TypeKind::Array(t) if t.kind == TypeKind::String));
    assert!(matches!(&rec.fields[13].ty.kind, TypeKind::Option(t) if t.kind == TypeKind::U64));

    let ms: Vec<_> = p.methods().collect();
    assert_eq!(
        ms.iter()
            .map(|m| (m.name.name.as_str(), m.ordinal.value))
            .collect::<Vec<_>>(),
        [("echo", 1), ("echo_records", 2), ("echo_caps", 3)]
    );
    assert_eq!(ms[1].error.as_ref().unwrap().name, "EchoError");
    assert!(ms[0].error.is_none());
    let caps = &ms[2].params;
    assert_eq!(caps[0].ty.kind, TypeKind::Cap(Some(ObjType::Frame)));
    assert!(
        matches!(&caps[1].ty.kind, TypeKind::Option(t) if matches!(&t.kind, TypeKind::Endpoint(pn) if pn.to_string() == "sample.Echo"))
    );
    assert_eq!(caps[2].ty.kind, TypeKind::Cap(None));
    assert_eq!(ms[2].results.len(), 3);
    assert_eq!(p.reserved().next().unwrap().ordinal.value, 4);

    // Positions are real: `echo` is on the line that declares it.
    let src = std::fs::read_to_string(&path).unwrap();
    let line = src
        .lines()
        .position(|l| l.trim_start().starts_with("echo @1"))
        .unwrap()
        + 1;
    assert_eq!(ms[0].name.span.line as usize, line);
    assert_eq!(ms[0].name.span.col, 5);
}

/// Compile `src` as `t/x.idl`; expect exactly the errors (line, col, message fragment).
fn rejects(src: &str, want: &[(u32, u32, &str)]) {
    let file = PathBuf::from("t/x.idl");
    let errs = match idlc::compile(&[(file.clone(), src.to_string())]) {
        Ok(_) => panic!("accepted:\n{src}"),
        Err(e) => e,
    };
    let got: Vec<String> = errs.iter().map(ToString::to_string).collect();
    assert_eq!(errs.len(), want.len(), "errors: {got:#?}");
    for (e, (line, col, frag)) in errs.iter().zip(want) {
        assert_eq!(e.file, file);
        assert_eq!((e.span.line, e.span.col), (*line, *col), "{e}");
        assert!(e.message.contains(frag), "`{e}` lacks `{frag}`");
    }
}

fn accepts(src: &str) -> idlc::Protocol {
    match idlc::compile(&[(PathBuf::from("t/x.idl"), src.to_string())]) {
        Ok(mut ps) => ps.remove(0),
        Err(e) => panic!(
            "{}",
            e.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

#[test]
fn minimal_and_trailing_commas() {
    let p = accepts(
        "protocol t.X version 0x2 { m @1 () -> (); enum E : u8 { a = 0, } struct S { a: u8, } }",
    );
    assert_eq!(p.version.value, 2);
    let p = accepts("protocol t.X version 1 {}");
    assert!(p.items.is_empty());
    // `////` is a plain comment; `///` text is kept.
    let p = accepts("//// not doc\n/// Doc.\nprotocol t.X version 1 {}");
    assert_eq!(p.docs, ["Doc."]);
    // `reserved` is still a usable method name.
    accepts("protocol t.X version 1 { reserved @1 () -> (); }");
}

#[test]
fn syntax_errors_have_positions() {
    rejects(
        "protocol t.X version 1 {\n  m @1 (a: u8) -> (a: u8)\n}",
        &[(3, 1, "expected `;`")],
    );
    rejects(
        "protocol t.X version 1 {\n  m @1 (a: u8 b: u8) -> ();\n}",
        &[(2, 15, "expected `)`")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (,) -> (); }",
        &[(1, 32, "expected an identifier")],
    );
    rejects(
        "protocol t.X version 1 { enum E : u64 { a = 0 } }",
        &[(1, 35, "`u8`, `u16` or `u32`")],
    );
    rejects(
        "protocol t.X version 1 { enum E : u8 { } }",
        &[(1, 40, "expected an identifier")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (a: cap<Page>) -> (); }",
        &[(1, 39, "object type")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (a: array<u8) -> (); }",
        &[(1, 43, "expected `>`")],
    );
    rejects(
        "protocol t.X version 1 { } protocol t.Y version 1 { }",
        &[(1, 28, "one protocol per file")],
    );
    rejects(
        "protocol t.X version 1 { m @1 () -> () $ }",
        &[(1, 40, "unexpected character")],
    );
    rejects(
        "protocol t.X version 0xg { }",
        &[(1, 22, "invalid hexadecimal")],
    );
    rejects(
        "protocol t.X version 99999999999999999999 { }",
        &[(1, 22, "64 bits")],
    );
    rejects(
        "protocol t.X version 1 { /// dangling\n }",
        &[(2, 2, "expected a method")],
    );
}

#[test]
fn version_and_ordinals() {
    rejects("protocol t.X version 0 {}", &[(1, 22, "version 0")]);
    rejects(
        "protocol t.X version 4294967296 {}",
        &[(1, 22, "between 1 and")],
    );
    rejects(
        "protocol t.X version 1 { m @0 () -> (); }",
        &[(1, 29, "between 1 and")],
    );
    rejects(
        "protocol t.X version 1 { m @0x100000000 () -> (); }",
        &[(1, 29, "between 1 and")],
    );
    accepts("protocol t.X version 1 { m @0xffffffff () -> (); }");
    rejects(
        "protocol t.X version 1 {\n a @1 () -> ();\n b @1 () -> ();\n}",
        &[(3, 5, "already used by `a`")],
    );
    rejects(
        "protocol t.X version 1 {\n reserved @2;\n b @2 () -> ();\n}",
        &[(3, 5, "is reserved")],
    );
    rejects(
        "protocol t.X version 1 {\n reserved @2;\n reserved @2;\n}",
        &[(3, 12, "already reserved")],
    );
}

#[test]
fn names_unique_per_scope() {
    rejects(
        "protocol t.X version 1 {\n struct A {}\n enum A : u8 { a = 0 }\n}",
        &[(3, 7, "duplicate name `A`")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (a: u8, a: u8) -> (); }",
        &[(1, 39, "parameters of `m`")],
    );
    rejects(
        "protocol t.X version 1 { m @1 () -> (a: u8, a: u8); }",
        &[(1, 45, "results of `m`")],
    );
    rejects(
        "protocol t.X version 1 { struct S { a: u8, a: u8 } }",
        &[(1, 44, "struct `S`")],
    );
    rejects(
        "protocol t.X version 1 { enum E : u8 { a = 0, a = 1 } }",
        &[(1, 47, "enum `E`")],
    );
    // The same name in different scopes is fine.
    accepts("protocol t.X version 1 { m @1 (a: u8) -> (a: u8); struct S { a: u8 } }");
}

#[test]
fn enum_values_and_errors() {
    rejects(
        "protocol t.X version 1 { enum E : u8 { a = 256 } }",
        &[(1, 44, "does not fit in u8")],
    );
    rejects(
        "protocol t.X version 1 { enum E : u16 { a = 0x10000 } }",
        &[(1, 45, "does not fit in u16")],
    );
    accepts("protocol t.X version 1 { enum E : u32 { a = 0xffffffff } }");
    rejects(
        "protocol t.X version 1 { enum E : u8 { a = 1, b = 1 } }",
        &[(1, 51, "already used by `a`")],
    );
    rejects(
        "protocol t.X version 1 { m @1 () -> () error E; }",
        &[(1, 46, "not declared")],
    );
    rejects(
        "protocol t.X version 1 { struct S {} m @1 () -> () error S; }",
        &[(1, 58, "not an enum")],
    );
}

#[test]
fn types_resolve_and_do_not_recurse() {
    rejects(
        "protocol t.X version 1 { m @1 (a: Nope) -> (); }",
        &[(1, 35, "unknown type `Nope`")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (a: option<m>) -> (); }",
        &[(1, 42, "not a struct or enum")],
    );
    rejects(
        "protocol t.X version 1 { struct S { s: S } }",
        &[(1, 33, "S -> S")],
    );
    rejects(
        "protocol t.X version 1 { struct S { s: array<S> } }",
        &[(1, 33, "contains itself")],
    );
    rejects(
        "protocol t.X version 1 {\n struct A { b: option<B> }\n struct B { a: A }\n}",
        &[(2, 9, "A -> B -> A"), (3, 9, "B -> A -> B")],
    );
    accepts("protocol t.X version 1 { struct A { b: B, c: B } struct B { x: u8 } }");
}

#[test]
fn spellings_and_keywords() {
    rejects(
        "protocol t.X version 1 { Get @1 () -> (); }",
        &[(1, 26, "method name `Get` must be snake_case")],
    );
    rejects(
        "protocol t.X version 1 { struct s_t {} }",
        &[(1, 33, "must be CamelCase")],
    );
    rejects(
        "protocol t.X version 1 { enum E_x : u8 { a = 0 } }",
        &[(1, 31, "must be CamelCase")],
    );
    rejects(
        "protocol t.X version 1 { enum E : u8 { A = 0 } }",
        &[(1, 40, "variant name `A`")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (aB: u8) -> (); }",
        &[(1, 32, "field name `aB`")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (a__b: u8) -> (); }",
        &[(1, 32, "snake_case")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (_a: u8) -> (); }",
        &[(1, 32, "snake_case")],
    );
    rejects(
        "protocol T.X version 1 {}",
        &[(1, 10, "must be snake_case")],
    );
    rejects("protocol t.x version 1 {}", &[(1, 12, "must be CamelCase")]);
    rejects(
        "protocol t.X version 1 { fn @1 () -> (); }",
        &[(1, 26, "Rust keyword")],
    );
    rejects(
        "protocol t.X version 1 { m @1 (type: u8) -> (); }",
        &[(1, 32, "Rust keyword")],
    );
    rejects(
        "protocol t.X version 1 { struct Self {} }",
        &[(1, 33, "Rust keyword")],
    );
    rejects(
        "protocol t.X version 1 { enum E : u8 { match = 0 } }",
        &[(1, 40, "Rust keyword")],
    );
}

#[test]
fn endpoints_resolve_within_the_set() {
    rejects(
        "protocol t.X version 1 { m @1 (e: endpoint<t.Other>) -> (); }",
        &[(1, 44, "t.Other")],
    );
    let a = "protocol t.A version 1 { m @1 (b: endpoint<t.B>) -> (); }".to_string();
    let b = "protocol t.B version 1 { m @1 () -> (a: option<endpoint<t.A>>); }".to_string();
    idlc::compile(&[(PathBuf::from("t/a.idl"), a), (PathBuf::from("t/b.idl"), b)]).unwrap();
    let x = "protocol t.A version 1 {}".to_string();
    let errs = idlc::compile(&[
        (PathBuf::from("t/a.idl"), x.clone()),
        (PathBuf::from("t/a2.idl"), x),
    ])
    .unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].file, PathBuf::from("t/a2.idl"));
    assert!(errs[0].message.contains("also defined"));
}

#[test]
fn errors_from_several_files_are_all_reported() {
    let errs = idlc::compile(&[
        (PathBuf::from("t/a.idl"), "protocol t.A version 0 {}".into()),
        (
            PathBuf::from("t/b.idl"),
            "protocol t.B version 1 { m @0 () -> (); }".into(),
        ),
    ])
    .unwrap_err();
    let got: Vec<String> = errs.iter().map(ToString::to_string).collect();
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[0].starts_with("t/a.idl:1:22: "), "{got:?}");
    assert!(got[1].starts_with("t/b.idl:1:29: "), "{got:?}");
}

#[test]
fn file_location_matches_protocol_name() {
    let dir = std::env::temp_dir().join(format!("idlc-loc-{}", std::process::id()));
    let good = dir.join("a").join("b").join("io_port_map.idl");
    let bad = dir.join("a").join("wrong.idl");
    std::fs::create_dir_all(good.parent().unwrap()).unwrap();
    std::fs::write(&good, "protocol a.b.IoPortMap version 1 {}").unwrap();
    std::fs::write(&bad, "protocol a.b.IoPortMap2 version 1 {}").unwrap();
    assert!(idlc::load(std::slice::from_ref(&good)).is_ok());
    let errs = idlc::load(std::slice::from_ref(&bad)).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].message.contains("a/b/io_port_map2.idl"),
        "{}",
        errs[0]
    );
    assert_eq!((errs[0].span.line, errs[0].span.col), (1, 10));
    let _ = std::fs::remove_dir_all(&dir);
}
