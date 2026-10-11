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

/// Minimal valid 1x1 white PNG (decodes in real decoders).
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0x0F, 0x04, 0x00,
    0x09, 0xFB, 0x03, 0xFD, 0xFB, 0x5E, 0x6B, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

fn call(
    bindings: &AthenaInvoke,
    store: &mut wasmtime::Store<()>,
    action: &str,
    format: &str,
    kind: &str,
    source: &str,
    assets: Vec<(&str, Vec<u8>)>,
) -> anyhow::Result<InvokeOutput> {
    let input = InvokeInput {
        action: action.to_string(),
        format: format.to_string(),
        kind: kind.to_string(),
        content: Blob {
            ref_: String::new(),
            data: source.as_bytes().to_vec(),
        },
        assets: assets
            .into_iter()
            .map(|(name, data)| Asset {
                name: name.to_string(),
                image: Blob {
                    ref_: String::new(),
                    data,
                },
            })
            .collect(),
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
        vec![],
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
        vec![],
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
        vec![],
    )?;
    anyhow::ensure!(pptx.artifact.data.starts_with(b"PK"), "not a zip");
    println!("render/pptx: {} bytes, zip ok", pptx.artifact.data.len());

    // Image assets ride the assets channel (name = the markdown URL).
    let with_image = call(
        &bindings,
        &mut store,
        "render",
        "pdf",
        "",
        "# Pic\n\n![pixel](img/pixel.png)\n",
        vec![("img/pixel.png", TINY_PNG.to_vec())],
    )?;
    anyhow::ensure!(with_image.artifact.data.starts_with(b"%PDF"), "not a PDF");
    anyhow::ensure!(
        with_image.warnings.is_empty(),
        "unexpected warnings: {:?}",
        with_image.warnings
    );
    println!(
        "render/pdf with image: {} bytes, no warnings",
        with_image.artifact.data.len()
    );

    // Missing image bytes: a warning; never a trap.
    let missing = call(
        &bindings,
        &mut store,
        "render",
        "pdf",
        "",
        "# Pic\n\n![gone](img/gone.png)\n",
        vec![],
    )?;
    anyhow::ensure!(
        missing.warnings.iter().any(|w| w.contains("img/gone.png")),
        "expected a missing-image warning, got {:?}",
        missing.warnings
    );
    println!("missing image warned: {}", missing.warnings[0]);

    let svg = call(
        &bindings,
        &mut store,
        "diagram",
        "svg",
        "mermaid",
        "flowchart LR\n  A-->B",
        vec![],
    )?;
    let svg_text = String::from_utf8(svg.artifact.data)?;
    anyhow::ensure!(svg_text.contains("<svg"), "not an SVG");
    println!("diagram/svg (mermaid): {} bytes", svg_text.len());

    // Empty kind: inferred (mermaid here).
    let auto = call(
        &bindings,
        &mut store,
        "diagram",
        "svg",
        "",
        "flowchart LR\n  A-->B",
        vec![],
    )?;
    anyhow::ensure!(
        String::from_utf8(auto.artifact.data)?.contains("<svg"),
        "auto-detect should pick mermaid"
    );
    println!("diagram/svg (kind inferred): ok");

    let png = call(
        &bindings,
        &mut store,
        "diagram",
        "png",
        "mermaid",
        "flowchart LR\n  A-->B",
        vec![],
    )?;
    anyhow::ensure!(
        png.artifact.data.starts_with(&[0x89, b'P', b'N', b'G']),
        "not a PNG"
    );
    println!("diagram/png (mermaid): {} bytes", png.artifact.data.len());

    // The bad-args arm must come back as a tool-error, never a trap.
    let err = call(&bindings, &mut store, "paint", "pdf", "", "x", vec![]);
    let err = err.expect_err("bad action must not succeed");
    println!("bad action refused: {err}");

    // An unrecognized asset must be a bad-args refusal, not a crash.
    let err = call(
        &bindings,
        &mut store,
        "render",
        "pdf",
        "",
        "![x](x.bin)",
        vec![("x.bin", b"not an image".to_vec())],
    );
    let err = err.expect_err("bad asset must not succeed");
    println!("bad asset refused: {err}");

    println!("component smoke run: ALL OK");
    Ok(())
}
