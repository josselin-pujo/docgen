//! Import-surface assertions on the built cdylib artifacts.
//!
//! These tests are artifact-driven: they skip (with a note) unless the
//! wasm has been built first. CI order is `cargo build --release
//! --target <t> -p docgen-wasm` then `cargo test -p docgen-wasm`.

use std::path::PathBuf;

fn artifact(target: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(target)
        .join("release/docgen_wasm.wasm");
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skipping: {} not built yet", path.display());
            None
        }
    }
}

/// (module, name) pairs the module imports.
fn imports(bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        if let Ok(wasmparser::Payload::ImportSection(reader)) = payload {
            for import in reader.into_imports().flatten() {
                out.push((import.module.to_string(), import.name.to_string()));
            }
        }
    }
    out
}

/// The unknown-unknown module must import NOTHING: the WIT world imports
/// nothing and mermaid's math chain (the historical source of the five
/// wasm-bindgen placeholders) is feature-gated off for this crate.
#[test]
fn unknown_unknown_has_zero_imports() {
    let Some(bytes) = artifact("wasm32-unknown-unknown") else {
        return;
    };
    let imports = imports(&bytes);
    assert!(
        imports.is_empty(),
        "wasm32-unknown-unknown module must import nothing, got: {imports:?}"
    );
}

/// The wasip1 module may only reference stub-satisfiable WASI preview1
/// functions (none of which docgen actually uses for IO).
#[test]
fn wasip1_imports_are_stub_satisfiable_wasi() {
    let Some(bytes) = artifact("wasm32-wasip1") else {
        return;
    };
    let allowed: &[&str] = &[
        "random_get",
        "environ_get",
        "environ_sizes_get",
        "clock_time_get",
        "fd_close",
        "fd_read",
        "fd_write",
        "fd_fdstat_get",
        "fd_filestat_get",
        "fd_prestat_get",
        "fd_prestat_dir_name",
        "path_open",
        "path_filestat_get",
        "proc_exit",
        "sched_yield",
    ];
    let imports = imports(&bytes);
    assert!(
        imports
            .iter()
            .all(|(module, name)| module == "wasi_snapshot_preview1"
                && allowed.contains(&name.as_str())),
        "unexpected imports beyond the stub-satisfiable WASI set: {imports:?}"
    );
}
