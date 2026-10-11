//! Rendered-asset plumbing: how diagram blocks become bytes without any
//! backend knowing about the others.
//!
//! `docgen-core` defines the [`DiagramRenderer`] trait; `docgen-mermaid`
//! (and later `docgen-excalidraw`) implement it; output backends consume the
//! resulting [`AssetMap`] keyed by block id.
//!
//! The same map also carries **inline images**: keys are the raw markdown
//! URLs, resolved to bytes by the caller (the CLI reads files relative to
//! the source; the plugin broker harvests workspace files into blob refs).

use std::collections::BTreeMap;

use crate::model::{DiagramFormat, Document};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetFormat {
    Svg,
    Png,
    Jpeg,
    Gif,
}

impl AssetFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "svg" => Some(Self::Svg),
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            _ => None,
        }
    }

    /// Sniffs the format from leading bytes — the four the renderers embed
    /// (PNG/JPEG/GIF magic; SVG via its XML prolog or root element).
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            return Some(Self::Png);
        }
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some(Self::Jpeg);
        }
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return Some(Self::Gif);
        }
        let head = &bytes[..bytes.len().min(512)];
        if let Ok(text) = std::str::from_utf8(head) {
            let t = text.trim_start_matches('\u{feff}').trim_start();
            if t.starts_with("<svg") || (t.starts_with("<?xml") && t.contains("<svg")) {
                return Some(Self::Svg);
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct RenderedAsset {
    pub format: AssetFormat,
    pub bytes: Vec<u8>,
}

/// name -> rendered bytes. Diagram blocks key by **block id**; inline
/// images key by the **raw markdown URL** as written by the author.
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

/// Pixel dimensions from the image header, for callers that need to size a
/// raster (the office writers); `None` when the format has no fixed pixel
/// size (SVG) or the header is malformed.
pub fn image_dimensions(bytes: &[u8], format: AssetFormat) -> Option<(u32, u32)> {
    match format {
        AssetFormat::Png => png_dimensions(bytes),
        AssetFormat::Jpeg => jpeg_dimensions(bytes),
        AssetFormat::Gif => gif_dimensions(bytes),
        AssetFormat::Svg => None,
    }
}

/// Width/height from the PNG IHDR chunk (first chunk after the signature).
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((width, height))
}

/// Width/height from the first JPEG SOF marker.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 8 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        i += 2;
        match marker {
            0x01 | 0xD0..=0xD7 => continue, // standalone markers
            0xD9 => return None,            // end of image
            _ => {}
        }
        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if len < 2 {
            return None;
        }
        // SOF0..SOF3, SOF5..SOF7, SOF9..SOF11, SOF13..SOF15 carry the frame
        // size; segment layout: len(2) precision(1) height(2) width(2) ...
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
            && i + 7 <= bytes.len()
        {
            let height = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            return Some((width, height));
        }
        i += len;
    }
    None
}

/// Width/height from the GIF screen descriptor.
fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || (!bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
    let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1x1 PNG header (signature + IHDR with dimensions; enough to sniff).
    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, // signature
        0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D', b'R', // length + type
        0x00, 0x00, 0x00, 0x2A, // width 42
        0x00, 0x00, 0x00, 0x2A, // height 42
    ];

    #[test]
    fn sniffs_formats_from_bytes() {
        assert_eq!(AssetFormat::from_bytes(PNG), Some(AssetFormat::Png));
        assert_eq!(
            AssetFormat::from_bytes(b"GIF89a\x40\x01\xf0\x00"),
            Some(AssetFormat::Gif)
        );
        assert_eq!(
            AssetFormat::from_bytes(b"\xef\xbb\xbf<?xml version=\"1.0\"?><svg xmlns=\"...\"/>"),
            Some(AssetFormat::Svg)
        );
        assert_eq!(AssetFormat::from_bytes(b"not an image at all"), None);
        assert_eq!(
            AssetFormat::from_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(AssetFormat::Jpeg)
        );
    }

    #[test]
    fn reads_dimensions_from_headers() {
        assert_eq!(image_dimensions(PNG, AssetFormat::Png), Some((42, 42)));
        assert_eq!(
            image_dimensions(b"GIF87a\x20\x03\x58\x02", AssetFormat::Gif),
            Some((800, 600))
        );
        // Minimal JPEG: SOI, one APP0 segment, then SOF0 carrying 200x100.
        let jpeg: &[u8] = &[
            0xFF, 0xD8, // SOI
            0xFF, 0xE0, 0x00, 0x10, // APP0, len 16
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x64, 0x00, 0xC8,
            0x00, // SOF0: 100 high, 200 wide
        ];
        assert_eq!(image_dimensions(jpeg, AssetFormat::Jpeg), Some((200, 100)));
        assert_eq!(image_dimensions(PNG, AssetFormat::Svg), None);
    }

    #[test]
    fn extensions_round_trip() {
        for (ext, format) in [
            ("svg", AssetFormat::Svg),
            ("png", AssetFormat::Png),
            ("jpg", AssetFormat::Jpeg),
            ("jpeg", AssetFormat::Jpeg),
            ("GIF", AssetFormat::Gif),
        ] {
            assert_eq!(AssetFormat::from_extension(ext), Some(format));
        }
        assert_eq!(AssetFormat::from_extension("bmp"), None);
    }
}
