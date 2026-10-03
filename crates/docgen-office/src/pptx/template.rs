//! PPTX template filling: keep a deck's masters/layouts/theme (its whole
//! look), append content slides generated from the document.
//!
//! The template is parsed for slide layouts; generated slides bind to the
//! best-matching layout via placeholders so the template's fonts, colors
//! and bullet styles apply. Layout placeholder geometry is copied onto the
//! slide shapes explicitly so every renderer places text identically.
//! Placeholder-less layouts fall back to proportional default boxes.

use std::io::Read;

use docgen_core::assets::AssetMap;
use docgen_core::model::Document;
use quick_xml::events::Event;
use quick_xml::Reader;

use super::slide::{Geom, MediaAlloc, Skin, emit_slide, slide_rels_xml};
use super::spec::{SlideSpec, slides_for};
use super::{build_package, parts};
use crate::docx::package::Package;
use crate::{Error, Result};

/// Everything remembered from the template package needed to extend it.
pub(crate) struct TemplateBase {
    /// All original entries, in original order.
    entries: Vec<(String, Vec<u8>)>,
    pub skin: Skin,
    /// Highest existing `ppt/slides/slideN.xml` number.
    next_slide_index: usize,
    /// Highest existing `p:sldId/@id` (or 255).
    next_sld_id: u64,
    /// Highest existing rId number in presentation.xml.rels.
    next_rid: usize,
    /// Highest existing `ppt/media/imageN.*` number.
    next_media_index: usize,
}

/// Fills a template deck: generated slides are appended after whatever
/// slides the template already contains.
pub fn fill_template(template: &[u8], doc: &Document, assets: &AssetMap) -> Result<Vec<u8>> {
    let base = parse_template(template)?;
    let specs = slides_for(doc);
    let skin = base.skin.clone();
    build_package(doc, &specs, assets, &skin, Some(base))
}

pub(crate) fn parse_template(template: &[u8]) -> Result<TemplateBase> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(template.to_vec()))
        .map_err(|e| Error::Template(format!("template is not a zip: {e}")))?;

    let mut entries = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| Error::Template(format!("reading entry {i}: {e}")))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| Error::Template(e.to_string()))?;
        entries.push((file.name().to_string(), bytes));
    }
    let text_of = |entries: &[(String, Vec<u8>)], name: &str| -> Option<String> {
        entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| String::from_utf8_lossy(b).to_string())
    };

    let presentation = text_of(&entries, "ppt/presentation.xml")
        .ok_or_else(|| Error::Template("missing ppt/presentation.xml".into()))?;
    let pres_rels = text_of(&entries, "ppt/_rels/presentation.xml.rels")
        .ok_or_else(|| Error::Template("missing ppt/_rels/presentation.xml.rels".into()))?;

    // Slide size for proportional fallback geometry.
    let (slide_w, slide_h) = slide_size(&presentation).unwrap_or((12_192_000, 6_858_000));

    // Layout targets: from each slide master's rels, master from pres rels.
    let master_targets: Vec<String> = rels_of(&pres_rels)
        .into_iter()
        .filter(|(_, kind, _)| kind == "slideMaster")
        .map(|(target, _, _)| normalize("ppt", &target))
        .collect();

    let mut layout_files: Vec<String> = Vec::new();
    for master in &master_targets {
        let rels_name = rels_name_for(master);
        if let Some(xml) = text_of(&entries, &rels_name) {
            for (target, kind, _) in rels_of(&xml) {
                if kind == "slideLayout" {
                    // Store relative to ppt/ (masters live one level down,
                    // hence the `..` in their relationship targets).
                    let file = normalize(
                        master.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("ppt"),
                        &target,
                    );
                    let file = file.strip_prefix("ppt/").unwrap_or(&file).to_string();
                    if !layout_files.contains(&file) {
                        layout_files.push(file);
                    }
                }
            }
        }
    }
    if layout_files.is_empty() {
        return Err(Error::Template("no slide layouts found".into()));
    }

    // Theme minor font — stamped explicitly on plain runs (see Skin).
    let body_font = master_targets
        .first()
        .and_then(|master| {
            let rels = text_of(&entries, &rels_name_for(master))?;
            let target = rels_of(&rels)
                .into_iter()
                .find(|(_, kind, _)| kind == "theme")
                .map(|(t, _, _)| t)?;
            let theme_path = normalize(
                master.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("ppt"),
                &target,
            );
            let theme = text_of(&entries, &theme_path)?;
            minor_font(&theme)
        })
        .unwrap_or_else(|| "Calibri".to_string());

    let mut layouts: Vec<Layout> = Vec::new();
    for file in &layout_files {
        let xml = match text_of(&entries, &format!("ppt/{file}")) {
            Some(x) => x,
            None => continue,
        };
        let mut layout = parse_layout(&xml, slide_w, slide_h);
        layout.file = file.clone();
        layouts.push(layout);
    }

    let title_idx = layouts
        .iter()
        .position(|l| l.ctr_title.is_some())
        .or_else(|| layouts.iter().position(|l| l.title.is_some() && l.subtitle.is_some()))
        .or_else(|| layouts.iter().position(|l| l.title.is_some()))
        .unwrap_or(0);
    let content_idx = layouts
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != title_idx)
        .find(|(_, l)| l.name.to_lowercase().contains("content") && l.title.is_some())
        .or_else(|| {
            layouts
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != title_idx)
                .find(|(_, l)| l.title.is_some() && l.body.is_some())
        })
        .or_else(|| {
            layouts
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != title_idx)
                .find(|(_, l)| l.title.is_some())
        })
        .map(|(i, _)| i)
        .unwrap_or(if layouts.len() > 1 { 1 } else { title_idx });

    let (title_layout, content_layout) = (&layouts[title_idx], &layouts[content_idx]);

    let skin = Skin {
        title_layout: format!("../{}", title_layout.file),
        content_layout: format!("../{}", content_layout.file),
        title_geom: content_layout
            .title
            .as_ref()
            .map(|p| p.geom)
            .unwrap_or(parts::TITLE_GEOM),
        ctr_title_geom: title_layout
            .ctr_title
            .as_ref()
            .map(|p| p.geom)
            .unwrap_or(parts::CTR_TITLE_GEOM),
        subtitle_geom: title_layout
            .subtitle
            .as_ref()
            .map(|p| p.geom)
            .unwrap_or(parts::SUBTITLE_GEOM),
        body_geom: content_layout
            .body
            .as_ref()
            .map(|p| p.geom)
            .unwrap_or(parts::BODY_GEOM),
        body_idx: content_layout.body.as_ref().and_then(|p| p.idx),
        body_font,
    };

    let next_slide_index = entries
        .iter()
        .filter_map(|(name, _)| {
            let rest = name.strip_prefix("ppt/slides/slide")?;
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            num.parse::<usize>().ok()
        })
        .max()
        .unwrap_or(0)
        + 1;

    let next_sld_id = {
        let mut max = 255u64;
        let mut reader = Reader::from_str(&presentation);
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    if e.local_name().as_ref() == "sldId" {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == "id"
                                && attr.key.prefix().is_none()
                            {
                                if let Ok(n) = attr.value.parse::<u64>() {
                                    max = max.max(n);
                                }
                            }
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }
        max + 1
    };

    let next_rid = rels_of(&pres_rels)
        .iter()
        .filter_map(|(_, _, id)| id.strip_prefix("rId").and_then(|n| n.parse::<usize>().ok()))
        .max()
        .unwrap_or(1)
        + 1;

    let next_media_index = entries
        .iter()
        .filter_map(|(name, _)| {
            let rest = name.strip_prefix("ppt/media/image")?;
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            num.parse::<usize>().ok()
        })
        .max()
        .unwrap_or(0)
        + 1;

    Ok(TemplateBase {
        entries,
        skin,
        next_slide_index,
        next_sld_id,
        next_rid,
        next_media_index,
    })
}

struct Ph {
    idx: Option<u32>,
    geom: Geom,
}

struct Layout {
    file: String,
    name: String,
    title: Option<Ph>,
    ctr_title: Option<Ph>,
    subtitle: Option<Ph>,
    body: Option<Ph>,
}

/// Parses a layout's placeholders. Missing xfrm falls back to
/// proportional boxes of the slide size.
fn parse_layout(xml: &str, slide_w: i64, slide_h: i64) -> Layout {
    let frac = |fx, fy, fw, fh| -> Geom {
        (
            slide_w * fx / 100,
            slide_h * fy / 100,
            slide_w * fw / 100,
            slide_h * fh / 100,
        )
    };
    let mut out = Layout {
        file: String::new(),
        name: String::new(),
        title: None,
        ctr_title: None,
        subtitle: None,
        body: None,
    };
    let mut reader = Reader::from_str(xml);
    let mut in_sp = false;
    let mut ph_type: Option<String> = None;
    let mut ph_idx: Option<u32> = None;
    let mut off: Option<(i64, i64)> = None;
    let mut ext: Option<(i64, i64)> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "cSld" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == "name" {
                                out.name = attr.value.to_string();
                            }
                        }
                    }
                    "sp" => {
                        in_sp = true;
                        ph_type = None;
                        ph_idx = None;
                        off = None;
                        ext = None;
                    }
                    "ph" if in_sp => {
                        for attr in e.attributes().flatten() {
                            match attr.key.local_name().as_ref() {
                                "type" => ph_type = Some(attr.value.to_string()),
                                "idx" => ph_idx = attr.value.parse::<u32>().ok(),
                                _ => {}
                            }
                        }
                    }
                    "off" if in_sp => {
                        off = coords(&e);
                    }
                    "ext" if in_sp => {
                        ext = coords(&e);
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let local = e.local_name();
                let name = local.as_ref();
                if name == "sp" && in_sp {
                    in_sp = false;
                    let geom = match (off, ext) {
                        (Some((x, y)), Some((cx, cy))) => (x, y, cx, cy),
                        _ => match ph_type.as_deref() {
                            Some("title") | Some("ctrTitle") => frac(4, 5, 92, 10),
                            _ => frac(4, 18, 92, 74),
                        },
                    };
                    let ph = Ph { idx: ph_idx, geom };
                    match ph_type.as_deref() {
                        Some("title") => out.title = Some(ph),
                        Some("ctrTitle") => out.ctr_title = Some(ph),
                        Some("subTitle") => out.subtitle = Some(ph),
                        Some("body") | None => {
                            // Unindexed phs other than title are body-like.
                            if ph_type.as_deref() == Some("body") || ph_idx.is_some() {
                                if out.body.is_none() {
                                    out.body = Some(ph);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

fn coords(e: &quick_xml::events::BytesStart) -> Option<(i64, i64)> {
    let mut x: Option<i64> = None;
    let mut y: Option<i64> = None;
    for attr in e.attributes().flatten() {
        match attr.key.local_name().as_ref() {
            "x" | "cx" => x = attr.value.parse().ok(),
            "y" | "cy" => y = attr.value.parse().ok(),
            _ => {}
        }
    }
    match (x, y) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    }
}

/// `(target, kind, id)` triples from a .rels part.
fn rels_of(xml: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == "Relationship" {
                    let mut id = String::new();
                    let mut target = String::new();
                    let mut kind = String::new();
                    for attr in e.attributes().flatten() {
                        match attr.key.local_name().as_ref() {
                            "Id" => id = attr.value.to_string(),
                            "Target" => target = attr.value.to_string(),
                            "Type" => kind = attr.value.to_string(),
                            _ => {}
                        }
                    }
                    let kind = kind.rsplit('/').next().unwrap_or_default().to_string();
                    out.push((target, kind, id));
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

/// `ppt/slides/slide1.xml` -> `ppt/slides/_rels/slide1.xml.rels`.
fn rels_name_for(part: &str) -> String {
    let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
    format!("{dir}/_rels/{file}.rels")
}

/// Resolves `target` (possibly with `..` segments) against `base_dir`,
/// returning a package-root-relative path.
fn normalize(base_dir: &str, target: &str) -> String {
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

/// The theme's minor (body) latin typeface.
fn minor_font(theme_xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(theme_xml);
    let mut in_minor = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = e.local_name();
                let name = local.as_ref();
                match name {
                    "minorFont" => in_minor = true,
                    "majorFont" => in_minor = false,
                    "latin" if in_minor => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == "typeface" {
                                let face = attr.value.to_string();
                                if !face.is_empty() {
                                    return Some(face);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let local = e.local_name();
                if local.as_ref() == "minorFont" {
                    in_minor = false;
                }
            }
            Ok(Event::Eof) => return None,
            Err(_) => return None,
            _ => {}
        }
    }
}

fn slide_size(presentation: &str) -> Option<(i64, i64)> {    let mut reader = Reader::from_str(presentation);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == "sldSz" {
                    let mut cx = None;
                    let mut cy = None;
                    for attr in e.attributes().flatten() {
                        match attr.key.local_name().as_ref() {
                            "cx" => cx = attr.value.parse().ok(),
                            "cy" => cy = attr.value.parse().ok(),
                            _ => {}
                        }
                    }
                    return Some((cx?, cy?));
                }
            }
            Ok(Event::Eof) => return None,
            Err(_) => return None,
            _ => {}
        }
    }
}

pub(crate) fn assemble_with_base(
    doc: &Document,
    specs: &[SlideSpec],
    assets: &AssetMap,
    skin: &Skin,
    base: TemplateBase,
) -> Result<Vec<u8>> {
    let mut media = MediaAlloc::new(base.next_media_index);
    let mut slides: Vec<(Vec<u8>, String, Vec<super::slide::Rel>)> = Vec::new();
    for spec in specs {
        let out = emit_slide(spec, skin, assets, &mut media)?;
        slides.push((out.xml, out.layout, out.rels));
    }

    let count = slides.len();
    let rids: Vec<usize> = (0..count).map(|i| base.next_rid + i).collect();
    let sld_ids: Vec<u64> = (0..count).map(|i| base.next_sld_id + i as u64).collect();

    let mut pkg = Package::new();
    for (name, bytes) in &base.entries {
        let bytes = match name.as_str() {
            "ppt/presentation.xml" => insert_sld_ids(bytes, &sld_ids, &rids).into_bytes(),
            "ppt/_rels/presentation.xml.rels" => {
                insert_slide_rels(bytes, base.next_slide_index, &rids).into_bytes()
            }
            "[Content_Types].xml" => {
                extend_content_types(bytes, base.next_slide_index, count).into_bytes()
            }
            "docProps/core.xml" => {
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
                parts::core_xml_for(doc.frontmatter.title.as_deref(), &creator, &date).into_bytes()
            }
            _ => bytes.clone(),
        };
        pkg.add(name.clone(), bytes);
    }
    for (i, (xml, layout, rels)) in slides.iter().enumerate() {
        let n = base.next_slide_index + i;
        pkg.add(format!("ppt/slides/slide{n}.xml"), xml.clone());
        pkg.add(
            format!("ppt/slides/_rels/slide{n}.xml.rels"),
            slide_rels_xml(layout, rels).into_bytes(),
        );
    }
    for (name, bytes) in &media.files {
        pkg.add(format!("ppt/media/{name}"), bytes.clone());
    }
    pkg.build()
}

fn insert_sld_ids(bytes: &[u8], sld_ids: &[u64], rids: &[usize]) -> String {
    let xml = String::from_utf8_lossy(bytes).to_string();
    let entries: String = sld_ids
        .iter()
        .zip(rids)
        .map(|(id, rid)| format!("<p:sldId id=\"{id}\" r:id=\"rId{rid}\"/>"))
        .collect();
    if let Some(pos) = xml.find("</p:sldIdLst>") {
        let mut out = String::with_capacity(xml.len() + entries.len());
        out.push_str(&xml[..pos]);
        out.push_str(&entries);
        out.push_str(&xml[pos..]);
        out
    } else if xml.contains("<p:sldIdLst/>") {
        xml.replacen(
            "<p:sldIdLst/>",
            &format!("<p:sldIdLst>{entries}</p:sldIdLst>"),
            1,
        )
    } else if let Some(pos) = xml.find("</p:presentation>") {
        let mut out = String::with_capacity(xml.len() + entries.len() + 32);
        out.push_str(&xml[..pos]);
        out.push_str(&format!("<p:sldIdLst>{entries}</p:sldIdLst>"));
        out.push_str(&xml[pos..]);
        out
    } else {
        xml
    }
}

fn insert_slide_rels(bytes: &[u8], first: usize, rids: &[usize]) -> String {
    let xml = String::from_utf8_lossy(bytes).to_string();
    let entries: String = rids
        .iter()
        .enumerate()
        .map(|(i, rid)| {
            format!(
                "<Relationship Id=\"rId{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{}.xml\"/>",
                first + i
            )
        })
        .collect();
    xml.replacen("</Relationships>", &format!("{entries}</Relationships>"), 1)
}

fn extend_content_types(bytes: &[u8], first: usize, count: usize) -> String {
    let xml = String::from_utf8_lossy(bytes).to_string();
    let mut adds = String::new();
    if !xml.contains("Extension=\"png\"") {
        adds.push_str(
            "<Default Extension=\"png\" ContentType=\"image/png\"/>",
        );
    }
    for i in 0..count {
        adds.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>",
            first + i
        ));
    }
    xml.replacen("</Types>", &format!("{adds}</Types>"), 1)
}
