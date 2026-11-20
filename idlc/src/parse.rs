//! Lexer and recursive-descent parser for the grammar of `spec/IDL.md`
//! section 2. Syntax errors stop the parse of a file at the first one; the
//! semantic rules are checked afterwards by [`super::check`].

use std::path::Path;

use super::Diagnostic;
use super::ast::{
    Enum, Field, Ident, Int, Item, Method, ObjType, PName, Protocol, Reserved, Span, Struct, Type,
    TypeKind, Variant, Width,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Int(u64),
    Doc(String),
    Punct(char),
    Arrow,
    Eof,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Int(v) => format!("integer `{v}`"),
            Tok::Doc(_) => "a doc comment".to_string(),
            Tok::Punct(c) => format!("`{c}`"),
            Tok::Arrow => "`->`".to_string(),
            Tok::Eof => "end of file".to_string(),
        }
    }
}

struct Token {
    tok: Tok,
    span: Span,
}

type PResult<T> = Result<T, (Span, String)>;

fn lex(src: &str) -> PResult<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    while i < chars.len() {
        let c = chars[i];
        let span = Span { line, col };
        let start = i;
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            col += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            let mut end = i;
            while end < chars.len() && chars[end] != '\n' {
                end += 1;
            }
            // `///` (but not `////`) is a doc comment.
            let is_doc = chars.get(i + 2) == Some(&'/') && chars.get(i + 3) != Some(&'/');
            if is_doc {
                let text: String = chars[i + 3..end].iter().collect();
                let text = text.strip_suffix('\r').unwrap_or(&text);
                let text = text.strip_prefix(' ').unwrap_or(text);
                out.push(Token {
                    tok: Tok::Doc(text.to_string()),
                    span,
                });
            }
            col += (end - i) as u32;
            i = end;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push(Token {
                tok: Tok::Ident(s),
                span,
            });
        } else if c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            let parsed = match s.strip_prefix("0x") {
                Some(hex) if !hex.is_empty() && hex.chars().all(|d| d.is_ascii_hexdigit()) => {
                    u64::from_str_radix(hex, 16).map_err(|_| "integer does not fit in 64 bits")
                }
                Some(_) => Err("invalid hexadecimal integer"),
                None if s.chars().all(|d| d.is_ascii_digit()) => s
                    .parse::<u64>()
                    .map_err(|_| "integer does not fit in 64 bits"),
                None => Err("invalid integer"),
            };
            match parsed {
                Ok(v) => out.push(Token {
                    tok: Tok::Int(v),
                    span,
                }),
                Err(e) => return Err((span, format!("{e}: `{s}`"))),
            }
        } else if c == '-' && chars.get(i + 1) == Some(&'>') {
            i += 2;
            out.push(Token {
                tok: Tok::Arrow,
                span,
            });
        } else if "{}()<>@:;,=.".contains(c) {
            i += 1;
            out.push(Token {
                tok: Tok::Punct(c),
                span,
            });
        } else {
            return Err((span, format!("unexpected character {c:?}")));
        }
        col += (i - start) as u32;
    }
    out.push(Token {
        tok: Tok::Eof,
        span: Span { line, col },
    });
    Ok(out)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn bump(&mut self) -> &Token {
        let t = &self.toks[self.pos];
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn unexpected<T>(&self, what: &str) -> PResult<T> {
        Err((
            self.span(),
            format!("expected {what}, found {}", self.peek().describe()),
        ))
    }

    fn is_punct(&self, c: char) -> bool {
        *self.peek() == Tok::Punct(c)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == w)
    }

    fn punct(&mut self, c: char) -> PResult<()> {
        if self.is_punct(c) {
            self.bump();
            Ok(())
        } else {
            self.unexpected(&format!("`{c}`"))
        }
    }

    fn word(&mut self, w: &str) -> PResult<Span> {
        if self.is_word(w) {
            Ok(self.bump().span)
        } else {
            self.unexpected(&format!("`{w}`"))
        }
    }

    fn ident(&mut self) -> PResult<Ident> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                let span = self.bump().span;
                Ok(Ident { name, span })
            }
            _ => self.unexpected("an identifier"),
        }
    }

    fn int(&mut self) -> PResult<Int> {
        match *self.peek() {
            Tok::Int(value) => {
                let span = self.bump().span;
                Ok(Int { value, span })
            }
            _ => self.unexpected("an integer"),
        }
    }

    fn docs(&mut self) -> Vec<String> {
        let mut docs = Vec::new();
        while let Tok::Doc(d) = self.peek() {
            docs.push(d.clone());
            self.bump();
        }
        docs
    }

    fn pname(&mut self) -> PResult<PName> {
        let mut segments = vec![self.ident()?];
        while self.is_punct('.') {
            self.bump();
            segments.push(self.ident()?);
        }
        Ok(PName { segments })
    }

    fn file(&mut self, file: &Path) -> PResult<Protocol> {
        let docs = self.docs();
        self.word("protocol")?;
        let name = self.pname()?;
        self.word("version")?;
        let version = self.int()?;
        self.punct('{')?;
        let mut items = Vec::new();
        loop {
            let item_docs = self.docs();
            if item_docs.is_empty() && self.is_punct('}') {
                break;
            }
            items.push(self.item(item_docs)?);
        }
        self.punct('}')?;
        if *self.peek() != Tok::Eof {
            return self.unexpected("end of file (one protocol per file)");
        }
        Ok(Protocol {
            file: file.to_path_buf(),
            docs,
            name,
            version,
            items,
        })
    }

    fn item(&mut self, docs: Vec<String>) -> PResult<Item> {
        let reserved_form = self.is_word("reserved")
            && *self.peek_at(1) == Tok::Punct('@')
            && matches!(self.peek_at(2), Tok::Int(_))
            && *self.peek_at(3) == Tok::Punct(';');
        if reserved_form {
            self.bump();
            self.punct('@')?;
            let ordinal = self.int()?;
            self.punct(';')?;
            return Ok(Item::Reserved(Reserved { docs, ordinal }));
        }
        if self.is_word("struct") {
            self.bump();
            let name = self.ident()?;
            self.punct('{')?;
            let fields = self.fields('}')?;
            self.punct('}')?;
            return Ok(Item::Struct(Struct { docs, name, fields }));
        }
        if self.is_word("enum") {
            self.bump();
            let name = self.ident()?;
            self.punct(':')?;
            let width_span = self.span();
            let width = match self.peek() {
                Tok::Ident(s) if s == "u8" => Width::U8,
                Tok::Ident(s) if s == "u16" => Width::U16,
                Tok::Ident(s) if s == "u32" => Width::U32,
                _ => return self.unexpected("`u8`, `u16` or `u32`"),
            };
            self.bump();
            self.punct('{')?;
            let mut variants = vec![self.variant()?];
            while self.is_punct(',') {
                self.bump();
                if self.is_punct('}') {
                    break;
                }
                variants.push(self.variant()?);
            }
            self.punct('}')?;
            return Ok(Item::Enum(Enum {
                docs,
                name,
                width,
                width_span,
                variants,
            }));
        }
        if !matches!(self.peek(), Tok::Ident(_)) {
            return self.unexpected("a method, `reserved`, `struct` or `enum`");
        }
        let name = self.ident()?;
        self.punct('@')?;
        let ordinal = self.int()?;
        self.punct('(')?;
        let params = self.fields(')')?;
        self.punct(')')?;
        if *self.peek() != Tok::Arrow {
            return self.unexpected("`->`");
        }
        self.bump();
        self.punct('(')?;
        let results = self.fields(')')?;
        self.punct(')')?;
        let error = if self.is_word("error") {
            self.bump();
            Some(self.ident()?)
        } else {
            None
        };
        self.punct(';')?;
        Ok(Item::Method(Method {
            docs,
            name,
            ordinal,
            params,
            results,
            error,
        }))
    }

    /// `fields?` up to (not including) `close`.
    fn fields(&mut self, close: char) -> PResult<Vec<Field>> {
        let mut fields = Vec::new();
        while !self.is_punct(close) {
            let docs = self.docs();
            let name = self.ident()?;
            self.punct(':')?;
            let ty = self.ty()?;
            fields.push(Field { docs, name, ty });
            if !self.is_punct(',') {
                break;
            }
            self.bump();
        }
        Ok(fields)
    }

    fn variant(&mut self) -> PResult<Variant> {
        let docs = self.docs();
        let name = self.ident()?;
        self.punct('=')?;
        let value = self.int()?;
        Ok(Variant { docs, name, value })
    }

    fn ty(&mut self) -> PResult<Type> {
        let span = self.span();
        let Tok::Ident(word) = self.peek().clone() else {
            return self.unexpected("a type");
        };
        self.bump();
        let kind = match word.as_str() {
            "u8" => TypeKind::U8,
            "u16" => TypeKind::U16,
            "u32" => TypeKind::U32,
            "u64" => TypeKind::U64,
            "i8" => TypeKind::I8,
            "i16" => TypeKind::I16,
            "i32" => TypeKind::I32,
            "i64" => TypeKind::I64,
            "bool" => TypeKind::Bool,
            "string" => TypeKind::String,
            "bytes" => TypeKind::Bytes,
            "array" | "option" => {
                self.punct('<')?;
                let inner = Box::new(self.ty()?);
                self.punct('>')?;
                if word == "array" {
                    TypeKind::Array(inner)
                } else {
                    TypeKind::Option(inner)
                }
            }
            "cap" => {
                if self.is_punct('<') {
                    self.bump();
                    let obj = match self.peek() {
                        Tok::Ident(s) => ObjType::from_name(s),
                        _ => None,
                    };
                    let Some(obj) = obj else {
                        let names: Vec<_> = ObjType::ALL.iter().map(|o| o.as_str()).collect();
                        return self.unexpected(&format!("an object type ({})", names.join(", ")));
                    };
                    self.bump();
                    self.punct('>')?;
                    TypeKind::Cap(Some(obj))
                } else {
                    TypeKind::Cap(None)
                }
            }
            "endpoint" => {
                self.punct('<')?;
                let p = self.pname()?;
                self.punct('>')?;
                TypeKind::Endpoint(p)
            }
            _ => TypeKind::Named(Ident { name: word, span }),
        };
        Ok(Type { kind, span })
    }
}

/// Parse the text of one protocol file. Reports the first syntax error.
pub fn parse_syntax(file: &Path, src: &str) -> Result<Protocol, Diagnostic> {
    let to_diag = |(span, message): (Span, String)| Diagnostic {
        file: file.to_path_buf(),
        span,
        message,
    };
    let toks = lex(src).map_err(to_diag)?;
    Parser { toks, pos: 0 }.file(file).map_err(to_diag)
}
