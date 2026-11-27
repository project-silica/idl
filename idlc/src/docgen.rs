//! The reference page of a protocol, generated from its source
//! (`spec/ARCHITECTURE.md` section 17: documentation generated from source).
//!
//! One Markdown page per protocol: what it is for, every method with its
//! parameters, results and errors, every type, the failures common to all
//! calls, and how the Rust code `silica-idlc rust` generates for it is used.
//! A client or a server can be written from this page and the runtime's own
//! documentation, without reading the protocol's source.

use super::ast::{Field, Item, Protocol, Type, TypeKind};
use super::rustgen::{camel, file_name, module_name};
use std::fmt::Write as _;

fn ty(t: &Type) -> String {
    match &t.kind {
        TypeKind::U8 => "u8".into(),
        TypeKind::U16 => "u16".into(),
        TypeKind::U32 => "u32".into(),
        TypeKind::U64 => "u64".into(),
        TypeKind::I8 => "i8".into(),
        TypeKind::I16 => "i16".into(),
        TypeKind::I32 => "i32".into(),
        TypeKind::I64 => "i64".into(),
        TypeKind::Bool => "bool".into(),
        TypeKind::String => "string".into(),
        TypeKind::Bytes => "bytes".into(),
        TypeKind::Array(e) => format!("array<{}>", ty(e)),
        TypeKind::Option(e) => format!("option<{}>", ty(e)),
        TypeKind::Cap(None) => "cap".into(),
        TypeKind::Cap(Some(o)) => format!("cap<{}>", o.as_str()),
        TypeKind::Endpoint(p) => format!("endpoint<{p}>"),
        TypeKind::Named(n) => n.name.clone(),
    }
}

fn docs(o: &mut String, lines: &[String]) {
    for l in lines {
        let _ = writeln!(o, "{}", l.trim());
    }
    if !lines.is_empty() {
        o.push('\n');
    }
}

fn fields(o: &mut String, heading: &str, fs: &[Field]) {
    if fs.is_empty() {
        let _ = writeln!(o, "{heading}: none.\n");
        return;
    }
    let _ = writeln!(
        o,
        "{heading}:\n\n| Name | Type | Meaning |\n| --- | --- | --- |"
    );
    for f in fs {
        let meaning = f
            .docs
            .iter()
            .map(|d| d.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(o, "| `{}` | `{}` | {} |", f.name.name, ty(&f.ty), meaning);
    }
    o.push('\n');
}

/// The exact Rust items of the generated module that a user names: every
/// struct and enum, the client's methods and the provider's trait, taken
/// from the generated code itself so the page cannot drift from it.
fn interface(o: &mut String, p: &Protocol) {
    let Ok(code) = super::rustgen::rust(p) else {
        return;
    };
    let mut out = String::new();
    let mut block = false; // inside a struct, enum or the Server trait
    let mut client = false; // inside `impl ... Client<T, C>`
    let mut dispatch = false; // inside the signature of `dispatch`
    let mut derive = ""; // the derive attribute before the next item
    for line in code.lines() {
        let t = line.trim_start();
        let indent = line.len() - t.len();
        if dispatch {
            out.push_str(line.get(4..).unwrap_or(""));
            if t.starts_with(')') {
                dispatch = false;
                out.truncate(out.trim_end_matches(" {").len());
                out.push_str(";\n\n");
            } else {
                out.push('\n');
            }
        } else if indent == 4 && t.starts_with("#[derive(") {
            derive = t;
        } else if indent == 4 && t.starts_with("pub fn dispatch<") {
            dispatch = true;
            out.push_str(t);
            out.push('\n');
        } else if indent == 4
            && t.starts_with("impl<T, C> ::core::convert::From<T> for Client<T, C>")
        {
            out.push_str("impl<T, C> From<T> for Client<T, C>;  // `Client::from(transport)`\n\n");
        } else if block {
            out.push_str(line.get(4..).unwrap_or(""));
            out.push('\n');
            if indent == 4 && t == "}" {
                block = false;
                out.push('\n');
            }
        } else if indent == 4
            && (t.starts_with("pub struct ")
                || t.starts_with("pub enum ")
                || t.starts_with("pub trait Server"))
        {
            block = !t.ends_with(';') && !t.ends_with('}');
            if !derive.is_empty() {
                out.push_str(derive);
                out.push('\n');
                derive = "";
            }
            out.push_str(t);
            out.push('\n');
            if !block {
                out.push('\n');
            }
        } else if indent == 4
            && t.starts_with("impl")
            && t.contains("Transport<C>")
            && t.contains(" Client<T, C>")
        {
            client = true;
            out.push_str("// Methods of Client<T, C>, for T: Transport<C>:\n");
        } else if client && indent == 4 && t == "}" {
            client = false;
            out.push('\n');
        } else if client && indent == 8 && t.starts_with("pub fn ") {
            out.push_str(t.trim_end_matches('{').trim_end());
            out.push_str(";\n");
        }
    }
    let _ = writeln!(
        o,
        "The items of `{}` a program names, exactly as generated (`bytes` is `&[u8]`, `string` is `&str`, \
         `array<T>` is `silica_idl_rt::Array`, built with `Array::new(&[..])` and read with `.len()` and \
         `.iter()`; a capability is a `C`; an enum has one more variant, `Unknown(raw)`, for a value a \
         newer provider replies with (a request carrying one is answered `unknown_field`); every struct and enum also implements `Clone`, `Debug` and `PartialEq`, and has no \
         fields but the ones shown):\n\n```rust\n{}```\n",
        module_name(p),
        out
    );
}

/// The Markdown reference page of protocol `p`.
pub fn markdown(p: &Protocol) -> String {
    let mut o = String::new();
    let module = module_name(p);
    let _ = writeln!(o, "# {} (version {})\n", p.name, p.version.value);
    docs(&mut o, &p.docs);

    let _ = writeln!(o, "## Methods\n");
    for m in p.methods() {
        let _ = writeln!(o, "### `{}` (ordinal {})\n", m.name.name, m.ordinal.value);
        docs(&mut o, &m.docs);
        fields(&mut o, "Parameters", &m.params);
        fields(&mut o, "Results", &m.results);
        match &m.error {
            Some(e) => {
                let _ = writeln!(
                    o,
                    "Fails with a value of `{}` (below), or with one of the failures common to every call.\n",
                    e.name
                );
            }
            None => {
                let _ = writeln!(
                    o,
                    "Has no error of its own; it can fail only with the failures common to every call.\n"
                );
            }
        }
    }
    for r in p.reserved() {
        let _ = writeln!(
            o,
            "Ordinal {} is retired and never reused.\n",
            r.ordinal.value
        );
    }

    if p.structs().next().is_some() || p.enums().next().is_some() {
        let _ = writeln!(o, "## Types\n");
    }
    for item in &p.items {
        match item {
            Item::Struct(s) => {
                let _ = writeln!(o, "### struct `{}`\n", s.name.name);
                docs(&mut o, &s.docs);
                fields(&mut o, "Fields", &s.fields);
            }
            Item::Enum(e) => {
                let _ = writeln!(o, "### enum `{}` : `{}`\n", e.name.name, e.width.as_str());
                docs(&mut o, &e.docs);
                let _ = writeln!(o, "| Value | Name | Meaning |\n| --- | --- | --- |");
                for v in &e.variants {
                    let meaning = v
                        .docs
                        .iter()
                        .map(|d| d.trim())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let _ = writeln!(o, "| {} | `{}` | {} |", v.value.value, v.name.name, meaning);
                }
                let _ = writeln!(
                    o,
                    "\nA receiver that meets a value it does not know keeps it as unknown; it is not an error.\n"
                );
            }
            _ => {}
        }
    }

    let _ = writeln!(
        o,
        "## Failures common to every call\n\n\
         | Failure | Meaning |\n| --- | --- |\n\
         | `revoked` | The caller's capability to the provider was revoked (also in the middle of the call) or was never valid. |\n\
         | `peer_gone` | The provider ended or restarted while the call was pending. |\n\
         | `cancelled` | The caller cancelled the pending call. |\n\
         | `too_large` | The request exceeds the message limits (4000 bytes, 4 capabilities); detected before sending. |\n\
         | `malformed_reply` | The reply could not be decoded. |\n\
         | `unknown_method` | The provider does not implement the method (it is older than the caller). |\n\
         | `malformed_request` | The provider could not decode the request. |\n\
         | `unknown_field` | The request carries a field or an enum value the provider does not know (it is older than the caller); nothing was done. |\n\
         | `reply_too_large` | The provider's reply exceeded the message limits. |\n\n\
         None of them carries reply data. A method is safe to retry after any of them unless its description says otherwise.\n"
    );

    let _ = writeln!(o, "## Using it from Rust\n");
    let _ = writeln!(
        o,
        "`silica-idlc rust <out-dir> <file.idl>` writes `{}` containing `pub mod {module}`. A crate includes it with \
         `include!(concat!(env!(\"OUT_DIR\"), \"/{}\"));` after generating it in its build script, and depends on `silica-idl-rt`.\n",
        file_name(p),
        file_name(p)
    );
    let _ = writeln!(
        o,
        "The module is generic over `C`, the capability type of the transport (on Silica, the runtime's capability handle).\n\n\
         **Calling.** `{module}::Client::from(transport)` wraps anything that implements `silica_idl_rt::Transport<C>`; \
         there is one method per protocol method, taking the parameters in order and returning \
         `Result<{module}::<Method>Reply, silica_idl_rt::CallError<E>>` where `E` is the method's error enum \
         (`core::convert::Infallible` when it has none). `CallError::Error(e)` is the method's own error; \
         `CallError::Transport(t)` one of the common failures. A reply borrows the client until it is dropped: \
         strings, bytes and arrays in it are views of the client's message buffer (one per client: the request is encoded into it and the reply arrives over it).\n"
    );
    interface(&mut o, p);
    let _ = writeln!(
        o,
        "| Method | Client call | Reply type | Error type |\n| --- | --- | --- | --- |"
    );
    for m in p.methods() {
        let args = m
            .params
            .iter()
            .map(|f| f.name.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let err = m
            .error
            .as_ref()
            .map_or("Infallible".to_string(), |e| e.name.clone());
        let _ = writeln!(
            o,
            "| `{}` | `client.{}({})` | `{}Reply` | `{}` |",
            m.name.name,
            m.name.name,
            args,
            camel(&m.name.name),
            err
        );
    }
    let _ = writeln!(
        o,
        "\n**Providing.** Implement `{module}::Server<C>`: one method per protocol method, taking the parameters in \
         order (capabilities by value: they are the provider's now) and returning the method's `<Method>Reply`, or \
         `Result<<Method>Reply, E>` when the method declares an error. For each request received, call \
         `{module}::dispatch(&mut server, request_bytes, request_caps, &mut reply_buffer, &mut reply_caps)`, which takes \
         the request's capability list by value, decodes, calls the method, encodes the reply (or the error, or \
         `unknown_method`, `malformed_request`, `reply_too_large`) and returns the reply's length; it cannot fail. \
         The runtime's server loop on Silica (`silica_rt::ipc::serve`) receives, calls a closure with exactly these \
         arguments, and sends the reply.\n"
    );
    o
}
