//! Media placement and figure numbering: a pure pass over a [`Document`]
//! (D-003, D-004, D-010).
//!
//! A **media block** is a top-level paragraph that is exactly one image, or
//! a fenced diagram. Every media block resolves to an effective placement
//! (frontmatter default, per-block `place=` override) and, when it has an
//! effective caption (`caption=` attribute, else the image's alt text), to a
//! document-wide figure number. Media without a caption still relocates but
//! renders unnumbered and stays out of the figures index.

use crate::model::{BlockKind, Document, Frontmatter, Placement};

/// One media block's resolved plan, in document order inside
/// [`MediaPlan::figures`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FigurePlan {
    /// The media block's id — also the link/bookmark anchor basis
    /// ([`fig_anchor`]).
    pub block_id: String,
    /// 1-based figure number in document order; `None` when the media block
    /// has no effective caption.
    pub number: Option<u32>,
    /// Effective caption: `caption=` attribute wins over the lone image's
    /// alt text (D-004). Diagrams have no default caption.
    pub caption: Option<String>,
    /// Resolved placement: `place=` override wins over the frontmatter
    /// default (D-003).
    pub placement: Placement,
}

impl FigurePlan {
    /// The rendered label ("Figure 3"); media without a number render
    /// unnumbered and never reach a label.
    pub fn label(&self) -> Option<String> {
        self.number.map(|n| format!("Figure {n}"))
    }
}

/// The document's media plan: numbering, placements, and the back-matter
/// structure (appendix order, index entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlan {
    pub default_placement: Placement,
    /// Whether the frontmatter asks for a figures index (D-003).
    pub index_requested: bool,
    /// Every top-level media block in document order.
    pub figures: Vec<FigurePlan>,
}

impl Default for MediaPlan {
    fn default() -> Self {
        Self {
            default_placement: Placement::Inline,
            index_requested: false,
            figures: Vec::new(),
        }
    }
}

impl MediaPlan {
    pub fn resolve(doc: &Document) -> Self {
        Self::resolve_with(doc, &doc.frontmatter)
    }

    pub fn resolve_with(doc: &Document, frontmatter: &Frontmatter) -> Self {
        let config = frontmatter.media();
        let mut figures = Vec::new();
        let mut next_number = 1u32;
        for block in &doc.blocks {
            let Some((url, alt)) = media_block(&block.kind) else {
                continue;
            };
            let caption = effective_caption(&block.annotations.caption, url, alt);
            let number = caption.as_ref().map(|_| {
                let n = next_number;
                next_number += 1;
                n
            });
            let placement = block.annotations.place.unwrap_or(config.placement);
            figures.push(FigurePlan {
                block_id: block.id.clone(),
                number,
                caption,
                placement,
            });
        }
        Self {
            default_placement: config.placement,
            index_requested: config.index,
            figures,
        }
    }

    /// The plan for a media block by id; `None` for non-media blocks.
    pub fn figure(&self, block_id: &str) -> Option<&FigurePlan> {
        self.figures.iter().find(|f| f.block_id == block_id)
    }

    /// Media blocks placed in the appendix, in document order (D-010: the
    /// appendix inherits the document orientation and this order).
    pub fn appendix_figures(&self) -> impl Iterator<Item = &FigurePlan> {
        self.figures
            .iter()
            .filter(|f| f.placement == Placement::Appendix)
    }

    /// Numbered figures in number order — the figures-index entries.
    pub fn index_figures(&self) -> impl Iterator<Item = &FigurePlan> {
        self.figures.iter().filter(|f| f.number.is_some())
    }
}

/// The media shape of a block: a lone-image paragraph's (url, alt) or a
/// diagram (url is `None`; the asset is keyed by block id).
fn media_block(kind: &BlockKind) -> Option<(Option<&str>, &[crate::model::Inline])> {
    match kind {
        BlockKind::Paragraph { .. } => {
            let (url, alt) = kind.lone_image()?;
            Some((Some(url), alt))
        }
        BlockKind::Diagram { .. } => Some((None, &[])),
        _ => None,
    }
}

fn effective_caption<'a>(
    annotation: &'a Option<String>,
    url: Option<&str>,
    alt: &'a [crate::model::Inline],
) -> Option<String> {
    if let Some(c) = annotation {
        let trimmed = c.trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        };
    }
    // Diagrams have no default caption; a lone image's alt text is its
    // caption (D-004).
    url?;
    let text = crate::model::Inline::plain_text(alt);
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// The anchor name shared by all renderers for a figure: internal links use
/// `#fig-<block id>` in markdown, the typst label is `<fig-<id>>` and the
/// Word bookmark derives from the same string.
pub fn fig_anchor(block_id: &str) -> String {
    format!("fig-{block_id}")
}

/// Normalizes a markdown fragment target: `#fig:x` (the figure-id
/// convention) resolves to the figure anchor `fig-x`; anything else is the
/// anchor as written (block id).
pub fn resolve_anchor(fragment: &str) -> String {
    match fragment.strip_prefix("fig:") {
        Some(rest) => fig_anchor(rest),
        None => fragment.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Annotations, Block, DiagramFormat, Inline};
    use crate::parse_document;

    fn lone_image(id: &str, alt: &str, ann: Annotations) -> Block {
        Block {
            id: id.to_owned(),
            kind: BlockKind::Paragraph {
                inline: vec![Inline::Image {
                    url: format!("{id}.png"),
                    alt: vec![Inline::Text(alt.to_owned())],
                }],
            },
            annotations: ann,
        }
    }

    fn diagram(id: &str, ann: Annotations) -> Block {
        Block {
            id: id.to_owned(),
            kind: BlockKind::Diagram {
                format: DiagramFormat::Mermaid,
                source: "graph TD".into(),
            },
            annotations: ann,
        }
    }

    fn doc(frontmatter: &str, blocks: Vec<Block>) -> Document {
        let mut d = parse_document(frontmatter).unwrap();
        d.blocks = blocks;
        d
    }

    #[test]
    fn empty_document_has_empty_plan() {
        let d = parse_document("# Nothing here\n\nPlain text.\n").unwrap();
        let plan = MediaPlan::resolve(&d);
        assert!(plan.figures.is_empty());
        assert_eq!(plan.default_placement, Placement::Inline);
        assert!(!plan.index_requested);
        assert_eq!(plan.appendix_figures().count(), 0);
        assert_eq!(plan.index_figures().count(), 0);
    }

    #[test]
    fn inline_default_numbers_captioned_media_in_document_order() {
        let d = doc(
            "---\ntitle: T\n---\n",
            vec![
                Block {
                    id: "intro".into(),
                    kind: BlockKind::Paragraph {
                        inline: vec![Inline::Text("prose".into())],
                    },
                    annotations: Annotations::default(),
                },
                lone_image("img1", "First caption", Annotations::default()),
                diagram("diag1", Annotations::default()), // no caption
                lone_image("img2", "Second caption", Annotations::default()),
            ],
        );
        let plan = MediaPlan::resolve(&d);
        assert_eq!(plan.figures.len(), 3, "all media blocks are planned");
        assert_eq!(plan.figure("img1").unwrap().number, Some(1));
        assert_eq!(plan.figure("diag1").unwrap().number, None);
        assert_eq!(
            plan.figure("img2").unwrap().number,
            Some(2),
            "uncaptioned media consumes no number"
        );
        assert_eq!(
            plan.figure("img2").unwrap().caption.as_deref(),
            Some("Second caption")
        );
        assert_eq!(plan.index_figures().count(), 2);
    }

    #[test]
    fn appendix_default_with_per_item_override() {
        let d = doc(
            "---\ntitle: T\nmedia:\n  placement: appendix\n  index: true\n---\n",
            vec![
                lone_image("img1", "Caption one", Annotations::default()),
                lone_image(
                    "img2",
                    "Stays inline",
                    Annotations {
                        place: Some(Placement::Inline),
                        ..Default::default()
                    },
                ),
                lone_image(
                    "img3",
                    "Forced out",
                    Annotations {
                        place: Some(Placement::Appendix),
                        ..Default::default()
                    },
                ),
            ],
        );
        let plan = MediaPlan::resolve(&d);
        assert_eq!(plan.default_placement, Placement::Appendix);
        assert!(plan.index_requested);
        let appendix: Vec<&str> = plan
            .appendix_figures()
            .map(|f| f.block_id.as_str())
            .collect();
        assert_eq!(appendix, ["img1", "img3"]);
        assert_eq!(plan.figure("img2").unwrap().placement, Placement::Inline);
        assert_eq!(plan.figure("img3").unwrap().placement, Placement::Appendix);
    }

    #[test]
    fn caption_attribute_overrides_alt_text() {
        let d = doc(
            "---\ntitle: T\n",
            vec![lone_image(
                "img1",
                "Alt caption",
                Annotations {
                    caption: Some("Overridden".into()),
                    ..Default::default()
                },
            )],
        );
        let plan = MediaPlan::resolve(&d);
        assert_eq!(
            plan.figure("img1").unwrap().caption.as_deref(),
            Some("Overridden")
        );
    }

    #[test]
    fn diagram_caption_needs_the_attribute() {
        let d = doc(
            "---\ntitle: T\n",
            vec![
                diagram("d1", Annotations::default()),
                diagram(
                    "d2",
                    Annotations {
                        caption: Some("Explicit".into()),
                        ..Default::default()
                    },
                ),
            ],
        );
        let plan = MediaPlan::resolve(&d);
        assert_eq!(plan.figure("d1").unwrap().caption, None);
        assert_eq!(plan.figure("d2").unwrap().number, Some(1));
    }

    #[test]
    fn empty_alt_text_is_no_caption() {
        let d = doc(
            "---\ntitle: T\n",
            vec![lone_image("img1", "", Annotations::default())],
        );
        let plan = MediaPlan::resolve(&d);
        assert_eq!(plan.figure("img1").unwrap().number, None);
        assert_eq!(plan.index_figures().count(), 0);
    }

    #[test]
    fn anchors_are_namespaced_and_stable() {
        assert_eq!(fig_anchor("arch"), "fig-arch");
    }
}
