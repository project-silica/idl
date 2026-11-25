//! Rust generation (`spec/IDL.md` section 6).
//!
//! For one protocol, one Rust module: the structs and enums; a request and a
//! reply type per method, each with encode and decode; a client generic over
//! `silica_idl_rt::Transport`; a server trait; and a `dispatch` function.
//! The generated code depends only on `silica_idl_rt` and `core`, and the
//! output is a deterministic function of the protocol.
//!
//! Capabilities move (`spec/IDL_RATIONALE.md` section 5):
//! encoding takes values by value, decoding takes the capability list by
//! value, and an `array` element type may not carry a capability
//! (`spec/IDL_RATIONALE.md` section 6).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::Diagnostic;
use super::ast::{Field, Item, Protocol, Span, Type, TypeKind};
use super::check::to_snake_case;

const RT: &str = "::silica_idl_rt";

/// Type names every generated module defines itself or uses as generic
/// parameters; no struct or enum of a protocol may take one.
pub const RESERVED_TYPE_NAMES: &[&str] = &["C", "Client", "S", "Server", "T"];

/// Name of the generated module: the last protocol name segment in
/// `snake_case` (`sample.Echo` -> `echo`).
pub fn module_name(p: &Protocol) -> String {
    let last = p.name.segments.last().map_or("", |s| s.name.as_str());
    to_snake_case(last)
}

/// File name for the generated module: every name segment in `snake_case`,
/// joined by `_` (`sample.Echo` -> `sample_echo.rs`).
pub fn file_name(p: &Protocol) -> String {
    let segs: Vec<String> = p
        .name
        .segments
        .iter()
        .map(|s| to_snake_case(&s.name))
        .collect();
    format!("{}.rs", segs.join("_"))
}

/// `too_many` -> `TooMany`.
pub fn camel(snake: &str) -> String {
    snake
        .split('_')
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Whether a type needs the lifetime `'a` and the capability parameter `C`.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Needs {
    lt: bool,
    cap: bool,
}

impl Needs {
    fn or(self, o: Needs) -> Needs {
        Needs {
            lt: self.lt || o.lt,
            cap: self.cap || o.cap,
        }
    }

    /// `<'a, C: Capability>` and the like, for a definition or impl.
    fn decl(self) -> String {
        match (self.lt, self.cap) {
            (false, false) => String::new(),
            (true, false) => "<'a>".into(),
            (false, true) => format!("<C: {RT}::Capability>"),
            (true, true) => format!("<'a, C: {RT}::Capability>"),
        }
    }

    /// `<'a, C>` and the like, for a use, with lifetime `lt`.
    fn args(self, lt: &str) -> String {
        match (self.lt, self.cap) {
            (false, false) => String::new(),
            (true, false) => format!("<{lt}>"),
            (false, true) => "<C>".into(),
            (true, true) => format!("<{lt}, C>"),
        }
    }
}

struct Gen<'p> {
    p: &'p Protocol,
    /// Struct name -> needs; enums are absent (they need nothing).
    structs: BTreeMap<String, Needs>,
    out: String,
}

/// Generate the Rust module for `p`, which must have passed
/// [`crate::compile`]. Fails on names the generated code reserves and on an
/// `array` whose element type can carry a capability.
pub fn rust(p: &Protocol) -> Result<String, Vec<Diagnostic>> {
    let mut errs = check_names(p);
    errs.extend(check_arrays(p));
    errs.sort_by_key(|d| (d.span.line, d.span.col));
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut g = Gen {
        p,
        structs: BTreeMap::new(),
        out: String::new(),
    };
    for s in p.structs() {
        let n = g.needs_struct(&s.name.name);
        g.structs.insert(s.name.name.clone(), n);
    }
    g.module();
    Ok(g.out)
}

fn check_names(p: &Protocol) -> Vec<Diagnostic> {
    let mut errs = Vec::new();
    let mut err = |span: Span, message: String| {
        errs.push(Diagnostic {
            file: p.file.clone(),
            span,
            message,
        })
    };
    // Every type name the module defines, with where it comes from.
    let mut types: BTreeMap<String, Span> = BTreeMap::new();
    for item in &p.items {
        if let Item::Struct(_) | Item::Enum(_) = item
            && let Some(n) = item.name()
        {
            if RESERVED_TYPE_NAMES.contains(&n.name.as_str()) {
                err(
                    n.span,
                    format!("`{}` is reserved by the generated Rust module", n.name),
                );
            }
            types.insert(n.name.clone(), n.span);
        }
    }
    for m in p.methods() {
        for suffix in ["Request", "Reply"] {
            let t = format!("{}{suffix}", camel(&m.name.name));
            if let Some(prev) = types.insert(t.clone(), m.name.span) {
                err(
                    m.name.span,
                    format!(
                        "generated type `{t}` of method `{}` collides with the name at {}:{}",
                        m.name.name, prev.line, prev.col
                    ),
                );
            }
        }
    }
    for e in p.enums() {
        let mut seen: BTreeMap<String, Span> = BTreeMap::new();
        seen.insert("Unknown".into(), e.name.span);
        for v in &e.variants {
            let r = camel(&v.name.name);
            if seen.insert(r.clone(), v.name.span).is_some() {
                err(
                    v.name.span,
                    format!(
                        "variant `{}` becomes Rust variant `{r}`, which is already taken",
                        v.name.name
                    ),
                );
            }
        }
    }
    errs
}

/// Whether a value of `ty` can carry a capability. `p` has passed
/// [`crate::compile`], so structs do not contain themselves.
fn carries_cap(p: &Protocol, ty: &Type) -> bool {
    match &ty.kind {
        TypeKind::Cap(_) | TypeKind::Endpoint(_) => true,
        TypeKind::Option(t) | TypeKind::Array(t) => carries_cap(p, t),
        TypeKind::Named(n) => match p.item(&n.name) {
            Some(Item::Struct(s)) => s.fields.iter().any(|f| carries_cap(p, &f.ty)),
            _ => false,
        },
        _ => false,
    }
}

/// Rejects every `array<T>` whose `T` can carry a capability: capabilities
/// move, and an array is a borrowed slice when sent and a view of the bytes
/// when received, so neither can own one
/// (`spec/IDL_RATIONALE.md` section 6).
fn check_arrays(p: &Protocol) -> Vec<Diagnostic> {
    fn walk(p: &Protocol, ty: &Type, errs: &mut Vec<Diagnostic>) {
        match &ty.kind {
            TypeKind::Array(t) => {
                if carries_cap(p, t) {
                    errs.push(Diagnostic {
                        file: p.file.clone(),
                        span: ty.span,
                        message: "an array element cannot carry a capability in generated Rust; \
                                  use up to four `option<cap>` fields"
                            .into(),
                    });
                } else {
                    walk(p, t, errs);
                }
            }
            TypeKind::Option(t) => walk(p, t, errs),
            _ => {}
        }
    }
    let mut errs = Vec::new();
    for item in &p.items {
        match item {
            Item::Struct(s) => s.fields.iter().for_each(|f| walk(p, &f.ty, &mut errs)),
            Item::Method(m) => m
                .params
                .iter()
                .chain(&m.results)
                .for_each(|f| walk(p, &f.ty, &mut errs)),
            _ => {}
        }
    }
    errs
}

/// The expected object type name of a capability type, as a Rust
/// `Option<&str>` expression: `cap` is unchecked, `cap<T>` is `T`, and
/// `endpoint<P>` is `Endpoint` (the protocol is a contract).
fn object_type(ty: &TypeKind) -> String {
    match ty {
        TypeKind::Cap(Some(o)) => format!("::core::option::Option::Some(\"{}\")", o.as_str()),
        TypeKind::Endpoint(_) => "::core::option::Option::Some(\"Endpoint\")".into(),
        _ => "::core::option::Option::None".into(),
    }
}

/// For a field of a capability type or an `option` of one, the expression
/// decoding it with the object type check (`d` is the decoder); else
/// `None`, and the field decodes generically.
fn cap_decode(ty: &Type) -> Option<String> {
    match &ty.kind {
        TypeKind::Cap(_) | TypeKind::Endpoint(_) => {
            Some(format!("d.cap({})", object_type(&ty.kind)))
        }
        TypeKind::Option(t) => cap_decode(t).map(|inner| format!("d.option(|d| {inner})")),
        _ => None,
    }
}

/// Doc comment lines at `indent`.
fn docs(out: &mut String, indent: &str, lines: &[String]) {
    for l in lines {
        if l.is_empty() {
            let _ = writeln!(out, "{indent}///");
        } else {
            let _ = writeln!(out, "{indent}/// {l}");
        }
    }
}

impl Gen<'_> {
    fn needs_struct(&self, name: &str) -> Needs {
        match self.p.item(name) {
            Some(Item::Struct(s)) => self.needs_fields(&s.fields),
            _ => Needs::default(),
        }
    }

    fn needs_fields(&self, fields: &[Field]) -> Needs {
        fields
            .iter()
            .fold(Needs::default(), |n, f| n.or(self.needs(&f.ty)))
    }

    fn needs(&self, ty: &Type) -> Needs {
        match &ty.kind {
            TypeKind::String | TypeKind::Bytes => Needs {
                lt: true,
                cap: false,
            },
            TypeKind::Array(_) => Needs {
                lt: true,
                cap: true,
            },
            TypeKind::Option(t) => self.needs(t),
            TypeKind::Cap(_) | TypeKind::Endpoint(_) => Needs {
                lt: false,
                cap: true,
            },
            TypeKind::Named(n) => match self.structs.get(&n.name) {
                Some(&k) => k,
                None => self.needs_struct(&n.name),
            },
            _ => Needs::default(),
        }
    }

    /// The Rust type of `ty`, with lifetime `lt`.
    fn ty(&self, ty: &Type, lt: &str) -> String {
        match &ty.kind {
            TypeKind::U8 => "u8".into(),
            TypeKind::U16 => "u16".into(),
            TypeKind::U32 => "u32".into(),
            TypeKind::U64 => "u64".into(),
            TypeKind::I8 => "i8".into(),
            TypeKind::I16 => "i16".into(),
            TypeKind::I32 => "i32".into(),
            TypeKind::I64 => "i64".into(),
            TypeKind::Bool => "bool".into(),
            TypeKind::String => format!("&{lt} str"),
            TypeKind::Bytes => format!("&{lt} [u8]"),
            TypeKind::Array(t) => format!("{RT}::Array<{lt}, {}, C>", self.ty(t, lt)),
            TypeKind::Option(t) => format!("::core::option::Option<{}>", self.ty(t, lt)),
            TypeKind::Cap(_) | TypeKind::Endpoint(_) => "C".into(),
            TypeKind::Named(n) => match self.p.item(&n.name) {
                Some(Item::Struct(_)) => format!("{}{}", n.name, self.needs(ty).args(lt)),
                _ => n.name.clone(),
            },
        }
    }

    fn module(&mut self) {
        let p = self.p;
        let mut o = String::new();
        let _ = writeln!(
            o,
            "// Generated by silica-idlc from protocol `{}` version {}. Do not edit.",
            p.name, p.version.value
        );
        docs(&mut o, "", &p.docs);
        let _ = writeln!(
            o,
            "#[allow(dead_code, unreachable_code, clippy::clone_on_copy, clippy::needless_lifetimes, clippy::too_many_arguments)]"
        );
        let _ = writeln!(o, "pub mod {} {{", module_name(p));
        let _ = writeln!(o, "    /// Full protocol name.");
        let _ = writeln!(o, "    pub const NAME: &str = \"{}\";", p.name);
        let _ = writeln!(
            o,
            "    /// Protocol version this module was generated from; sent in every header."
        );
        let _ = writeln!(o, "    pub const VERSION: u32 = {};", p.version.value);
        self.out = o;
        for item in &p.items {
            match item {
                Item::Struct(s) => {
                    self.record(&s.docs, &s.name.name, &s.fields);
                }
                Item::Enum(_) => self.enumeration(item),
                Item::Method(_) | Item::Reserved(_) => {}
            }
        }
        for m in p.methods() {
            let ord = m.ordinal.value;
            let req = format!("{}Request", camel(&m.name.name));
            let rep = format!("{}Reply", camel(&m.name.name));
            let err = self.error_type(m.error.as_ref().map(|e| e.name.as_str()));
            let mut d = vec![format!(
                "Parameters of `{}` (ordinal {ord}); see [`Server::{}`].",
                m.name.name, m.name.name
            )];
            self.record(&d, &req, &m.params);
            self.message_impl(&req, &m.params, ord, false, &err);
            d[0] = format!(
                "Results of `{}` (ordinal {ord}); see [`Server::{}`].",
                m.name.name, m.name.name
            );
            self.record(&d, &rep, &m.results);
            self.message_impl(&rep, &m.results, ord, true, &err);
        }
        self.client();
        self.server();
        self.dispatch();
        self.out.push_str("}\n");
    }

    fn error_type(&self, e: Option<&str>) -> String {
        e.map_or_else(|| "::core::convert::Infallible".into(), str::to_string)
    }

    /// A struct, request or reply: a Rust struct encoded as a field list.
    fn record(&mut self, d: &[String], name: &str, fields: &[Field]) {
        let n = self.needs_fields(fields);
        let mut o = String::new();
        o.push('\n');
        docs(&mut o, "    ", d);
        // A struct that has the parameter `C` only through arrays carries no
        // capability, so it is Clone, PartialEq and Debug whatever `C` is: a
        // derive would demand them of `C`.
        let manual = n.cap && !fields.iter().any(|f| carries_cap(self.p, &f.ty));
        if !manual {
            let _ = writeln!(o, "    #[derive(Clone, Debug, PartialEq)]");
        }
        let _ = writeln!(o, "    pub struct {name}{} {{", n.decl());
        for f in fields {
            docs(&mut o, "        ", &f.docs);
            let _ = writeln!(o, "        pub {}: {},", f.name.name, self.ty(&f.ty, "'a"));
        }
        let _ = writeln!(o, "    }}");
        let ty = format!("{name}{}", n.args("'a"));
        if manual {
            let decl = n.decl();
            let _ = writeln!(o);
            let _ = writeln!(o, "    impl{decl} ::core::clone::Clone for {ty} {{");
            let _ = writeln!(o, "        fn clone(&self) -> Self {{");
            let _ = writeln!(o, "            Self {{");
            for f in fields {
                let _ = writeln!(
                    o,
                    "                {0}: ::core::clone::Clone::clone(&self.{0}),",
                    f.name.name
                );
            }
            let _ = writeln!(o, "            }}");
            let _ = writeln!(o, "        }}");
            let _ = writeln!(o, "    }}");
            let _ = writeln!(o);
            let _ = writeln!(o, "    impl{decl} ::core::cmp::PartialEq for {ty} {{");
            let _ = writeln!(o, "        fn eq(&self, other: &Self) -> bool {{");
            let mut conds: Vec<String> = fields
                .iter()
                .map(|f| format!("self.{0} == other.{0}", f.name.name))
                .collect();
            if conds.is_empty() {
                conds.push("true".into());
            }
            let _ = writeln!(o, "            {}", conds.join(" && "));
            let _ = writeln!(o, "        }}");
            let _ = writeln!(o, "    }}");
            let _ = writeln!(o);
            let _ = writeln!(o, "    impl{decl} ::core::fmt::Debug for {ty} {{");
            let _ = writeln!(
                o,
                "        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {{"
            );
            let _ = write!(o, "            f.debug_struct(\"{name}\")");
            for fl in fields {
                let _ = write!(o, ".field(\"{0}\", &self.{0})", fl.name.name);
            }
            let _ = writeln!(o, ".finish()");
            let _ = writeln!(o, "        }}");
            let _ = writeln!(o, "    }}");
        }
        let enc_decl = if n.lt {
            format!("<'a, C: {RT}::Capability>")
        } else {
            format!("<C: {RT}::Capability>")
        };
        let _ = writeln!(o);
        let _ = writeln!(o, "    impl{enc_decl} {RT}::Encode<C> for {ty} {{");
        let _ = writeln!(
            o,
            "        fn encode(&self, e: &mut {RT}::Encoder<'_, C>) -> ::core::result::Result<(), {RT}::EncodeError> {{"
        );
        let _ = writeln!(o, "            let at = e.begin_list()?;");
        // Fields at the end that hold their defaults are left out: a
        // decoder gives them back from their absence, and an older
        // provider sees exactly the request it knows (`spec/IDL.md` 3, 4).
        let keep = if fields.is_empty() { "let keep" } else { "let mut keep" };
        let _ = writeln!(o, "            {keep} = e.len();");
        for f in fields {
            let _ = writeln!(
                o,
                "            {RT}::Encode::<C>::encode(&self.{}, e)?;",
                f.name.name
            );
            let _ = writeln!(
                o,
                "            if !{RT}::Encode::<C>::is_default(&self.{}) {{ keep = e.len(); }}",
                f.name.name
            );
        }
        let _ = writeln!(o, "            e.truncate(keep);");
        let _ = writeln!(o, "            e.end_list(at)");
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o);
        let _ = writeln!(o, "        fn is_default(&self) -> bool {{");
        let all: Vec<String> = fields
            .iter()
            .map(|f| format!("{RT}::Encode::<C>::is_default(&self.{})", f.name.name))
            .collect();
        let all = if all.is_empty() { "true".to_string() } else { all.join(" && ") };
        let _ = writeln!(o, "            {all}");
        let _ = writeln!(o, "        }}");
        let moving: Vec<&Field> = fields
            .iter()
            .filter(|f| carries_cap(self.p, &f.ty))
            .collect();
        if !moving.is_empty() {
            let _ = writeln!(o);
            let _ = writeln!(
                o,
                "        fn move_caps(self, caps: &mut {RT}::CapList<C>) {{"
            );
            for f in moving {
                let _ = writeln!(
                    o,
                    "            {RT}::Encode::<C>::move_caps(self.{}, caps);",
                    f.name.name
                );
            }
            let _ = writeln!(o, "        }}");
        }
        let _ = writeln!(o, "    }}");
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "    impl<'a, C: {RT}::Capability> {RT}::Decode<'a, C> for {ty} {{"
        );
        let _ = writeln!(
            o,
            "        fn decode(d: &mut {RT}::Decoder<'a, '_, C>) -> ::core::result::Result<Self, {RT}::DecodeError> {{"
        );
        let _ = writeln!(o, "            let saved = d.begin_list()?;");
        let _ = writeln!(o, "            let v = Self {{");
        for f in fields {
            match cap_decode(&f.ty) {
                Some(dec) => {
                    let missing = if matches!(f.ty.kind, TypeKind::Option(_)) {
                        "::core::option::Option::Some(::core::option::Option::None)"
                    } else {
                        "::core::option::Option::None"
                    };
                    let _ = writeln!(
                        o,
                        "                {}: d.field_with(|d| {dec}, {missing})?,",
                        f.name.name
                    );
                }
                None => {
                    let _ = writeln!(o, "                {}: d.field()?,", f.name.name);
                }
            }
        }
        let _ = writeln!(o, "            }};");
        let _ = writeln!(o, "            d.end_list(saved)?;");
        let _ = writeln!(o, "            ::core::result::Result::Ok(v)");
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o);
        let _ = writeln!(o, "        fn missing() -> ::core::option::Option<Self> {{");
        let _ = writeln!(o, "            ::core::option::Option::Some(Self {{");
        for f in fields {
            let _ = writeln!(
                o,
                "                {}: {RT}::missing::<'a, C, _>()?,",
                f.name.name
            );
        }
        let _ = writeln!(o, "            }})");
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o, "    }}");
        self.out.push_str(&o);
    }

    /// `ORDINAL`, `encode_message` and `decode_message` of a request or reply.
    fn message_impl(&mut self, name: &str, fields: &[Field], ord: u64, reply: bool, err: &str) {
        let n = self.needs_fields(fields);
        let mg = if n.cap {
            String::new()
        } else {
            format!("<C: {RT}::Capability>")
        };
        let lt = if n.lt { "'a" } else { "'_" };
        let mut o = String::new();
        let _ = writeln!(o);
        let _ = writeln!(o, "    impl{} {name}{} {{", n.decl(), n.args("'a"));
        let _ = writeln!(o, "        /// Method ordinal.");
        let _ = writeln!(o, "        pub const ORDINAL: u32 = {ord};");
        let _ = writeln!(o);
        let what = if reply { "an `ok` reply" } else { "a request" };
        let _ = writeln!(
            o,
            "        /// Encodes `self` as {what}: a complete message in `buf`; its capabilities move into `caps`. Returns the length."
        );
        let _ = writeln!(
            o,
            "        pub fn encode_message{mg}(self, buf: &mut [u8], caps: &mut {RT}::CapList<C>) -> ::core::result::Result<usize, {RT}::EncodeError> {{"
        );
        let f = if reply {
            "encode_reply"
        } else {
            "encode_request"
        };
        let _ = writeln!(o, "            {RT}::{f}(buf, caps, {ord}, VERSION, self)");
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o);
        if reply {
            let _ = writeln!(
                o,
                "        /// Decodes a reply message as the caller observes it: the results, the method's error, or a transport error. Capabilities move out of `caps`."
            );
            let _ = writeln!(
                o,
                "        pub fn decode_message{mg}(msg: &{lt} [u8], caps: {RT}::CapList<C>) -> ::core::result::Result<Self, {RT}::CallError<{err}>> {{"
            );
            let _ = writeln!(o, "            {RT}::decode_reply(msg, caps, {ord})");
        } else {
            let _ = writeln!(
                o,
                "        /// Decodes a request message. Capabilities move out of `caps`."
            );
            let _ = writeln!(
                o,
                "        pub fn decode_message{mg}(msg: &{lt} [u8], caps: {RT}::CapList<C>) -> ::core::result::Result<Self, {RT}::DecodeError> {{"
            );
            let _ = writeln!(o, "            {RT}::decode_request(msg, caps, {ord})");
        }
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o, "    }}");
        self.out.push_str(&o);
    }

    fn enumeration(&mut self, item: &Item) {
        let Item::Enum(e) = item else { return };
        let w = e.width.as_str();
        let name = &e.name.name;
        let mut o = String::new();
        let _ = writeln!(o);
        docs(&mut o, "    ", &e.docs);
        let _ = writeln!(o, "    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]");
        let _ = writeln!(o, "    pub enum {name} {{");
        for v in &e.variants {
            docs(&mut o, "        ", &v.docs);
            let _ = writeln!(o, "        {},", camel(&v.name.name));
        }
        let _ = writeln!(
            o,
            "        /// A value this version does not know (a newer peer); not malformed."
        );
        let _ = writeln!(o, "        Unknown({w}),");
        let _ = writeln!(o, "    }}");
        let _ = writeln!(o);
        let _ = writeln!(o, "    impl {name} {{");
        let _ = writeln!(o, "        /// The variant with wire value `raw`.");
        let _ = writeln!(o, "        pub const fn from_raw(raw: {w}) -> Self {{");
        let _ = writeln!(o, "            match raw {{");
        for v in &e.variants {
            let _ = writeln!(
                o,
                "                {} => Self::{},",
                v.value.value,
                camel(&v.name.name)
            );
        }
        let _ = writeln!(o, "                other => Self::Unknown(other),");
        let _ = writeln!(o, "            }}");
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o);
        let _ = writeln!(o, "        /// The wire value.");
        let _ = writeln!(o, "        pub const fn raw(self) -> {w} {{");
        let _ = writeln!(o, "            match self {{");
        for v in &e.variants {
            let _ = writeln!(
                o,
                "                Self::{} => {},",
                camel(&v.name.name),
                v.value.value
            );
        }
        let _ = writeln!(o, "                Self::Unknown(raw) => raw,");
        let _ = writeln!(o, "            }}");
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o, "    }}");
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "    impl<C: {RT}::Capability> {RT}::Encode<C> for {name} {{"
        );
        let _ = writeln!(
            o,
            "        fn encode(&self, e: &mut {RT}::Encoder<'_, C>) -> ::core::result::Result<(), {RT}::EncodeError> {{"
        );
        let _ = writeln!(o, "            {RT}::Encode::<C>::encode(&self.raw(), e)");
        let _ = writeln!(o, "        }}");
        if e.variants.iter().any(|v| v.value.value == 0) {
            let _ = writeln!(o);
            let _ = writeln!(o, "        fn is_default(&self) -> bool {{");
            let _ = writeln!(o, "            self.raw() == 0");
            let _ = writeln!(o, "        }}");
        }
        let _ = writeln!(o, "    }}");
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "    impl<'a, C: {RT}::Capability> {RT}::Decode<'a, C> for {name} {{"
        );
        let _ = writeln!(
            o,
            "        fn decode(d: &mut {RT}::Decoder<'a, '_, C>) -> ::core::result::Result<Self, {RT}::DecodeError> {{"
        );
        let _ = writeln!(
            o,
            "            ::core::result::Result::Ok(Self::from_raw(<{w} as {RT}::Decode<'a, C>>::decode(d)?))"
        );
        let _ = writeln!(o, "        }}");
        if e.variants.iter().any(|v| v.value.value == 0) {
            let _ = writeln!(o);
            let _ = writeln!(o, "        fn missing() -> ::core::option::Option<Self> {{");
            let _ = writeln!(
                o,
                "            ::core::option::Option::Some(Self::from_raw(0))"
            );
            let _ = writeln!(o, "        }}");
        }
        let _ = writeln!(o, "    }}");
        self.out.push_str(&o);
    }

    /// `name: Type` for each field, with lifetime `lt`, comma-separated with
    /// a leading comma.
    fn params(&self, fields: &[Field], lt: &str) -> String {
        fields
            .iter()
            .map(|f| format!(", {}: {}", f.name.name, self.ty(&f.ty, lt)))
            .collect()
    }

    fn client(&mut self) {
        let p = self.p;
        let mut o = String::new();
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "    /// Client of `{}`: one method per protocol method. A reply borrows the client until dropped.",
            p.name
        );
        let _ = writeln!(o, "    pub struct Client<T, C> {{");
        let _ = writeln!(o, "        pub core: {RT}::ClientCore<T, C>,");
        let _ = writeln!(o, "    }}");
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "    impl<T, C> ::core::convert::From<T> for Client<T, C> {{"
        );
        let _ = writeln!(o, "        fn from(transport: T) -> Self {{");
        let _ = writeln!(
            o,
            "            Self {{ core: {RT}::ClientCore::new(transport) }}"
        );
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o, "    }}");
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "    impl<T: {RT}::Transport<C>, C: {RT}::Capability> Client<T, C> {{"
        );
        let mut first = true;
        for m in p.methods() {
            if !first {
                let _ = writeln!(o);
            }
            first = false;
            let rep = format!("{}Reply", camel(&m.name.name));
            let rn = self.needs_fields(&m.results);
            let err = self.error_type(m.error.as_ref().map(|e| e.name.as_str()));
            docs(&mut o, "        ", &m.docs);
            let _ = writeln!(
                o,
                "        pub fn {}<'s>(&'s mut self{}) -> ::core::result::Result<{rep}{}, {RT}::CallError<{err}>> {{",
                m.name.name,
                self.params(&m.params, "'_"),
                rn.args("'s")
            );
            let names: Vec<&str> = m.params.iter().map(|f| f.name.name.as_str()).collect();
            let _ = writeln!(
                o,
                "            self.core.call({}, VERSION, {}Request {{ {} }})",
                m.ordinal.value,
                camel(&m.name.name),
                names.join(", ")
            );
            let _ = writeln!(o, "        }}");
        }
        let _ = writeln!(o, "    }}");
        self.out.push_str(&o);
    }

    fn server(&mut self) {
        let p = self.p;
        let mut o = String::new();
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "    /// Provider side of `{}`: one method per protocol method. [`dispatch`] calls it.",
            p.name
        );
        let _ = writeln!(o, "    pub trait Server<C: {RT}::Capability> {{");
        let mut first = true;
        for m in p.methods() {
            if !first {
                let _ = writeln!(o);
            }
            first = false;
            let rep = format!("{}Reply", camel(&m.name.name));
            let rn = self.needs_fields(&m.results);
            let ret = match &m.error {
                Some(e) => format!("::core::result::Result<{rep}{}, {}>", rn.args("'a"), e.name),
                None => format!("{rep}{}", rn.args("'a")),
            };
            docs(&mut o, "        ", &m.docs);
            // The lifetime is named only where a parameter or the reply
            // borrows from the request: what the reference page shows is
            // what an implementer writes, and nothing in it is needless.
            let params = self.params(&m.params, "'a");
            let (decl, this) = if params.contains("'a") || ret.contains("'a") {
                ("<'a>", "&'a mut self")
            } else {
                ("", "&mut self")
            };
            let _ = writeln!(
                o,
                "        fn {}{decl}({this}{params}) -> {ret};",
                m.name.name
            );
        }
        let _ = writeln!(o, "    }}");
        self.out.push_str(&o);
    }

    fn dispatch(&mut self) {
        let p = self.p;
        let mut o = String::new();
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "    /// Decodes `request` (its capabilities move out of `caps`), calls `server`, and encodes the reply into `reply` and `reply_caps`; returns the reply length."
        );
        let _ = writeln!(
            o,
            "    /// Answers `unknown_method`, `malformed_request` and, when the server's reply exceeds the limits, `reply_too_large` itself (releasing the reply's capabilities)."
        );
        let _ = writeln!(
            o,
            "    /// Cannot fail: a header-only reply fits any `reply` of at least `HEADER_LEN` bytes (a shorter one gets length 0)."
        );
        let _ = writeln!(
            o,
            "    pub fn dispatch<C: {RT}::Capability, S: Server<C> + ?Sized>("
        );
        let _ = writeln!(o, "        server: &mut S,");
        let _ = writeln!(o, "        request: &[u8],");
        let _ = writeln!(o, "        caps: {RT}::CapList<C>,");
        let _ = writeln!(o, "        reply: &mut [u8],");
        let _ = writeln!(o, "        reply_caps: &mut {RT}::CapList<C>,");
        let _ = writeln!(o, "    ) -> usize {{");
        let _ = writeln!(
            o,
            "        let ordinal = match {RT}::request_ordinal(request) {{"
        );
        let _ = writeln!(
            o,
            "            ::core::result::Result::Ok(ordinal) => ordinal,"
        );
        let _ = writeln!(
            o,
            "            ::core::result::Result::Err(raw) => return {RT}::status_reply(reply, reply_caps, raw, VERSION, {RT}::Status::MalformedRequest),"
        );
        let _ = writeln!(o, "        }};");
        let _ = writeln!(
            o,
            "        let encoded: ::core::result::Result<usize, {RT}::EncodeError> = match ordinal {{"
        );
        for m in p.methods() {
            let ord = m.ordinal.value;
            let req = format!("{}Request", camel(&m.name.name));
            let args: Vec<String> = m
                .params
                .iter()
                .map(|f| format!("req.{}", f.name.name))
                .collect();
            let call = format!("server.{}({})", m.name.name, args.join(", "));
            let _ = writeln!(
                o,
                "            {ord} => match {req}::decode_message(request, caps) {{"
            );
            // A method without parameters decodes its (empty) request only to
            // validate it; the binding would be unused.
            let bind = if m.params.is_empty() { "_req" } else { "req" };
            let _ = writeln!(
                o,
                "                ::core::result::Result::Ok({bind}) => {{"
            );
            if m.error.is_some() {
                let _ = writeln!(o, "                    match {call} {{");
                let _ = writeln!(
                    o,
                    "                        ::core::result::Result::Ok(rep) => rep.encode_message(reply, reply_caps),"
                );
                let _ = writeln!(
                    o,
                    "                        ::core::result::Result::Err(err) => {RT}::encode_error_reply(reply, reply_caps, {ord}, VERSION, &err),"
                );
                let _ = writeln!(o, "                    }}");
            } else {
                let _ = writeln!(
                    o,
                    "                    {call}.encode_message(reply, reply_caps)"
                );
            }
            let _ = writeln!(o, "                }}");
            let _ = writeln!(
                o,
                "                ::core::result::Result::Err(e) => return {RT}::status_reply(reply, reply_caps, {ord}, VERSION, {RT}::refusal(e)),"
            );
            let _ = writeln!(o, "            }},");
        }
        let _ = writeln!(
            o,
            "            _ => return {RT}::status_reply(reply, reply_caps, ordinal, VERSION, {RT}::Status::UnknownMethod),"
        );
        let _ = writeln!(o, "        }};");
        let _ = writeln!(o, "        match encoded {{");
        let _ = writeln!(o, "            ::core::result::Result::Ok(n) => n,");
        let _ = writeln!(
            o,
            "            ::core::result::Result::Err(_) => {RT}::status_reply(reply, reply_caps, ordinal, VERSION, {RT}::Status::ReplyTooLarge),"
        );
        let _ = writeln!(o, "        }}");
        let _ = writeln!(o, "    }}");
        self.out.push_str(&o);
    }
}
