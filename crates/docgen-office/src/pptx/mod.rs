//! PPTX backend public API. See `spec.rs` for the markdown → slides
//! layout algorithm, `slide.rs` for slide XML emission, `parts.rs` for the
//! static package parts and `template.rs` for template filling.
//!
//! Fresh decks are generated from scratch (master + two layouts + theme).
//! Diff decks render the new revision with colored revision chips on
//! changed slides, a removed-content slide and a revision summary —
//! PresentationML has no native tracked-changes equivalent of `w:ins`/
//! `w:del`, so changes are surfaced visually and in slide order instead.

mod parts;
mod slide;
mod spec;
mod template;

use docgen_core::assets::AssetMap;
use docgen_core::diff::{self, DiffOptions};
use docgen_core::model::Document;
use docgen_core::Result as CoreResult;

use crate::docx::package::Package;
use crate::{Error, Result};

pub use template::fill_template;

use parts::*;
use slide::{MediaAlloc, Skin, SlideOut, emit_slide, slide_rels_xml};
use spec::{SlideSpec, slides_for, slides_for_diff};

/// Renders a document to a fresh `.pptx`. Diagram blocks must be present
/// in `assets` as PNG bytes.
pub fn render_pptx(doc: &Document, assets: &AssetMap) -> Result<Vec<u8>> {
    let specs = slides_for(doc);
    build_package(doc, &specs, assets, &Skin::default(), None)
}

/// Renders the diff between two revisions as a `.pptx`: slides come from
/// `new`, changed slides carry a colored chip (NEW/UPDATED/MOVED),
/// followed by a "Removed in this revision" slide and a summary.
/// `old_assets` is currently unused (removed diagrams are summarized as
/// text, not shown) but kept for API symmetry with the docx backend.
pub fn render_diff_pptx(
    old: &Document,
    new: &Document,
    _old_assets: &AssetMap,
    new_assets: &AssetMap,
    identity: &crate::docx::RevisionIdentity,
) -> Result<Vec<u8>> {
    let plan = diff::plan_diff_with(old, new, &DiffOptions::default());
    let specs = slides_for_diff(new, &plan, (&identity.author, &identity.date));
    build_package(new, &specs, new_assets, &Skin::default(), None)
}

/// Assembles the whole package from emitted slides. `base` carries the
/// template package to extend (see [`fill_template`]); `None` builds the
/// default master/layouts/theme from scratch.
pub(crate) fn build_package(
    doc: &Document,
    specs: &[SlideSpec],
    assets: &AssetMap,
    skin: &Skin,
    base: Option<template::TemplateBase>,
) -> Result<Vec<u8>> {
    if let Some(base) = base {
        return template::assemble_with_base(doc, specs, assets, skin, base);
    }

    let mut media = MediaAlloc::new(1);
    let mut slides: Vec<SlideOut> = Vec::new();
    for spec in specs {
        slides.push(emit_slide(spec, skin, assets, &mut media)?);
    }

    let rids: Vec<usize> = (0..slides.len()).map(|i| i + 2).collect();
    let creator = doc
        .frontmatter
        .authors
        .first()
        .cloned()
        .unwrap_or_else(|| "docgen".to_string());
    let date = doc
        .frontmatter
        .date
        .as_deref()
        .map(|d| format!("{d}T00:00:00Z"))
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());

    let mut pkg = Package::new();
    pkg.add("[Content_Types].xml", content_types_xml(slides.len()).into_bytes());
    pkg.add("_rels/.rels", ROOT_RELS.as_bytes().to_vec());
    pkg.add(
        "docProps/core.xml",
        core_xml_for(doc.frontmatter.title.as_deref(), &creator, &date).into_bytes(),
    );
    pkg.add("docProps/app.xml", app_xml().as_bytes().to_vec());
    pkg.add("ppt/presentation.xml", presentation_xml(&rids).into_bytes());
    pkg.add(
        "ppt/_rels/presentation.xml.rels",
        presentation_rels(slides.len()).into_bytes(),
    );
    pkg.add("ppt/presProps.xml", PRESPROPS_XML.as_bytes().to_vec());
    pkg.add("ppt/viewProps.xml", VIEWPROPS_XML.as_bytes().to_vec());
    pkg.add("ppt/tableStyles.xml", TABLESTYLES_XML.as_bytes().to_vec());
    pkg.add("ppt/slideMasters/slideMaster1.xml", slide_master_xml().into_bytes());
    pkg.add(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        MASTER_RELS.as_bytes().to_vec(),
    );
    pkg.add("ppt/slideLayouts/slideLayout1.xml", slide_layout_title_xml().into_bytes());
    pkg.add(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        layout_rels().as_bytes().to_vec(),
    );
    pkg.add("ppt/slideLayouts/slideLayout2.xml", slide_layout_content_xml().into_bytes());
    pkg.add(
        "ppt/slideLayouts/_rels/slideLayout2.xml.rels",
        layout_rels().as_bytes().to_vec(),
    );
    pkg.add("ppt/theme/theme1.xml", theme_xml().as_bytes().to_vec());
    for (i, out) in slides.iter().enumerate() {
        pkg.add(format!("ppt/slides/slide{}.xml", i + 1), out.xml.clone());
        pkg.add(
            format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
            slide_rels_xml(&out.layout, &out.rels).into_bytes(),
        );
    }
    for (name, bytes) in &media.files {
        pkg.add(format!("ppt/media/{name}"), bytes.clone());
    }
    pkg.build()
}

/// Validates that a buffer looks like a pptx we can process: a zip whose
/// content types and presentation parse, whose `sldIdLst` references
/// existing slides, and whose slides reference existing layouts.
pub fn validate_pptx(bytes: &[u8]) -> CoreResult<()> {
    use std::io::Read;

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec()))
        .map_err(|e| docgen_core::Error::Backend(format!("not a zip: {e}")))?;

    let mut read = |name: &str| -> CoreResult<String> {
        let mut out = String::new();
        zip.by_name(name)
            .map_err(|_| docgen_core::Error::Backend(format!("missing {name}")))?
            .read_to_string(&mut out)
            .map_err(|e| docgen_core::Error::Backend(e.to_string()))?;
        Ok(out)
    };

    let content_types = read("[Content_Types].xml")?;
    parse_xml(&content_types)?;
    let presentation = read("ppt/presentation.xml")?;
    parse_xml(&presentation)?;
    let pres_rels = read("ppt/_rels/presentation.xml.rels")?;

    // rId -> target pairs as they appear in presentation.xml.rels.
    let rels = parse_rels(&pres_rels);
    for rid in slide_rids(&presentation) {
        let target = rels
            .iter()
            .find(|(id, _)| *id == rid)
            .map(|(_, t)| t.clone())
            .ok_or_else(|| docgen_core::Error::Backend(format!("slide {rid} has no relationship")))?;
        let path = format!("ppt/{target}");
        let slide_xml = read(&path)?;
        parse_xml(&slide_xml)?;
        check_tables(&slide_xml)?;
        // The slide's layout must exist too.
        let rels_name = format!(
            "ppt/slides/_rels/{}",
            path.rsplit('/').next().unwrap_or_default().to_string() + ".rels"
        );
        if let Ok(slide_rels_xml) = read(&rels_name) {
            for (_, target) in parse_rels(&slide_rels_xml) {
                if target.contains("slideLayout") {
                    let layout = resolve_relative("ppt/slides", &target);
                    read(&layout).map_err(|_| {
                        docgen_core::Error::Backend(format!("missing layout {layout}"))
                    })?;
                }
            }
        }
    }
    Ok(())
}

/// Well-formedness plus structural invariants PowerPoint enforces at
/// load (violations surface as its "repair" dialog, which strips content).
fn parse_xml(xml: &str) -> CoreResult<()> {
    use quick_xml::Reader;
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(docgen_core::Error::Backend(format!("bad XML: {e}"))),
        }
    }
}

/// DrawingML tables must declare their columns inside `a:tblGrid`;
/// PowerPoint discards the whole table otherwise.
fn check_tables(slide_xml: &str) -> CoreResult<()> {
    if !slide_xml.contains("<a:tbl") {
        return Ok(());
    }
    let mut ok = true;
    let mut depth_tbl = 0usize;
    let mut depth_grid = 0usize;
    let mut seen_grid = false;
    let mut reader = quick_xml::Reader::from_str(slide_xml);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                let local = e.local_name();
                let name = local.as_ref();
                if name == "a:tbl" {
                    depth_tbl += 1;
                    seen_grid = false;
                } else if name == "a:tblGrid" && depth_tbl > 0 {
                    depth_grid += 1;
                    seen_grid = true;
                } else if name == "a:tr" && depth_tbl > 0 && depth_grid == 0 {
                    ok = false; // row before any tblGrid
                }
            }
            Ok(quick_xml::events::Event::End(e)) => {
                let local = e.local_name();
                let name = local.as_ref();
                if name == "a:tbl" {
                    depth_tbl = depth_tbl.saturating_sub(1);
                    if !seen_grid {
                        ok = false;
                    }
                } else if name == "a:tblGrid" {
                    depth_grid = depth_grid.saturating_sub(1);
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => {
                return Err(docgen_core::Error::Backend(format!("bad XML: {e}")))
            }
            _ => {}
        }
    }
    if !ok {
        return Err(docgen_core::Error::Backend(
            "table without a:tblGrid (PowerPoint would strip it)".to_string(),
        ));
    }
    Ok(())
}

/// `(rId, target)` pairs from a .rels part.
fn parse_rels(xml: &str) -> Vec<(String, String)> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == "Relationship" {
                    let mut id = String::new();
                    let mut target = String::new();
                    for attr in e.attributes().flatten() {
                        match attr.key.local_name().as_ref() {
                            "Id" => id = attr.value.to_string(),
                            "Target" => target = attr.value.to_string(),
                            _ => {}
                        }
                    }
                    out.push((id, target));
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

/// r:id values referenced from `p:sldIdLst`.
fn slide_rids(presentation: &str) -> Vec<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(presentation);
    let mut in_list = false;
    let mut out = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = e.local_name();
                let name = local.as_ref();
                if name == "sldIdLst" {
                    in_list = true;
                } else if in_list && name == "sldId" {
                    for attr in e.attributes().flatten() {
                        // r:id (prefixed), not the numeric id attribute.
                        if attr.key.local_name().as_ref() == "id"
                            && attr.key.prefix().is_some()
                        {
                            out.push(attr.value.to_string());
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == "sldIdLst" {
                    in_list = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Resolves a relationship target like `../slideLayouts/x.xml` against its
/// source part directory.
fn resolve_relative(base_dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = base_dir.split('/').collect();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            _ => parts.push(seg),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use docgen_core::assets::{AssetFormat, RenderedAsset};
    use docgen_core::parse_document;
    use std::collections::BTreeMap;

    // Minimal valid 1x1 white PNG.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49,
        0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
        0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D,
        0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42,
        0x60, 0x82,
    ];

    fn png_asset() -> RenderedAsset {
        RenderedAsset {
            format: AssetFormat::Png,
            bytes: TINY_PNG.to_vec(),
        }
    }

    fn read_part(bytes: &[u8], name: &str) -> String {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        use std::io::Read;
        let mut out = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut out).unwrap();
        out
    }

    fn part_exists(bytes: &[u8], name: &str) -> bool {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        zip.by_name(name).is_ok()
    }

    const SAMPLE: &str = "---\ntitle: Deck\nsubtitle: A deck\nauthors: [a]\n---\n\
\n# Overview {#o}\n\nSome **bold** lead.\n\n- one\n- two\n  - nested\n\n\
## Status {#s}\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";

    #[test]
    fn fresh_deck_is_valid_and_deterministic() {
        let doc = parse_document(SAMPLE).unwrap();
        let first = render_pptx(&doc, &BTreeMap::new()).unwrap();
        let second = render_pptx(&doc, &BTreeMap::new()).unwrap();
        assert_eq!(first, second, "output must be byte-stable");
        validate_pptx(&first).unwrap();

        let presentation = read_part(&first, "ppt/presentation.xml");
        let slide_count = presentation.matches("<p:sldId ").count();
        assert_eq!(slide_count, 3, "title + overview + status table");

        let title = read_part(&first, "ppt/slides/slide1.xml");
        assert!(title.contains("ctrTitle"), "centered title placeholder");
        assert!(title.contains("Deck"));
        let overview = read_part(&first, "ppt/slides/slide2.xml");
        assert!(overview.contains("b=\"1\""), "bold run");
        let status = read_part(&first, "ppt/slides/slide3.xml");
        assert!(status.contains("a:tbl"), "table shape");
        // PowerPoint strips tables whose gridCols are not wrapped in
        // tblGrid (surfaces as its "repair" dialog) — regression guard.
        assert!(
            status.contains("<a:tblGrid>"),
            "gridCols must be wrapped in a:tblGrid"
        );
        assert!(
            status.matches("<a:gridCol").count() == 2,
            "two columns in tblGrid"
        );

        for part in [
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideLayouts/slideLayout1.xml",
            "ppt/slideLayouts/slideLayout2.xml",
            "ppt/theme/theme1.xml",
            "[Content_Types].xml",
            "_rels/.rels",
        ] {
            assert!(part_exists(&first, part), "missing {part}");
        }
    }

    #[test]
    fn diagram_images_are_embedded() {
        let doc = parse_document("```mermaid {#d}\nflowchart TD\nA-->B\n```\n").unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("d".to_string(), png_asset());
        let bytes = render_pptx(&doc, &assets).unwrap();
        validate_pptx(&bytes).unwrap();
        assert!(part_exists(&bytes, "ppt/media/image1.png"), "media file");
        let slide = read_part(&bytes, "ppt/slides/slide1.xml");
        assert!(slide.contains("r:embed"), "picture references image");
        assert!(slide.contains("p:pic"), "picture shape");
    }

    #[test]
    fn links_become_external_rels() {
        let doc =
            parse_document("# T {#t}\n\n- see [docs](https://example.com/x)\n").unwrap();
        let bytes = render_pptx(&doc, &BTreeMap::new()).unwrap();
        validate_pptx(&bytes).unwrap();
        let rels = read_part(&bytes, "ppt/slides/_rels/slide1.xml.rels");
        assert!(
            rels.contains("TargetMode=\"External\"") && rels.contains("https://example.com/x"),
            "hyperlink rel: {rels}"
        );
        let slide = read_part(&bytes, "ppt/slides/slide1.xml");
        assert!(slide.contains("a:hlinkClick"), "clickable link");
    }

    #[test]
    fn diff_deck_tags_slides_and_summarizes() {
        let old = parse_document("# Title {#t}\n\nHello world.\n\n- old item\n").unwrap();
        let new = parse_document("# Title {#t}\n\nHello globe.\n\n- old item\n\n## New {#n}\n\nstuff\n")
            .unwrap();
        let identity = crate::docx::RevisionIdentity::new("Rev", "2026-10-03T10:00:00Z");
        let bytes = render_diff_pptx(&old, &new, &BTreeMap::new(), &BTreeMap::new(), &identity)
            .unwrap();
        validate_pptx(&bytes).unwrap();

        let all_slides = (1..=6)
            .filter(|i| part_exists(&bytes, &format!("ppt/slides/slide{i}.xml")))
            .map(|i| read_part(&bytes, &format!("ppt/slides/slide{i}.xml")))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all_slides.contains("UPDATED"), "changed slide chip");
        assert!(all_slides.contains("NEW"), "added section chip");
        assert!(all_slides.contains("Revision summary"), "summary slide");
        assert!(all_slides.contains("by Rev on 2026-10-03T10:00:00Z"));
    }

    #[test]
    fn ordered_lists_carry_numbers() {
        let doc = parse_document("# T {#t}\n\n3. three\n4. four\n").unwrap();
        let bytes = render_pptx(&doc, &BTreeMap::new()).unwrap();
        let slide = read_part(&bytes, "ppt/slides/slide1.xml");
        assert!(
            slide.contains("arabicPeriod") && slide.contains("startAt"),
            "auto-numbered bullets: {}",
            &slide[slide.find("buAutoNum").map(|i| i.saturating_sub(80)).unwrap_or(0)
                ..slide.find("buAutoNum").map(|i| i + 120).unwrap_or(100)]
        );
    }

    #[test]
    fn template_fill_appends_slides() {
        // A "template" is any deck: ours qualifies.
        let tmpl_doc = parse_document("---\ntitle: Shell\n---\n\n# Keep {#k}\n\nkept\n").unwrap();
        let template = render_pptx(&tmpl_doc, &BTreeMap::new()).unwrap();

        let doc = parse_document("# Fresh {#f}\n\ncontent\n").unwrap();
        let filled = fill_template(&template, &doc, &BTreeMap::new()).unwrap();
        validate_pptx(&filled).unwrap();

        let before = read_part(&template, "ppt/presentation.xml").matches("<p:sldId ").count();
        let after = read_part(&filled, "ppt/presentation.xml").matches("<p:sldId ").count();
        assert!(after > before, "slides appended: {before} -> {after}");
        // Original slides kept (slide1 is the template's own title slide),
        // new deck appended after them.
        assert!(read_part(&filled, "ppt/slides/slide1.xml").contains("Shell"));
        assert!(read_part(&filled, "ppt/slides/slide2.xml").contains("Keep"));
        assert!(read_part(&filled, "ppt/slides/slide3.xml").contains("Fresh"));
    }
}
