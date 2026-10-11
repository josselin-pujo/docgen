//! docgen as an Athena tool component: package `athena:tool@0.1.0`, world
//! `athena-invoke` — one `run` export, zero imports.
//!
//! The source rides in as a host-filled blob holder (the broker derefs a
//! workspace file or blob-puts inline content), the artifact rides out as
//! a host-minted holder (the broker materializes it with `fs.writeBlob`).
//! Argument validation and fs delivery live in the broker; this module is
//! pure compute.
//!
//! Build (core module, zero imports — same recipe as `docgen-wasm`):
//!
//! ```text
//! cargo build -p docgen-athena --release --target wasm32-unknown-unknown
//! wasm-tools component new \
//!   target/wasm32-unknown-unknown/release/docgen_athena.wasm \
//!   -o target/docgen-athena.component.wasm
//! ```

// The generated ABI wrappers flatten the 4-field input record into ~10
// positional arguments — that is wit-bindgen's calling convention, not
// something to tidy here.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "wit",
    world: "athena-invoke",
});

/// Deterministic entropy for wasm32-unknown-unknown, where getrandom has
/// no backend (same stub as `docgen-wasm`; consumers are typst's `lorem()`
/// and unseeded roughr paths — docgen's own outputs never depend on it).
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

use docgen_core::assets::render_diagrams_with;
use docgen_core::parse_document;
use docgen_excalidraw::ExcalidrawRenderer;
use docgen_mermaid::MermaidRenderer;
use docgen_typst::TypstRenderer;

struct Component;

fn tool_err(message: String, code: &str) -> ToolError {
    ToolError {
        message,
        code: Some(code.to_string()),
    }
}

fn bad_args(message: String) -> ToolError {
    tool_err(message, "bad-args")
}

/// Panics become traps at the WIT boundary — and a trap quarantines the
/// plugin — so nothing below may panic: catch and convert to a tool error
/// (same discipline as `docgen-wasm`).
fn guard<T>(
    f: impl FnOnce() -> docgen_core::Result<T> + std::panic::UnwindSafe,
) -> Result<T, ToolError> {
    std::panic::catch_unwind(f)
        .map_err(|p| {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic inside docgen".to_string());
            tool_err(format!("docgen panicked: {msg}"), "panic")
        })
        .and_then(|r| r.map_err(|e| tool_err(e.to_string(), "render-failed")))
}

fn render_document(source: &str, format: &str) -> docgen_core::Result<Vec<u8>> {
    let doc = parse_document(source)?;
    match format {
        "pdf" => {
            let (m, e) = (MermaidRenderer::new(), ExcalidrawRenderer::new());
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            let (pdf, _warnings) =
                TypstRenderer::new_embedded().render_pdf(&doc, &outcome.assets)?;
            Ok(pdf)
        }
        "docx" => {
            let (m, e) = (MermaidRenderer::raster(), ExcalidrawRenderer::raster());
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            let bytes = docgen_office::docx::render_docx(&doc, &outcome.assets)
                .map_err(|e| docgen_core::Error::Backend(e.to_string()))?;
            docgen_office::docx::validate_docx(&bytes)?;
            Ok(bytes)
        }
        "pptx" => {
            let (m, e) = (MermaidRenderer::raster(), ExcalidrawRenderer::raster());
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            let bytes = docgen_office::pptx::render_pptx(&doc, &outcome.assets)
                .map_err(|e| docgen_core::Error::Backend(e.to_string()))?;
            docgen_office::pptx::validate_pptx(&bytes)?;
            Ok(bytes)
        }
        // `run` validates the pair before calling; keep an honest error
        // rather than unreachable!/panic (a trap would quarantine).
        other => Err(docgen_core::Error::Backend(format!(
            "unsupported render format {other:?} (use pdf, docx, pptx)"
        ))),
    }
}

fn render_diagram(source: &str, kind: &str, format: &str) -> docgen_core::Result<Vec<u8>> {
    match (kind, format) {
        ("mermaid", "svg") => Ok(MermaidRenderer::new().render_svg(source)?.into_bytes()),
        ("mermaid", "png") => MermaidRenderer::new().render_png(source),
        ("excalidraw", "svg") => Ok(docgen_excalidraw::render_source_svg(source)?.into_bytes()),
        ("excalidraw", "png") => docgen_excalidraw::render_source_png(source),
        other => Err(docgen_core::Error::Backend(format!(
            "unsupported diagram kind/format {other:?} \
             (kind: mermaid, excalidraw; format: svg, png)"
        ))),
    }
}

impl Guest for Component {
    fn run(input: InvokeInput) -> Result<InvokeOutput, ToolError> {
        // Model-facing args are validated before any compute: these are
        // bad-args refusals, not render failures.
        match (input.action.as_str(), input.format.as_str()) {
            ("render", "pdf" | "docx" | "pptx") | ("diagram", "svg" | "png") => {}
            ("render", other) => {
                return Err(bad_args(format!(
                    "render: unsupported format {other:?} (use pdf, docx, pptx)"
                )));
            }
            ("diagram", other) => {
                return Err(bad_args(format!(
                    "diagram: unsupported format {other:?} (use svg, png)"
                )));
            }
            (other, _) => {
                return Err(bad_args(format!(
                    "unknown action {other:?} (use render or diagram)"
                )));
            }
        }
        if input.action == "diagram" && !matches!(input.kind.as_str(), "mermaid" | "excalidraw") {
            return Err(bad_args(format!(
                "diagram: unknown kind {:?} (use mermaid or excalidraw)",
                input.kind
            )));
        }
        let source = String::from_utf8(input.content.data)
            .map_err(|_| bad_args("source is not valid UTF-8".to_string()))?;
        let action = input.action;
        let format = input.format;
        let kind = input.kind;
        let bytes = guard(move || match action.as_str() {
            "render" => render_document(&source, &format),
            _ => render_diagram(&source, &kind, &format),
        })?;
        Ok(InvokeOutput {
            artifact: Blob {
                ref_: String::new(),
                data: bytes,
            },
        })
    }
}

export!(Component);
