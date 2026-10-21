use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// A parsed document: YAML frontmatter plus a flat list of blocks, each
/// carrying a stable [`Block::id`] used as the join key for structural diffing
/// and for asset bookkeeping (rendered diagrams etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub frontmatter: Frontmatter,
    pub blocks: Vec<Block>,
}

impl Document {
    /// Every inline-image URL in document order, deduplicated (first
    /// occurrence wins). The caller decides how to resolve them: the CLI
    /// reads files relative to the source, the plugin brokers harvest
    /// workspace blobs.
    pub fn image_urls(&self) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        for block in &self.blocks {
            collect_block_urls(block, &mut seen, &mut out);
        }
        out
    }
}

fn collect_block_urls(block: &Block, seen: &mut HashSet<String>, out: &mut Vec<String>) {
    match &block.kind {
        BlockKind::Heading { inline, .. } | BlockKind::Paragraph { inline } => {
            collect_inline_urls(inline, seen, out);
        }
        BlockKind::List { items, .. } => {
            for item in items {
                collect_inline_urls(&item.content, seen, out);
                for child in &item.children {
                    collect_block_urls(child, seen, out);
                }
            }
        }
        BlockKind::Table { rows } => {
            for row in rows {
                for cell in &row.cells {
                    collect_inline_urls(cell, seen, out);
                }
            }
        }
        BlockKind::BlockQuote { blocks } => {
            for child in blocks {
                collect_block_urls(child, seen, out);
            }
        }
        BlockKind::CodeBlock { .. } | BlockKind::Diagram { .. } | BlockKind::ThematicBreak => {}
    }
}

fn collect_inline_urls(inlines: &[Inline], seen: &mut HashSet<String>, out: &mut Vec<String>) {
    for inline in inlines {
        match inline {
            Inline::Image { url, .. } => {
                if seen.insert(url.clone()) {
                    out.push(url.clone());
                }
            }
            Inline::Strong(inner) | Inline::Emphasis(inner) | Inline::Strikethrough(inner) => {
                collect_inline_urls(inner, seen, out);
            }
            Inline::Link { text, .. } => collect_inline_urls(text, seen, out),
            Inline::Text(_)
            | Inline::Code(_)
            | Inline::Math { .. }
            | Inline::SoftBreak
            | Inline::HardBreak => {}
        }
    }
}

/// Rich frontmatter. The named fields cover what the renderers understand;
/// anything else a user writes is preserved verbatim in [`Frontmatter::extra`]
/// so downstream tooling (templates, diffing) can use arbitrary metadata.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    pub date: Option<String>,
    /// Document-level version string, propagated into outputs.
    pub version: Option<String>,
    /// Stable identity of the document itself (used when diffing two
    /// revisions of the same doc vs. two different docs).
    pub doc_id: Option<String>,
    /// Everything else the author wrote in the YAML block.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Frontmatter {
    /// Typed view of the document-level `media:` map (D-003, D-009):
    /// `media: { placement: inline|appendix, index: bool }`. The map itself
    /// stays in `extra`, so renderer metadata passthrough (typst `meta`)
    /// still carries the values.
    pub fn media(&self) -> MediaConfig {
        match self.extra.get("media") {
            Some(serde_json::Value::Null) | None => MediaConfig::default(),
            Some(v) => serde_json::from_value(v.clone()).unwrap_or_default(),
        }
    }
}

/// Document-level media defaults from frontmatter (D-003): where media
/// renders unless a block overrides it, and whether to emit a figures
/// index. Zero-value = fully inline, no index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaConfig {
    #[serde(default)]
    pub placement: Placement,
    #[serde(default)]
    pub index: bool,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            placement: Placement::Inline,
            index: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Stable identifier: explicit (`{#id}` heading attribute, fenced-block
    /// info attribute) when given, derived otherwise (slug for headings,
    /// content hash for everything else). Never contains `/`.
    pub id: String,
    pub kind: BlockKind,
    /// Authoring annotations (placement, caption, page geometry, breaks,
    /// keeps) parsed from attribute syntax. Deliberately separate from `id`:
    /// the id is the diff join key and diagram asset key and never carries
    /// annotation values, and annotations are invisible to the structural
    /// diff (D-001, D-008).
    #[serde(default, skip_serializing_if = "Annotations::is_default")]
    pub annotations: Annotations,
}

impl Block {
    /// Whether keep-together applies to this block: figures and diagrams
    /// keep by default, anything else only when `keep=true` is set (D-002).
    pub fn keeps_together(&self) -> bool {
        match self.annotations.keep {
            Some(keep) => keep,
            None => self.kind.lone_image().is_some() || self.kind.is_diagram(),
        }
    }
}

/// Per-block authoring annotations (D-001, D-009). Every field is optional:
/// the zero value means "no directives", which renders exactly like a
/// document without them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Annotations {
    /// `place=inline|appendix` — media placement override (D-003).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Placement>,
    /// `caption="…"` — caption override; a lone image's alt text is the
    /// default caption when this is unset (D-004). Diagrams only get a
    /// caption through this attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// `page=landscape|portrait` — real page-geometry change for this block;
    /// on a heading it spans the section (D-005, D-010).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Orientation>,
    /// `new-page` — start this block on a fresh page (D-002).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub new_page: bool,
    /// `keep=true|false` — keep-together override; figures keep by default
    /// (D-002).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<bool>,
}

impl Annotations {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Where a media block renders (D-003).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    #[default]
    Inline,
    Appendix,
}

/// Page orientation: always a real geometry change, never rotated content
/// (D-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Landscape,
    Portrait,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BlockKind {
    Heading {
        level: u8,
        inline: Vec<Inline>,
    },
    Paragraph {
        inline: Vec<Inline>,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    /// A fenced block whose info string marks diagram source.
    Diagram {
        format: DiagramFormat,
        source: String,
    },
    List {
        ordered: bool,
        /// First item's number, for ordered lists that do not start at 1.
        start: u64,
        items: Vec<ListItem>,
    },
    Table {
        rows: Vec<TableRow>,
    },
    BlockQuote {
        blocks: Vec<Block>,
    },
    ThematicBreak,
}

impl BlockKind {
    /// The single image of a paragraph that holds exactly one image and no
    /// other content — the markdown shape of a figure (D-004). Returns the
    /// URL and the alt text (the default caption).
    pub fn lone_image(&self) -> Option<(&str, &[Inline])> {
        match self {
            BlockKind::Paragraph { inline } => match inline.as_slice() {
                [Inline::Image { url, alt }] => Some((url.as_str(), alt.as_slice())),
                _ => None,
            },
            _ => None,
        }
    }

    /// A fenced mermaid/excalidraw block — the other media shape (D-004).
    pub fn is_diagram(&self) -> bool {
        matches!(self, BlockKind::Diagram { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagramFormat {
    Mermaid,
    Excalidraw,
}

impl DiagramFormat {
    pub fn from_language(lang: &str) -> Option<Self> {
        match lang.trim().to_ascii_lowercase().as_str() {
            "mermaid" => Some(Self::Mermaid),
            "excalidraw" => Some(Self::Excalidraw),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mermaid => "mermaid",
            Self::Excalidraw => "excalidraw",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub content: Vec<Inline>,
    /// Nested blocks (sub-lists, code blocks inside the item, ...).
    pub children: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    pub is_header: bool,
    pub cells: Vec<Vec<Inline>>,
}

/// Inline content. Kept structured (rather than pre-rendered markup) so each
/// backend (typst, docx, pptx) and the structural differ can walk it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Code(String),
    Link {
        url: String,
        text: Vec<Inline>,
    },
    Image {
        url: String,
        alt: Vec<Inline>,
    },
    /// Math source (LaTeX subset), authored as `$…$` (inline) or a
    /// display-math paragraph (`$$…$$`). Typeset by the PDF backend; other
    /// backends degrade to [`Inline::math_literal`].
    Math {
        latex: String,
        display: bool,
    },
    SoftBreak,
    HardBreak,
}

impl Inline {
    /// Flattened plain text (used for heading slugs, docx alt text, ...).
    pub fn plain_text(inlines: &[Inline]) -> String {
        let mut out = String::new();
        Self::write_plain(inlines, &mut out);
        out
    }

    /// The literal-text form of a math span — exactly what these sources
    /// rendered before math existed, so backends without a typesetter (and
    /// the text flattening) stay byte-stable.
    pub fn math_literal(latex: &str, display: bool) -> String {
        if display {
            format!("$${latex}$$")
        } else {
            format!("${latex}$")
        }
    }

    fn write_plain(inlines: &[Inline], out: &mut String) {
        for inline in inlines {
            match inline {
                Inline::Text(t) => out.push_str(t),
                Inline::Code(t) => {
                    out.push_str(t);
                }
                Inline::Strong(inner) | Inline::Emphasis(inner) | Inline::Strikethrough(inner) => {
                    Self::write_plain(inner, out);
                }
                Inline::Link { text, .. } => Self::write_plain(text, out),
                Inline::Image { alt, .. } => Self::write_plain(alt, out),
                Inline::Math { latex, display } => {
                    out.push_str(&Self::math_literal(latex, *display))
                }
                Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotations_default_is_empty_and_keeps_serialization_stable() {
        let doc = Document {
            frontmatter: Frontmatter::default(),
            blocks: vec![Block {
                id: "p-1".into(),
                kind: BlockKind::Paragraph {
                    inline: vec![Inline::Text("x".into())],
                },
                annotations: Annotations::default(),
            }],
        };
        let json = serde_json::to_string(&doc).unwrap();
        assert!(!json.contains("annotations"), "{json}");
        let round: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(round, doc);
    }

    #[test]
    fn annotations_survive_serialization_when_set() {
        let block = Block {
            id: "d".into(),
            kind: BlockKind::Diagram {
                format: DiagramFormat::Mermaid,
                source: "graph TD".into(),
            },
            annotations: Annotations {
                place: Some(Placement::Appendix),
                page: Some(Orientation::Landscape),
                new_page: true,
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"place\":\"appendix\""), "{json}");
        assert!(json.contains("\"page\":\"landscape\""), "{json}");
        assert!(json.contains("\"new_page\":true"), "{json}");
        let round: Block = serde_json::from_str(&json).unwrap();
        assert_eq!(round, block);
    }

    #[test]
    fn keeps_together_defaults_on_for_media_only() {
        let figure = Block {
            id: "img".into(),
            kind: BlockKind::Paragraph {
                inline: vec![Inline::Image {
                    url: "u.png".into(),
                    alt: vec![Inline::Text("alt".into())],
                }],
            },
            annotations: Annotations::default(),
        };
        assert!(figure.keeps_together());
        let diagram = Block {
            id: "d".into(),
            kind: BlockKind::Diagram {
                format: DiagramFormat::Mermaid,
                source: String::new(),
            },
            annotations: Annotations::default(),
        };
        assert!(diagram.keeps_together());
        let relaxed = Block {
            annotations: Annotations {
                keep: Some(false),
                ..Default::default()
            },
            ..diagram.clone()
        };
        assert!(!relaxed.keeps_together());
        let prose = Block {
            id: "p".into(),
            kind: BlockKind::Paragraph {
                inline: vec![Inline::Text("plain".into())],
            },
            annotations: Annotations::default(),
        };
        assert!(!prose.keeps_together());
    }

    #[test]
    fn lone_image_recognition() {
        let lone = BlockKind::Paragraph {
            inline: vec![Inline::Image {
                url: "u.png".into(),
                alt: vec![Inline::Text("cap".into())],
            }],
        };
        assert_eq!(lone.lone_image().map(|(u, _)| u), Some("u.png"));
        let mixed = BlockKind::Paragraph {
            inline: vec![
                Inline::Text("see ".into()),
                Inline::Image {
                    url: "u.png".into(),
                    alt: vec![],
                },
            ],
        };
        assert_eq!(mixed.lone_image(), None);
        assert!(!mixed.is_diagram());
    }
}
