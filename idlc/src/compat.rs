//! Evolution check between two versions of one protocol (`spec/IDL.md`
//! section 4): a new version may only
//! add. `silica-idlc check-compat <old> <new>` runs [`check_compat`].
//!
//! Rules, each a diagnostic with file, line and column:
//!
//! - The protocol name is unchanged. `version` never decreases and increases
//!   when anything other than doc comments changed.
//! - Every method, struct, enum and `reserved` ordinal of the old version is
//!   still there under the same name and kind, and the old items keep their
//!   relative order; new items may be inserted anywhere.
//! - A method keeps its ordinal and its `error`. A parameter list, result list
//!   or struct keeps its fields (name and type) as a prefix; appended fields
//!   need a default and may not carry a capability at any depth.
//! - An enum keeps its width and every variant's name and value, in the same
//!   relative order; new variants may be inserted anywhere.
//! - Doc comments may change freely.

use std::path::Path;

use super::ast::{Enum, Field, Item, Protocol, Span, Type, TypeKind};
use super::{Diagnostic, sorted};

struct Cx<'a> {
    old: &'a Protocol,
    new: &'a Protocol,
    out: Vec<Diagnostic>,
}

impl Cx<'_> {
    /// A diagnostic about something present in the new file.
    fn at_new(&mut self, span: Span, message: String) {
        self.push(&self.new.file.clone(), span, message);
    }

    /// A diagnostic about something of the old file missing from the new.
    fn at_old(&mut self, span: Span, message: String) {
        let message = format!("{message} (in {})", self.new.file.display());
        self.push(&self.old.file.clone(), span, message);
    }

    fn push(&mut self, file: &Path, span: Span, message: String) {
        self.out.push(Diagnostic {
            file: file.to_path_buf(),
            span,
            message,
        });
    }
}

/// Every way `new` is not an additive evolution of `old`. Both must already
/// pass [`super::parse()`]. Empty means compatible. Sorted by file and position.
pub fn check_compat(old: &Protocol, new: &Protocol) -> Vec<Diagnostic> {
    let mut cx = Cx {
        old,
        new,
        out: Vec::new(),
    };
    let (on, nn) = (old.name.to_string(), new.name.to_string());
    if on != nn {
        cx.at_new(
            new.name.span(),
            format!("protocol renamed from `{on}` to `{nn}`"),
        );
        return sorted(cx.out);
    }
    items(&mut cx);
    let (ov, nv) = (old.version.value, new.version.value);
    if nv < ov {
        cx.at_new(
            new.version.span,
            format!("version decreased from {ov} to {nv}"),
        );
    } else if nv == ov && !same_ignoring_docs(old, new) {
        cx.at_new(
            new.version.span,
            format!("additive change without a version increment (still {nv})"),
        );
    }
    sorted(cx.out)
}

/// Identity of an item across versions: kind and name, or the ordinal of a
/// `reserved` item.
#[derive(PartialEq, Eq)]
enum Key<'a> {
    Named(&'a str),
    Reserved(u64),
}

fn key(i: &Item) -> Key<'_> {
    match i {
        Item::Reserved(r) => Key::Reserved(r.ordinal.value),
        _ => Key::Named(&i.name().expect("named item").name),
    }
}

fn kind(i: &Item) -> &'static str {
    match i {
        Item::Method(_) => "method",
        Item::Reserved(_) => "reserved ordinal",
        Item::Struct(_) => "struct",
        Item::Enum(_) => "enum",
    }
}

fn item_span(i: &Item) -> Span {
    match i {
        Item::Reserved(r) => r.ordinal.span,
        _ => i.name().expect("named item").span,
    }
}

fn label(i: &Item) -> String {
    match i {
        Item::Reserved(r) => format!("reserved @{}", r.ordinal.value),
        _ => format!("{} `{}`", kind(i), i.name().expect("named item").name),
    }
}

fn items(cx: &mut Cx<'_>) {
    let (old, new) = (cx.old, cx.new);
    let mut last: Option<(usize, &Item)> = None;
    for oi in &old.items {
        let Some((ni_idx, ni)) = new
            .items
            .iter()
            .enumerate()
            .find(|(_, n)| key(n) == key(oi))
        else {
            cx.at_old(item_span(oi), format!("{} removed", label(oi)));
            continue;
        };
        if std::mem::discriminant(oi) != std::mem::discriminant(ni) {
            cx.at_new(
                item_span(ni),
                format!("{} changed to a {}", label(oi), kind(ni)),
            );
            continue;
        }
        if let Some((prev_idx, prev)) = last
            && ni_idx < prev_idx
        {
            cx.at_new(
                item_span(ni),
                format!("{} moved before {}", label(ni), label(prev)),
            );
        }
        last = Some((ni_idx, ni));
        match (oi, ni) {
            (Item::Method(o), Item::Method(n)) => {
                if o.ordinal.value != n.ordinal.value {
                    cx.at_new(
                        n.ordinal.span,
                        format!(
                            "ordinal of method `{}` changed from {} to {}",
                            n.name.name, o.ordinal.value, n.ordinal.value
                        ),
                    );
                }
                let (oe, ne) = (
                    o.error.as_ref().map(|e| e.name.as_str()),
                    n.error.as_ref().map(|e| e.name.as_str()),
                );
                if oe != ne {
                    let span = n.error.as_ref().map_or(n.name.span, |e| e.span);
                    cx.at_new(
                        span,
                        format!(
                            "error of method `{}` changed from {} to {}",
                            n.name.name,
                            oe.map_or("none".into(), |e| format!("`{e}`")),
                            ne.map_or("none".into(), |e| format!("`{e}`")),
                        ),
                    );
                }
                let what = format!("parameters of method `{}`", n.name.name);
                fields(cx, &o.params, &n.params, &what, n.name.span);
                let what = format!("results of method `{}`", n.name.name);
                fields(cx, &o.results, &n.results, &what, n.name.span);
            }
            (Item::Struct(o), Item::Struct(n)) => {
                let what = format!("fields of struct `{}`", n.name.name);
                fields(cx, &o.fields, &n.fields, &what, n.name.span);
            }
            (Item::Enum(o), Item::Enum(n)) => enumeration(cx, o, n),
            _ => {}
        }
    }
}

fn fields(cx: &mut Cx<'_>, old: &[Field], new: &[Field], what: &str, owner: Span) {
    for (i, of) in old.iter().enumerate() {
        let Some(nf) = new.get(i) else {
            cx.at_new(owner, format!("{what}: field `{}` removed", of.name.name));
            continue;
        };
        if of.name.name != nf.name.name {
            let message = if let Some(j) = new.iter().position(|f| f.name.name == of.name.name) {
                format!(
                    "{what}: field `{}` moved from position {} to {}",
                    of.name.name,
                    i + 1,
                    j + 1
                )
            } else if old.iter().any(|f| f.name.name == nf.name.name) {
                format!(
                    "{what}: field `{}` at position {} replaced by `{}`",
                    of.name.name,
                    i + 1,
                    nf.name.name
                )
            } else {
                format!(
                    "{what}: field `{}` renamed to `{}`",
                    of.name.name, nf.name.name
                )
            };
            cx.at_new(nf.name.span, message);
        } else if !same_type(&of.ty, &nf.ty) {
            cx.at_new(
                nf.ty.span,
                format!(
                    "{what}: type of field `{}` changed from `{}` to `{}`",
                    nf.name.name,
                    type_str(&of.ty),
                    type_str(&nf.ty)
                ),
            );
        }
    }
    for nf in new.iter().skip(old.len()) {
        if carries_cap(cx.new, &nf.ty, 0) {
            cx.at_new(
                nf.ty.span,
                format!(
                    "{what}: appended field `{}` of type `{}` can carry a capability, which an older peer cannot skip (spec/IDL.md section 4)",
                    nf.name.name,
                    type_str(&nf.ty)
                ),
            );
        } else if !has_default(cx.new, &nf.ty, 0) {
            cx.at_new(
                nf.ty.span,
                format!(
                    "{what}: appended field `{}` of type `{}` has no default (spec/IDL.md section 4)",
                    nf.name.name,
                    type_str(&nf.ty)
                ),
            );
        }
    }
}

fn enumeration(cx: &mut Cx<'_>, o: &Enum, n: &Enum) {
    if o.width != n.width {
        cx.at_new(
            n.width_span,
            format!(
                "width of enum `{}` changed from {} to {}",
                n.name.name,
                o.width.as_str(),
                n.width.as_str()
            ),
        );
    }
    let mut last: Option<(usize, &str)> = None;
    for ov in &o.variants {
        let Some((idx, nv)) = n
            .variants
            .iter()
            .enumerate()
            .find(|(_, v)| v.name.name == ov.name.name)
        else {
            cx.at_old(
                ov.name.span,
                format!(
                    "variant `{}` of enum `{}` removed",
                    ov.name.name, o.name.name
                ),
            );
            continue;
        };
        if ov.value.value != nv.value.value {
            cx.at_new(
                nv.value.span,
                format!(
                    "value of variant `{}` of enum `{}` changed from {} to {}",
                    nv.name.name, n.name.name, ov.value.value, nv.value.value
                ),
            );
        }
        if let Some((prev_idx, prev)) = last
            && idx < prev_idx
        {
            cx.at_new(
                nv.name.span,
                format!(
                    "variant `{}` of enum `{}` moved before `{prev}`",
                    nv.name.name, n.name.name
                ),
            );
        }
        last = Some((idx, &nv.name.name));
    }
}

/// Whether a value of type `ty` can reference a capability (`cap`, `cap<T>`,
/// `endpoint<P>`), at any depth. An older decoder skips an unknown field's
/// bytes but not the capabilities they index, so such a field cannot be
/// appended (`spec/IDL_RATIONALE.md` section 3).
fn carries_cap(p: &Protocol, ty: &Type, depth: u32) -> bool {
    if depth > 64 {
        return true;
    }
    match &ty.kind {
        TypeKind::Cap(_) | TypeKind::Endpoint(_) => true,
        TypeKind::Array(t) | TypeKind::Option(t) => carries_cap(p, t, depth + 1),
        TypeKind::Named(id) => match p.item(&id.name) {
            Some(Item::Struct(s)) => s.fields.iter().any(|f| carries_cap(p, &f.ty, depth + 1)),
            _ => false,
        },
        _ => false,
    }
}

/// Whether an absent field of type `ty` has a default (`spec/IDL.md`
/// section 4). `depth` bounds the walk; the parser already rejects recursive
/// structs.
fn has_default(p: &Protocol, ty: &Type, depth: u32) -> bool {
    if depth > 64 {
        return false;
    }
    match &ty.kind {
        TypeKind::U8
        | TypeKind::U16
        | TypeKind::U32
        | TypeKind::U64
        | TypeKind::I8
        | TypeKind::I16
        | TypeKind::I32
        | TypeKind::I64
        | TypeKind::Bool
        | TypeKind::String
        | TypeKind::Bytes
        | TypeKind::Array(_)
        | TypeKind::Option(_) => true,
        TypeKind::Cap(_) | TypeKind::Endpoint(_) => false,
        TypeKind::Named(id) => match p.item(&id.name) {
            Some(Item::Enum(e)) => e.variants.iter().any(|v| v.value.value == 0),
            Some(Item::Struct(s)) => s.fields.iter().all(|f| has_default(p, &f.ty, depth + 1)),
            _ => false,
        },
    }
}

/// Type equality as written, ignoring positions. Named types compare by
/// name; the named items themselves are compared as items.
fn same_type(a: &Type, b: &Type) -> bool {
    match (&a.kind, &b.kind) {
        (TypeKind::Array(x), TypeKind::Array(y)) | (TypeKind::Option(x), TypeKind::Option(y)) => {
            same_type(x, y)
        }
        (TypeKind::Named(x), TypeKind::Named(y)) => x.name == y.name,
        (TypeKind::Endpoint(x), TypeKind::Endpoint(y)) => x.to_string() == y.to_string(),
        (x, y) => x == y,
    }
}

fn type_str(t: &Type) -> String {
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
        TypeKind::Array(x) => format!("array<{}>", type_str(x)),
        TypeKind::Option(x) => format!("option<{}>", type_str(x)),
        TypeKind::Cap(None) => "cap".into(),
        TypeKind::Cap(Some(o)) => format!("cap<{}>", o.as_str()),
        TypeKind::Endpoint(p) => format!("endpoint<{p}>"),
        TypeKind::Named(id) => id.name.clone(),
    }
}

/// Whether two versions differ only in doc comments (and positions).
fn same_ignoring_docs(a: &Protocol, b: &Protocol) -> bool {
    a.items.len() == b.items.len()
        && a.items.iter().zip(&b.items).all(|(x, y)| match (x, y) {
            (Item::Method(x), Item::Method(y)) => {
                x.name.name == y.name.name
                    && x.ordinal.value == y.ordinal.value
                    && x.error.as_ref().map(|e| &e.name) == y.error.as_ref().map(|e| &e.name)
                    && same_fields(&x.params, &y.params)
                    && same_fields(&x.results, &y.results)
            }
            (Item::Reserved(x), Item::Reserved(y)) => x.ordinal.value == y.ordinal.value,
            (Item::Struct(x), Item::Struct(y)) => {
                x.name.name == y.name.name && same_fields(&x.fields, &y.fields)
            }
            (Item::Enum(x), Item::Enum(y)) => {
                x.name.name == y.name.name
                    && x.width == y.width
                    && x.variants.len() == y.variants.len()
                    && x.variants
                        .iter()
                        .zip(&y.variants)
                        .all(|(v, w)| v.name.name == w.name.name && v.value.value == w.value.value)
            }
            _ => false,
        })
}

fn same_fields(a: &[Field], b: &[Field]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.name.name == y.name.name && same_type(&x.ty, &y.ty))
}
