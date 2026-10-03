//! document.xml emission: markdown model → OOXML body, optionally wrapped in
//! native tracked changes (`w:ins`/`w:del`) with comments for moves and
//! diagram replacements.

use docgen_core::assets::AssetMap;
use docgen_core::diff::{flatten_runs_pub, ContentChange, DiffPlan, LineOp, Run, RunOp, Step};
use docgen_core::model::{Block, BlockKind, Document, Inline, ListItem, TableRow};
use quick_xml::Reader;

use super::package::{png_dimensions, Package};
use super::parts;
use super::xml::Xml;
use super::RevisionIdentity;
use crate::{Error, Result};

/// Content width of an A4 page with 2 cm margins, in EMU (17 cm · 360000).
const CONTENT_WIDTH_EMU: f64 = 6_120_000.0;
const REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

#[derive(Clone, Copy)]
pub(crate) struct RevisionMode<'a> {
    pub plan: &'a DiffPlan,
    pub old_assets: &'a AssetMap,
    pub identity: &'a RevisionIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    None,
    Insert,
    Delete,
}

impl Mark {
    fn element(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Insert => Some("w:ins"),
            Self::Delete => Some("w:del"),
        }
    }
}

pub(crate) struct Media {
    pub(crate) files: Vec<(String, Vec<u8>)>,
    pub(crate) rels: Vec<(String, String)>, // rId -> media/imageN.png
    next_rid: usize,
    next_docpr: usize,
}

impl Media {
    pub(crate) fn new(next_rid: usize) -> Self {
        Self {
            files: Vec::new(),
            rels: Vec::new(),
            next_rid,
            next_docpr: 1,
        }
    }

    fn register(&mut self, png: &[u8]) -> (String, u64, u64) {
        let n = self.files.len() + 1;
        let name = format!("media/image{n}.png");
        let rid = format!("rId{}", self.next_rid);
        self.next_rid += 1;
        self.rels.push((rid.clone(), name.clone()));
        self.files.push((format!("word/{name}"), png.to_vec()));
        let (w, h) = png_dimensions(png).unwrap_or((600, 400));
        // 96 dpi assumed; clamp to content width.
        let mut cx = w as f64 / 96.0 * 914_400.0;
        let mut cy = h as f64 / 96.0 * 914_400.0;
        if cx > CONTENT_WIDTH_EMU {
            let scale = CONTENT_WIDTH_EMU / cx;
            cx *= scale;
            cy *= scale;
        }
        (rid, cx.round() as u64, cy.round() as u64)
    }
}

pub(crate) struct Comment {
    id: usize,
    text: String,
}

/// Direct formatting applied when emitting into foreign templates (whose
/// styles.xml we do not control): instead of referencing `pStyle` values,
/// runs and paragraphs carry the formatting themselves.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DirectProps {
    bold: bool,
    italic: bool,
    mono: bool,
    size: Option<u32>, // half-points
    color: Option<&'static str>,
}

fn direct_props_for(style: &str) -> DirectProps {
    match style {
        "Title" => DirectProps {
            bold: true,
            size: Some(52),
            ..Default::default()
        },
        "Subtitle" => DirectProps {
            italic: true,
            size: Some(26),
            color: Some("595959"),
            ..Default::default()
        },
        "Byline" => DirectProps {
            size: Some(19),
            color: Some("595959"),
            ..Default::default()
        },
        "Heading1" => DirectProps {
            bold: true,
            size: Some(32),
            color: Some("2F5496"),
            ..Default::default()
        },
        "Heading2" => DirectProps {
            bold: true,
            size: Some(26),
            color: Some("2F5496"),
            ..Default::default()
        },
        "Heading3" | "Heading4" => DirectProps {
            bold: true,
            size: Some(24),
            color: Some("1F3763"),
            ..Default::default()
        },
        "Heading5" | "Heading6" => DirectProps {
            bold: true,
            color: Some("2F5496"),
            ..Default::default()
        },
        "SourceCode" => DirectProps {
            mono: true,
            size: Some(18),
            ..Default::default()
        },
        "Quote" => DirectProps {
            italic: true,
            color: Some("404040"),
            ..Default::default()
        },
        "Caption" => DirectProps {
            italic: true,
            size: Some(19),
            color: Some("595959"),
            ..Default::default()
        },
        _ => DirectProps::default(),
    }
}

pub(crate) struct Emitter<'a> {
    xml: Xml,
    media: Media,
    comments: Vec<Comment>,
    revision: Option<RevisionMode<'a>>,
    next_rev_id: usize,
    /// Comment anchored to the block being emitted; consumed by the block's
    /// first paragraph.
    pending_comment: Option<usize>,
    in_block: bool,
    /// Direct formatting mode (foreign templates without our styles).
    direct: bool,
    props: DirectProps,
}

impl<'a> Emitter<'a> {
    fn new(revision: Option<RevisionMode<'a>>) -> Self {
        Self {
            xml: Xml::new(),
            // rId100..rId105 are reserved for the fixed part relationships
            // (styles, numbering, settings, fonts, theme, comments).
            media: Media::new(200),
            comments: Vec::new(),
            revision,
            next_rev_id: 1,
            pending_comment: None,
            in_block: false,
            direct: false,
            props: DirectProps::default(),
        }
    }

    /// Emitter for embedding bodies into foreign templates: direct
    /// formatting, image relationship ids starting above `next_rid`.
    pub(crate) fn for_template(next_rid: usize) -> Self {
        Self {
            xml: Xml::new(),
            media: Media::new(next_rid),
            comments: Vec::new(),
            revision: None,
            next_rev_id: 1,
            pending_comment: None,
            in_block: false,
            direct: true,
            props: DirectProps::default(),
        }
    }

    pub(crate) fn into_parts(self) -> (Vec<u8>, Media, Vec<Comment>) {
        (self.xml.finish(), self.media, self.comments)
    }
    fn identity(&self) -> (&str, &str) {
        match &self.revision {
            Some(mode) => (mode.identity.author.as_str(), mode.identity.date.as_str()),
            None => ("docgen", "1970-01-01T00:00:00Z"),
        }
    }

    fn next_rev(&mut self) -> usize {
        let id = self.next_rev_id;
        self.next_rev_id += 1;
        id
    }

    fn add_comment(&mut self, text: impl Into<String>) {
        let id = self.comments.len();
        self.comments.push(Comment {
            id,
            text: text.into(),
        });
        self.pending_comment = Some(id);
    }

    fn rev_attrs(&mut self) -> Vec<(String, String)> {
        let id = self.next_rev();
        let (author, date) = self.identity();
        vec![
            ("w:id".to_string(), id.to_string()),
            ("w:author".to_string(), author.to_string()),
            ("w:date".to_string(), date.to_string()),
        ]
    }

    /// Opens `w:ins`/`w:del` around a run group; returns the element to
    /// close, if any.
    fn open_mark(&mut self, mark: Mark) -> Option<&'static str> {
        let element = mark.element()?;
        let attrs = self.rev_attrs();
        let borrowed: Vec<(&str, &str)> = attrs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.xml.start(element, &borrowed);
        Some(element)
    }

    /// Paragraph-mark revision: the paragraph itself is inserted/deleted.
    fn para_mark(&mut self, mark: Mark) {
        if let Some(element) = mark.element() {
            let attrs = self.rev_attrs();
            let borrowed: Vec<(&str, &str)> = attrs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            self.xml.start("w:rPr", &[]);
            self.xml.start(element, &borrowed);
            self.xml.end(element);
            self.xml.end("w:rPr");
        }
    }

    fn row_mark(&mut self, mark: Mark) {
        if let Some(element) = mark.element() {
            let attrs = self.rev_attrs();
            let borrowed: Vec<(&str, &str)> = attrs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            self.xml.start(element, &borrowed);
            self.xml.end(element);
        }
    }

    fn comment_start(&mut self) {
        if let Some(id) = self.pending_comment {
            if self.in_block {
                self.xml
                    .empty("w:commentRangeStart", &[("w:id", &id.to_string())]);
            }
        }
    }

    fn comment_end(&mut self) {
        if let Some(id) = self.pending_comment.take() {
            if self.in_block {
                self.in_block = false;
                self.xml
                    .empty("w:commentRangeEnd", &[("w:id", &id.to_string())]);
                self.xml.start("w:r", &[]);
                self.xml.start("w:rPr", &[]);
                self.xml.empty("w:rStyle", &[("w:val", "CommentReference")]);
                self.xml.end("w:rPr");
                self.xml
                    .empty("w:commentReference", &[("w:id", &id.to_string())]);
                self.xml.end("w:r");
            }
        }
    }

    // -- paragraph-level primitives ------------------------------------

    fn paragraph_open(&mut self, style: Option<&str>, mark: Mark, centered: bool) {
        self.xml.start("w:p", &[]);
        self.xml.start("w:pPr", &[]);
        self.props = style.map(direct_props_for).unwrap_or_default();
        if let Some(style) = style {
            if self.direct {
                // Direct formatting instead of a style reference.
                if centered || self.needs_center(style) {
                    self.xml.empty("w:jc", &[("w:val", "center")]);
                }
                match style {
                    "SourceCode" => {
                        self.xml.empty(
                            "w:shd",
                            &[("w:val", "clear"), ("w:color", "auto"), ("w:fill", "F2F2F2")],
                        );
                        self.xml
                            .empty("w:ind", &[("w:left", "284")]);
                    }
                    "Quote" => {
                        self.xml.empty("w:ind", &[("w:left", "567")]);
                    }
                    "Heading1" => {
                        self.xml
                            .empty("w:spacing", &[("w:before", "320")]);
                    }
                    "Heading2" => {
                        self.xml
                            .empty("w:spacing", &[("w:before", "240")]);
                    }
                    "Heading3" => {
                        self.xml
                            .empty("w:spacing", &[("w:before", "200")]);
                    }
                    _ => {}
                }
            } else {
                self.xml.empty("w:pStyle", &[("w:val", style)]);
                if centered {
                    self.xml.empty("w:jc", &[("w:val", "center")]);
                }
            }
        } else if centered {
            self.xml.empty("w:jc", &[("w:val", "center")]);
        }
        self.para_mark(mark);
        self.xml.end("w:pPr");
        self.comment_start();
    }

    fn needs_center(&self, style: &str) -> bool {
        matches!(style, "Title" | "Subtitle" | "Caption")
    }

    fn paragraph_close(&mut self) {
        self.comment_end();
        self.xml.end("w:p");
        self.props = DirectProps::default();
    }

    fn write_run(&mut self, run: &Run, mark: Mark) {
        let props = self.props;
        self.xml.start("w:r", &[]);
        self.xml.start("w:rPr", &[]);
        let mono = run.code || props.mono;
        if run.bold || props.bold {
            self.xml.empty("w:b", &[]);
        }
        if run.italic || props.italic {
            self.xml.empty("w:i", &[]);
        }
        if run.strike {
            self.xml.empty("w:strike", &[]);
        }
        if mono {
            self.xml
                .empty("w:rFonts", &[("w:ascii", "Consolas"), ("w:hAnsi", "Consolas")]);
        }
        if let Some(size) = props.size {
            self.xml.empty("w:sz", &[("w:val", &size.to_string())]);
        }
        if let Some(color) = props.color {
            self.xml.empty("w:color", &[("w:val", color)]);
        }
        self.xml.end("w:rPr");
        let text_elem = if mark == Mark::Delete {
            "w:delText"
        } else {
            "w:t"
        };
        self.xml.start(text_elem, &[("xml:space", "preserve")]);
        self.xml.text(&run.text);
        self.xml.end(text_elem);
        self.xml.end("w:r");
    }

    fn write_runs(&mut self, inline: &[Inline], mark: Mark) {
        let runs = flatten_runs_pub(inline);
        let close = self.open_mark(mark);
        for run in &runs {
            self.write_run(run, mark);
        }
        if let Some(el) = close {
            self.xml.end(el);
        }
    }

    fn write_paragraph(
        &mut self,
        inline: &[Inline],
        style: Option<&str>,
        mark: Mark,
    ) {
        self.paragraph_open(style, mark, false);
        self.write_runs(inline, mark);
        self.paragraph_close();
    }

    // -- block emission -------------------------------------------------

    pub(crate) fn write_cover(&mut self, fm: &docgen_core::model::Frontmatter) {
        if let Some(title) = &fm.title {
            self.paragraph_open(Some("Title"), Mark::None, false);
            self.write_runs(&[Inline::Text(title.clone())], Mark::None);
            self.paragraph_close();
        }
        if let Some(subtitle) = &fm.subtitle {
            self.paragraph_open(Some("Subtitle"), Mark::None, false);
            self.write_runs(&[Inline::Text(subtitle.clone())], Mark::None);
            self.paragraph_close();
        }
        let byline = [
            if fm.authors.is_empty() {
                None
            } else {
                Some(fm.authors.join(", "))
            },
            fm.date.clone(),
            fm.version.clone(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        if !byline.is_empty() {
            self.paragraph_open(Some("Byline"), Mark::None, false);
            self.write_runs(&[Inline::Text(byline)], Mark::None);
            self.paragraph_close();
        }
    }

    pub(crate) fn write_block(&mut self, block: &Block, mark: Mark, assets: &AssetMap) {        self.in_block = true;
        match &block.kind {
            BlockKind::Heading { level, inline } => {
                let style = format!("Heading{}", (*level).clamp(1, 6));
                self.write_paragraph(inline, Some(&style), mark);
            }
            BlockKind::Paragraph { inline } => {
                self.write_paragraph(inline, None, mark);
            }
            BlockKind::CodeBlock { code, .. } => {
                self.write_code_block(code, mark);
            }
            BlockKind::Diagram { .. } => {
                self.write_diagram(&block.id, mark, assets);
            }
            BlockKind::List { ordered, items, .. } => {
                self.write_list(*ordered, items, 0, mark);
            }
            BlockKind::Table { rows } => {
                self.write_table(rows, mark);
            }
            BlockKind::BlockQuote { blocks } => {
                for child in blocks {
                    if let BlockKind::Paragraph { inline } = &child.kind {
                        self.write_paragraph(inline, Some("Quote"), mark);
                    }
                }
            }
            BlockKind::ThematicBreak => {
                self.paragraph_open(None, mark, false);
                // Empty paragraph with a bottom border.
                self.xml.start("w:pPr", &[]);
                self.xml.start("w:pBdr", &[]);
                self.xml.empty(
                    "w:bottom",
                    &[("w:val", "single"), ("w:sz", "6"), ("w:space", "1"), ("w:color", "BFBFBF")],
                );
                self.xml.end("w:pBdr");
                self.xml.end("w:pPr");
                self.paragraph_close();
            }
        }
        self.in_block = false;
    }

    fn write_code_block(&mut self, code: &str, mark: Mark) {
        let lines: Vec<&str> = if code.is_empty() {
            vec![""]
        } else {
            code.lines().collect()
        };
        for (i, line) in lines.iter().enumerate() {
            self.paragraph_open(Some("SourceCode"), mark, false);
            if i == 0 {
                // comment anchors already consumed above if pending
            }
            let close = self.open_mark(mark);
            self.write_run(
                &Run {
                    code: true,
                    text: if line.is_empty() { " ".to_string() } else { (*line).to_string() },
                    ..Default::default()
                },
                mark,
            );
            if let Some(el) = close {
                self.xml.end(el);
            }
            self.paragraph_close();
        }
        self.paragraph_open(Some("CodeGap"), mark, false);
        self.paragraph_close();
    }

    fn write_diagram(&mut self, block_id: &str, mark: Mark, assets: &AssetMap) {
        let Some(asset) = assets.get(block_id) else {
            self.write_paragraph(
                &[Inline::Text(format!("diagram not rendered ({block_id})"))],
                Some("Quote"),
                Mark::None,
            );
            return;
        };
        let (rid, cx, cy) = self.media.register(&asset.bytes);
        let docpr = self.media.next_docpr;
        self.media.next_docpr += 1;

        self.paragraph_open(None, mark, true);
        let close = self.open_mark(mark);
        self.write_drawing(rid, docpr, cx, cy);
        if let Some(el) = close {
            self.xml.end(el);
        }
        self.paragraph_close();
    }

    fn write_drawing(&mut self, rid: String, docpr: usize, cx: u64, cy: u64) {
        let cx = cx.to_string();
        let cy = cy.to_string();
        let docpr_id = docpr.to_string();
        let name = format!("Diagram {docpr}");
        self.xml.start("w:r", &[]);
        self.xml.start("w:drawing", &[]);
        self.xml.start(
            "wp:inline",
            &[("distT", "0"), ("distB", "0"), ("distL", "0"), ("distR", "0")],
        );
        self.xml.empty("wp:extent", &[("cx", &cx), ("cy", &cy)]);
        self.xml
            .empty("wp:effectExtent", &[("l", "0"), ("t", "0"), ("r", "0"), ("b", "0")]);
        self.xml
            .empty("wp:docPr", &[("id", &docpr_id), ("name", &name)]);
        self.xml.start("wp:cNvGraphicFramePr", &[]);
        self.xml.empty("a:graphicFrameLocks", &[("noChangeAspect", "1")]);
        self.xml.end("wp:cNvGraphicFramePr");
        self.xml.start("a:graphic", &[]);
        self.xml.start(
            "a:graphicData",
            &[("uri", "http://schemas.openxmlformats.org/drawingml/2006/picture")],
        );
        self.xml.start("pic:pic", &[]);
        self.xml.start("pic:nvPicPr", &[]);
        self.xml.empty("pic:cNvPr", &[("id", "0"), ("name", &name)]);
        self.xml.empty("pic:cNvPicPr", &[]);
        self.xml.end("pic:nvPicPr");
        self.xml.start("pic:blipFill", &[]);
        self.xml.empty("a:blip", &[("r:embed", &rid)]);
        self.xml.start("a:stretch", &[]);
        self.xml.empty("a:fillRect", &[]);
        self.xml.end("a:stretch");
        self.xml.end("pic:blipFill");
        self.xml.start("pic:spPr", &[]);
        self.xml.start("a:xfrm", &[]);
        self.xml.empty("a:off", &[("x", "0"), ("y", "0")]);
        self.xml.empty("a:ext", &[("cx", &cx), ("cy", &cy)]);
        self.xml.end("a:xfrm");
        self.xml.start("a:prstGeom", &[("prst", "rect")]);
        self.xml.start("a:avLst", &[]);
        self.xml.end("a:avLst");
        self.xml.end("a:prstGeom");
        self.xml.end("pic:spPr");
        self.xml.end("pic:pic");
        self.xml.end("a:graphicData");
        self.xml.end("a:graphic");
        self.xml.end("wp:inline");
        self.xml.end("w:drawing");
        self.xml.end("w:r");
    }

    fn write_list(&mut self, ordered: bool, items: &[ListItem], level: usize, mark: Mark) {
        let num_id = if ordered { 2 } else { 1 };
        let mut counter = 0u64;
        for item in items {
            counter += 1;
            self.xml.start("w:p", &[]);
            self.xml.start("w:pPr", &[]);
            if self.direct {
                // Foreign template: manual markers instead of numbering.xml
                // references (which the template may not define).
                self.xml.empty(
                    "w:ind",
                    &[("w:left", &(720 * (level as u32 + 1)).to_string())],
                );
            } else {
                self.xml.start("w:numPr", &[]);
                self.xml
                    .empty("w:ilvl", &[("w:val", &level.clamp(0, 3).to_string())]);
                self.xml.empty("w:numId", &[("w:val", &num_id.to_string())]);
                self.xml.end("w:numPr");
            }
            self.para_mark(mark);
            self.xml.end("w:pPr");
            self.comment_start();
            if self.direct {
                let marker = if ordered {
                    format!("{counter}. ")
                } else {
                    "• ".to_string()
                };
                let close = self.open_mark(mark);
                self.write_run(
                    &Run {
                        text: marker,
                        ..Default::default()
                    },
                    mark,
                );
                if let Some(el) = close {
                    self.xml.end(el);
                }
            }
            self.write_runs(&item.content, mark);
            self.comment_end();
            self.xml.end("w:p");
            for child in &item.children {
                if let BlockKind::List { ordered, items, .. } = &child.kind {
                    self.write_list(*ordered, items, level + 1, mark);
                }
            }
        }
    }

    fn table_open(&mut self, cols: usize) {
        self.xml.start("w:tbl", &[]);
        self.xml.start("w:tblPr", &[]);
        self.xml.empty("w:tblW", &[("w:w", "5000"), ("w:type", "pct")]);
        self.xml.start("w:tblBorders", &[]);
        for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
            self.xml.empty(
                &format!("w:{side}"),
                &[("w:val", "single"), ("w:sz", "4"), ("w:space", "0"), ("w:color", "A6A6A6")],
            );
        }
        self.xml.end("w:tblBorders");
        self.xml.end("w:tblPr");
        self.xml.start("w:tblGrid", &[]);
        for _ in 0..cols {
            self.xml
                .empty("w:gridCol", &[("w:w", &(9_000 / cols as u32).to_string())]);
        }
        self.xml.end("w:tblGrid");
    }

    fn write_row(&mut self, cells: &[Vec<Inline>], header: bool, mark: Mark) {
        self.xml.start("w:tr", &[]);
        self.xml.start("w:trPr", &[]);
        self.xml.empty("w:cantSplit", &[]);
        if header {
            self.xml.empty("w:tblHeader", &[]);
        }
        self.row_mark(mark);
        self.xml.end("w:trPr");
        for cell in cells {
            self.xml.start("w:tc", &[]);
            self.xml.start("w:tcPr", &[]);
            if header {
                self.xml.empty(
                    "w:shd",
                    &[("w:val", "clear"), ("w:color", "auto"), ("w:fill", "D9E2F3")],
                );
            }
            self.xml.end("w:tcPr");
            let inline: Vec<Inline> = if header {
                // Header cells render bold.
                cell.iter()
                    .map(|i| Inline::Strong(vec![i.clone()]))
                    .collect()
            } else {
                cell.clone()
            };
            self.write_paragraph(&inline, None, if header { Mark::None } else { mark });
            self.xml.end("w:tc");
        }
        self.xml.end("w:tr");
    }

    fn write_table(&mut self, rows: &[TableRow], mark: Mark) {
        let cols = rows.first().map(|r| r.cells.len()).unwrap_or(0).max(1);
        self.table_open(cols);
        for row in rows {
            self.write_row(&row.cells, row.is_header, mark);
        }
        self.xml.end("w:tbl");
        self.paragraph_open(Some("CodeGap"), mark, false);
        self.paragraph_close();
    }

    // -- diff steps ------------------------------------------------------

    fn write_step(&mut self, step: &Step, old_assets: &AssetMap, new_assets: &AssetMap) {
        match step {
            Step::Keep(block) => self.write_block(block, Mark::None, new_assets),
            Step::Add(block) => self.write_block(block, Mark::Insert, new_assets),
            Step::Remove(block) => self.write_block(block, Mark::Delete, old_assets),
            Step::Move { block, from } => {
                self.add_comment(format!("Block moved (was position {}).", from + 1));
                self.write_block(block, Mark::None, new_assets);
            }
            Step::Modify {
                old,
                new,
                content,
                fuzzy: _,
            } => self.write_modify(old, new, content, old_assets, new_assets),
        }
    }

    fn write_modify(
        &mut self,
        old: &Block,
        new: &Block,
        content: &ContentChange,
        old_assets: &AssetMap,
        new_assets: &AssetMap,
    ) {
        match content {
            ContentChange::Runs(ops) => {
                let style = match &new.kind {
                    BlockKind::Heading { level, .. } => {
                        Some(format!("Heading{}", (*level).clamp(1, 6)))
                    }
                    _ => None,
                };
                self.paragraph_open(style.as_deref(), Mark::None, false);
                for op in ops {
                    match op {
                        RunOp::Equal(run) => self.write_run(run, Mark::None),
                        RunOp::Delete(run) => {
                            if let Some(el) = self.open_mark(Mark::Delete) {
                                self.write_run(run, Mark::Delete);
                                self.xml.end(el);
                            }
                        }
                        RunOp::Insert(run) => {
                            if let Some(el) = self.open_mark(Mark::Insert) {
                                self.write_run(run, Mark::Insert);
                                self.xml.end(el);
                            }
                        }
                    }
                }
                self.paragraph_close();
            }
            ContentChange::Lines(ops) => {
                for op in ops {
                    let (text, mark) = op_line(op);
                    self.paragraph_open(Some("SourceCode"), mark, false);
                    let close = self.open_mark(mark);
                    self.write_run(
                        &Run {
                            code: true,
                            text: if text.is_empty() { " ".to_string() } else { text.to_string() },
                            ..Default::default()
                        },
                        mark,
                    );
                    if let Some(el) = close {
                        self.xml.end(el);
                    }
                    self.paragraph_close();
                }
                self.paragraph_open(Some("CodeGap"), Mark::None, false);
                self.paragraph_close();
            }
            ContentChange::Items(ops) => {
                let ordered = matches!(&new.kind, BlockKind::List { ordered: true, .. });
                let num_id = if ordered { 2 } else { 1 };
                for op in ops {
                    let (text, mark) = op_line(op);
                    self.xml.start("w:p", &[]);
                    self.xml.start("w:pPr", &[]);
                    self.xml.start("w:numPr", &[]);
                    self.xml.empty("w:ilvl", &[("w:val", "0")]);
                    self.xml.empty("w:numId", &[("w:val", &num_id.to_string())]);
                    self.xml.end("w:numPr");
                    self.para_mark(mark);
                    self.xml.end("w:pPr");
                    let close = self.open_mark(mark);
                    self.write_run(
                        &Run {
                            text: text.to_string(),
                            ..Default::default()
                        },
                        mark,
                    );
                    if let Some(el) = close {
                        self.xml.end(el);
                    }
                    self.xml.end("w:p");
                }
            }
            ContentChange::Rows(ops) => {
                let old_rows: Vec<TableRow> = match &old.kind {
                    BlockKind::Table { rows } => rows.clone(),
                    _ => Vec::new(),
                };
                let cols = old_rows
                    .first()
                    .map(|r| r.cells.len())
                    .unwrap_or(1)
                    .max(1);
                self.table_open(cols);
                let mut old_iter = old_rows.iter();
                for op in ops {
                    let mark = match op {
                        LineOp::Equal(_) => Mark::None,
                        LineOp::Delete(_) => Mark::Delete,
                        LineOp::Insert(_) => Mark::Insert,
                    };
                    let row_ref = match op {
                        LineOp::Equal(_) | LineOp::Delete(_) => old_iter.next(),
                        LineOp::Insert(_) => None,
                    };
                    let (cells, header) = match row_ref {
                        Some(row) => (row.cells.clone(), row.is_header),
                        None => {
                            let text = op_text(op);
                            (
                                text.split(" | ")
                                    .map(|c| vec![Inline::Text(c.to_string())])
                                    .collect(),
                                false,
                            )
                        }
                    };
                    self.write_row(&cells, header, mark);
                }
                self.xml.end("w:tbl");
                self.paragraph_open(Some("CodeGap"), Mark::None, false);
                self.paragraph_close();
            }
            ContentChange::Source { .. } => {
                self.add_comment("Diagram updated: replaced render; review the change.");
                self.in_block = true;
                if old_assets.get(&old.id).is_some() {
                    self.write_diagram(&old.id, Mark::Delete, old_assets);
                } else if let BlockKind::Diagram { source, .. } = &old.kind {
                    self.write_paragraph(
                        &[Inline::Text(format!("[diagram removed: {}]", truncate(source, 80)))],
                        Some("Quote"),
                        Mark::Delete,
                    );
                }
                if new_assets.get(&new.id).is_some() {
                    self.write_diagram(&new.id, Mark::Insert, new_assets);
                } else if let BlockKind::Diagram { source, .. } = &new.kind {
                    self.write_paragraph(
                        &[Inline::Text(format!("[updated diagram: {}]", truncate(source, 80)))],
                        Some("Quote"),
                        Mark::Insert,
                    );
                }
                self.in_block = false;
            }
            ContentChange::Replaced => {
                self.write_block(old, Mark::Delete, old_assets);
                self.write_block(new, Mark::Insert, new_assets);
            }
        }
    }
}

fn op_line(op: &LineOp) -> (&str, Mark) {
    match op {
        LineOp::Equal(t) => (t.as_str(), Mark::None),
        LineOp::Delete(t) => (t.as_str(), Mark::Delete),
        LineOp::Insert(t) => (t.as_str(), Mark::Insert),
    }
}

fn op_text(op: &LineOp) -> &str {
    match op {
        LineOp::Equal(t) | LineOp::Delete(t) | LineOp::Insert(t) => t.as_str(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..s.char_indices().take(max).last().map(|(i, c)| i + c.len_utf8()).unwrap_or(max)])
    }
}

// ---------------------------------------------------------------------------
// Package assembly

pub(crate) fn render(
    doc: &Document,
    assets: &AssetMap,
    revision: Option<RevisionMode<'_>>,
) -> Result<Vec<u8>> {
    let mut emitter = Emitter::new(revision);
    emitter.write_cover(&doc.frontmatter);
    match emitter.revision {
        Some(mode) => {
            for step in &mode.plan.steps {
                emitter.write_step(step, mode.old_assets, assets);
            }
        }
        None => {
            for block in &doc.blocks {
                emitter.write_block(block, Mark::None, assets);
            }
        }
    }

    let identity = emitter.identity();
    let (author, date) = (identity.0.to_string(), identity.1.to_string());
    let body_xml = emitter.xml.finish();
    let comments = emitter.comments;
    let media = emitter.media;
    let comments_xml = if comments.is_empty() {
        None
    } else {
        Some(build_comments_xml(&comments, &author, &date))
    };

    let creator = if doc.frontmatter.authors.is_empty() {
        author
    } else {
        doc.frontmatter.authors.join(", ")
    };
    assemble(doc, body_xml, media, comments_xml, &creator, &date)
}

fn build_comments_xml(comments: &[Comment], author: &str, date: &str) -> String {
    let initials: String = author
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(3)
        .collect::<String>()
        .to_uppercase();
    let mut xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:comments xmlns:w="{W_NS}">"#
    );
    for comment in comments {
        let text = comment
            .text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        xml.push_str(&format!(
            "\n<w:comment w:id=\"{}\" w:author=\"{}\" w:date=\"{}\" w:initials=\"{}\"><w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p></w:comment>",
            comment.id,
            super::xml::esc_attr(author),
            super::xml::esc_attr(date),
            super::xml::esc_attr(&initials),
        ));
    }
    xml.push_str("\n</w:comments>");
    xml
}

fn rel_kind(target: &str) -> &'static str {
    match target {
        "styles.xml" => "styles",
        "numbering.xml" => "numbering",
        "settings.xml" => "settings",
        "fontTable.xml" => "fontTable",
        "theme/theme1.xml" => "theme",
        "comments.xml" => "comments",
        _ => "image",
    }
}

fn assemble(
    doc: &Document,
    body_xml: Vec<u8>,
    media: Media,
    comments_xml: Option<String>,
    creator: &str,
    date: &str,
) -> Result<Vec<u8>> {
    let mut rels: Vec<(String, String)> = vec![
        ("rId100".into(), "styles.xml".into()),
        ("rId101".into(), "numbering.xml".into()),
        ("rId102".into(), "settings.xml".into()),
        ("rId103".into(), "fontTable.xml".into()),
        ("rId104".into(), "theme/theme1.xml".into()),
    ];
    if comments_xml.is_some() {
        rels.push(("rId105".into(), "comments.xml".into()));
    }
    rels.extend(media.rels.clone());

    let mut document_rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (rid, target) in &rels {
        document_rels.push_str(&format!(
            "\n<Relationship Id=\"{}\" Type=\"{REL_TYPE}{}\" Target=\"{}\"/>",
            super::xml::esc_attr(rid),
            rel_kind(target),
            super::xml::esc_attr(target),
        ));
    }
    document_rels.push_str("\n</Relationships>");

    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>"#,
    );
    if !media.files.is_empty() {
        content_types
            .push_str("\n<Default Extension=\"png\" ContentType=\"image/png\"/>");
    }
    let overrides: [(&str, &str); 8] = [
        ("/word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"),
        ("/word/styles.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"),
        ("/word/numbering.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"),
        ("/word/settings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"),
        ("/word/fontTable.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"),
        ("/word/theme/theme1.xml", "application/vnd.openxmlformats-officedocument.theme+xml"),
        ("/docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml"),
        ("/docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml"),
    ];
    for (part, ctype) in overrides {
        content_types.push_str(&format!(
            "\n<Override PartName=\"{part}\" ContentType=\"{ctype}\"/>"
        ));
    }
    if comments_xml.is_some() {
        content_types.push_str("\n<Override PartName=\"/word/comments.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml\"/>");
    }
    content_types.push_str("\n</Types>");

    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>"#;

    let namespaces = concat!(
        "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" ",
        "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" ",
        "xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" ",
        "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" ",
        "xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\""
    );
    let mut doc_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document {namespaces}><w:body>"
    )
    .into_bytes();
    doc_xml.extend_from_slice(&body_xml);
    doc_xml.extend_from_slice(
        b"<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" w:header=\"709\" w:footer=\"709\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>",
    );

    let mut package = Package::new();
    package.add("[Content_Types].xml", content_types.into_bytes());
    package.add("_rels/.rels", root_rels.as_bytes().to_vec());
    package.add(
        "docProps/core.xml",
        parts::core_xml(doc.frontmatter.title.as_deref(), creator, date).into_bytes(),
    );
    package.add("docProps/app.xml", parts::APP_XML.as_bytes().to_vec());
    package.add("word/document.xml", doc_xml);
    package.add("word/_rels/document.xml.rels", document_rels.into_bytes());
    package.add("word/styles.xml", parts::STYLES_XML.as_bytes().to_vec());
    package.add("word/numbering.xml", parts::NUMBERING_XML.as_bytes().to_vec());
    package.add("word/settings.xml", parts::SETTINGS_XML.as_bytes().to_vec());
    package.add("word/fontTable.xml", parts::FONTTABLE_XML.as_bytes().to_vec());
    package.add("word/theme/theme1.xml", parts::THEME_XML.as_bytes().to_vec());
    if let Some(comments) = comments_xml {
        package.add("word/comments.xml", comments.into_bytes());
    }
    for (name, bytes) in media.files {
        package.add(name, bytes);
    }
    package.build()
}

pub(crate) fn validate(bytes: &[u8]) -> docgen_core::Result<()> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| docgen_core::Error::Backend(format!("not a zip: {e}")))?;
    let mut xml = String::new();
    {
        let mut document = zip
            .by_name("word/document.xml")
            .map_err(|_| docgen_core::Error::Backend("missing word/document.xml".into()))?;
        std::io::Read::read_to_string(&mut document, &mut xml)
            .map_err(|e| docgen_core::Error::Backend(format!("reading document.xml: {e}")))?;
    }

    let mut reader = Reader::from_str(&xml);
    use quick_xml::events::Event;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(docgen_core::Error::Backend(format!(
                    "document.xml is not well-formed XML: {e}"
                )))
            }
        }
    }
    Ok(())
}

// Keep Error import used even if future refactors drop call sites.
const _: Option<Error> = None;
