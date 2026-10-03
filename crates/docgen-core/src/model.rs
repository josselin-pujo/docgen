use serde::{Deserialize, Serialize};

/// A parsed document: YAML frontmatter plus a flat list of blocks, each
/// carrying a stable [`Block::id`] used as the join key for structural diffing
/// and for asset bookkeeping (rendered diagrams etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub frontmatter: Frontmatter,
    pub blocks: Vec<Block>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Stable identifier: explicit (`{#id}` heading attribute, fenced-block
    /// info attribute) when given, derived otherwise (slug for headings,
    /// content hash for everything else). Never contains `/`.
    pub id: String,
    pub kind: BlockKind,
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
                Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            }
        }
    }
}
