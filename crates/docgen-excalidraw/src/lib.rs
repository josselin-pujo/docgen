//! docgen-excalidraw: native Excalidraw support for docgen.
//!
//! - [`parse_scene`] parses raw `.excalidraw` JSON (files or code blocks)
//!   into a permissive serde model that round-trips losslessly.
//! - [`render_svg`] draws it as a standalone SVG — hand-drawn geometry via
//!   [`roughr`](https://crates.io/crates/roughr) (seeded per element, so
//!   output is byte-deterministic), text shaped into SVG paths from
//!   bundled fonts, images embedded from their data URLs. With the
//!   `raster` feature, [`render_png`] rasterizes that SVG (no fontdb
//!   needed — text is already geometry).
//! - [`diff_scenes`] is the element-id structural diff used to summarize
//!   externally-edited scenes in review comments.
//!
//! No IO: strings and byte buffers only.

pub mod diff;
pub mod fonts;
pub mod model;
pub mod render;

pub use diff::{ElementChange, SceneDiff, diff_scenes};
pub use model::{AppState, Element, Scene, SceneFile};
pub use render::render_svg;

use docgen_core::assets::{AssetFormat, DiagramRenderer, RenderedAsset};
use docgen_core::model::DiagramFormat;
use docgen_core::{Error, Result};

#[derive(Debug, thiserror::Error)]
pub enum ExcalidrawError {
    #[error("invalid excalidraw scene: {0}")]
    Invalid(String),
}

impl From<ExcalidrawError> for Error {
    fn from(e: ExcalidrawError) -> Self {
        Error::Diagram(e.to_string())
    }
}

/// Parses `.excalidraw` JSON (accepts a full scene or a bare elements
/// array, as embedded in markdown code blocks).
pub fn parse_scene(source: &str) -> Result<Scene> {
    let trimmed = source.trim_start();
    if trimmed.starts_with('[') {
        let elements: Vec<Element> = serde_json::from_str(source)
            .map_err(|e| ExcalidrawError::Invalid(format!("elements array: {e}")))?;
        return Ok(Scene {
            elements,
            ..Default::default()
        });
    }
    serde_json::from_str(source).map_err(|e| ExcalidrawError::Invalid(e.to_string()).into())
}

/// Renders a scene source (raw JSON) directly to SVG.
pub fn render_source_svg(source: &str) -> Result<String> {
    let scene = parse_scene(source)?;
    render_svg(&scene)
}

/// Renders a scene source (raw JSON) directly to PNG (requires the
/// `raster` feature; used by the office backends).
#[cfg(feature = "raster")]
pub fn render_source_png(source: &str) -> Result<Vec<u8>> {
    let svg = render_source_svg(source)?;
    rasterize(&svg)
}

/// Rasterizes an SVG string to PNG bytes at 2x device scale.
#[cfg(feature = "raster")]
pub fn rasterize(svg: &str) -> Result<Vec<u8>> {
    use resvg::usvg::{self, Transform};
    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &opts)
        .map_err(|e| ExcalidrawError::Invalid(format!("svg parse: {e}")))?;
    let size = tree.size();
    let scale = 2.0f32;
    let w = (size.width() * scale).ceil().max(1.0) as u32;
    let h = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| ExcalidrawError::Invalid("pixmap alloc".into()))?;
    resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|e| ExcalidrawError::Invalid(format!("png encode: {e}")).into())
}

/// Renders `.excalidraw` scenes as assets. `raster` mode produces PNG for
/// the office backends; the default emits SVG for typst.
pub struct ExcalidrawRenderer {
    #[allow(dead_code)]
    raster: bool,
}

impl ExcalidrawRenderer {
    pub fn new() -> Self {
        Self { raster: false }
    }

    #[cfg(feature = "raster")]
    pub fn raster() -> Self {
        Self { raster: true }
    }

    /// Diagram sources are stored verbatim in the model; this renders
    /// them as SVG bytes.
    pub fn render_svg_bytes(&self, source: &str) -> Result<Vec<u8>> {
        Ok(render_source_svg(source)?.into_bytes())
    }
}

impl Default for ExcalidrawRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagramRenderer for ExcalidrawRenderer {
    fn render(
        &self,
        _block_id: &str,
        format: DiagramFormat,
        source: &str,
    ) -> Result<RenderedAsset> {
        match format {
            DiagramFormat::Excalidraw => {
                #[cfg(feature = "raster")]
                if self.raster {
                    return Ok(RenderedAsset {
                        format: AssetFormat::Png,
                        bytes: render_source_png(source)?,
                    });
                }
                Ok(RenderedAsset {
                    format: AssetFormat::Svg,
                    bytes: self.render_svg_bytes(source)?,
                })
            }
            DiagramFormat::Mermaid => Err(Error::Diagram(format!(
                "unsupported diagram format '{}' in excalidraw renderer",
                format.as_str()
            ))),
        }
    }

    fn name(&self) -> &'static str {
        "excalidraw"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r##"{
      "type": "excalidraw",
      "version": 2,
      "elements": [
        {"type": "rectangle", "id": "r1", "x": 10, "y": 20, "width": 120, "height": 60,
         "strokeColor": "#1e1e1e", "backgroundColor": "#a5d8ff", "fillStyle": "solid",
         "strokeWidth": 2, "roughness": 1, "opacity": 100, "seed": 42, "angle": 0,
         "roundness": null, "version": 1, "versionNonce": 1, "isDeleted": false},
        {"type": "arrow", "id": "a1", "x": 140, "y": 50, "width": 80, "height": 0,
         "points": [[0,0],[80,0]], "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
         "fillStyle": "hachure", "strokeWidth": 1, "strokeStyle": "solid", "roughness": 1,
         "opacity": 100, "seed": 7, "endArrowhead": "arrow", "startArrowhead": null,
         "angle": 0, "version": 1, "versionNonce": 2, "isDeleted": false},
        {"type": "ellipse", "id": "e1", "x": 240, "y": 20, "width": 90, "height": 60,
         "strokeColor": "#e03131", "backgroundColor": "transparent", "fillStyle": "hachure",
         "strokeWidth": 2, "roughness": 2, "opacity": 100, "seed": 11, "angle": 0,
         "version": 1, "versionNonce": 4, "isDeleted": false},
        {"type": "text", "id": "t1", "x": 20, "y": 35, "width": 100, "height": 25,
         "text": "docgen", "fontSize": 20, "fontFamily": 4, "textAlign": "center",
         "verticalAlign": "middle", "containerId": "r1", "strokeColor": "#1e1e1e",
         "backgroundColor": "transparent", "fillStyle": "hachure", "strokeWidth": 1,
         "roughness": 1, "opacity": 100, "seed": 9, "angle": 0, "lineHeight": 1.25,
         "version": 1, "versionNonce": 3, "isDeleted": false}
      ],
      "appState": {"viewBackgroundColor": "#ffffff"},
      "files": {}
    }"##;

    #[test]
    fn renders_scene_to_svg_deterministically() {
        let a = render_source_svg(SCENE).unwrap();
        let b = render_source_svg(SCENE).unwrap();
        assert_eq!(a, b, "output must be byte-stable (seeded roughness)");
        assert!(a.contains("<svg"));
        assert!(a.starts_with("<svg") || a.contains("viewBox"));
        // Roughr stroke path for the rectangle, fill for its background.
        assert!(a.contains("fill=\"#a5d8ff\""), "solid background fill");
        // Text shaped as paths.
        assert!(a.matches("<path d=\"M").count() >= 1, "text glyph paths");
        assert!(!a.contains("<text"), "no font dependency downstream");
    }

    #[test]
    fn arrowhead_strokes_present() {
        let svg = render_source_svg(SCENE).unwrap();
        // 2 strokes for the arrowhead + 1 line = at least 3 stroke paths.
        assert!(svg.matches("<path").count() >= 3);
    }

    #[test]
    fn renderer_impl_matches_diagram_trait() {
        use docgen_core::assets::DiagramRenderer as _;
        let r = ExcalidrawRenderer::new();
        let asset = r.render("x", DiagramFormat::Excalidraw, SCENE).unwrap();
        assert_eq!(asset.format, AssetFormat::Svg);
        assert!(String::from_utf8(asset.bytes).unwrap().contains("<svg"));
        assert!(r.render("x", DiagramFormat::Mermaid, "A-->B").is_err());
    }

    #[test]
    fn bare_elements_array_parses() {
        let scene = parse_scene(r#"[{"type": "rectangle", "id": "r", "x": 0, "y": 0}]"#)
            .unwrap();
        assert_eq!(scene.elements.len(), 1);
    }
}
