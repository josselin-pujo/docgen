//! Artifact-driven regression guard for mermaid PNG label text in the
//! componentized module.
//!
//! Dogfooding a hosted Athena session rendered mermaid PNG exports with
//! shapes but no label text: on `wasm32-unknown-unknown` merman-export's
//! raster fontdb is empty (`load_system_fonts()` finds nothing), so usvg
//! silently dropped every glyph. The fix registers the typst-assets font set
//! into that database (see `docgen-mermaid::ensure_raster_fonts`). This test
//! pins the outcome end-to-end through the real component.
//!
//! Skips (with a note) unless the component has been componentized first
//! (plugin `build.sh`); same artifact-driven discipline as `imports.rs`.
//!
//! Run: `cargo test -p docgen-athena --test mermaid_png_text`

use wasmtime::component::{Component, Linker};

wasmtime::component::bindgen!({
    path: "wit",
    world: "athena-invoke",
});

/// The dogfood-session flowchart. Deterministic output: the text-free
/// (pre-fix) render of exactly this source was 6,725 bytes in both debug and
/// release-LTO builds, so a healthy margin above that proves glyphs are back
/// without pinning the exact layout.
const SOURCE: &str = "flowchart LR\n  A[Markdown + YAML] --> B[docgen core]\n  B --> C[Figures &amp; captions]\n  B --> D[Page control]\n  B --> E[TOC &amp; links]\n  C --> F[(PDF · DOCX · PPTX)]\n  D --> F\n  E --> F\n";

/// Below this, a 7-node flowchart cannot carry its label glyphs.
const TEXT_PRESENT_FLOOR_BYTES: usize = 9_000;

#[test]
fn component_mermaid_png_carries_label_text() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/docgen-athena.component.wasm");
    let Ok(component_bytes) = std::fs::read(&path) else {
        eprintln!(
            "skipping: {} not built yet (plugin build.sh componentizes it)",
            path.display()
        );
        return;
    };

    let engine = wasmtime::Engine::default();
    let component =
        Component::from_binary(&engine, &component_bytes).expect("component should deserialize");
    let mut store = wasmtime::Store::new(&engine, ());
    let bindings = AthenaInvoke::instantiate(&mut store, &component, &Linker::new(&engine))
        .expect("component should instantiate");

    let input = InvokeInput {
        action: "diagram".to_string(),
        format: "png".to_string(),
        kind: "mermaid".to_string(),
        content: Blob {
            ref_: String::new(),
            data: SOURCE.as_bytes().to_vec(),
        },
        assets: vec![],
    };
    let out = bindings
        .call_run(&mut store, &input)
        .expect("invoke should not trap")
        .expect("diagram png should not be a tool error");

    assert_eq!(out.artifact.data.first(), Some(&0x89), "not a PNG");
    assert!(
        out.artifact.data.len() >= TEXT_PRESENT_FLOOR_BYTES,
        "mermaid PNG is {} bytes — below the label-text floor of {TEXT_PRESENT_FLOOR_BYTES}. \
         Did the raster font registration regress (empty fontdb drops all glyphs)?",
        out.artifact.data.len()
    );
}
