//! M0 scaffold check: the sample protocol the M0 exit test round-trips exists.
use std::path::Path;

#[test]
fn sample_protocol_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let sample = root
        .join(idlc::PROTOCOL_DIR)
        .join("sample")
        .join("echo.idl");
    assert!(sample.is_file(), "missing {}", sample.display());
}
