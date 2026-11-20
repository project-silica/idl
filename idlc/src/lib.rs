//! The Silica IDL compiler (component A06): parser and semantic checks of the
//! grammar fixed by `spec/IDL.md`, and the Rust generator of
//! section 6 (runtime: `silica-idl-rt`), and the evolution check of
//! section 4 ([`compat::check_compat`]).
//!
//! Entry points: [`parse`] for one file's text, [`compile`] for a set of
//! files compiled together, [`load`] to read and compile files from disk,
//! [`rustgen::rust`] for the Rust module of one compiled protocol,
//! [`check_compat_files`] to compare two versions of one protocol.

pub mod ast;
pub mod check;
pub mod compat;
pub mod parse;
pub mod rustgen;

use std::fmt;
use std::path::{Path, PathBuf};

pub use ast::Protocol;

/// Directory, relative to the `idl` repository root, holding protocol files.
pub const PROTOCOL_DIR: &str = "protocols";

/// File extension of protocol definitions.
pub const EXTENSION: &str = "idl";

/// One compile error, with the file, line and column it refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub span: ast::Span,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file.display(),
            self.span.line,
            self.span.col,
            self.message
        )
    }
}

fn sorted(mut d: Vec<Diagnostic>) -> Vec<Diagnostic> {
    d.sort_by(|a, b| {
        (&a.file, a.span)
            .cmp(&(&b.file, b.span))
            .then_with(|| a.message.cmp(&b.message))
    });
    d
}

/// Parse one protocol file and check every rule that needs only that file.
/// `file` is used for diagnostics only. Errors are sorted by position.
pub fn parse(file: &Path, src: &str) -> Result<Protocol, Vec<Diagnostic>> {
    let p = parse::parse_syntax(file, src).map_err(|d| vec![d])?;
    let errs = check::check_protocol(&p);
    if errs.is_empty() {
        Ok(p)
    } else {
        Err(sorted(errs))
    }
}

/// Parse and check a set of files compiled together (so `endpoint<P>` may
/// name a protocol of any of them). Returns the protocols in input order, or
/// every diagnostic of every file, sorted by file and position.
pub fn compile(sources: &[(PathBuf, String)]) -> Result<Vec<Protocol>, Vec<Diagnostic>> {
    let mut protocols = Vec::new();
    let mut errs = Vec::new();
    for (file, src) in sources {
        match parse(file, src) {
            Ok(p) => protocols.push(p),
            Err(e) => errs.extend(e),
        }
    }
    // Set rules only once every file parsed: a broken file would otherwise
    // make every `endpoint<P>` naming it look unresolved.
    if errs.is_empty() {
        errs.extend(check::check_set(&protocols));
    }
    if errs.is_empty() {
        Ok(protocols)
    } else {
        Err(sorted(errs))
    }
}

/// Read `paths` from disk and [`compile`] them. Also checks that protocol
/// `a.b.Name` is in a file ending in `a/b/name.idl` (`spec/IDL.md` section 1).
pub fn load(paths: &[PathBuf]) -> Result<Vec<Protocol>, Vec<Diagnostic>> {
    let mut sources = Vec::new();
    let mut errs = Vec::new();
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(s) => sources.push((path.clone(), s)),
            Err(e) => errs.push(Diagnostic {
                file: path.clone(),
                span: ast::Span { line: 1, col: 1 },
                message: format!("cannot read file: {e}"),
            }),
        }
    }
    match compile(&sources) {
        Ok(ps) => {
            errs.extend(ps.iter().filter_map(check::check_location));
            if errs.is_empty() {
                Ok(ps)
            } else {
                Err(sorted(errs))
            }
        }
        Err(e) => {
            errs.extend(e);
            Err(sorted(errs))
        }
    }
}

/// Read two versions of one protocol, check each on its own ([`parse`]) and
/// then that `new` only adds to `old` ([`compat::check_compat`],
/// `spec/IDL.md` section 4). Rules that need other files (`endpoint<P>`
/// resolution) are left to [`load`].
pub fn check_compat_files(old: &Path, new: &Path) -> Result<(), Vec<Diagnostic>> {
    let read = |path: &Path| {
        std::fs::read_to_string(path).map_err(|e| {
            vec![Diagnostic {
                file: path.to_path_buf(),
                span: ast::Span { line: 1, col: 1 },
                message: format!("cannot read file: {e}"),
            }]
        })
    };
    let (os, ns) = (read(old), read(new));
    let (os, ns) = match (os, ns) {
        (Ok(o), Ok(n)) => (o, n),
        (o, n) => {
            let mut errs = o.err().unwrap_or_default();
            errs.extend(n.err().unwrap_or_default());
            return Err(errs);
        }
    };
    let (op, np) = (parse(old, &os), parse(new, &ns));
    let (op, np) = match (op, np) {
        (Ok(o), Ok(n)) => (o, n),
        (o, n) => {
            let mut errs = o.err().unwrap_or_default();
            errs.extend(n.err().unwrap_or_default());
            return Err(sorted(errs));
        }
    };
    let errs = compat::check_compat(&op, &np);
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}
