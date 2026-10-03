//! Wasm embedding surface for the docgen libraries.
//!
//! Today this is a **probe**: a C-ABI entry point that runs the full
//! parse → diagram-render → typst-compile pipeline in memory, so we can
//! build a cdylib per wasm target and inspect which host functions the
//! module actually imports (answer: how much of WASI, if any, is needed).
//!
//! The real surface (M5) is a WIT component — see the workspace README.

use docgen_core::assets::render_diagrams;
use docgen_core::parse_document;
use docgen_mermaid::MermaidRenderer;
use docgen_typst::TypstRenderer;

/// Renders `markdown` (with a mermaid block, if any) to PDF, entirely in
/// memory. Returns the PDF length, or 0 on failure. The bytes stay inside the
/// module — this exists to force-link the whole pipeline into the cdylib.
#[unsafe(no_mangle)]
pub extern "C" fn docgen_probe_pdf_len(markdown: *const u8, len: usize) -> usize {
    let bytes = unsafe { std::slice::from_raw_parts(markdown, len) };
    let markdown = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let doc = match parse_document(markdown) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    let assets = match render_diagrams(&doc, &MermaidRenderer::new()) {
        Ok(a) => a.assets,
        Err(_) => return 0,
    };
    match TypstRenderer::new_embedded().render_pdf(&doc, &assets) {
        Ok((pdf, _)) => pdf.len(),
        Err(_) => 0,
    }
}
