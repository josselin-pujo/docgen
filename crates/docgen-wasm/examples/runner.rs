//! Host-side smoke runner: loads the componentized module and calls every
//! export end-to-end. No WASI — the component imports nothing.
//!
//! Usage:
//! ```text
//! cargo build --release --target wasm32-unknown-unknown -p docgen-wasm
//! wasm-tools component new \
//!   target/wasm32-unknown-unknown/release/docgen_wasm.wasm -o target/docgen.wasm
//! cargo run -p docgen-wasm --example runner
//! ```

use wasmtime::component::{Component, Linker};

wasmtime::component::bindgen!({
    path: "wit",
    world: "docgen",
});

use exports::docgen::render::render::DiagramFormat;

fn main() -> anyhow::Result<()> {
    let engine = wasmtime::Engine::default();
    let component_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/docgen.wasm");
    let component = Component::from_file(&engine, &component_path)?;
    let mut store = wasmtime::Store::new(&engine, ());
    let bindings = Docgen::instantiate(&mut store, &component, &Linker::new(&engine))?;

    let render = bindings.docgen_render_render();

    println!("version: {}", render.call_version(&mut store)?);

    let pdf = render.call_render_pdf(&mut store, "# Hello *component*\n\n- a\n- b\n")?;
    let pdf = pdf.map_err(|e| anyhow::anyhow!("render-pdf failed: {e}"))?;
    anyhow::ensure!(pdf.starts_with(b"%PDF"), "not a PDF");
    println!("render-pdf: {} bytes, %PDF magic ok", pdf.len());

    let docx = render.call_render_docx(&mut store, "# Hello\n\nworld")?;
    let docx = docx.map_err(|e| anyhow::anyhow!("render-docx failed: {e}"))?;
    anyhow::ensure!(docx.starts_with(b"PK"), "not a zip");
    println!("render-docx: {} bytes, zip magic ok", docx.len());

    let pptx = render.call_render_pptx(&mut store, "# Deck\n\n- one")?;
    let pptx = pptx.map_err(|e| anyhow::anyhow!("render-pptx failed: {e}"))?;
    anyhow::ensure!(pptx.starts_with(b"PK"), "not a zip");
    println!("render-pptx: {} bytes, zip magic ok", pptx.len());

    let svg = render.call_render_diagram_svg(
        &mut store,
        "flowchart LR\n  A-->B",
        DiagramFormat::Mermaid,
    )?;
    let svg = svg.map_err(|e| anyhow::anyhow!("render-diagram-svg failed: {e}"))?;
    anyhow::ensure!(svg.contains("<svg"), "not an SVG");
    println!("render-diagram-svg (mermaid): {} bytes", svg.len());

    println!("component smoke run: ALL EXPORTS OK");
    Ok(())
}
