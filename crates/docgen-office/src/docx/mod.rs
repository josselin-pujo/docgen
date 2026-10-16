//! DOCX backend public API. See `document.rs` for emission and
//! `template.rs` for template filling.

pub(crate) mod document;
pub(crate) mod package;
pub(crate) mod parts;
mod template;
pub(crate) mod xml;

use docgen_core::Result as CoreResult;
use docgen_core::assets::AssetMap;
use docgen_core::diff::{self, DiffOptions};
use docgen_core::model::{Document, Frontmatter};

use crate::{Error, Result};

pub use template::fill_template;

/// Author/date attributed to tracked changes and comments in diff output.
/// There is deliberately no clock in this crate: pass an explicit date
/// (ISO-8601, e.g. `2026-10-03T12:00:00Z`), or the document frontmatter
/// date is used, else the epoch.
#[derive(Debug, Clone)]
pub struct RevisionIdentity {
    pub author: String,
    pub date: String,
}

impl RevisionIdentity {
    pub fn new(author: impl Into<String>, date: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            date: date.into(),
        }
    }

    pub fn from_frontmatter(author: impl Into<String>, fm: &Frontmatter) -> Self {
        let date = fm
            .date
            .as_deref()
            .map(|d| format!("{d}T00:00:00Z"))
            .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
        Self {
            author: author.into(),
            date,
        }
    }
}

/// Renders a document to a fresh `.docx` (no revision marks).
/// Diagram blocks must be present in `assets` as PNG bytes. A `toc: true`
/// frontmatter key synthesizes a static, internally linked table of
/// contents (heading titles, no field codes).
pub fn render_docx(doc: &Document, assets: &AssetMap) -> Result<Vec<u8>> {
    document::render(doc, assets, None).map_err(|e| match e {
        Error::Xml(m) | Error::Zip(m) | Error::Docx(m) | Error::Template(m) | Error::Pptx(m) => {
            Error::Docx(m)
        }
    })
}

/// Renders the diff between two documents as a `.docx` carrying native
/// tracked changes: equal content flows through unmarked, deletions are
/// `w:del`, insertions `w:ins`, at word/line/item/row granularity; moves and
/// diagram replacements additionally get anchored Word comments.
/// `old_assets`/`new_assets` supply PNG renders for deleted/inserted
/// diagrams respectively.
///
/// **Parked, not public API.** The implementation works and stays covered
/// by unit + golden tests, but it is deliberately unexposed (no CLI flag,
/// no wasm export, no `pub` here) until bidirectional updates — how a
/// reviewed/edited DOCX flows back into the markdown source — are designed.
/// Re-expose once that round-trip story is settled.
#[allow(dead_code)] // no non-test caller by design; see the park note above
pub(crate) fn render_diff_docx(
    old: &Document,
    new: &Document,
    old_assets: &AssetMap,
    new_assets: &AssetMap,
    identity: &RevisionIdentity,
) -> Result<Vec<u8>> {
    let plan = diff::plan_diff_with(old, new, &DiffOptions::default());
    document::render(
        new,
        new_assets,
        Some(document::RevisionMode {
            plan: &plan,
            old_assets,
            identity,
        }),
    )
}

/// Validates that a buffer looks like a docx we can process: a zip
/// containing `word/document.xml` that parses as XML.
pub fn validate_docx(bytes: &[u8]) -> CoreResult<()> {
    document::validate(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use docgen_core::assets::{AssetFormat, RenderedAsset};
    use docgen_core::parse_document;
    use std::collections::BTreeMap;

    /// In-crate twin of `tests/golden.rs::check` for the parked diff
    /// renderer (it is `pub(crate)`, so the integration test can't see it).
    /// Same golden files, same hashes — regenerate with
    /// `UPDATE_GOLDEN=1 cargo test`.
    fn golden(name: &str, bytes: &[u8]) {
        use sha2::{Digest, Sha256};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(format!("{name}.sha256"));
        let digest: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if std::env::var("UPDATE_GOLDEN").is_ok() {
            std::fs::write(&path, &digest).unwrap();
            eprintln!("updated golden {name}");
            return;
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("golden file for {name} missing; run UPDATE_GOLDEN=1"));
        assert_eq!(digest, expected.trim(), "{name} output changed");
    }

    /// Minimal valid 1x1 white PNG.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8,
        0x0F, 0x04, 0x00, 0x09, 0xFB, 0x03, 0xFD, 0xFB, 0x5E, 0x6B, 0x2B, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn png_asset() -> RenderedAsset {
        RenderedAsset {
            format: AssetFormat::Png,
            bytes: TINY_PNG.to_vec(),
        }
    }

    fn unzip_document(bytes: &[u8]) -> String {
        let mut zip =
            zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("valid zip");
        let mut xml = String::new();
        use std::io::Read;
        zip.by_name("word/document.xml")
            .expect("document.xml")
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    #[test]
    fn fresh_render_is_valid_and_deterministic() {
        let doc = parse_document(
            "---\ntitle: T\nauthors: [a]\n---\n\n# Head {#h}\n\nText with **bold**.\n",
        )
        .unwrap();
        let first = render_docx(&doc, &BTreeMap::new()).unwrap();
        let second = render_docx(&doc, &BTreeMap::new()).unwrap();
        assert_eq!(first, second, "output must be byte-stable");
        validate_docx(&first).unwrap();

        let xml = unzip_document(&first);
        assert!(xml.contains("w:pStyle w:val=\"Heading1\""), "heading style");
        assert!(xml.contains("<w:b/>"), "bold run");
        assert!(xml.contains("T"), "title text present");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(first.clone())).unwrap();
        for part in [
            "word/styles.xml",
            "word/numbering.xml",
            "word/settings.xml",
            "word/fontTable.xml",
            "[Content_Types].xml",
            "_rels/.rels",
        ] {
            assert!(zip.by_name(part).is_ok(), "missing {part}");
        }
    }

    #[test]
    fn toc_synthesizes_linked_contents() {
        let doc = parse_document(
            "---\ntitle: T\ntoc: true\n---\n\n# First {#first}\n\nText.\n\n## Second {#second}\n\nMore.\n\n### Deep {#deep}\n\nMore still.\n\n#### Too deep {#deep4}\n\nNot listed.\n",
        )
        .unwrap();
        let bytes = render_docx(&doc, &BTreeMap::new()).unwrap();
        validate_docx(&bytes).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("Contents"), "contents heading");
        assert_eq!(xml.matches("<w:bookmarkStart ").count(), 3, "bookmarks");
        assert_eq!(xml.matches("<w:hyperlink w:anchor=").count(), 3, "links");
        assert!(xml.contains("w:name=\"first\""), "anchor names");
        assert!(xml.contains("<w:rStyle w:val=\"Hyperlink\"/>"));
        assert!(!xml.contains("w:name=\"deep4\""), "level 4 is not listed");

        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut styles = String::new();
        use std::io::Read;
        zip.by_name("word/styles.xml")
            .expect("styles part")
            .read_to_string(&mut styles)
            .unwrap();
        assert!(styles.contains("w:styleId=\"TOC1\""));
        assert!(styles.contains("w:styleId=\"Hyperlink\""));
    }

    #[test]
    fn no_toc_means_no_bookmarks_and_lean_styles() {
        let doc = parse_document("---\ntitle: T\n---\n\n# First {#first}\n\nText.\n").unwrap();
        let bytes = render_docx(&doc, &BTreeMap::new()).unwrap();
        let xml = unzip_document(&bytes);
        assert!(!xml.contains("w:bookmarkStart"));
        assert!(!xml.contains("w:hyperlink"));
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut styles = String::new();
        use std::io::Read;
        zip.by_name("word/styles.xml")
            .expect("styles part")
            .read_to_string(&mut styles)
            .unwrap();
        assert!(!styles.contains("TOC1"), "no TOC styles without a TOC");
    }

    #[test]
    fn inline_images_are_embedded_with_alt_text() {
        let doc = parse_document("Before ![logo](img/logo.png) after.\n").unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("img/logo.png".to_string(), png_asset());
        let bytes = render_docx(&doc, &assets).unwrap();
        validate_docx(&bytes).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("w:drawing"), "drawing element");
        assert!(xml.contains("descr=\"logo\""), "alt text on docPr");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert!(zip.by_name("word/media/image1.png").is_ok(), "media file");
    }

    #[test]
    fn missing_images_fall_back_to_a_visible_marker() {
        let doc = parse_document("![logo](img/logo.png)\n").unwrap();
        let bytes = render_docx(&doc, &BTreeMap::new()).unwrap();
        let xml = unzip_document(&bytes);
        // Runs are word-split; the marker and the caption survive as text.
        assert!(xml.contains("embedded: "), "marker visible: {xml}");
        assert!(!xml.contains("w:drawing"), "no drawing without bytes");
    }

    #[test]
    fn diagram_images_are_embedded() {
        let doc = parse_document("```mermaid {#d}\nflowchart TD\nA-->B\n```\n").unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("d".to_string(), png_asset());
        let bytes = render_docx(&doc, &assets).unwrap();
        validate_docx(&bytes).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("w:drawing"), "drawing element");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert!(zip.by_name("word/media/image1.png").is_ok(), "media file");
    }

    #[test]
    fn diff_docx_carries_tracked_changes() {
        let old =
            parse_document("# Title {#t}\n\nHello world content.\n\n```rust\nfn a() {}\n```\n")
                .unwrap();
        let new = parse_document(
            "# Title {#t}\n\nHello globe content.\n\n```rust\nfn a() {}\nfn b() {}\n```\n",
        )
        .unwrap();
        let identity = RevisionIdentity::new("Reviewer", "2026-10-03T10:00:00Z");
        let bytes =
            render_diff_docx(&old, &new, &BTreeMap::new(), &BTreeMap::new(), &identity).unwrap();
        validate_docx(&bytes).unwrap();

        let xml = unzip_document(&bytes);
        assert!(xml.contains("<w:ins "), "insertion marks");
        assert!(xml.contains("<w:del "), "deletion marks");
        assert!(xml.contains("w:delText"), "deleted text");
        assert!(xml.contains("w:author=\"Reviewer\""), "revision author");
        assert!(xml.contains("globe"), "inserted word");

        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
        assert!(
            zip.by_name("word/comments.xml").is_ok() || !xml.contains("commentReference"),
            "comments part iff comments referenced"
        );
    }

    #[test]
    fn diff_docx_marks_moves_and_diagrams_with_comments() {
        let old =
            parse_document("# A {#a}\n\n# B {#b}\n\n```mermaid {#d}\nflowchart TD\nA-->B\n```\n")
                .unwrap();
        // Move B before A; change the diagram source.
        let new =
            parse_document("# B {#b}\n\n# A {#a}\n\n```mermaid {#d}\nflowchart TD\nA-->C\n```\n")
                .unwrap();
        let mut old_assets = BTreeMap::new();
        old_assets.insert("d".to_string(), png_asset());
        let mut new_assets = BTreeMap::new();
        new_assets.insert("d".to_string(), png_asset());
        let identity = RevisionIdentity::new("Mover", "2026-10-03T11:00:00Z");
        let plan = docgen_core::diff::plan_diff(&old, &new);
        assert_eq!(
            plan.steps
                .iter()
                .filter(|s| matches!(s, docgen_core::diff::Step::Move { .. }))
                .count(),
            1
        );
        assert!(plan.steps.iter().any(|s| matches!(
            s,
            docgen_core::diff::Step::Modify {
                content: docgen_core::diff::ContentChange::Source { .. },
                ..
            }
        )));
        let bytes = render_diff_docx(&old, &new, &old_assets, &new_assets, &identity).unwrap();
        validate_docx(&bytes).unwrap();

        let xml = unzip_document(&bytes);
        assert!(xml.contains("commentRangeStart"), "move comment anchored");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut comments = String::new();
        use std::io::Read;
        zip.by_name("word/comments.xml")
            .expect("comments part")
            .read_to_string(&mut comments)
            .unwrap();
        assert!(comments.contains("moved"), "move comment text: {comments}");
        assert!(
            comments.contains("Diagram updated"),
            "diagram comment: {comments}"
        );
    }

    #[test]
    fn golden_diff_docx_is_byte_stable() {
        let old = parse_document(
            "---\ntitle: Golden\nsubtitle: fixtures\nauthors: [docgen]\n---\n\n# Head {#h}\n\nText with **bold**, _italic_, `code` and [links](https://example.com).\n\n1. first\n2. second\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n",
        )
        .unwrap();
        let new = parse_document(
            "---\ntitle: Golden\nsubtitle: fixtures\nauthors: [docgen]\n---\n\n# Head {#h}\n\nText with **bold**, _italics_, `code` and [links](https://example.com) edited.\n\n1. first\n2. second\n3. inserted\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n",
        )
        .unwrap();
        let identity = RevisionIdentity::new("golden", "2026-10-03T00:00:00Z");
        let bytes =
            render_diff_docx(&old, &new, &BTreeMap::new(), &BTreeMap::new(), &identity).unwrap();
        golden("docx-diff", &bytes);
    }

    #[test]
    fn template_fill_replaces_placeholders_and_body() {
        // Build a "template" with our own writer: placeholders in text and a
        // body marker paragraph.
        let template_doc = parse_document(
            "---\ntitle: Shell\n---\n\nReport: {{title}} ({{version}})\n\n{{docgen:body}}\n\nFooter after body.\n",
        )
        .unwrap();
        let template = render_docx(&template_doc, &BTreeMap::new()).unwrap();

        let doc = parse_document(
            "---\ntitle: Actual Title\nversion: 7.2\n---\n\n# Section {#s}\n\nBody text.\n",
        )
        .unwrap();
        let filled = fill_template(&template, &doc, &BTreeMap::new()).unwrap();
        validate_docx(&filled).unwrap();

        let xml = unzip_document(&filled);
        assert!(xml.contains("Actual Title"), "placeholder substituted");
        assert!(xml.contains("7.2"), "version substituted");
        // Runs are word-granular, so check the fragments separately.
        assert!(
            xml.contains("Body") && xml.contains("text."),
            "body injected"
        );
        assert!(
            xml.contains("Footer") && xml.contains("after"),
            "template tail preserved"
        );
        assert!(!xml.contains("{{"), "no leftover markers");
    }

    #[test]
    fn template_requires_body_marker() {
        let template_doc = parse_document("No marker here.\n").unwrap();
        let template = render_docx(&template_doc, &BTreeMap::new()).unwrap();
        let doc = parse_document("Body.\n").unwrap();
        let err = fill_template(&template, &doc, &BTreeMap::new());
        assert!(err.is_err());
    }

    #[test]
    fn internal_links_become_anchor_hyperlinks() {
        let doc = parse_document(
            "# Top {#top}

See [the section](#top) and [nowhere](#missing-target).
",
        )
        .unwrap();
        let bytes = render_docx(&doc, &BTreeMap::new()).unwrap();
        validate_docx(&bytes).unwrap();
        let xml = unzip_document(&bytes);
        // The target gets a bookmark; the link becomes an anchor hyperlink.
        assert!(xml.contains("w:name=\"top\""), "{xml}");
        assert!(xml.contains("<w:hyperlink w:anchor=\"top\">"), "{xml}");
        assert!(xml.contains("<w:rStyle w:val=\"Hyperlink\"/>"), "{xml}");
        // An unknown target stays plain text (no dead hyperlink).
        assert_eq!(xml.matches("<w:hyperlink").count(), 1, "{xml}");
    }

    #[test]
    fn figures_get_anchor_bookmarks() {
        let doc = parse_document(
            "```mermaid {#arch}
flowchart TD
A-->B
```
",
        )
        .unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("arch".to_string(), png_asset());
        let bytes = render_docx(&doc, &assets).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("w:name=\"fig_arch\""), "{xml}");
    }

    #[test]
    fn captioned_diagrams_carry_captions_and_keeps() {
        let doc = parse_document(
            "```mermaid {#arch} caption=\"System\"
flowchart TD
A-->B
```
",
        )
        .unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("arch".to_string(), png_asset());
        let bytes = render_docx(&doc, &assets).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("w:pStyle w:val=\"Caption\""), "{xml}");
        assert!(xml.contains("Figure "), "numbered caption: {xml}");
        assert!(xml.contains("System"), "caption text: {xml}");
        assert!(
            xml.contains("<w:keepNext/>"),
            "image keeps its caption: {xml}"
        );
    }

    #[test]
    fn captioned_lone_images_are_figures_with_bookmarks() {
        let doc = parse_document(
            "![The hero](h.png){#fig:hero}
",
        )
        .unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("h.png".to_string(), png_asset());
        let bytes = render_docx(&doc, &assets).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("w:name=\"fig_hero\""), "{xml}");
        assert!(xml.contains("w:jc w:val=\"center\""), "centered: {xml}");
        assert!(xml.contains("w:pStyle w:val=\"Caption\""), "{xml}");
        assert!(xml.contains("The hero"), "alt text is the caption: {xml}");
    }

    #[test]
    fn new_page_breaks_before_the_block() {
        let doc = parse_document(
            "Intro.

# A {new-page}

Text.
",
        )
        .unwrap();
        let bytes = render_docx(&doc, &BTreeMap::new()).unwrap();
        let xml = unzip_document(&bytes);
        assert!(
            xml.contains("w:pStyle w:val=\"Heading1\"/><w:pageBreakBefore/>"),
            "{xml}"
        );
    }

    #[test]
    fn keep_false_relaxes_the_figure() {
        let doc = parse_document(
            "```mermaid {#d} caption=\"X\" keep=false
graph TD
```
",
        )
        .unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("d".to_string(), png_asset());
        let bytes = render_docx(&doc, &assets).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("Caption"), "caption still present: {xml}");
        assert!(!xml.contains("<w:keepNext/>"), "{xml}");
    }

    #[test]
    fn mixed_orientation_builds_mid_body_sections() {
        let doc = parse_document(
            "# A {#a}

Portrait.

# Wide {#w page=landscape}

Wide section.

## Deep {#d}

Still wide.

# B {#b}

Portrait again.
",
        )
        .unwrap();
        let bytes = render_docx(&doc, &BTreeMap::new()).unwrap();
        validate_docx(&bytes).unwrap();
        let xml = unzip_document(&bytes);
        // The landscape section closes mid-body; the final section is
        // portrait (trailing sectPr): portrait | landscape | portrait.
        assert_eq!(xml.matches("<w:sectPr>").count(), 3, "{xml}");
        assert!(
            xml.contains("<w:pgSz w:w=\"16838\" w:h=\"11906\" w:orient=\"landscape\"/>"),
            "{xml}"
        );
        let landscape_at = xml.find("w:orient=\"landscape\"").expect("landscape");
        let trailing_at = xml.rfind("<w:sectPr>").expect("trailing");
        assert!(landscape_at < trailing_at, "{xml}");
        assert!(
            xml[trailing_at..].contains("w:w=\"11906\""),
            "final section portrait: {xml}"
        );
    }

    #[test]
    fn landscape_figures_get_their_own_section() {
        let doc = parse_document(
            "Before.

```mermaid {#d} page=landscape caption=\"Big\"
graph TD
```

After.
",
        )
        .unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("d".to_string(), png_asset());
        let bytes = render_docx(&doc, &assets).unwrap();
        validate_docx(&bytes).unwrap();
        let xml = unzip_document(&bytes);
        // portrait | figure landscape | portrait(trailing)
        assert_eq!(xml.matches("<w:sectPr>").count(), 3, "{xml}");
        assert_eq!(xml.matches("w:orient=\"landscape\"").count(), 1, "{xml}");
        let drawing_at = xml.find("w:drawing").expect("figure");
        let landscape_at = xml.find("w:orient=\"landscape\"").expect("landscape");
        let trailing_at = xml.rfind("<w:sectPr>").expect("trailing");
        assert!(
            drawing_at < landscape_at && landscape_at < trailing_at,
            "the landscape section closes after the figure: {xml}"
        );
    }
}
