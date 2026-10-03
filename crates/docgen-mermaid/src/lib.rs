//! Mermaid diagram rendering for docgen, backed by the headless Rust
//! `merman` engine (parse → layout → SVG, no browser, no JavaScript).
//!
//! Binary (PNG) output reuses merman's raster export, which renders from the
//! validated SVG — so vector and raster stay visually identical.

use docgen_core::assets::{AssetFormat, DiagramRenderer, RenderedAsset};
use docgen_core::model::DiagramFormat;
use docgen_core::{Error, Result};
use merman::svg::export::RasterOptions;
use merman::svg::{CssOverridePolicy, ScopedCssPostprocessor, SvgPipeline};
use merman::{
    OperationControl, PngRequest, RenderOutput, RenderRequest, Renderer, SvgRequest,
};

/// merman's flowchart CSS hardcodes `opacity: 0.5` on edge-label plates
/// (`#merman .edgeLabel rect`), parroting mermaid.js — half the edge line
/// shows through the plate and the label reads as struck-through. This
/// override restores a fully opaque plate and applies to both the SVG
/// string and merman's internal raster export (the pipeline runs before
/// finalization, so both consumers see it).
const OPAQUE_EDGE_PLATE_CSS: &str = ".edgeLabel rect { opacity: 1 !important; }";

/// How mermaid edge labels deal with the edge line passing under them.
///
/// Mermaid draws an opaque plate behind edge labels; without it, the edge
/// line strikes through the label text (labels sit on the line). A white
/// plate is invisible on white pages while still masking the line, so it is
/// the default. Use [`EdgeLabelBackground::Transparent`] for colored page
/// backgrounds (typst templates) where a white plate would show as a box.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EdgeLabelBackground {
    /// No plate at all; edge lines are visible through label text.
    Transparent,
    /// Opaque white plate (invisible on white pages, masks edge lines).
    #[default]
    White,
    /// Any CSS/SVG color, e.g. `"#f5f5f5"`.
    Color(String),
}

impl EdgeLabelBackground {
    fn css_value(&self) -> String {
        match self {
            Self::Transparent => "transparent".to_string(),
            Self::White => "#ffffff".to_string(),
            Self::Color(c) => c.clone(),
        }
    }
}

/// Reusable merman renderer. Cheap to keep alive, safe to share.
pub struct MermaidRenderer {
    renderer: Renderer,
    /// Emit PNG instead of SVG (e.g. for consumers with no SVG support).
    pub raster: bool,
    /// Edge label plate strategy (see [`EdgeLabelBackground`]).
    pub edge_label_background: EdgeLabelBackground,
}

impl Default for MermaidRenderer {
    fn default() -> Self {
        Self {
            renderer: Renderer::new(),
            raster: false,
            edge_label_background: EdgeLabelBackground::default(),
        }
    }
}

impl MermaidRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn raster() -> Self {
        Self {
            renderer: Renderer::new(),
            raster: true,
            ..Self::default()
        }
    }

    pub fn render_svg(&self, source: &str) -> Result<String> {
        let output = self
            .renderer
            .render(RenderRequest::svg(
                &self.prepare(source),
                OperationControl::new(),
                self.svg_request(),
            ))
            .map_err(|e| Error::Diagram(format!("mermaid: {e}")))?;
        match output {
            RenderOutput::Svg(Some(svg)) => Ok(svg.svg().to_string()),
            _ => Err(Error::Diagram(
                "source did not contain a recognizable Mermaid diagram".into(),
            )),
        }
    }

    pub fn render_png(&self, source: &str) -> Result<Vec<u8>> {
        let output = self
            .renderer
            .render(RenderRequest::png(
                &self.prepare(source),
                OperationControl::new(),
                PngRequest {
                    svg: self.svg_request(),
                    options: RasterOptions::default(),
                },
            ))
            .map_err(|e| Error::Diagram(format!("mermaid: {e}")))?;
        match output {
            RenderOutput::Png(Some(raster)) => Ok(raster.bytes),
            _ => Err(Error::Diagram(
                "source did not contain a recognizable Mermaid diagram".into(),
            )),
        }
    }

    /// SVG request with the edge-plate opacity fix pipelined in.
    fn svg_request(&self) -> SvgRequest {
        SvgRequest {
            pipeline: Some(
                SvgPipeline::resvg_safe().with_postprocessor(
                    ScopedCssPostprocessor::new(OPAQUE_EDGE_PLATE_CSS)
                        .with_override_policy(CssOverridePolicy::StripExistingImportant),
                ),
            ),
            ..SvgRequest::default()
        }
    }

    /// Mermaid's default label flavor is HTML inside `<foreignObject>`, which
    /// browser-less SVG consumers (typst/usvg, most rasterizers) drop entirely —
    /// diagrams would render without any label text. Unless the author already
    /// set an init directive, force native SVG text labels and the configured
    /// edge-label plate.
    fn prepare(&self, source: &str) -> String {
        if source.trim_start().starts_with("%%{init") {
            return source.to_string();
        }
        format!(
            "%%{{init: {{\"htmlLabels\": false, \"themeVariables\": {{\"edgeLabelBackground\": \"{}\"}}}}}}%%\n{source}",
            self.edge_label_background.css_value(),
        )
    }
}

impl DiagramRenderer for MermaidRenderer {
    fn render(&self, _block_id: &str, format: DiagramFormat, source: &str) -> Result<RenderedAsset> {
        match format {
            DiagramFormat::Mermaid => {
                if self.raster {
                    Ok(RenderedAsset {
                        format: AssetFormat::Png,
                        bytes: self.render_png(source)?,
                    })
                } else {
                    Ok(RenderedAsset {
                        format: AssetFormat::Svg,
                        bytes: self.render_svg(source)?.into_bytes(),
                    })
                }
            }
            DiagramFormat::Excalidraw => Err(Error::Diagram(format!(
                "unsupported diagram format '{}' in mermaid renderer",
                format.as_str()
            ))),
        }
    }

    fn name(&self) -> &'static str {
        "merman"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_flowchart_svg() {
        let r = MermaidRenderer::new();
        let svg = r
            .render_svg("flowchart TD\nA[Start] --> B[Done]")
            .expect("render");
        assert!(svg.contains("<svg"), "no svg element in output");
    }

    #[test]
    fn svg_labels_survive_headless_consumers() {
        let r = MermaidRenderer::new();
        let svg = r
            .render_svg("flowchart LR\nMD[Markdown] --> Core[Model]")
            .expect("render");
        assert!(
            !svg.contains("foreignObject"),
            "labels must be native <text>, not foreignObject HTML"
        );
        assert!(svg.contains("<text"), "expected native text labels");
    }

    #[test]
    fn edge_label_background_defaults_to_white() {
        let r = MermaidRenderer::new();
        let svg = r
            .render_svg("flowchart LR\nMD[Markdown] -->|via| Core[Model]")
            .expect("render");
        let css = extract_css(&svg);
        assert!(
            css.iter().any(|rule| { rule.contains("edgeLabel") && rule.contains("fill:#ffffff") }),
            "edgeLabel background not white in CSS: {css:?}"
        );
    }

    #[test]
    fn transparent_edge_label_background_is_opt_in() {
        let r = MermaidRenderer {
            edge_label_background: EdgeLabelBackground::Transparent,
            ..MermaidRenderer::new()
        };
        let svg = r
            .render_svg("flowchart LR\nMD[Markdown] -->|via| Core[Model]")
            .expect("render");
        let css = extract_css(&svg);
        assert!(
            css.iter()
                .any(|rule| rule.contains("edgeLabel") && rule.contains("fill:transparent")),
            "edgeLabel background not transparent in CSS: {css:?}"
        );
    }

    fn extract_css(svg: &str) -> Vec<String> {
        let start = svg.find("<style").map(|i| i + "<style".len());
        let end = svg.find("</style>");
        match (start, end) {
            (Some(s), Some(e)) if s < e => svg[s..e]
                .split('}')
                .map(|r| format!("{r}}}"))
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn author_init_directive_is_respected() {
        let r = MermaidRenderer::new();
        assert!(r.prepare("flowchart TD\nA-->B").starts_with("%%{init"));
        let custom = "%%{init: {\"theme\": \"dark\"}}%%\nflowchart TD\nA-->B";
        assert_eq!(r.prepare(custom), custom);
    }

    #[test]
    fn edge_label_plates_are_opaque() {
        // merman hardcodes opacity:0.5 on edge-label plates, which lets the
        // edge line show through; our pipeline override must restore full
        // opacity in the emitted CSS.
        let r = MermaidRenderer::new();
        let svg = r
            .render_svg("flowchart LR\n    A[One] -->|edge label| B[Two]\n")
            .expect("render");
        assert!(
            svg.contains("data-merman-postprocess=\"scoped-css\"")
                && svg.contains(".edgeLabel rect { opacity: 1 !important;}"),
            "edge-label plate opacity override missing from postprocessed CSS"
        );
    }
}
