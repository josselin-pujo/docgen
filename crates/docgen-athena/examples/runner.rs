//! Host-side smoke runner: loads the componentized module and calls `run`
//! end-to-end through wasmtime. No WASI — the component imports nothing.
//!
//! Usage (expects `target/docgen-athena.component.wasm`, see `src/lib.rs`):
//!
//! ```text
//! cargo run -p docgen-athena --example runner
//! ```

use wasmtime::component::{Component, Linker};

wasmtime::component::bindgen!({
    path: "wit",
    world: "athena-invoke",
});

fn call(
    bindings: &AthenaInvoke,
    store: &mut wasmtime::Store<()>,
    action: &str,
    format: &str,
    kind: &str,
    source: &str,
) -> anyhow::Result<InvokeOutput> {
    let input = InvokeInput {
        action: action.to_string(),
        format: format.to_string(),
        kind: kind.to_string(),
        content: Blob {
            ref_: String::new(),
            data: source.as_bytes().to_vec(),
        },
    };
    bindings
        .call_run(store, &input)?
        .map_err(|e| anyhow::anyhow!("tool-error: {e:?}"))
}

fn main() -> anyhow::Result<()> {
    let engine = wasmtime::Engine::default();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/docgen-athena.component.wasm");
    let component = Component::from_file(&engine, &path)?;
    let mut store = wasmtime::Store::new(&engine, ());
    let bindings = AthenaInvoke::instantiate(&mut store, &component, &Linker::new(&engine))?;

    let pdf = call(
        &bindings,
        &mut store,
        "render",
        "pdf",
        "",
        "# Hello *component*\n\n- a\n- b\n",
    )?;
    anyhow::ensure!(pdf.artifact.data.starts_with(b"%PDF"), "not a PDF");
    println!("render/pdf: {} bytes, %PDF ok", pdf.artifact.data.len());

    let docx = call(
        &bindings,
        &mut store,
        "render",
        "docx",
        "",
        "# Hello\n\nworld",
    )?;
    anyhow::ensure!(docx.artifact.data.starts_with(b"PK"), "not a zip");
    println!("render/docx: {} bytes, zip ok", docx.artifact.data.len());

    let pptx = call(
        &bindings,
        &mut store,
        "render",
        "pptx",
        "",
        "# Deck\n\n- one",
    )?;
    anyhow::ensure!(pptx.artifact.data.starts_with(b"PK"), "not a zip");
    println!("render/pptx: {} bytes, zip ok", pptx.artifact.data.len());

    let svg = call(
        &bindings,
        &mut store,
        "diagram",
        "svg",
        "mermaid",
        "flowchart LR\n  A-->B",
    )?;
    let svg_text = String::from_utf8(svg.artifact.data)?;
    anyhow::ensure!(svg_text.contains("<svg"), "not an SVG");
    println!("diagram/svg (mermaid): {} bytes", svg_text.len());

    let png = call(
        &bindings,
        &mut store,
        "diagram",
        "png",
        "mermaid",
        "flowchart LR\n  A-->B",
    )?;
    anyhow::ensure!(
        png.artifact.data.starts_with(&[0x89, b'P', b'N', b'G']),
        "not a PNG"
    );
    println!("diagram/png (mermaid): {} bytes", png.artifact.data.len());

    // The bad-args arm must come back as a tool-error, never a trap.
    let err = call(&bindings, &mut store, "paint", "pdf", "", "x");
    let err = err.expect_err("bad action must not succeed");
    println!("bad action refused: {err}");

    println!("component smoke run: ALL OK");
    Ok(())
}
