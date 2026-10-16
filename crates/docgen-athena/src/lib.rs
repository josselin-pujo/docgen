//! docgen as an Athena tool component: package `athena:tool@0.1.0`, world
//! `athena-invoke` — one `run` export, zero imports.
//!
//! The source rides in as a host-filled blob holder (the broker derefs a
//! workspace file or blob-puts inline content), image assets ride in as
//! named holder lists, and the artifact rides out as a host-minted holder.
//! Argument validation and delivery live in the brokers; this module is
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

use docgen_core::assets::{AssetFormat, AssetMap, RenderedAsset, render_diagrams_with};
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

fn render_document(
    source: &str,
    format: &str,
    mut assets: AssetMap,
) -> docgen_core::Result<(Vec<u8>, Vec<String>)> {
    let doc = parse_document(source)?;
    let mut warnings: Vec<String> = Vec::new();
    for url in doc.image_urls() {
        if !assets.contains_key(&url) {
            warnings.push(format!(
                "image {url}: no bytes provided — rendered as a visible placeholder"
            ));
        }
    }
    match format {
        "pdf" => {
            let (m, e) = (MermaidRenderer::new(), ExcalidrawRenderer::new());
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            warnings.extend(skipped_warnings(&outcome.skipped));
            assets.extend(outcome.assets);
            let (pdf, typst_warnings) = TypstRenderer::new_embedded().render_pdf(&doc, &assets)?;
            warnings.extend(typst_warnings);
            Ok((pdf, warnings))
        }
        "docx" => {
            let (m, e) = (MermaidRenderer::raster(), ExcalidrawRenderer::raster());
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            warnings.extend(skipped_warnings(&outcome.skipped));
            assets.extend(outcome.assets);
            let bytes = docgen_office::docx::render_docx(&doc, &assets)
                .map_err(|e| docgen_core::Error::Backend(e.to_string()))?;
            docgen_office::docx::validate_docx(&bytes)?;
            Ok((bytes, warnings))
        }
        "pptx" => {
            let (m, e) = (MermaidRenderer::raster(), ExcalidrawRenderer::raster());
            let outcome = render_diagrams_with(&doc, &[&m, &e])?;
            warnings.extend(skipped_warnings(&outcome.skipped));
            assets.extend(outcome.assets);
            // pptx is frozen: page/media directives are ignored, loudly
            // (same warnings the CLI surfaces).
            warnings.extend(docgen_office::pptx::unsupported_directive_warnings(&doc));
            let bytes = docgen_office::pptx::render_pptx(&doc, &assets)
                .map_err(|e| docgen_core::Error::Backend(e.to_string()))?;
            docgen_office::pptx::validate_pptx(&bytes)?;
            Ok((bytes, warnings))
        }
        // `run` validates the pair before calling; keep an honest error
        // rather than unreachable!/panic (a trap would quarantine).
        other => Err(docgen_core::Error::Backend(format!(
            "unsupported render format {other:?} (use pdf, docx, pptx)"
        ))),
    }
}

fn skipped_warnings(skipped: &[String]) -> Vec<String> {
    skipped
        .iter()
        .map(|s| format!("diagram {s}: no renderer available — placeholder emitted"))
        .collect()
}

fn render_diagram(
    source: &str,
    kind: &str,
    format: &str,
) -> docgen_core::Result<(Vec<u8>, Vec<String>)> {
    // Empty kind: infer exactly like the CLI (excalidraw JSON vs mermaid).
    let kind = if kind.is_empty() {
        detect_kind(source)
    } else {
        kind
    };
    let bytes = match (kind, format) {
        ("mermaid", "svg") => MermaidRenderer::new()
            .render_svg(source)
            .map(String::into_bytes),
        ("mermaid", "png") => MermaidRenderer::new().render_png(source),
        ("excalidraw", "svg") => {
            docgen_excalidraw::render_source_svg(source).map(String::into_bytes)
        }
        ("excalidraw", "png") => docgen_excalidraw::render_source_png(source),
        other => Err(docgen_core::Error::Backend(format!(
            "unsupported diagram kind/format {other:?} \
             (kind: mermaid, excalidraw; format: svg, png)"
        ))),
    }?;
    Ok((bytes, Vec::new()))
}

/// Mirrors the CLI's detection: JSON objects mentioning `elements` (or an
/// excalidraw `type`) are excalidraw; everything else is mermaid.
fn detect_kind(source: &str) -> &'static str {
    let trimmed = source.trim_start();
    if trimmed.starts_with('{')
        && (source.contains("\"elements\"") || source.contains("\"type\": \"excalidraw\""))
    {
        "excalidraw"
    } else {
        "mermaid"
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
        if input.action == "diagram"
            && !matches!(input.kind.as_str(), "" | "mermaid" | "excalidraw")
        {
            return Err(bad_args(format!(
                "diagram: unknown kind {:?} (use mermaid or excalidraw, or omit to infer)",
                input.kind
            )));
        }
        let source = String::from_utf8(input.content.data)
            .map_err(|_| bad_args("source is not valid UTF-8".to_string()))?;
        let mut assets = AssetMap::new();
        for asset in &input.assets {
            let format = AssetFormat::from_bytes(&asset.image.data).ok_or_else(|| {
                bad_args(format!(
                    "asset {:?}: unrecognized image format (png/jpeg/gif/svg)",
                    asset.name
                ))
            })?;
            assets.insert(
                asset.name.clone(),
                RenderedAsset {
                    format,
                    bytes: asset.image.data.clone(),
                },
            );
        }
        let action = input.action;
        let format = input.format;
        let kind = input.kind;
        let (bytes, warnings) = guard(move || match action.as_str() {
            "render" => render_document(&source, &format, assets),
            _ => render_diagram(&source, &kind, &format),
        })?;
        Ok(InvokeOutput {
            artifact: Blob {
                ref_: String::new(),
                data: bytes,
            },
            warnings,
        })
    }
}

export!(Component);
