//! DOCX template filling.
//!
//! A template is any `.docx` (typically your organization's house style).
//! docgen fills it in two ways:
//!
//! - `{{key}}` placeholders inside text runs are replaced with frontmatter
//!   values (`{{title}}`, `{{version}}`, `{{some-custom-key}}`, ...).
//!   Placeholders must live inside a single run (`w:t`); runs split by
//!   spell-checking are not merged yet (documented limitation).
//! - A paragraph whose entire text is `{{docgen:body}}` is replaced by the
//!   rendered document body. In template mode the body uses **direct
//!   formatting** (bold/size/color on runs, manual list markers) instead of
//!   referencing styles the template may not define. Diagram images are
//!   appended to `word/media/` with fresh relationship ids.
//!
//! Everything else in the template is preserved byte-for-byte.

use std::io::Read;

use docgen_core::assets::AssetMap;
use docgen_core::model::{Document, Frontmatter};
use quick_xml::events::{BytesText, Event};
use quick_xml::{Reader, Writer};

use super::document::{Emitter, Media};
use super::package::Package;
use crate::{Error, Result};

pub const BODY_MARKER: &str = "{{docgen:body}}";

/// Fills a template. Diagram blocks must be present in `assets` as PNG.
pub fn fill_template(template: &[u8], doc: &Document, assets: &AssetMap) -> Result<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(template.to_vec()))
        .map_err(|e| Error::Template(format!("not a docx (zip): {e}")))?;

    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let document_xml = read_entry(&mut zip, "word/document.xml")?;
    let mut rels_xml = String::from_utf8_lossy(&read_entry(
        &mut zip,
        "word/_rels/document.xml.rels",
    )
    .unwrap_or_else(|_| fallback_rels().into_bytes()))
    .into_owned();
    let mut content_types = read_entry(&mut zip, "[Content_Types].xml")?;

    let next_rid = next_relationship_id(rels_xml.as_bytes());
    let (filled_xml, media) = fill_document(&document_xml, doc, assets, next_rid)?;

    if !media.rels.is_empty() {
        let mut additions = String::new();
        for (rid, target) in &media.rels {
            additions.push_str(&format!(
                "<Relationship Id=\"{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"{target}\"/>",
                super::xml::esc_attr(rid),
            ));
        }
        rels_xml = splice_before(&rels_xml, "</Relationships>", &additions);
        let ct = String::from_utf8_lossy(&content_types).to_string();
        if !ct.contains("Extension=\"png\"") {
            content_types = splice_before(
                &ct,
                "</Types>",
                "<Default Extension=\"png\" ContentType=\"image/png\"/>",
            )
            .into_bytes();
        }
    }

    let mut package = Package::new();
    for name in &names {
        let bytes = if name == "word/document.xml" {
            filled_xml.clone()
        } else if name == "word/_rels/document.xml.rels" {
            rels_xml.clone().into_bytes()
        } else if name == "[Content_Types].xml" {
            content_types.clone()
        } else {
            let mut buf = Vec::new();
            zip.by_name(name)
                .map_err(|e| Error::Template(format!("reading {name}: {e}")))?
                .read_to_end(&mut buf)
                .map_err(|e| Error::Template(format!("reading {name}: {e}")))?;
            buf
        };
        package.add(name.clone(), bytes);
    }
    for (name, bytes) in media.files {
        package.add(name, bytes);
    }
    package.build()
}

fn fallback_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="document.xml"/>
</Relationships>"#
    .to_string()
}

fn read_entry(zip: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut entry = zip
        .by_name(name)
        .map_err(|e| Error::Template(format!("missing {name}: {e}")))?;
    entry
        .read_to_end(&mut buf)
        .map_err(|e| Error::Template(format!("reading {name}: {e}")))?;
    Ok(buf)
}

fn next_relationship_id(rels_xml: &[u8]) -> usize {
    let text = String::from_utf8_lossy(rels_xml);
    let mut max = 0;
    let mut rest = text.as_ref();
    while let Some(pos) = rest.find("Id=\"rId") {
        rest = &rest[pos + 7..];
        let end = rest.find('"').unwrap_or(0);
        if let Ok(n) = rest[..end].parse::<usize>() {
            max = max.max(n);
        }
    }
    max + 1
}

fn splice_before(xml: &str, marker: &str, addition: &str) -> String {
    match xml.rfind(marker) {
        Some(pos) => format!("{}{}{}", &xml[..pos], addition, &xml[pos..]),
        None => format!("{xml}{addition}"),
    }
}

/// Rewrites document.xml: placeholders substituted, body-marker paragraph
/// replaced by the rendered body (templates may contain several markers; the
/// first wins, later ones are dropped). Returns the new XML and media.
fn fill_document(
    document_xml: &[u8],
    doc: &Document,
    assets: &AssetMap,
    next_rid: usize,
) -> Result<(Vec<u8>, Media)> {
    let text = std::str::from_utf8(document_xml)
        .map_err(|e| Error::Template(format!("document.xml is not UTF-8: {e}")))?;
    let mut reader = Reader::from_str(text);
    let mut main: Writer<Vec<u8>> = Writer::new(Vec::new());
    let mut scratch: Writer<Vec<u8>> = Writer::new(Vec::new());

    let mut in_wt = false;
    let mut in_paragraph = false;
    let mut depth_in_para = 0usize;
    let mut paragraph_is_body = false;
    let mut body_emitted = false;
    let mut media = Media::new(next_rid);

    loop {
        let event = reader
            .read_event()
            .map_err(|e| Error::Xml(format!("template parse: {e}")))?;
        let owned = event.into_owned();
        match &owned {
            Event::Eof => break,
            Event::Start(el) => {
                let local = el.local_name(); let name = local.as_ref();
                if in_paragraph {
                    depth_in_para += 1;
                }
                if name == "p" && !in_paragraph {
                    in_paragraph = true;
                    depth_in_para = 1;
                    paragraph_is_body = false;
                }
                if is_text_element(name) {
                    in_wt = true;
                }
                let w = if in_paragraph {
                    &mut scratch
                } else {
                    &mut main
                };
                w.write_event(owned.clone())
                    .map_err(|e| Error::Xml(e.to_string()))?;
            }
            Event::Empty(el) => {
                let local = el.local_name(); let name = local.as_ref();
                if is_text_element(name) {
                    in_wt = false;
                }
                let w = if in_paragraph { &mut scratch } else { &mut main };
                w.write_event(owned.clone())
                    .map_err(|e| Error::Xml(e.to_string()))?;
            }
            Event::Text(t) => {
                let borrowed = t.borrow();
                let raw: &str = borrowed.as_ref();
                let unescaped = quick_xml::escape::unescape(raw)
                    .map_err(|e| Error::Xml(e.to_string()))?
                    .to_string();
                let event = if in_wt {
                    if unescaped.trim() == BODY_MARKER {
                        paragraph_is_body = true;
                        continue; // drop the marker text itself
                    }
                    let substituted = substitute(&unescaped, &doc.frontmatter);
                    let escaped = quick_xml::escape::escape(&substituted).into_owned();
                    Event::Text(BytesText::from_escaped(escaped))
                } else {
                    owned.clone()
                };
                let w = if in_paragraph { &mut scratch } else { &mut main };
                w.write_event(event)
                    .map_err(|e| Error::Xml(e.to_string()))?;
            }
            Event::End(el) => {
                let local = el.local_name(); let name = local.as_ref();
                if is_text_element(name) {
                    in_wt = false;
                }
                if in_paragraph && name == "p" && depth_in_para == 1 {
                    let scratch_bytes =
                        std::mem::replace(&mut scratch, Writer::new(Vec::new())).into_inner();
                    if paragraph_is_body && !body_emitted {
                        body_emitted = true;
                        // Replace the marker paragraph entirely.
                        let mut emitter = Emitter::for_template(next_rid);
                        emitter.write_cover(&doc.frontmatter);
                        for block in &doc.blocks {
                            emitter.write_block(block, super::document::Mark::None, assets);
                        }
                        let (body_xml, body_media, _comments) = emitter.into_parts();
                        media = body_media;
                        main.write_event(Event::Text(BytesText::from_escaped(
                            String::from_utf8_lossy(&body_xml).into_owned(),
                        )))
                        .map_err(|e| Error::Xml(e.to_string()))?;
                    } else {
                        // Keep the paragraph: buffered events plus the
                        // closing tag we are processing right now.
                        main.write_event(Event::Text(BytesText::from_escaped(
                            String::from_utf8_lossy(&scratch_bytes).into_owned(),
                        )))
                        .map_err(|e| Error::Xml(e.to_string()))?;
                        main.write_event(owned.clone())
                            .map_err(|e| Error::Xml(e.to_string()))?;
                    }
                    in_paragraph = false;
                    depth_in_para = 0;
                    paragraph_is_body = false;
                } else {
                    if in_paragraph {
                        depth_in_para = depth_in_para.saturating_sub(1);
                    }
                    let w = if in_paragraph { &mut scratch } else { &mut main };
                    w.write_event(owned.clone())
                        .map_err(|e| Error::Xml(e.to_string()))?;
                }
            }
            _ => {
                let w = if in_paragraph { &mut scratch } else { &mut main };
                w.write_event(owned.clone())
                    .map_err(|e| Error::Xml(e.to_string()))?;
            }
        }
    }

    if !body_emitted {
        return Err(Error::Template(
            "no {{docgen:body}} marker paragraph found in template".into(),
        ));
    }
    Ok((main.into_inner(), media))
}

fn is_text_element(name: &str) -> bool {
    name == "t" || name.ends_with(":t")
}

fn substitute(text: &str, fm: &Frontmatter) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(rel_end) = rest[start + 2..].find("}}") else {
            out.push_str(rest);
            return out;
        };
        let end = start + 2 + rel_end;
        out.push_str(&rest[..start]);
        let key = &rest[start + 2..end];
        match meta_value(fm, key.trim()) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..end + 2]),
        }
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

fn meta_value(fm: &Frontmatter, key: &str) -> Option<String> {
    match key {
        "title" => fm.title.clone(),
        "subtitle" => fm.subtitle.clone(),
        "authors" => (!fm.authors.is_empty()).then(|| fm.authors.join(", ")),
        "date" => fm.date.clone(),
        "version" => fm.version.clone(),
        "doc_id" | "doc-id" | "docId" => fm.doc_id.clone(),
        _ => coerce(fm.extra.get(key)?),
    }
}

fn coerce(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(coerce).collect();
            Some(parts.join(", "))
        }
        _ => None,
    }
}
