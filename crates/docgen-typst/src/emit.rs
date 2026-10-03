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
//! blocks become `#image(...)` references into the world's virtual files.

use std::collections::HashSet;

use docgen_core::assets::{AssetFormat, AssetMap};
use docgen_core::model::{Block, BlockKind, Document, Inline, ListItem};

/// Names of the virtual files the template can address.
pub const BODY_PATH: &str = "docgen-body.typ";
pub const META_PATH: &str = "docgen-meta.typ";

/// The built-in template: plain A4 article with a centered cover.
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

#include "docgen-body.typ"
"##;

/// Everything the in-memory world needs to compile one document.
pub(crate) struct Emitted {
    /// The main source (template).
    pub main: String,
    /// Additional typst sources addressable by relative path.
    pub sources: Vec<(String, String)>,
    /// Binary virtual files (rendered diagrams).
    pub files: Vec<(String, Vec<u8>)>,
}

/// `template` replaces the built-in default; it can `#import`/`#include` the
/// virtual files above.
pub(crate) fn emit(doc: &Document, assets: &AssetMap, template: Option<&str>) -> Emitted {
    let mut body = String::with_capacity(4096);
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut used_names: HashSet<String> = Default::default();

    for block in &doc.blocks {
        emit_block(block, assets, &mut files, &mut used_names, &mut body);
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
        files,
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
    field(
        "title",
        &opt_str(&fm.title),
        &mut out,
    );
    field("subtitle", &opt_str(&fm.subtitle), &mut out);
    field("authors", &serde_json::Value::Array(
        fm.authors
            .iter()
            .map(|a| serde_json::Value::String(a.clone()))
            .collect(),
    ), &mut out);
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

fn emit_block(
    block: &Block,
    assets: &AssetMap,
    files: &mut Vec<(String, Vec<u8>)>,
    used_names: &mut HashSet<String>,
    out: &mut String,
) {
    match &block.kind {
        BlockKind::Heading { level, inline } => {
            let hashes = "=".repeat((*level).clamp(1, 6) as usize);
            out.push_str(&format!("{hashes} {}\n\n", inline_markup(inline)));
        }
        BlockKind::Paragraph { inline } => {
            out.push_str(&format!("{}\n\n", inline_markup(inline)));
        }
        BlockKind::CodeBlock { language, code } => {
            let fence = fence_for(code);
            let lang = language.as_deref().unwrap_or("");
            out.push_str(&format!("{fence}{lang}\n{code}\n{fence}\n\n"));
        }
        BlockKind::Diagram { .. } => {
            match assets.get(&block.id) {
                Some(asset) => {
                    let name = asset_file_name(block, asset.format, used_names);
                    files.push((format!("assets/{name}"), asset.bytes.clone()));
                    out.push_str(&format!(
                        "#align(center)[#image(\"assets/{name}\", width: 92%)]\n\n"
                    ));
                }
                None => {
                    // No engine for this diagram (yet): keep it visible so
                    // reviews catch silent degradation.
                    let src = match &block.kind {
                        BlockKind::Diagram { source, .. } => source.clone(),
                        _ => unreachable!(),
                    };
                    let fence = fence_for(&src);
                    out.push_str(
                        "#block(fill: rgb(255, 244, 229), inset: 8pt, radius: 4pt)[\n  #text(size: 0.8em)[#box[diagram not rendered]]\n",
                    );
                    out.push_str(&format!("  {fence}\n{src}\n  {fence}\n]\n\n"));
                }
            }
        }
        BlockKind::List { ordered, start, items } => {
            emit_list(*ordered, *start, items, 0, out);
            out.push('\n');
        }
        BlockKind::Table { rows } => {
            emit_table(rows, out);
        }
        BlockKind::BlockQuote { blocks } => {
            out.push_str("#quote(block: true)[\n");
            for child in blocks {
                if let BlockKind::Paragraph { inline } = &child.kind {
                    out.push_str(&format!("  {}\n\n", inline_markup(inline)));
                }
            }
            out.push_str("]\n\n");
        }
        BlockKind::ThematicBreak => {
            out.push_str("#line(length: 100%)\n\n");
        }
    }
}

fn emit_list(ordered: bool, start: u64, items: &[ListItem], depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    for (i, item) in items.iter().enumerate() {
        let marker = if ordered {
            format!("{}.", start + i as u64)
        } else {
            "-".to_string()
        };
        out.push_str(&format!(
            "{indent}{marker} {}\n",
            inline_markup(&item.content)
        ));
        for child in &item.children {
            if let BlockKind::List { ordered, start, items } = &child.kind {
                emit_list(*ordered, *start, items, depth + 1, out);
            }
        }
    }
}

fn emit_table(rows: &[docgen_core::model::TableRow], out: &mut String) {
    let Some(first) = rows.first() else { return };
    let cols = first.cells.len().max(1);
    out.push_str(&format!("#table(columns: {cols},\n"));
    for (ri, row) in rows.iter().enumerate() {
        let cells: Vec<String> = row
            .cells
            .iter()
            .map(|c| format!("[{}]", inline_markup(c)))
            .collect();
        if ri == 0 && row.is_header {
            out.push_str(&format!("  table.header({}),\n", cells.join(", ")));
        } else {
            out.push_str(&format!("  {},\n", cells.join(", ")));
        }
    }
    out.push_str(")\n\n");
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

/// Inline content → typst markup.
fn inline_markup(inlines: &[Inline]) -> String {
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
                out.push_str(&format!("*{}*", inline_markup(inner)));
            }
            Inline::Emphasis(inner) => {
                out.push_str(&format!("_{}_", inline_markup(inner)));
            }
            Inline::Strikethrough(inner) => {
                out.push_str(&format!("#text(strike: true)[{}]", inline_markup(inner)));
            }
            Inline::Link { url, text } => {
                out.push_str(&format!(
                    "#link(\"{}\")[{}]",
                    esc_str(url),
                    inline_markup(text)
                ));
            }
            Inline::Image { url, alt } => {
                // Local image files are resolved by the host in a later
                // milestone; until then keep the alt text visible and linked.
                out.push_str(&format!(
                    "#link(\"{}\")[{}]",
                    esc_str(url),
                    inline_markup(alt)
                ));
            }
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
            '\\' | '#' | '*' | '_' | '`' | '[' | ']' | '$' | '<' | '>' | '"' | '\'' | '@'
            | '/' | '%' | '^' | '~' | '&' => {
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
}
