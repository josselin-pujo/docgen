//! Wasm embedding surface for docgen: a WIT component exporting the whole
//! rendering pipeline (see `wit/docgen.wit`). The world imports nothing —
//! every engine is compiled in and works on in-memory buffers only, so the
//! component runs on any host that speaks the component model, WASI or not.
//!
//! Build (core module, zero imports):
//!
//! ```text
//! cargo build -p docgen-wasm --release --target wasm32-unknown-unknown
//! wasm-tools component new \
//!   target/wasm32-unknown-unknown/release/docgen_wasm.wasm \
//!   -o target/docgen.wasm
//! ```
//!
//! Mermaid's katex/math chain is the one thing that would drag host
//! imports (wasm-bindgen placeholders) into the module, so this crate
//! builds `docgen-mermaid` with default features off; diagrams using
//! `$…$` math degrade here but work in the native CLI.

wit_bindgen::generate!({
    path: "wit",
    world: "docgen",
});

/// Deterministic entropy for wasm32-unknown-unknown, where getrandom has
/// no backend. Registration only takes effect on targets that would not
/// compile otherwise, so native builds keep the real OS RNG. Consumers
/// are typst's `lorem()` and unseeded roughr paths; docgen's own outputs
/// never depend on randomness.
fn deterministic_random(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
    let mut s = STATE.fetch_add(0x2545_F491_4F6C_DD1D, Ordering::Relaxed);
    for chunk in buf.chunks_mut(8) {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        chunk.copy_from_slice(&s.to_le_bytes()[..chunk.len()]);
    }
    Ok(())
}

getrandom::register_custom_getrandom!(deterministic_random);

use crate::exports::docgen::render::render::DiagramFormat;
use crate::exports::docgen::render::render::Guest;
use docgen_core::assets::render_diagrams_with;
use docgen_core::parse_document;
use docgen_excalidraw::ExcalidrawRenderer;
use docgen_mermaid::MermaidRenderer;
use docgen_typst::TypstRenderer;

struct Component;

/// Panics become traps at the WIT boundary; catching them here turns them
/// into ordinary error strings instead.
fn guard<T>(
    f: impl FnOnce() -> docgen_core::Result<T> + std::panic::UnwindSafe,
) -> Result<T, String> {
    std::panic::catch_unwind(f)
        .map_err(|p| {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic inside docgen".to_string());
            format!("docgen panicked: {msg}")
        })
        .and_then(|r| r.map_err(|e| e.to_string()))
}

fn vector_renderers() -> (MermaidRenderer, ExcalidrawRenderer) {
    (MermaidRenderer::new(), ExcalidrawRenderer::new())
}

fn raster_renderers() -> (MermaidRenderer, ExcalidrawRenderer) {
    (MermaidRenderer::raster(), ExcalidrawRenderer::raster())
}

impl Guest for Component {
    fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    fn render_pdf(markdown: String) -> Result<Vec<u8>, String> {
        guard(|| {
            let doc = parse_document(&markdown)?;
            let (m, e) = vector_renderers();
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            let (pdf, _warnings) =
                TypstRenderer::new_embedded().render_pdf(&doc, &outcome.assets)?;
            Ok(pdf)
        })
    }

    fn render_docx(markdown: String) -> Result<Vec<u8>, String> {
        guard(|| {
            let doc = parse_document(&markdown)?;
            let (m, e) = raster_renderers();
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            let bytes = docgen_office::docx::render_docx(&doc, &outcome.assets)
                .map_err(|e| docgen_core::Error::Backend(e.to_string()))?;
            docgen_office::docx::validate_docx(&bytes)?;
            Ok(bytes)
        })
    }

    fn render_pptx(markdown: String) -> Result<Vec<u8>, String> {
        guard(|| {
            let doc = parse_document(&markdown)?;
            let (m, e) = raster_renderers();
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            let bytes = docgen_office::pptx::render_pptx(&doc, &outcome.assets)
                .map_err(|e| docgen_core::Error::Backend(e.to_string()))?;
            docgen_office::pptx::validate_pptx(&bytes)?;
            Ok(bytes)
        })
    }

    fn render_diagram_svg(source: String, format: DiagramFormat) -> Result<String, String> {
        guard(|| match format {
            DiagramFormat::Mermaid => MermaidRenderer::new().render_svg(&source),
            DiagramFormat::Excalidraw => docgen_excalidraw::render_source_svg(&source),
        })
    }
}

export!(Component);
