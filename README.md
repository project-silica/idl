# idl

Every Silica interface definition (`.idl`), the IDL compiler (`silica-idlc`)
and the generated-binding conformance tests. This is the stable contract of the
platform: protocols evolve additively (`spec/ARCHITECTURE.md` section 4).

Belongs here: protocol definitions, the compiler, its runtime support crate and
conformance tests. Does not belong here: protocol implementations (providers
live in their own repos), kernel code, anything depending on another Silica
repository. `idl` depends on nothing.

Build and test: `cargo test` (host only). License: MIT OR Apache-2.0.
