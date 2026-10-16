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
use docgen_core::media::{FigurePlan, MediaPlan, fig_anchor, page_spans, resolve_anchor};
use docgen_core::model::{Block, BlockKind, Document, Inline, ListItem, Orientation, Placement};

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
        page_ctl: template.is_none(),
    };

    let mut body = String::with_capacity(4096);
    let mut appendix = String::new();
    let mut appendix_count = 0usize;
    // Page controls ride the default template only — custom templates own
    // their page setup (D-006).
    let page_ctl = template.is_none();
    let spans = page_spans(doc);
    let mut orientation = Orientation::Portrait;
    for (i, block) in doc.blocks.iter().enumerate() {
        // Media placed in the appendix leaves a "Figure N" reference at its
        // original position and renders in the back matter (D-003, D-010).
        // Uncaptioned media relocates without a reference.
        if let Some(fp) = em.plan.figure(&block.id)
            && fp.placement == Placement::Appendix
        {
            if let Some(label) = fp.label() {
                body.push_str(&format!(
                    "{}[{}]

",
                    internal_link(&fig_anchor(&block.id)),
                    esc(&label)
                ));
            }
            let image = block.kind.lone_image();
            em.figure_block(block, image, spans[i], &mut appendix);
            appendix_count += 1;
            continue;
        }
        if page_ctl {
            let want = spans[i];
            // A figure's `page=` always forces its own page (D-010 (3));
            // when the geometry changes, typst's page set rule breaks
            // anyway, so an explicit break is only needed within one
            // orientation.
            let forced = block.annotations.new_page
                || (block.annotations.page.is_some()
                    && (block.kind.lone_image().is_some() || block.kind.is_diagram()));
            if forced && want == orientation {
                body.push_str("#pagebreak()\n\n");
            }
            if want != orientation {
                let flipped = matches!(want, Orientation::Landscape);
                body.push_str(&format!("#set page(flipped: {flipped})\n\n"));
                orientation = want;
            }
        }
        em.block(block, spans[i], &mut body);
    }
    em.back_matter(&mut body, &appendix, appendix_count, orientation);

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
    /// Page controls and fit-to-page ride the default template only, where
    /// the content box is known (D-006, D-007).
    page_ctl: bool,
}

impl<'a> Emitter<'a> {
    fn block(&mut self, block: &Block, orientation: Orientation, out: &mut String) {
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
                Some((url, alt)) => self.figure_block(block, Some((url, alt)), orientation, out),
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
                self.figure_block(block, None, orientation, out);
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
    fn figure_block(
        &mut self,
        block: &Block,
        image: Option<(&str, &[Inline])>,
        orientation: Orientation,
        out: &mut String,
    ) {
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
        let img = self.image_spec(block, asset, name, orientation);

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

impl<'a> Emitter<'a> {
    /// Appends the media back matter (D-003, D-010): the figures appendix
    /// (content already rendered into `appendix` during the body walk) and
    /// the figures index with page numbers. Both start on a fresh page and
    /// inherit the document orientation — an open landscape section is
    /// closed here explicitly.
    fn back_matter(
        &self,
        body: &mut String,
        appendix: &str,
        appendix_count: usize,
        orientation: Orientation,
    ) {
        let index_entries: Vec<&FigurePlan> = self.plan.index_figures().collect();
        let with_index = self.plan.index_requested && !index_entries.is_empty();
        if appendix_count == 0 && !with_index {
            return;
        }
        if orientation != Orientation::Portrait {
            // The page set rule starts the fresh page itself.
            body.push_str(
                "#set page(flipped: false)

",
            );
        } else {
            body.push_str(
                "#pagebreak()

",
            );
        }
        if appendix_count > 0 {
            body.push_str(
                "#heading(numbering: none, level: 1)[Figures]

",
            );
            body.push_str(appendix);
        }
        if with_index {
            if appendix_count > 0 {
                body.push_str(
                    "#pagebreak()

",
                );
            }
            body.push_str(
                "#heading(numbering: none, level: 1)[List of Figures]

",
            );
            for f in index_entries {
                body.push_str(&self.index_entry(f));
            }
        }
    }

    /// One index line: "Figure N: caption", filled to a right-aligned page
    /// number resolved from the figure's label.
    fn index_entry(&self, f: &FigurePlan) -> String {
        let anchor = typst_label(&fig_anchor(&f.block_id));
        format!(
            "#context[#link(<{anchor}>)[Figure {n}]: {caption} #h(1fr) p. #query(<{anchor}>).first().location().page()]

",
            anchor = anchor,
            n = f.number.unwrap_or_default(),
            caption = esc(f.caption.as_deref().unwrap_or("")),
        )
    }
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

/// Default-template content box: A4 (595.276 x 841.890 pt) minus typst's
/// 2.5 cm default margins — the geometry the fit-to-page computation and
/// the shrink warning rely on (D-007). Custom templates are not fitted
/// (their geometry is unknown).
const CONTENT_W_PT: f64 = 453.543;
const CONTENT_H_PT: f64 = 700.157;
/// Horizontal share a media image may take (the historical 92% width).
const MEDIA_W_SHARE: f64 = 0.92;
/// Vertical allowance for the caption line and its spacing above it.
const CAPTION_RESERVE_PT: f64 = 30.0;
/// Warn when a figure must shrink below this fraction of natural size
/// (compile-time constant, D-007).
const SHRINK_WARN_FLOOR: f64 = 0.5;

impl<'a> Emitter<'a> {
    /// The `#image(...)` markup for a media asset: scaled to fit the page
    /// content box (width *and* height) when its natural size overflows,
    /// otherwise the historical 92% width. A scale below
    /// [`SHRINK_WARN_FLOOR`] produces a deterministic warning.
    fn image_spec(
        &mut self,
        block: &Block,
        asset: &docgen_core::assets::RenderedAsset,
        name: String,
        orientation: Orientation,
    ) -> String {
        let plain = || format!("#align(center)[#image(\"assets/{name}\", width: 92%)]");
        if !self.page_ctl {
            return plain();
        }
        let Some((nat_w, nat_h)) = natural_size(&asset.bytes, asset.format) else {
            return plain();
        };
        if nat_w <= 0.0 || nat_h <= 0.0 {
            return plain();
        }
        let (avail_w, avail_h) = match orientation {
            Orientation::Portrait => (CONTENT_W_PT, CONTENT_H_PT),
            Orientation::Landscape => (CONTENT_H_PT, CONTENT_W_PT),
        };
        let captioned = self
            .plan
            .figure(&block.id)
            .and_then(|f| f.caption.as_ref())
            .is_some();
        let reserve = if captioned { CAPTION_RESERVE_PT } else { 0.0 };
        let max_w = MEDIA_W_SHARE * avail_w;
        let max_h = (MEDIA_W_SHARE * avail_h - reserve).max(0.0);
        let scale = (max_w / nat_w).min(max_h / nat_h).min(1.0);
        if scale >= 1.0 {
            return plain();
        }
        if scale < SHRINK_WARN_FLOOR {
            self.warnings.push(format!(
                "figure '{}' scales to {}% of its natural size to fit the page",
                block.id,
                (scale * 100.0).round() as i64
            ));
        }
        format!(
            "#align(center)[#image(\"assets/{name}\", width: {:.2}pt)]",
            nat_w * scale
        )
    }
}

/// Natural size in pt. typst sizes raster pixels and SVG user units 1:1 as
/// pt (verified empirically for this embedded version).
fn natural_size(bytes: &[u8], format: AssetFormat) -> Option<(f64, f64)> {
    match format {
        AssetFormat::Svg => svg_natural_size(bytes),
        f => docgen_core::assets::image_dimensions(bytes, f).map(|(w, h)| (w as f64, h as f64)),
    }
}

/// Natural size of an SVG in pt, from its root `width`/`height`
/// attributes (falling back to the `viewBox` extents when both are
/// missing). `None` when unparseable — the caller then keeps the plain
/// percentage width.
fn svg_natural_size(bytes: &[u8]) -> Option<(f64, f64)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let start = text.find("<svg")?;
    let root = &text[start..text.len().min(start + 2048)];
    let root = &root[..root.find('>')?];
    let width = svg_root_attr(root, "width").and_then(|v| parse_svg_len(&v));
    let height = svg_root_attr(root, "height").and_then(|v| parse_svg_len(&v));
    if let (Some(w), Some(h)) = (width, height) {
        return Some((w, h));
    }
    if width.is_none() && height.is_none() {
        let vb = svg_root_attr(root, "viewBox")?;
        let nums: Vec<f64> = vb
            .split([',', ' '])
            .filter_map(|t| t.trim().parse().ok())
            .collect();
        if nums.len() == 4 {
            return Some((nums[2], nums[3]));
        }
    }
    None
}

fn svg_root_attr(root: &str, name: &str) -> Option<String> {
    let i = root.find(&format!("{name}="))?;
    let rest = &root[i + name.len() + 1..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_owned())
}

/// SVG length in pt: unitless and `px` are typst user units (1 pt), the
/// absolute CSS units convert.
fn parse_svg_len(value: &str) -> Option<f64> {
    let value = value.trim();
    let split = value
        .find(|c: char| c.is_ascii_alphabetic() || c == '%')
        .unwrap_or(value.len());
    let n: f64 = value[..split].trim().parse().ok()?;
    Some(match value[split..].trim() {
        "" | "px" | "pt" => n,
        "in" => n * 72.0,
        "cm" => n * 72.0 / 2.54,
        "mm" => n * 72.0 / 25.4,
        "pc" => n * 12.0,
        "q" => n * 72.0 / 101.6,
        _ => return None,
    })
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
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
                0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78,
                0x9C, 0x63, 0xF8, 0x0F, 0x04, 0x00, 0x09, 0xFB, 0x03, 0xFD, 0xFB, 0x5E, 0x6B, 0x2B,
                0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
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

    /// Landscape sections produce genuinely flipped page geometry (D-005,
    /// D-010); content is never rotated.
    #[test]
    fn landscape_sections_flip_page_geometry() {
        let doc = docgen_core::parse_document(
            "# Portrait {#p}

Text.

# Wide {#w page=landscape}

Wide text.

## Deep {#d}

Still wide.

# Back {#b}

Portrait again.
",
        )
        .unwrap();
        let (paged, warnings) = crate::TypstRenderer::new_embedded()
            .compile(&doc, &Default::default())
            .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let sizes: Vec<String> = paged
            .pages()
            .iter()
            .map(|p| format!("{:?}", p.frame.size()))
            .collect();
        assert!(
            sizes
                .iter()
                .any(|s| s.contains("841.8897637795276pt, 595.2755905511812pt")),
            "expected a landscape page: {sizes:?}"
        );
        // The section ends: the last page is portrait again.
        assert!(
            sizes
                .last()
                .unwrap()
                .contains("595.2755905511812pt, 841.8897637795276pt"),
            "expected portrait after the section: {sizes:?}"
        );
    }

    #[test]
    fn new_page_emits_pagebreak() {
        let doc = docgen_core::parse_document(
            "A.

## C {new-page}

D.
",
        )
        .unwrap();
        let out = emit(&doc, &Default::default(), None);
        assert!(body_of(&out).contains("#pagebreak()"), "{}", body_of(&out));
    }

    #[test]
    fn custom_templates_get_no_page_controls() {
        let doc = docgen_core::parse_document(
            "# H {page=landscape new-page}

T.
",
        )
        .unwrap();
        let out = emit(
            &doc,
            &Default::default(),
            Some(
                "#include \"docgen-body.typ\"
",
            ),
        );
        let body = body_of(&out);
        assert!(!body.contains("pagebreak"), "{body}");
        assert!(!body.contains("flipped"), "{body}");
    }

    #[test]
    fn heading_spans_cover_their_section_only() {
        let doc = docgen_core::parse_document(
            "# A {page=landscape}

a.

## A1

a1.

# B

b.
",
        )
        .unwrap();
        let out = emit(&doc, &Default::default(), None);
        let body = body_of(&out);
        let flipped_on = body.matches("#set page(flipped: true)").count();
        let flipped_off = body.matches("#set page(flipped: false)").count();
        assert_eq!((flipped_on, flipped_off), (1, 1), "{body}");
        // The landscape set page comes before A's content and the flip back
        // before B's heading line.
        let b_pos = body.find("= B <b>").unwrap();
        let flip_back = body.find("#set page(flipped: false)").unwrap();
        assert!(flip_back < b_pos, "{body}");
    }

    #[test]
    fn svg_natural_size_parses_units_and_viewbox() {
        let svg = |attrs: &str| {
            format!(
                "<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\" {attrs}><rect/></svg>"
            )
            .into_bytes()
        };
        assert_eq!(
            svg_natural_size(&svg("width=\"400\" height=\"200\"")),
            Some((400.0, 200.0))
        );
        assert_eq!(
            svg_natural_size(&svg("width=\"400px\" height=\"200px\"")),
            Some((400.0, 200.0))
        );
        assert_eq!(
            svg_natural_size(&svg("width=\"10mm\" height=\"1in\"")),
            Some((10.0 * 72.0 / 25.4, 72.0))
        );
        assert_eq!(
            svg_natural_size(&svg("viewBox=\"0 0 640 480\"")),
            Some((640.0, 480.0))
        );
        assert_eq!(svg_natural_size(&svg("width=\"100%\"")), None);
    }

    /// An oversized tall diagram must render fully inside the page content
    /// box (width *and* height) — verified against the compiled frame (D-007).
    /// Scale lands at ~0.68, above the 0.5 warning floor.
    #[test]
    fn oversized_figure_fits_the_page() {
        let doc = docgen_core::parse_document(
            "![Tall](tall.svg){#fig:tall}
",
        )
        .unwrap();
        let mut assets = AssetMap::new();
        assets.insert(
            "tall.svg".to_string(),
            RenderedAsset {
                format: AssetFormat::Svg,
                bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="900"><rect width="400" height="900"/></svg>"#
                    .to_vec(),
            },
        );
        let (paged, warnings) = crate::TypstRenderer::new_embedded()
            .compile(&doc, &assets)
            .unwrap();
        assert_eq!(paged.pages().len(), 1, "must fit one portrait page");
        let sizes = frame_image_sizes(&paged);
        assert_eq!(sizes.len(), 1, "{sizes:?}");
        let (w, h) = sizes[0];
        // Captioned: max_h = 0.92*700.157 - 30 = 614.14; scale = 614.14/900
        // = 0.682 → 272.9 x 614.1 pt.
        assert!(
            w > 272.0 && w < 274.0 && h > 613.0 && h < 615.0,
            "expected the scaled fit, got {w}x{h} (content box 453.5x700.2)"
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// A deliberately huge figure is scaled below the 0.5 floor and produces
    /// the deterministic shrink warning.
    #[test]
    fn huge_figure_warns_below_the_shrink_floor() {
        let doc = docgen_core::parse_document(
            "![Huge](huge.svg){#fig:huge}
",
        )
        .unwrap();
        let mut assets = AssetMap::new();
        assets.insert(
            "huge.svg".to_string(),
            RenderedAsset {
                format: AssetFormat::Svg,
                bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="2000" height="4000"><rect width="2000" height="4000"/></svg>"#
                    .to_vec(),
            },
        );
        let renderer = crate::TypstRenderer::new_embedded();
        let (pdf, warnings) = renderer.render_pdf(&doc, &assets).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(
            warnings
                .iter()
                .any(|w| { w.contains("figure 'huge'") && w.contains("scales to 15%") }),
            "{warnings:?}"
        );
    }

    #[test]
    fn small_figures_keep_the_percentage_width() {
        let doc = docgen_core::parse_document(
            "![Small](s.svg){#fig:s}
",
        )
        .unwrap();
        let mut assets = AssetMap::new();
        assets.insert(
            "s.svg".to_string(),
            RenderedAsset {
                format: AssetFormat::Svg,
                bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="80"><rect width="100" height="80"/></svg>"#
                    .to_vec(),
            },
        );
        let out = emit(&doc, &assets, None);
        assert!(body_of(&out).contains("width: 92%"), "{}", body_of(&out));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn landscape_section_fits_against_swapped_geometry() {
        let doc = docgen_core::parse_document(
            "# Wide {#w page=landscape}

![Tall](tall.svg){#fig:tall}
",
        )
        .unwrap();
        let mut assets = AssetMap::new();
        assets.insert(
            "tall.svg".to_string(),
            RenderedAsset {
                format: AssetFormat::Svg,
                bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="3000"><rect width="100" height="3000"/></svg>"#
                    .to_vec(),
            },
        );
        let out = emit(&doc, &assets, None);
        // Captioned landscape: max_h = 0.92*453.543 - 30 = 387.26;
        // scale = 387.26/3000 = 12.91pt wide.
        assert!(
            body_of(&out).contains("width: 12.91pt"),
            "{}",
            body_of(&out)
        );
    }

    fn frame_image_sizes(paged: &typst_layout::PagedDocument) -> Vec<(f64, f64)> {
        fn walk(frame: &typst::layout::Frame, out: &mut Vec<(f64, f64)>) {
            for (_pos, item) in frame.items() {
                match item {
                    typst::layout::FrameItem::Group(g) => walk(&g.frame, out),
                    typst::layout::FrameItem::Image(_, size, _) => {
                        out.push((size.x.to_pt(), size.y.to_pt()))
                    }
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for page in paged.pages() {
            walk(&page.frame, &mut out);
        }
        out
    }

    #[test]
    fn appendix_mode_relocates_and_leaves_references() {
        let doc = docgen_core::parse_document(
            "---
title: T
media:
  placement: appendix
---

Intro.

```mermaid {#d1} caption=\"One\"
graph TD
```

![Two](t.png){#fig:t}
",
        )
        .unwrap();
        let mut assets = AssetMap::new();
        assets.insert("d1".to_string(), named_png());
        assets.insert("t.png".to_string(), named_png());
        let out = emit(&doc, &assets, None);
        let body = body_of(&out);
        // References at the original positions...
        assert!(body.contains("#link(<fig-d1>)[Figure 1]"), "{body}");
        assert!(body.contains("#link(<fig-t>)[Figure 2]"), "{body}");
        // ...and the figures themselves only in the back matter, after the
        // appendix heading, in document order.
        let appendix_at = body
            .find("#heading(numbering: none, level: 1)[Figures]")
            .expect("appendix heading");
        let tail = &body[appendix_at..];
        let d1_at = tail.find("<fig-d1>").expect("d1 label");
        let t_at = tail.find("<fig-t>").expect("t label");
        assert!(appendix_at < d1_at && d1_at < t_at, "{body}");
        assert!(
            !body[..appendix_at].contains("Figure 1: One"),
            "figure content must not render at the inline position: {body}"
        );
    }

    #[test]
    fn figures_index_lists_numbered_figures_with_pages() {
        let doc = docgen_core::parse_document(
            "---
title: T
media:
  index: true
---

![Only](o.png){#fig:o}
",
        )
        .unwrap();
        let mut assets = AssetMap::new();
        assets.insert("o.png".to_string(), named_png());
        let out = emit(&doc, &assets, None);
        let body = body_of(&out);
        assert!(
            body.contains("#heading(numbering: none, level: 1)[List of Figures]"),
            "{body}"
        );
        assert!(
            body.contains(
                "#context[#link(<fig-o>)[Figure 1]: Only #h(1fr) p. #query(<fig-o>).first().location().page()]"
            ),
            "{body}"
        );
        // No appendix content → no appendix heading.
        assert!(!body.contains("[Figures]"), "{body}");
    }

    /// The back-matter markup (label links, location/page expressions) must
    /// compile — dangling labels or bad context code are typst errors.
    #[test]
    fn back_matter_compiles_with_working_links() {
        let doc = docgen_core::parse_document(
            "---
title: T
media:
  placement: appendix
  index: true
---

Intro.

```mermaid {#d1} caption=\"One\"
graph TD
```

More.

![Two](t.png){#fig:t}

# Wide {#w page=landscape}

Wide section after which the back matter resets orientation.
",
        )
        .unwrap();
        let mut assets = AssetMap::new();
        assets.insert("d1".to_string(), tiny_png());
        assets.insert("t.png".to_string(), tiny_png());
        let (paged, warnings) = crate::TypstRenderer::new_embedded()
            .compile(&doc, &assets)
            .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        // Cover/body + appendix pages + index page.
        assert!(paged.pages().len() >= 3, "{}", paged.pages().len());
        // The back matter closed the landscape section: the last page is
        // portrait (D-010: the appendix inherits the document orientation).
        let last = format!("{:?}", paged.pages().last().unwrap().frame.size());
        assert!(
            last.contains("595.2755905511812pt, 841.8897637795276pt"),
            "{last}"
        );
    }
}
