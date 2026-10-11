//! Import-surface assertion on the built cdylib (artifact-driven: skips
//! unless the wasm has been built first — CI order is `cargo build
//! --release --target wasm32-unknown-unknown -p docgen-athena` then
//! `cargo test -p docgen-athena`).
//!
//! The `athena-invoke` world imports nothing; the core module must too.

use std::path::PathBuf;

#[test]
fn unknown_unknown_has_zero_imports() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/wasm32-unknown-unknown/release/docgen_athena.wasm");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipping: {} not built yet", path.display());
        return;
    };
    let mut imports = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let Ok(wasmparser::Payload::ImportSection(reader)) = payload {
            for import in reader.into_imports().flatten() {
                imports.push((import.module.to_string(), import.name.to_string()));
            }
        }
    }
    assert!(
        imports.is_empty(),
        "docgen-athena must import nothing, got: {imports:?}"
    );
}
