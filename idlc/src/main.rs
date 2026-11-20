//! `silica-idlc` command line.
//!
//! `silica-idlc check <file.idl>...` parses and checks the files as one set;
//! prints one `ERROR:` line per diagnostic and exits 1 on any, 0 otherwise.
//! `silica-idlc rust <out-dir> <file.idl>...` also writes one Rust module per
//! protocol to `<out-dir>/<a>_<b>_<name>.rs` (`spec/IDL.md` section 6).
//! `silica-idlc check-compat <old.idl> <new.idl>` checks that the new version
//! of a protocol only adds to the old one (`spec/IDL.md` section 4); one
//! `ERROR:` line per violation and exit 1 on any, 0 otherwise.
//! Usage errors exit 2.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn report(errs: Vec<idlc::Diagnostic>) -> ExitCode {
    for e in errs {
        eprintln!("ERROR: {e}");
    }
    ExitCode::from(1)
}

fn rust(out: &Path, paths: &[PathBuf]) -> ExitCode {
    let ps = match idlc::load(paths) {
        Ok(ps) => ps,
        Err(errs) => return report(errs),
    };
    for p in &ps {
        let code = match idlc::rustgen::rust(p) {
            Ok(c) => c,
            Err(errs) => return report(errs),
        };
        let file = out.join(idlc::rustgen::file_name(p));
        if let Err(e) = std::fs::write(&file, code) {
            eprintln!("ERROR: {}: cannot write: {e}", file.display());
            return ExitCode::from(1);
        }
    }
    println!("silica-idlc: {} module(s) written", ps.len());
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") if args.len() > 1 => {
            let paths: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
            match idlc::load(&paths) {
                Ok(ps) => {
                    println!("silica-idlc: {} protocol(s) ok", ps.len());
                    ExitCode::SUCCESS
                }
                Err(errs) => report(errs),
            }
        }
        Some("rust") if args.len() > 2 => {
            let paths: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
            rust(Path::new(&args[1]), &paths)
        }
        Some("check-compat") if args.len() == 3 => {
            match idlc::check_compat_files(Path::new(&args[1]), Path::new(&args[2])) {
                Ok(()) => {
                    println!("silica-idlc: compatible");
                    ExitCode::SUCCESS
                }
                Err(errs) => report(errs),
            }
        }
        _ => {
            eprintln!(
                "ERROR: usage: silica-idlc check <file.idl>... | silica-idlc rust <out-dir> <file.idl>... | silica-idlc check-compat <old.idl> <new.idl>"
            );
            ExitCode::from(2)
        }
    }
}
