//! Rendered-asset plumbing: how diagram blocks become bytes without any
//! backend knowing about the others.
//!
//! `docgen-core` defines the [`DiagramRenderer`] trait; `docgen-mermaid`
//! (and later `docgen-excalidraw`) implement it; output backends consume the
//! resulting [`AssetMap`] keyed by block id.

use std::collections::BTreeMap;

use crate::model::{DiagramFormat, Document};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetFormat {
    Svg,
    Png,
}

impl AssetFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "svg" => Some(Self::Svg),
            "png" => Some(Self::Png),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderedAsset {
    pub format: AssetFormat,
    pub bytes: Vec<u8>,
}

/// block id -> rendered bytes.
pub type AssetMap = BTreeMap<String, RenderedAsset>;

/// Turns diagram source into a rendered asset. Implemented by engine crates
/// (`docgen-mermaid`, later `docgen-excalidraw`); everything IO-shaped stays
/// out of this interface so it works identically native and under wasm.
pub trait DiagramRenderer {
    fn render(&self, block_id: &str, format: DiagramFormat, source: &str) -> Result<RenderedAsset>;

    /// Human-readable name for error messages and CLI logging.
    fn name(&self) -> &'static str;
}

/// Outcome of rendering every diagram block in a document.
pub struct DiagramOutcome {
    pub assets: AssetMap,
    /// Block ids (with format) no registered renderer could handle; output
    /// backends should emit a visible placeholder for these.
    pub skipped: Vec<String>,
}

/// Renders every diagram block in `doc` through `renderer`.
///
/// Hard renderer failures (invalid mermaid source, ...) abort; a missing
/// engine for a format only skips the block so partial toolchains still
/// produce output.
pub fn render_diagrams(doc: &Document, renderer: &dyn DiagramRenderer) -> Result<DiagramOutcome> {
    let mut outcome = DiagramOutcome {
        assets: AssetMap::new(),
        skipped: Vec::new(),
    };

    let mut failures: Vec<String> = Vec::new();
    for block in &doc.blocks {
        if let crate::model::BlockKind::Diagram { format, source } = &block.kind {
            match renderer.render(&block.id, *format, source) {
                Ok(asset) => {
                    outcome.assets.insert(block.id.clone(), asset);
                }
                Err(Error::Diagram(msg)) if msg.starts_with("unsupported") => {
                    outcome
                        .skipped
                        .push(format!("{} ({})", block.id, format.as_str()));
                }
                Err(e) => failures.push(format!("{}: {e}", block.id)),
            }
        }
    }

    if !failures.is_empty() {
        return Err(Error::Diagram(format!(
            "diagram rendering failed for: {}",
            failures.join(", ")
        )));
    }
    Ok(outcome)
}

/// Renders every diagram block through a chain of renderers: the first
/// one that claims the block's format wins. A block every renderer
/// declines is skipped (visible placeholder downstream); a renderer that
/// claims a format but fails on the source aborts.
pub fn render_diagrams_with(
    doc: &Document,
    renderers: &[&dyn DiagramRenderer],
) -> Result<DiagramOutcome> {
    let mut outcome = DiagramOutcome {
        assets: AssetMap::new(),
        skipped: Vec::new(),
    };

    let mut failures: Vec<String> = Vec::new();
    for block in &doc.blocks {
        if let crate::model::BlockKind::Diagram { format, source } = &block.kind {
            let mut handled = false;
            let mut last_err = None;
            for renderer in renderers {
                match renderer.render(&block.id, *format, source) {
                    Ok(asset) => {
                        outcome.assets.insert(block.id.clone(), asset);
                        handled = true;
                        break;
                    }
                    Err(Error::Diagram(msg)) if msg.starts_with("unsupported") => continue,
                    Err(e) => {
                        last_err = Some(e);
                        break;
                    }
                }
            }
            if !handled {
                if let Some(e) = last_err {
                    failures.push(format!("{}: {e}", block.id));
                } else {
                    outcome
                        .skipped
                        .push(format!("{} ({})", block.id, format.as_str()));
                }
            }
        }
    }

    if !failures.is_empty() {
        return Err(Error::Diagram(format!(
            "diagram rendering failed for: {}",
            failures.join(", ")
        )));
    }
    Ok(outcome)
}
