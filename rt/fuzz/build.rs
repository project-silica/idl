//! Generates the sample protocol (`protocols/sample/echo.idl`, which uses
//! every construct of the IDL) into `OUT_DIR` with the compiler itself, so
//! the fuzz target decodes exactly what generated code decodes.

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let src = dir.join("../../protocols/sample/echo.idl");
    println!("cargo::rerun-if-changed={}", src.display());
    let ps = idlc::load(&[src]).unwrap_or_else(|e| panic!("sample protocol does not compile: {e:?}"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    for p in &ps {
        let code = idlc::rustgen::rust(p).unwrap_or_else(|e| panic!("sample protocol does not generate: {e:?}"));
        std::fs::write(out.join(idlc::rustgen::file_name(p)), code).expect("write generated module");
    }
}
