//! The generated reference page says everything a client or provider needs:
//! every method with its ordinal, parameters, results and error, every type
//! with its values, the common failures, and the Rust names to use.
use std::path::Path;

fn page() -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../protocols/sample/echo.idl");
    let src = std::fs::read_to_string(&file).unwrap();
    let p = idlc::parse(&file, &src).unwrap();
    idlc::docgen::markdown(&p)
}

#[test]
fn every_method_type_and_failure_is_on_the_page() {
    let page = page();
    for want in [
        "# sample.Echo (version 1)",
        "### `echo` (ordinal 1)",
        "### `echo_records` (ordinal 2)",
        "### `echo_caps` (ordinal 3)",
        "Ordinal 4 is retired and never reused.",
        "| `records` | `array<Record>` |",
        "| `peer` | `option<endpoint<sample.Echo>>` |",
        "| `frame` | `cap<Frame>` |",
        "Fails with a value of `EchoError`",
        "### enum `EchoError` : `u32`",
        "| 2 | `too_many` | More records were given than the provider accepts. |",
        "### struct `Record`",
        "| `revoked` |",
        "| `unknown_method` |",
        "pub mod echo",
        "| `echo_records` | `client.echo_records(records)` | `EchoRecordsReply` | `EchoError` |",
        "| `echo` | `client.echo(data)` | `EchoReply` | `Infallible` |",
        "echo::dispatch(&mut server,",
        "pub struct EchoReply<'a> {",
        "pub trait Server<C: ::silica_idl_rt::Capability> {",
        "fn echo_caps<'a>(&'a mut self, frame: C, peer: ::core::option::Option<C>, any: C) -> EchoCapsReply<C>;",
        "pub fn echo<'s>(&'s mut self, data: &'_ [u8])",
        "`CallError::Error(e)` is the method's own error",
        "the request's capability list by value",
    ] {
        assert!(page.contains(want), "the page lacks: {want}\n{page}");
    }
}

#[test]
fn the_documentation_of_the_source_is_carried_over_verbatim() {
    let page = page();
    assert!(
        page.contains("Returns `records` unchanged. Fails with `empty` when no record is given.")
    );
    assert!(page.contains("A record with one field of every non-capability type."));
}
