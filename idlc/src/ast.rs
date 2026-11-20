//! Typed syntax tree of one protocol file (`spec/IDL.md` section 2).
//!
//! Every name, number and type carries the position where it was written, so
//! the semantic checks can report file, line and column.

use std::fmt;
use std::path::PathBuf;

/// A 1-based line and column (column counted in characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

/// An identifier as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// An integer literal (decimal or `0x` hexadecimal) as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Int {
    pub value: u64,
    pub span: Span,
}

/// A dotted protocol name such as `sample.Echo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PName {
    pub segments: Vec<Ident>,
}

impl PName {
    /// Position of the first segment.
    pub fn span(&self) -> Span {
        self.segments[0].span
    }
}

impl fmt::Display for PName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, s) in self.segments.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            f.write_str(&s.name)?;
        }
        Ok(())
    }
}

/// One parsed `.idl` file: exactly one protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Protocol {
    /// The file the protocol was read from, as given to the parser.
    pub file: PathBuf,
    pub docs: Vec<String>,
    pub name: PName,
    pub version: Int,
    pub items: Vec<Item>,
}

impl Protocol {
    /// The item declared with `name`, if any.
    pub fn item(&self, name: &str) -> Option<&Item> {
        self.items
            .iter()
            .find(|i| i.name().is_some_and(|n| n.name == name))
    }

    pub fn methods(&self) -> impl Iterator<Item = &Method> {
        self.items.iter().filter_map(|i| match i {
            Item::Method(m) => Some(m),
            _ => None,
        })
    }

    pub fn structs(&self) -> impl Iterator<Item = &Struct> {
        self.items.iter().filter_map(|i| match i {
            Item::Struct(s) => Some(s),
            _ => None,
        })
    }

    pub fn enums(&self) -> impl Iterator<Item = &Enum> {
        self.items.iter().filter_map(|i| match i {
            Item::Enum(e) => Some(e),
            _ => None,
        })
    }

    pub fn reserved(&self) -> impl Iterator<Item = &Reserved> {
        self.items.iter().filter_map(|i| match i {
            Item::Reserved(r) => Some(r),
            _ => None,
        })
    }
}

/// An item of a protocol, in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Method(Method),
    Reserved(Reserved),
    Struct(Struct),
    Enum(Enum),
}

impl Item {
    /// The declared name; `reserved` items have none.
    pub fn name(&self) -> Option<&Ident> {
        match self {
            Item::Method(m) => Some(&m.name),
            Item::Reserved(_) => None,
            Item::Struct(s) => Some(&s.name),
            Item::Enum(e) => Some(&e.name),
        }
    }
}

/// `name @ordinal (params) -> (results) error E;`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    pub docs: Vec<String>,
    pub name: Ident,
    pub ordinal: Int,
    pub params: Vec<Field>,
    pub results: Vec<Field>,
    pub error: Option<Ident>,
}

/// `reserved @ordinal;`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reserved {
    pub docs: Vec<String>,
    pub ordinal: Int,
}

/// `struct Name { fields }`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Struct {
    pub docs: Vec<String>,
    pub name: Ident,
    pub fields: Vec<Field>,
}

/// `enum Name : width { variants }`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enum {
    pub docs: Vec<String>,
    pub name: Ident,
    pub width: Width,
    pub width_span: Span,
    pub variants: Vec<Variant>,
}

/// Declared width of an enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    U8,
    U16,
    U32,
}

impl Width {
    /// Largest value the width holds.
    pub fn max(self) -> u64 {
        match self {
            Width::U8 => u8::MAX.into(),
            Width::U16 => u16::MAX.into(),
            Width::U32 => u32::MAX.into(),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Width::U8 => "u8",
            Width::U16 => "u16",
            Width::U32 => "u32",
        }
    }
}

/// `name = value`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub docs: Vec<String>,
    pub name: Ident,
    pub value: Int,
}

/// `name: type` in a parameter list, result list or struct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub docs: Vec<String>,
    pub name: Ident,
    pub ty: Type,
}

/// A type as written, with its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Bool,
    String,
    Bytes,
    Array(Box<Type>),
    Option(Box<Type>),
    /// `cap` (`None`) or `cap<T>`.
    Cap(Option<ObjType>),
    /// `endpoint<P>`.
    Endpoint(PName),
    /// A struct or enum of this protocol.
    Named(Ident),
}

/// Kernel object types a `cap<T>` may name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjType {
    Untyped,
    CapSpace,
    Thread,
    AddressSpace,
    Frame,
    Endpoint,
    Notification,
    SchedContext,
    Hw,
    Vcpu,
}

impl ObjType {
    pub const ALL: [ObjType; 10] = [
        ObjType::Untyped,
        ObjType::CapSpace,
        ObjType::Thread,
        ObjType::AddressSpace,
        ObjType::Frame,
        ObjType::Endpoint,
        ObjType::Notification,
        ObjType::SchedContext,
        ObjType::Hw,
        ObjType::Vcpu,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ObjType::Untyped => "Untyped",
            ObjType::CapSpace => "CapSpace",
            ObjType::Thread => "Thread",
            ObjType::AddressSpace => "AddressSpace",
            ObjType::Frame => "Frame",
            ObjType::Endpoint => "Endpoint",
            ObjType::Notification => "Notification",
            ObjType::SchedContext => "SchedContext",
            ObjType::Hw => "Hw",
            ObjType::Vcpu => "Vcpu",
        }
    }

    pub fn from_name(s: &str) -> Option<ObjType> {
        ObjType::ALL.into_iter().find(|o| o.as_str() == s)
    }
}
