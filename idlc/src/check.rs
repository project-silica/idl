//! Semantic rules of `spec/IDL.md` sections 1 and 2. Every violation is one
//! [`Diagnostic`] with file, line and column; all of them are reported.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::Diagnostic;
use super::ast::{Field, Ident, Int, Item, PName, Protocol, Span, Type, TypeKind};

/// Rust keywords (strict, reserved and weak, edition 2024). No IDL name may be one.
const RUST_KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "gen",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "abstract",
    "become",
    "box",
    "do",
    "final",
    "macro",
    "override",
    "priv",
    "typeof",
    "unsized",
    "virtual",
    "yield",
    "try",
    "union",
    "raw",
    "macro_rules",
];

/// `snake_case`: lowercase words of letters and digits joined by single underscores.
pub fn is_snake_case(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.split('_').all(|w| {
            !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// `CamelCase`: an uppercase letter, then letters and digits, no underscores.
pub fn is_camel_case(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_uppercase()) && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// `EchoError` -> `echo_error` (used for the file name of a protocol).
pub fn to_snake_case(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in cs.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev_lower =
                i > 0 && (cs[i - 1].is_ascii_lowercase() || cs[i - 1].is_ascii_digit());
            let acronym_end = i > 0
                && cs[i - 1].is_ascii_uppercase()
                && cs.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if prev_lower || acronym_end {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

struct Ctx<'a> {
    file: &'a Path,
    out: Vec<Diagnostic>,
}

impl Ctx<'_> {
    fn err(&mut self, span: Span, message: String) {
        self.out.push(Diagnostic {
            file: self.file.to_path_buf(),
            span,
            message,
        });
    }

    fn spelling(&mut self, id: &Ident, what: &str, camel: bool) {
        if RUST_KEYWORDS.contains(&id.name.as_str()) {
            self.err(
                id.span,
                format!("{what} name `{}` is a Rust keyword", id.name),
            );
        } else if camel && !is_camel_case(&id.name) {
            self.err(
                id.span,
                format!("{what} name `{}` must be CamelCase", id.name),
            );
        } else if !camel && !is_snake_case(&id.name) {
            self.err(
                id.span,
                format!("{what} name `{}` must be snake_case", id.name),
            );
        }
    }

    fn unique<'i>(&mut self, names: impl IntoIterator<Item = &'i Ident>, scope: &str) {
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for id in names {
            if let Some(first) = seen.get(id.name.as_str()) {
                let first = *first;
                self.err(
                    id.span,
                    format!(
                        "duplicate name `{}` in {scope} (first at {}:{})",
                        id.name, first.line, first.col
                    ),
                );
            } else {
                seen.insert(&id.name, id.span);
            }
        }
    }
}

/// Every rule that can be checked within one protocol file.
pub fn check_protocol(p: &Protocol) -> Vec<Diagnostic> {
    let mut cx = Ctx {
        file: &p.file,
        out: Vec::new(),
    };

    // Protocol name: segments snake_case except the last, which is CamelCase.
    let n = p.name.segments.len();
    for (i, seg) in p.name.segments.iter().enumerate() {
        cx.spelling(seg, "protocol", i + 1 == n);
    }

    // Version: 1 ..= 2^32-1 (the wire header carries it as u32).
    if p.version.value < 1 || p.version.value > u64::from(u32::MAX) {
        cx.err(
            p.version.span,
            format!(
                "version {} must be between 1 and {}",
                p.version.value,
                u32::MAX
            ),
        );
    }

    // Item names: spelling and uniqueness.
    for item in &p.items {
        match item {
            Item::Method(m) => cx.spelling(&m.name, "method", false),
            Item::Struct(s) => cx.spelling(&s.name, "struct", true),
            Item::Enum(e) => cx.spelling(&e.name, "enum", true),
            Item::Reserved(_) => {}
        }
    }
    cx.unique(
        p.items.iter().filter_map(Item::name),
        &format!("protocol `{}`", p.name),
    );

    // Ordinals.
    let in_range = |o: &Int| o.value >= 1 && o.value <= u64::from(u32::MAX);
    let mut reserved: HashMap<u64, Span> = HashMap::new();
    for r in p.reserved() {
        if !in_range(&r.ordinal) {
            cx.err(
                r.ordinal.span,
                format!(
                    "ordinal {} must be between 1 and {}",
                    r.ordinal.value,
                    u32::MAX
                ),
            );
        } else if let Some(first) = reserved.get(&r.ordinal.value) {
            let first = *first;
            cx.err(
                r.ordinal.span,
                format!(
                    "ordinal {} is already reserved (at {}:{})",
                    r.ordinal.value, first.line, first.col
                ),
            );
        } else {
            reserved.insert(r.ordinal.value, r.ordinal.span);
        }
    }
    let mut used: HashMap<u64, (&str, Span)> = HashMap::new();
    for m in p.methods() {
        let o = m.ordinal;
        if !in_range(&o) {
            cx.err(
                o.span,
                format!("ordinal {} must be between 1 and {}", o.value, u32::MAX),
            );
        } else if let Some(r) = reserved.get(&o.value) {
            let r = *r;
            cx.err(
                o.span,
                format!(
                    "ordinal {} of `{}` is reserved (at {}:{})",
                    o.value, m.name.name, r.line, r.col
                ),
            );
        } else if let Some((other, first)) = used.get(&o.value) {
            let (other, first) = (*other, *first);
            cx.err(
                o.span,
                format!(
                    "ordinal {} of `{}` is already used by `{other}` (at {}:{})",
                    o.value, m.name.name, first.line, first.col
                ),
            );
        } else {
            used.insert(o.value, (&m.name.name, o.span));
        }
    }

    // Field lists: spelling, uniqueness, types.
    let mut lists: Vec<(&[Field], String)> = Vec::new();
    for m in p.methods() {
        lists.push((&m.params, format!("parameters of `{}`", m.name.name)));
        lists.push((&m.results, format!("results of `{}`", m.name.name)));
    }
    for s in p.structs() {
        lists.push((&s.fields, format!("struct `{}`", s.name.name)));
    }
    for (fields, scope) in &lists {
        for f in *fields {
            cx.spelling(&f.name, "field", false);
            check_type(&mut cx, p, &f.ty);
        }
        cx.unique(fields.iter().map(|f| &f.name), scope);
    }

    // Enums: variant spelling, uniqueness, values fit and are unique.
    for e in p.enums() {
        let mut values: HashMap<u64, (&str, Span)> = HashMap::new();
        for v in &e.variants {
            cx.spelling(&v.name, "variant", false);
            if v.value.value > e.width.max() {
                cx.err(
                    v.value.span,
                    format!(
                        "value {} of `{}` does not fit in {}",
                        v.value.value,
                        v.name.name,
                        e.width.as_str()
                    ),
                );
            } else if let Some((other, first)) = values.get(&v.value.value) {
                let (other, first) = (*other, *first);
                cx.err(
                    v.value.span,
                    format!(
                        "value {} of `{}` is already used by `{other}` (at {}:{})",
                        v.value.value, v.name.name, first.line, first.col
                    ),
                );
            } else {
                values.insert(v.value.value, (&v.name.name, v.value.span));
            }
        }
        cx.unique(
            e.variants.iter().map(|v| &v.name),
            &format!("enum `{}`", e.name.name),
        );
    }

    // Method errors name an enum of the protocol.
    for m in p.methods() {
        if let Some(err) = &m.error {
            match p.item(&err.name) {
                Some(Item::Enum(_)) => {}
                Some(_) => cx.err(
                    err.span,
                    format!("error `{}` of `{}` is not an enum", err.name, m.name.name),
                ),
                None => cx.err(
                    err.span,
                    format!(
                        "error `{}` of `{}` is not declared in this protocol",
                        err.name, m.name.name
                    ),
                ),
            }
        }
    }

    // No struct contains itself, directly or through other structs.
    let mut contains: HashMap<&str, Vec<&str>> = HashMap::new();
    for s in p.structs() {
        let mut refs = Vec::new();
        for f in &s.fields {
            named_structs(p, &f.ty, &mut refs);
        }
        contains.insert(&s.name.name, refs);
    }
    for s in p.structs() {
        if let Some(path) = cycle_from(&s.name.name, &contains) {
            cx.err(
                s.name.span,
                format!(
                    "struct `{}` contains itself ({})",
                    s.name.name,
                    path.join(" -> ")
                ),
            );
        }
    }

    // No array element carries a capability, at any depth: capabilities
    // travel in the message's list in encoding order, and every decoding
    // reads an array's elements without them (section 3), so a protocol
    // declaring one could never send it. A rule of the language, not of
    // one backend (`spec/IDL_RATIONALE.md` section 6).
    let fields = p
        .structs()
        .flat_map(|s| s.fields.iter())
        .chain(p.methods().flat_map(|m| m.params.iter().chain(&m.results)));
    for f in fields {
        arrays_without_caps(&mut cx, p, &f.ty);
    }

    cx.out
}

/// Reports every `array<T>` within `ty` whose `T` can carry a capability.
fn arrays_without_caps(cx: &mut Ctx<'_>, p: &Protocol, ty: &Type) {
    match &ty.kind {
        TypeKind::Array(t) if carries_cap(p, t, &mut HashSet::new()) => cx.err(
            ty.span,
            "an array element cannot carry a capability (capabilities travel beside the bytes, \
             one per field); use up to four `option<cap>` fields"
                .into(),
        ),
        TypeKind::Array(t) | TypeKind::Option(t) => arrays_without_caps(cx, p, t),
        _ => {}
    }
}

/// Whether a value of type `ty` can reference a capability, at any depth.
/// `seen` holds the structs already looked into: a struct that contains
/// itself is reported on its own, and adds nothing here.
fn carries_cap<'p>(p: &'p Protocol, ty: &'p Type, seen: &mut HashSet<&'p str>) -> bool {
    match &ty.kind {
        TypeKind::Cap(_) | TypeKind::Endpoint(_) => true,
        TypeKind::Array(t) | TypeKind::Option(t) => carries_cap(p, t, seen),
        TypeKind::Named(id) => match p.item(&id.name) {
            Some(Item::Struct(s)) if seen.insert(&s.name.name) => {
                s.fields.iter().any(|f| carries_cap(p, &f.ty, seen))
            }
            _ => false,
        },
        _ => false,
    }
}

fn check_type(cx: &mut Ctx<'_>, p: &Protocol, ty: &Type) {
    match &ty.kind {
        TypeKind::Array(t) | TypeKind::Option(t) => check_type(cx, p, t),
        TypeKind::Named(id) => match p.item(&id.name) {
            Some(Item::Struct(_) | Item::Enum(_)) => {}
            Some(_) => cx.err(id.span, format!("`{}` is not a struct or enum", id.name)),
            None => cx.err(id.span, format!("unknown type `{}`", id.name)),
        },
        _ => {}
    }
}

fn named_structs<'p>(p: &'p Protocol, ty: &'p Type, out: &mut Vec<&'p str>) {
    match &ty.kind {
        TypeKind::Array(t) | TypeKind::Option(t) => named_structs(p, t, out),
        TypeKind::Named(id) if matches!(p.item(&id.name), Some(Item::Struct(_))) => {
            out.push(&id.name)
        }
        _ => {}
    }
}

/// A path `start -> ... -> start` through `contains`, if one exists.
fn cycle_from<'a>(
    start: &'a str,
    contains: &HashMap<&'a str, Vec<&'a str>>,
) -> Option<Vec<&'a str>> {
    fn dfs<'a>(
        at: &'a str,
        start: &'a str,
        contains: &HashMap<&'a str, Vec<&'a str>>,
        seen: &mut HashSet<&'a str>,
        path: &mut Vec<&'a str>,
    ) -> bool {
        for &next in contains.get(at).map(Vec::as_slice).unwrap_or(&[]) {
            path.push(next);
            if next == start {
                return true;
            }
            if seen.insert(next) && dfs(next, start, contains, seen, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut path = vec![start];
    let mut seen = HashSet::new();
    dfs(start, start, contains, &mut seen, &mut path).then_some(path)
}

/// Rules over the set of files compiled together: protocol names are unique
/// and every `endpoint<P>` resolves to one of them.
pub fn check_set(protocols: &[Protocol]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut names: HashMap<String, &Protocol> = HashMap::new();
    for p in protocols {
        let key = p.name.to_string();
        if let Some(first) = names.get(&key) {
            out.push(Diagnostic {
                file: p.file.clone(),
                span: p.name.span(),
                message: format!(
                    "protocol `{key}` is also defined in {}",
                    first.file.display()
                ),
            });
        } else {
            names.insert(key, p);
        }
    }
    for p in protocols {
        let mut fields: Vec<&Field> = Vec::new();
        for m in p.methods() {
            fields.extend(m.params.iter().chain(&m.results));
        }
        for s in p.structs() {
            fields.extend(&s.fields);
        }
        for f in fields {
            endpoints(&f.ty, &mut |pn: &PName| {
                if !names.contains_key(&pn.to_string()) {
                    out.push(Diagnostic {
                        file: p.file.clone(),
                        span: pn.span(),
                        message: format!(
                            "endpoint protocol `{pn}` is not among the files compiled together"
                        ),
                    });
                }
            });
        }
    }
    out
}

fn endpoints(ty: &Type, f: &mut impl FnMut(&PName)) {
    match &ty.kind {
        TypeKind::Array(t) | TypeKind::Option(t) => endpoints(t, f),
        TypeKind::Endpoint(pn) => f(pn),
        _ => {}
    }
}

/// `spec/IDL.md` section 1: protocol `a.b.Name` lives in `.../a/b/name.idl`.
pub fn check_location(p: &Protocol) -> Option<Diagnostic> {
    let segs = &p.name.segments;
    let mut expected: Vec<String> = segs[..segs.len() - 1]
        .iter()
        .map(|s| s.name.clone())
        .collect();
    expected.push(format!("{}.idl", to_snake_case(&segs[segs.len() - 1].name)));
    let actual: Vec<String> = p
        .file
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if actual.ends_with(&expected) {
        return None;
    }
    Some(Diagnostic {
        file: p.file.clone(),
        span: p.name.span(),
        message: format!(
            "protocol `{}` must be in a file ending in `{}`",
            p.name,
            expected.join("/")
        ),
    })
}
