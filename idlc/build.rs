//! Generates the Rust module of every protocol under `../protocols` into
//! `OUT_DIR`, so the tests compile and exercise generated code
//! (`tests/round_trip.rs`), and of every version of the skew fixture under
//! `tests/versions/<v>` into `OUT_DIR/versions/<v>` (`tests/versioning.rs`),
//! and of the transport fixture `tests/transport` into `OUT_DIR/transport`
//! (`tests/transport.rs`).
//! Uses the compiler's own sources.

#[path = "src/lib.rs"]
#[allow(dead_code, unused_imports)]
mod idlc;

use std::path::{Path, PathBuf};

fn idl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            idl_files(&p, out);
        } else if p.extension().is_some_and(|x| x == idlc::EXTENSION) {
            out.push(p);
        }
    }
}

/// Compiles every protocol under `dir` and writes its Rust module to `out`.
fn generate(dir: &Path, out: &Path) {
    println!("cargo::rerun-if-changed={}", dir.display());
    let mut files = Vec::new();
    idl_files(dir, &mut files);
    for f in &files {
        println!("cargo::rerun-if-changed={}", f.display());
    }
    let ps = idlc::load(&files).unwrap_or_else(|errs| {
        for e in errs {
            println!("cargo::warning=ERROR: {e}");
        }
        panic!("protocols do not compile");
    });
    std::fs::create_dir_all(out).expect("create output directory");
    for p in &ps {
        let code = idlc::rustgen::rust(p).unwrap_or_else(|errs| {
            for e in errs {
                println!("cargo::warning=ERROR: {e}");
            }
            panic!("protocol {} does not generate", p.name);
        });
        std::fs::write(out.join(idlc::rustgen::file_name(p)), code)
            .expect("write generated module");
    }
}

fn main() {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    println!("cargo::rerun-if-changed=src");
    generate(&manifest.join("..").join(idlc::PROTOCOL_DIR), &out);
    generate(
        &manifest.join("tests").join("transport"),
        &out.join("transport"),
    );
    for v in ["v1", "v2"] {
        generate(
            &manifest.join("tests").join("versions").join(v),
            &out.join("versions").join(v),
        );
    }
}
