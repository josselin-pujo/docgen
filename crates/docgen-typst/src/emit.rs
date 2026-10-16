//! Lowering of the docgen document model to typst markup.
//!
//! Compilation always happens against three virtual files:
//!
//! - `main.typ` — the template (custom, or [`DEFAULT_TEMPLATE`]), prefixed by
//!   generated `#set document(...)` metadata rules from frontmatter;
//! - `docgen-body.typ` — the document's blocks, no preamble, no cover;
//! - `docgen-meta.typ` — the full frontmatter as a typst dictionary `meta`
//!   (typed fields always present, `none` when unset; unknown YAML keys kept).
//!
//! A template is therefore ordinary typst: `#import "docgen-meta.typ": meta`
//! for metadata, `#include "docgen-body.typ"` where the body belongs.
//!
//! Text is escaped defensively: every typst-special character in prose gets a
//! backslash, so author content can never change markup structure. Diagram
//! blocks and inline images become `#image(...)` references into the world's
//! virtual files.
//!
//! Media blocks (lone-image paragraphs and diagrams) render as figures:
//! captioned ones carry a document-wide number, a caption line and a
//! `<fig-…>` label; internal links (`[text](#anchor)`) and the figures back
//! matter link through those labels (D-004, D-006).

use std::collections::HashSet;

use docgen_core::assets::{AssetFormat, AssetMap};
use docgen_core::media::{MediaPlan, fig_anchor, resolve_anchor};
use docgen_core::model::{Block, BlockKind, Document, Inline, ListItem};

/// Names of the virtual files the template can address.
pub const BODY_PATH: &str = "docgen-body.typ";
pub const META_PATH: &str = "docgen-meta.typ";

/// The built-in template: plain A4 article with a centered cover, plus a
/// `Contents` outline when `toc: true` is set in the frontmatter.
const DEFAULT_TEMPLATE: &str = r##"#import "docgen-meta.typ": meta
#set page(paper: "a4")
#set heading(numbering: "1.1")

#if meta.title != none or meta.subtitle != none [
  #align(center)[
    #if meta.title != none [
      #text(size: 1.75em, weight: "bold")[#meta.title]
      #v(0.4em)
    ]
    #if meta.subtitle != none [
      #emph[#meta.subtitle]
      #v(0.6em)
    ]
    #let parts = (
      if meta.authors.len() > 0 [#meta.authors.join(", ")],
      if meta.date != none [#meta.date],
      if meta.version != none [#meta.version],
    ).filter(p => p != none)
    #if parts.len() > 0 [
      #text(size: 0.9em, fill: luma(100))[#parts.join(" \u{b7} ")]
    ]
  ]
  #v(2em)
]

#if meta.at("toc", default: false) == true [
  #outline(title: [Contents], depth: 3)
  #v(1.5em)
]

#include "docgen-body.typ"
"##;

/// Everything the in-memory world needs to compile one document.
pub struct Emitted {
    /// The main source (template).
    pub main: String,
    /// Additional typst sources addressable by relative path.
    pub sources: Vec<(String, String)>,
    /// Binary virtual files (rendered diagrams).
    pub files: Vec<(String, Vec<u8>)>,
    /// Deterministic docgen-side warnings (fit-to-page shrink, D-007),
    /// surfaced alongside typst's own.
    pub warnings: Vec<String>,
}

/// `template` replaces the built-in default; it can `#import`/`#include` the
/// virtual files above.
/// Emits the typst sources for a document (main + virtual modules).
/// Public for fuzzing and golden tests; the renderer compiles these.
pub fn emit(doc: &Document, assets: &AssetMap, template: Option<&str>) -> Emitted {
    let mut em = Emitter {
        assets,
        files: Vec::new(),
        used_names: HashSet::new(),
        plan: MediaPlan::resolve(doc),
        warnings: Vec::new(),
    };

    let mut body = String::with_capacity(4096);
    for block in &doc.blocks {
        em.block(block, &mut body);
    }

    // Document metadata is global; generated unconditionally (a custom
    // template may override it with its own `#set document(...)`).
    let mut main = String::new();
    if let Some(title) = &doc.frontmatter.title {
        main.push_str(&format!("#set document(title: \"{}\")\n", esc_str(title)));
    }
    if !doc.frontmatter.authors.is_empty() {
        let authors: Vec<String> = doc
            .frontmatter
            .authors
            .iter()
            .map(|a| format!("\"{}\"", esc_str(a)))
            .collect();
        main.push_str(&format!(
            "#set document(author: ({},))\n",
            authors.join(", ")
        ));
    }
    main.push_str(template.unwrap_or(DEFAULT_TEMPLATE));

    Emitted {
        main,
        sources: vec![
            (BODY_PATH.to_string(), body),
            (META_PATH.to_string(), emit_meta(doc)),
        ],
        files: em.files,
        warnings: em.warnings,
    }
}

struct Emitter<'a> {
    assets: &'a AssetMap,
    files: Vec<(String, Vec<u8>)>,
    used_names: HashSet<String>,
    plan: MediaPlan,
    warnings: Vec<String>,
}

impl<'a> Emitter<'a> {
    fn block(&mut self, block: &Block, out: &mut String) {
        match &block.kind {
            BlockKind::Heading { level, inline } => {
                let hashes = "=".repeat((*level).clamp(1, 6) as usize);
                out.push_str(&format!(
                    "{hashes} {} <{}>\n\n",
                    inline_markup(inline, self.assets, &mut self.files, &mut self.used_names),
                    typst_label(&block.id),
                ));
            }
            BlockKind::Paragraph { inline } => match block.kind.lone_image() {
                // A lone-image paragraph is a figure (D-004): captioned and
                // numbered via the media plan, else exactly as before.
                Some((url, alt)) => self.figure_block(block, Some((url, alt)), out),
                None => out.push_str(&format!(
                    "{}\n\n",
                    inline_markup(inline, self.assets, &mut self.files, &mut self.used_names)
                )),
            },
            BlockKind::CodeBlock { language, code } => {
                let fence = fence_for(code);
                let lang = language.as_deref().unwrap_or("");
                out.push_str(&format!("{fence}{lang}\n{code}\n{fence}\n\n"));
            }
            BlockKind::Diagram { .. } => {
                self.figure_block(block, None, out);
            }
            BlockKind::List {
                ordered,
                start,
                items,
            } => {
                self.list(*ordered, *start, items, 0, out);
                out.push('\n');
            }
            BlockKind::Table { rows } => {
                self.table(rows, out);
            }
            BlockKind::BlockQuote { blocks } => {
                out.push_str("#quote(block: true)[\n");
                for child in blocks {
                    if let BlockKind::Paragraph { inline } = &child.kind {
                        out.push_str(&format!(
                            "  {}\n\n",
                            inline_markup(
                                inline,
                                self.assets,
                                &mut self.files,
                                &mut self.used_names
                            )
                        ));
                    }
                }
                out.push_str("]\n\n");
            }
            BlockKind::ThematicBreak => {
                out.push_str("#line(length: 100%)\n\n");
            }
        }
    }

    /// Emits a media block (lone-image paragraph or diagram) as a figure.
    /// Captioned media (a number in the media plan) gets a keep-together
    /// block, a "Figure N: caption" line and a `<fig-…>` label; uncaptioned
    /// media renders exactly as it always has.
    fn figure_block(&mut self, block: &Block, image: Option<(&str, &[Inline])>, out: &mut String) {
        // Diagram without a rendered asset: keep it visible so reviews
        // catch silent degradation.
        if image.is_none() && !self.assets.contains_key(&block.id) {
            let src = match &block.kind {
                BlockKind::Diagram { source, .. } => source.clone(),
                _ => unreachable!(),
            };
            let fence = fence_for(&src);
            out.push_str(
                "#block(fill: rgb(255, 244, 229), inset: 8pt, radius: 4pt)[\n  #text(size: 0.8em)[#box[diagram not rendered]]\n",
            );
            out.push_str(&format!("  {fence}\n{src}\n  {fence}\n]\n\n"));
            return;
        }
        // Lone image without bytes: the plain linked-alt-text rendering.
        if let Some((url, alt)) = image
            && !self.assets.contains_key(url)
        {
            out.push_str(&format!(
                "{}\n\n",
                inline_markup(
                    &[Inline::Image {
                        url: url.to_string(),
                        alt: alt.to_vec(),
                    }],
                    self.assets,
                    &mut self.files,
                    &mut self.used_names,
                )
            ));
            return;
        }

        let asset = match image {
            Some((url, _)) => self.assets.get(url).unwrap(),
            None => self.assets.get(&block.id).unwrap(),
        };
        let name = match image {
            Some((url, _)) => image_file_name(url, asset.format, &mut self.used_names),
            None => asset_file_name(block, asset.format, &mut self.used_names),
        };
        self.files
            .push((format!("assets/{name}"), asset.bytes.clone()));
        let img = format!("#align(center)[#image(\"assets/{name}\", width: 92%)]");

        let plan = self.plan.figure(&block.id);
        match (
            plan.and_then(|f| f.number),
            plan.and_then(|f| f.caption.clone()),
        ) {
            (Some(n), Some(c)) => {
                if block.keeps_together() {
                    out.push_str(&format!(
                        "#block(breakable: false)[\n  {img}\n  #v(0.5em)\n  #align(center)[#text(size: 0.85em)[Figure {n}: {}]]\n]\n",
                        esc(&c)
                    ));
                } else {
                    out.push_str(&format!(
                        "{img}\n#v(0.5em)\n#align(center)[#text(size: 0.85em)[Figure {n}: {}]]\n",
                        esc(&c)
                    ));
                }
            }
            _ => out.push_str(&format!("{img}\n\n")),
        }
        out.push_str(&format!("<{}>\n\n", typst_label(&fig_anchor(&block.id))));
    }

    fn list(
        &mut self,
        ordered: bool,
        start: u64,
        items: &[ListItem],
        depth: usize,
        out: &mut String,
    ) {
        let indent = "  ".repeat(depth);
        for (i, item) in items.iter().enumerate() {
            let marker = if ordered {
                format!("{}.", start + i as u64)
            } else {
                "-".to_string()
            };
            out.push_str(&format!(
                "{indent}{marker} {}\n",
                inline_markup(
                    &item.content,
                    self.assets,
                    &mut self.files,
                    &mut self.used_names
                )
            ));
            for child in &item.children {
                if let BlockKind::List {
                    ordered,
                    start,
                    items,
                } = &child.kind
                {
                    self.list(*ordered, *start, items, depth + 1, out);
                }
            }
        }
    }

    fn table(&mut self, rows: &[docgen_core::model::TableRow], out: &mut String) {
        let Some(first) = rows.first() else { return };
        let cols = first.cells.len().max(1);
        out.push_str(&format!("#table(columns: {cols},\n"));
        for (ri, row) in rows.iter().enumerate() {
            let cells: Vec<String> = row
                .cells
                .iter()
                .map(|c| {
                    format!(
                        "[{}]",
                        inline_markup(c, self.assets, &mut self.files, &mut self.used_names)
                    )
                })
                .collect();
            if ri == 0 && row.is_header {
                out.push_str(&format!("  table.header({}),\n", cells.join(", ")));
            } else {
                out.push_str(&format!("  {},\n", cells.join(", ")));
            }
        }
        out.push_str(")\n\n");
    }
}

/// Frontmatter → `#let meta = ( ... )` with every value typst-literal.
fn emit_meta(doc: &Document) -> String {
    let fm = &doc.frontmatter;
    let mut out = String::from("#let meta = (\n");
    let field = |key: &str, value: &serde_json::Value, out: &mut String| {
        out.push_str(&format!(
            "  \"{}\": {},\n",
            esc_str(key),
            json_to_typst(value)
        ));
    };
    field("title", &opt_str(&fm.title), &mut out);
    field("subtitle", &opt_str(&fm.subtitle), &mut out);
    field(
        "authors",
        &serde_json::Value::Array(
            fm.authors
                .iter()
                .map(|a| serde_json::Value::String(a.clone()))
                .collect(),
        ),
        &mut out,
    );
    field("date", &opt_str(&fm.date), &mut out);
    field("version", &opt_str(&fm.version), &mut out);
    field("doc_id", &opt_str(&fm.doc_id), &mut out);
    for (key, value) in &fm.extra {
        field(key, value, &mut out);
    }
    out.push_str(")\n");
    out
}

fn opt_str(v: &Option<String>) -> serde_json::Value {
    match v {
        Some(s) => serde_json::Value::String(s.clone()),
        None => serde_json::Value::Null,
    }
}

/// serde JSON value → typst literal (dicts, arrays, strings, numbers, bools,
/// null → none).
fn json_to_typst(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "none".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                let mut s = format!("{f}");
                if !s.contains('.') && !s.contains('e') && !s.contains("inf") && !s.contains("NaN")
                {
                    s.push_str(".0");
                }
                s
            }
        }
        serde_json::Value::String(s) => format!("\"{}\"", esc_str(s)),
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                return "()".to_string();
            }
            let inner: Vec<String> = items.iter().map(json_to_typst).collect();
            format!("({},)", inner.join(", "))
        }
        serde_json::Value::Object(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("\"{}\": {}", esc_str(k), json_to_typst(v)))
                .collect();
            if inner.is_empty() {
                "(:)".to_string()
            } else {
                format!("({})", inner.join(", "))
            }
        }
    }
}

fn asset_file_name(block: &Block, format: AssetFormat, used: &mut HashSet<String>) -> String {
    let sanitize: String = block
        .id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = sanitize.trim_matches('.').to_string();
    let mut name = format!("{base}.{}", format.extension());
    let mut n = 1;
    while !used.insert(name.clone()) {
        n += 1;
        name = format!("{base}-{n}.{}", format.extension());
    }
    name
}

/// File name for an inline image asset: derived from the URL's last path
/// segment, deduplicated against `used`.
fn image_file_name(url: &str, format: AssetFormat, used: &mut HashSet<String>) -> String {
    let segment = url.rsplit(['/', '\\']).next().unwrap_or(url);
    let stem = segment.rsplit_once('.').map(|(s, _)| s).unwrap_or(segment);
    let sanitized: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = sanitized.trim_matches('.').to_string();
    let base = if base.is_empty() {
        "image".to_string()
    } else {
        base
    };
    let mut name = format!("{base}.{}", format.extension());
    let mut n = 1;
    while !used.insert(name.clone()) {
        n += 1;
        name = format!("{base}-{n}.{}", format.extension());
    }
    name
}

/// Typst label names allow alphanumerics plus `_-.` and `:`; anything else
/// becomes `_` so authored ids can never break label syntax.
fn typst_label(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Internal link markup to a docgen anchor (block id or `fig-…` figure
/// anchor).
fn internal_link(anchor: &str) -> String {
    format!("#link(<{}>)", typst_label(anchor))
}

/// Inline content → typst markup. Images with bytes in `assets` become
/// `#image("assets/<name>")`; unknown ones keep their alt text visible
/// (linked), so reviews catch missing files instead of silent gaps.
/// Links to `#fragment` URLs become internal label links; everything else
/// is an external `#link("url")`.
fn inline_markup(
    inlines: &[Inline],
    assets: &AssetMap,
    files: &mut Vec<(String, Vec<u8>)>,
    used_names: &mut HashSet<String>,
) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => out.push_str(&esc(t)),
            Inline::Code(t) => {
                if t.contains('`') {
                    out.push_str(&format!("`` {t} ``"));
                } else {
                    out.push_str(&format!("`{t}`"));
                }
            }
            Inline::Strong(inner) => {
                out.push_str(&format!(
                    "*{}*",
                    inline_markup(inner, assets, files, used_names)
                ));
            }
            Inline::Emphasis(inner) => {
                out.push_str(&format!(
                    "_{}_",
                    inline_markup(inner, assets, files, used_names)
                ));
            }
            Inline::Strikethrough(inner) => {
                out.push_str(&format!(
                    "#text(strike: true)[{}]",
                    inline_markup(inner, assets, files, used_names)
                ));
            }
            Inline::Link { url, text } if url.starts_with('#') => {
                let anchor = resolve_anchor(url.trim_start_matches('#'));
                out.push_str(&format!(
                    "{}[{}]",
                    internal_link(&anchor),
                    inline_markup(text, assets, files, used_names)
                ));
            }
            Inline::Link { url, text } => {
                out.push_str(&format!(
                    "#link(\"{}\")[{}]",
                    esc_str(url),
                    inline_markup(text, assets, files, used_names)
                ));
            }
            Inline::Image { url, alt } => match assets.get(url) {
                Some(asset) => {
                    let name = image_file_name(url, asset.format, used_names);
                    files.push((format!("assets/{name}"), asset.bytes.clone()));
                    out.push_str(&format!("#image(\"assets/{name}\")"));
                }
                None => {
                    // No bytes (missing file, external URL, ...): keep the
                    // alt text visible and linked.
                    out.push_str(&format!(
                        "#link(\"{}\")[{}]",
                        esc_str(url),
                        inline_markup(alt, assets, files, used_names)
                    ));
                }
            },
            Inline::SoftBreak => out.push(' '),
            Inline::HardBreak => out.push_str("\\ "),
        }
    }
    out
}

/// Escapes typst markup specials so arbitrary author text is inert.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '#' | '*' | '_' | '`' | '[' | ']' | '$' | '<' | '>' | '"' | '\'' | '@' | '/'
            | '%' | '^' | '~' | '&' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Escapes a typst string literal body.
fn esc_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Picks a backtick fence longer than any backtick run in `code`.
fn fence_for(code: &str) -> String {
    let mut max_run = 0;
    let mut cur = 0;
    for c in code.chars() {
        if c == '`' {
            cur += 1;
            max_run = max_run.max(cur);
        } else {
            cur = 0;
        }
    }
    "`".repeat(max_run.max(2) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use docgen_core::assets::RenderedAsset;

    #[test]
    fn escapes_markup() {
        assert_eq!(esc("a#b"), "a\\#b");
        assert_eq!(esc("[x]"), "\\[x\\]");
    }

    #[test]
    fn fence_grows() {
        assert_eq!(fence_for("plain"), "```");
        assert_eq!(fence_for("a ``` b"), "````");
    }

    #[test]
    fn meta_serializes_typst_literals() {
        let doc = docgen_core::parse_document(
            "---\ntitle: T\nanswer: 42\nratio: 1.5\nflags: [1, 2]\nnested:\n  k: v\n---\n\nx\n",
        )
        .unwrap();
        let meta = emit_meta(&doc);
        assert!(meta.contains("\"title\": \"T\""), "{meta}");
        assert!(meta.contains("\"answer\": 42"), "{meta}");
        assert!(meta.contains("\"ratio\": 1.5"), "{meta}");
        assert!(meta.contains("(1, 2,)"), "{meta}");
        assert!(meta.contains("\"nested\": (\"k\": \"v\")"), "{meta}");
    }

    #[test]
    fn media_map_flows_into_meta() {
        let doc = docgen_core::parse_document(
            "---\ntitle: T\nmedia:\n  placement: appendix\n  index: true\n---\n\nx\n",
        )
        .unwrap();
        let meta = emit_meta(&doc);
        assert!(meta.contains("\"media\": ("), "{meta}");
        assert!(meta.contains("\"placement\": \"appendix\""), "{meta}");
        assert!(meta.contains("\"index\": true"), "{meta}");
    }

    fn named_png() -> RenderedAsset {
        RenderedAsset {
            format: AssetFormat::Png,
            bytes: vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        }
    }

    /// A real decodable 1x1 white PNG, for tests that compile.
    fn tiny_png() -> RenderedAsset {
        RenderedAsset {
            format: AssetFormat::Png,
            bytes: vec![
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49,
                0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
                0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44,
                0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0x0F, 0x04, 0x00, 0x09, 0xFB, 0x03, 0xFD,
                0xFB, 0x5E, 0x6B, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
                0x42, 0x60, 0x82,
            ],
        }
    }

    fn body_of(out: &Emitted) -> &str {
        &out.sources
            .iter()
            .find(|(name, _)| name == BODY_PATH)
            .expect("body source")
            .1
    }

    #[test]
    fn inline_image_embeds_and_registers_the_file() {
        let doc = docgen_core::parse_document("Text ![logo](img/logo.png) more.\n").unwrap();
        let mut assets = AssetMap::new();
        assets.insert("img/logo.png".to_string(), named_png());
        let out = emit(&doc, &assets, None);
        assert!(
            body_of(&out).contains("#image(\"assets/logo.png\")"),
            "{}",
            body_of(&out)
        );
        assert!(out.files.iter().any(|(name, _)| name == "assets/logo.png"));
    }

    #[test]
    fn image_only_paragraph_becomes_centered_figure() {
        // Empty alt text → no caption → exactly the plain rendering.
        let doc = docgen_core::parse_document("![](img/hero.png)\n").unwrap();
        let mut assets = AssetMap::new();
        assets.insert("img/hero.png".to_string(), named_png());
        let out = emit(&doc, &assets, None);
        assert!(
            body_of(&out).contains("#align(center)[#image(\"assets/hero.png\", width: 92%)]"),
            "{}",
            body_of(&out)
        );
        assert!(!body_of(&out).contains("Figure"), "{}", body_of(&out));
    }

    #[test]
    fn captioned_figure_carries_number_caption_label_and_keep() {
        let doc = docgen_core::parse_document("![The hero](img/hero.png){#fig:hero}\n").unwrap();
        let mut assets = AssetMap::new();
        assets.insert("img/hero.png".to_string(), named_png());
        let out = emit(&doc, &assets, None);
        let body = body_of(&out);
        assert!(body.contains("#block(breakable: false)["), "{body}");
        assert!(body.contains("Figure 1: The hero"), "{body}");
        assert!(body.contains("<fig-hero>"), "{body}");
    }

    #[test]
    fn keep_false_relaxes_the_figure_block() {
        let doc = docgen_core::parse_document("![The hero](img/hero.png){#fig:hero keep=false}\n")
            .unwrap();
        let mut assets = AssetMap::new();
        assets.insert("img/hero.png".to_string(), named_png());
        let out = emit(&doc, &assets, None);
        let body = body_of(&out);
        assert!(!body.contains("breakable: false"), "{body}");
        assert!(body.contains("Figure 1: The hero"), "{body}");
    }

    #[test]
    fn captioned_diagram_is_a_numbered_figure() {
        let doc =
            docgen_core::parse_document("```mermaid {#arch} caption=\"System\"\ngraph TD\n```\n")
                .unwrap();
        let mut assets = AssetMap::new();
        assets.insert("arch".to_string(), named_png());
        let out = emit(&doc, &assets, None);
        let body = body_of(&out);
        assert!(body.contains("Figure 1: System"), "{body}");
        assert!(body.contains("<fig-arch>"), "{body}");
    }

    #[test]
    fn headings_carry_labels_and_internal_links_target_them() {
        let doc = docgen_core::parse_document(
            "# Overview {#ov}\n\nSee [details](#ov) and [arch](#fig:arch).\n",
        )
        .unwrap();
        let out = emit(&doc, &Default::default(), None);
        let body = body_of(&out);
        assert!(body.contains("= Overview <ov>\n\n"), "{body}");
        assert!(body.contains("#link(<ov>)[details]"), "{body}");
        assert!(body.contains("#link(<fig-arch>)[arch]"), "{body}");
    }

    #[test]
    fn external_links_stay_string_targets() {
        let doc = docgen_core::parse_document("[site](https://x.example)\n").unwrap();
        let out = emit(&doc, &Default::default(), None);
        assert!(
            body_of(&out).contains("#link(\"https://x.example\")[site]"),
            "{}",
            body_of(&out)
        );
    }

    #[test]
    fn missing_image_keeps_linked_alt_text() {
        let doc = docgen_core::parse_document("![gone](img/gone.png)\n").unwrap();
        let out = emit(&doc, &AssetMap::new(), None);
        assert!(
            body_of(&out).contains("#link(\"img/gone.png\")[gone]"),
            "{}",
            body_of(&out)
        );
    }

    /// Dangling labels are typst errors, so compiling a doc whose links
    /// target emitted labels and figure anchors proves the wiring.
    #[test]
    fn internal_links_compile_against_emitted_labels() {
        let doc = docgen_core::parse_document(
            "```mermaid {#arch} caption=\"System\"\ngraph TD\n```\n\nSee [the diagram](#fig:arch) and [intro](#intro).\n\n# Intro {#intro}\n",
        )
        .unwrap();
        let mut assets = AssetMap::new();
        assets.insert("arch".to_string(), tiny_png());
        let (pdf, warnings) = crate::TypstRenderer::new_embedded()
            .render_pdf(&doc, &assets)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
