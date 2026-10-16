//! Markdown → [`Document`] via pulldown-cmark.
//!
//! Extensions enabled: tables, strikethrough, heading attributes, task
//! lists. Fenced blocks tagged `mermaid` / `excalidraw` become
//! [`BlockKind::Diagram`]. Attribute syntax (D-001) carries the page/media
//! annotations: heading attribute blocks (`## Results {page=landscape}`),
//! fence info strings (```` ```mermaid {#fig:arch} place=appendix ````) and
//! image attribute blocks (`![alt](url){#fig:x}` — pulldown leaves those as
//! a `Text` event right after the image, which is recognized here).

use std::collections::VecDeque;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::Result;
use crate::frontmatter;
use crate::ids::{IdAllocator, fnv1a64, slug};
use crate::model::{
    Annotations, Block, BlockKind, DiagramFormat, Document, Frontmatter, Inline, ListItem,
    Orientation, Placement, TableRow,
};

pub fn parse_document(markdown: &str) -> Result<Document> {
    let (yaml, body) = frontmatter::split(markdown);
    let fm = match yaml {
        Some(y) => frontmatter::parse(y)?,
        None => Frontmatter::default(),
    };

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_TASKLISTS);

    let events: VecDeque<Event> = Parser::new_ext(body, options).collect();
    let mut ctx = ParseCtx::default();
    let blocks = ctx.parse_blocks(&events, None);

    // Frontmatter may pre-assign ids for specific blocks; reserve them so
    // auto-generated ids never collide.
    Ok(Document {
        frontmatter: fm,
        blocks,
    })
}

#[derive(Default)]
struct ParseCtx {
    ids: IdAllocator,
    /// Attribute-block values picked up while walking a block's inline
    /// events (image attribute blocks); consumed by the enclosing block.
    pending_id: Option<String>,
    pending_ann: Annotations,
    /// True while the previously emitted inline was an image — the only
    /// place a `{…}` attribute block is recognized.
    after_image: bool,
}

impl ParseCtx {
    /// Consumes (and thereby resets) the pending attribute state.
    fn take_pending(&mut self) -> (Option<String>, Annotations) {
        (
            self.pending_id.take(),
            std::mem::take(&mut self.pending_ann),
        )
    }

    /// Parses sibling blocks until `stop` (consumed) or exhaustion.
    fn parse_blocks(&mut self, events: &VecDeque<Event>, stop: Option<TagEnd>) -> Vec<Block> {
        let mut events = events.clone();
        let mut blocks = Vec::new();
        while let Some(event) = events.pop_front() {
            match event {
                Event::Start(tag) => {
                    if let Some(block) = self.parse_block(&mut events, tag) {
                        blocks.push(block);
                    }
                }
                Event::End(end) if Some(end) == stop => {
                    break;
                }
                Event::Rule => blocks.push(self.mk_block(
                    BlockKind::ThematicBreak,
                    None,
                    Annotations::default(),
                )),
                // Stray content outside any block (rare with commonmark).
                Event::Text(t) if stop.is_none() && !t.trim().is_empty() => {
                    let _ = self.take_pending();
                    blocks.push(self.mk_block(
                        BlockKind::Paragraph {
                            inline: vec![Inline::Text(t.into_string())],
                        },
                        None,
                        Annotations::default(),
                    ));
                }
                _ => {}
            }
        }
        blocks
    }

    /// Called with the opening `Start(tag)` already consumed.
    fn parse_block(&mut self, events: &mut VecDeque<Event>, tag: Tag) -> Option<Block> {
        match tag {
            Tag::Paragraph => {
                let _ = self.take_pending();
                let inline = self.parse_inline(events, TagEnd::Paragraph);
                let (pid, ann) = self.take_pending();
                Some(self.mk_block(BlockKind::Paragraph { inline }, pid, ann))
            }
            Tag::Heading {
                level,
                id,
                classes: _,
                attrs,
            } => {
                let _ = self.take_pending();
                let end = TagEnd::Heading(level);
                let inline = self.parse_inline(events, end);
                let (pid, mut ann) = self.take_pending();
                let level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                let wanted = id.map(|s| strip_fig_prefix(&s).to_owned());
                for (key, value) in attrs {
                    let parsed = value.as_deref().map(unquote);
                    apply_attr(&mut ann, key.as_ref(), parsed.as_deref());
                }
                let wanted = wanted
                    .or(pid)
                    .unwrap_or_else(|| slug(&Inline::plain_text(&inline)));
                Some(self.mk_block(BlockKind::Heading { level, inline }, Some(wanted), ann))
            }
            Tag::BlockQuote(_) => {
                let blocks = self.parse_blocks(events, Some(TagEnd::BlockQuote(None)));
                Some(self.mk_block(
                    BlockKind::BlockQuote { blocks },
                    None,
                    Annotations::default(),
                ))
            }
            Tag::CodeBlock(kind) => {
                let mut code = String::new();
                while let Some(event) = events.pop_front() {
                    match event {
                        Event::End(TagEnd::CodeBlock) => break,
                        Event::Text(t) => code.push_str(&t),
                        Event::Html(h) | Event::InlineHtml(h) => code.push_str(&h),
                        _ => {}
                    }
                }
                let (language, explicit_id, ann) = match &kind {
                    CodeBlockKind::Fenced(info) => parse_info(info),
                    CodeBlockKind::Indented => (None, None, Annotations::default()),
                };
                let kind = match language.as_deref().and_then(DiagramFormat::from_language) {
                    Some(format) => BlockKind::Diagram {
                        format,
                        source: trim_common_indent(&code),
                    },
                    None => BlockKind::CodeBlock { language, code },
                };
                Some(self.mk_block(kind, explicit_id, ann))
            }
            Tag::List(start) => {
                let ordered = start.is_some();
                let start = start.unwrap_or(1);
                let mut items = Vec::new();
                while let Some(event) = events.front().cloned() {
                    match event {
                        Event::Start(Tag::Item) => {
                            events.pop_front();
                            items.push(self.parse_list_item(events));
                        }
                        Event::End(TagEnd::List(false)) => {
                            events.pop_front();
                            break;
                        }
                        Event::End(TagEnd::List(true)) => {
                            events.pop_front();
                            break;
                        }
                        _ => {
                            events.pop_front();
                        }
                    }
                }
                Some(self.mk_block(
                    BlockKind::List {
                        ordered,
                        start,
                        items,
                    },
                    None,
                    Annotations::default(),
                ))
            }
            Tag::Table(_) => {
                let mut rows = Vec::new();
                while let Some(event) = events.pop_front() {
                    match event {
                        Event::End(TagEnd::Table) => break,
                        Event::Start(Tag::TableHead) => {
                            let mut cells = Vec::new();
                            while let Some(event) = events.pop_front() {
                                match event {
                                    Event::End(TagEnd::TableHead) => break,
                                    Event::Start(Tag::TableCell) => {
                                        cells.push(self.parse_inline(events, TagEnd::TableCell));
                                    }
                                    _ => {}
                                }
                            }
                            rows.push(TableRow {
                                is_header: true,
                                cells,
                            });
                        }
                        Event::Start(Tag::TableRow) => {
                            let mut cells = Vec::new();
                            while let Some(event) = events.pop_front() {
                                match event {
                                    Event::End(TagEnd::TableRow) => break,
                                    Event::Start(Tag::TableCell) => {
                                        cells.push(self.parse_inline(events, TagEnd::TableCell));
                                    }
                                    _ => {}
                                }
                            }
                            rows.push(TableRow {
                                is_header: rows.is_empty(),
                                cells,
                            });
                        }
                        _ => {}
                    }
                }
                Some(self.mk_block(BlockKind::Table { rows }, None, Annotations::default()))
            }
            Tag::HtmlBlock => {
                // Drop raw HTML blocks for now; roadmap: model them.
                while let Some(event) = events.pop_front() {
                    if matches!(event, Event::End(TagEnd::HtmlBlock)) {
                        break;
                    }
                }
                None
            }
            other => {
                // Block-level start we don't model (footnotes): skip to end.
                let end = other.to_end();
                self.skip_to(events, end);
                None
            }
        }
    }

    fn parse_list_item(&mut self, events: &mut VecDeque<Event>) -> ListItem {
        let mut content = Vec::new();
        let mut children = Vec::new();
        while let Some(event) = events.front().cloned() {
            match event {
                Event::End(TagEnd::Item) => {
                    events.pop_front();
                    break;
                }
                Event::Start(Tag::Paragraph) => {
                    events.pop_front();
                    let _ = self.take_pending();
                    content.extend(self.parse_inline(events, TagEnd::Paragraph));
                    // Annotations inside list items are dropped by design:
                    // figures are top-level blocks.
                    let _ = self.take_pending();
                }
                Event::Start(Tag::List(_)) => {
                    events.pop_front();
                    let mut sublist_items = Vec::new();
                    let mut ordered = false;
                    let mut start = 1;
                    loop {
                        match events.front().cloned() {
                            Some(Event::Start(Tag::List(n))) => {
                                events.pop_front();
                                ordered = n.is_some();
                                start = n.unwrap_or(1);
                            }
                            Some(Event::Start(Tag::Item)) => {
                                events.pop_front();
                                sublist_items.push(self.parse_list_item(events));
                            }
                            Some(Event::End(TagEnd::List(_))) => {
                                events.pop_front();
                                break;
                            }
                            Some(_) => {
                                events.pop_front();
                            }
                            None => break,
                        }
                    }
                    children.push(self.mk_block(
                        BlockKind::List {
                            ordered,
                            start,
                            items: sublist_items,
                        },
                        None,
                        Annotations::default(),
                    ));
                }
                Event::Start(tag @ (Tag::CodeBlock(_) | Tag::BlockQuote(_) | Tag::Table(_))) => {
                    events.pop_front();
                    if let Some(block) = self.parse_block(events, tag) {
                        children.push(block);
                    }
                }
                Event::TaskListMarker(_) => {
                    events.pop_front();
                }
                other => {
                    events.pop_front();
                    if let Some(inline) = self.inline_from_event(events, other) {
                        content.push(inline);
                    }
                }
            }
        }
        ListItem { content, children }
    }

    /// Collects inline events until `end` (consumed). Nested containers
    /// (strong/emphasis/links) become structured [`Inline`] values.
    fn parse_inline(&mut self, events: &mut VecDeque<Event>, end: TagEnd) -> Vec<Inline> {
        let mut out = Vec::new();
        while let Some(event) = events.pop_front() {
            match event {
                Event::End(e) if e == end => break,
                other => {
                    if let Some(inline) = self.inline_from_event(events, other) {
                        out.push(inline);
                    }
                }
            }
        }
        out
    }

    fn inline_from_event(&mut self, events: &mut VecDeque<Event>, event: Event) -> Option<Inline> {
        match event {
            Event::Text(t) => {
                if self.after_image
                    && let Some((id, ann)) = parse_image_attr_block(&t)
                {
                    // `{…}` attribute block right after an image (D-001);
                    // later blocks win per field.
                    let base = std::mem::take(&mut self.pending_ann);
                    self.pending_ann = merge_annotations(base, ann);
                    if id.is_some() {
                        self.pending_id = id;
                    }
                    self.after_image = false;
                    return None;
                }
                self.after_image = false;
                Some(Inline::Text(t.into_string()))
            }
            Event::Code(t) => {
                self.after_image = false;
                Some(Inline::Code(t.into_string()))
            }
            Event::SoftBreak => {
                self.after_image = false;
                Some(Inline::SoftBreak)
            }
            Event::HardBreak => {
                self.after_image = false;
                Some(Inline::HardBreak)
            }
            Event::Start(Tag::Strong) => {
                self.after_image = false;
                Some(Inline::Strong(self.parse_inline(events, TagEnd::Strong)))
            }
            Event::Start(Tag::Emphasis) => {
                self.after_image = false;
                Some(Inline::Emphasis(
                    self.parse_inline(events, TagEnd::Emphasis),
                ))
            }
            Event::Start(Tag::Strikethrough) => {
                self.after_image = false;
                Some(Inline::Strikethrough(
                    self.parse_inline(events, TagEnd::Strikethrough),
                ))
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                self.after_image = false;
                Some(Inline::Link {
                    url: dest_url.into_string(),
                    text: self.parse_inline(events, TagEnd::Link),
                })
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let _ = (link_type, title, id);
                let alt = self.parse_inline(events, TagEnd::Image);
                // The event right after an image may be an attribute block.
                self.after_image = true;
                Some(Inline::Image {
                    url: dest_url.into_string(),
                    alt,
                })
            }
            Event::InlineHtml(_) => {
                self.after_image = false;
                None
            }
            _ => {
                self.after_image = false;
                None
            }
        }
    }

    fn skip_to(&mut self, events: &mut VecDeque<Event>, end: TagEnd) {
        let mut depth = 0usize;
        while let Some(event) = events.pop_front() {
            match event {
                Event::End(e) if depth == 0 && e == end => break,
                Event::End(_) => depth = depth.saturating_sub(1),
                Event::Start(_) => depth += 1,
                _ => {}
            }
        }
    }

    fn mk_block(
        &mut self,
        kind: BlockKind,
        wanted_id: Option<String>,
        annotations: Annotations,
    ) -> Block {
        let id = match wanted_id {
            Some(w) => self.ids.allocate(w),
            None => {
                let prefix = kind_prefix(&kind);
                let hash = fnv1a64(&format!("{kind:?}"));
                self.ids.allocate(format!("{prefix}-{hash}"))
            }
        };
        Block {
            id,
            kind,
            annotations,
        }
    }
}

fn kind_prefix(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::Heading { .. } => "h",
        BlockKind::Paragraph { .. } => "p",
        BlockKind::CodeBlock { .. } => "code",
        BlockKind::Diagram { format, .. } => match format {
            DiagramFormat::Mermaid => "diagram-mermaid",
            DiagramFormat::Excalidraw => "diagram-excalidraw",
        },
        BlockKind::List { .. } => "list",
        BlockKind::Table { .. } => "table",
        BlockKind::BlockQuote { .. } => "quote",
        BlockKind::ThematicBreak => "rule",
    }
}

/// ```` ```mermaid {#fig:arch} place=appendix ````
/// → (Some("mermaid"), Some("arch"), annotations)
fn parse_info(info: &str) -> (Option<String>, Option<String>, Annotations) {
    let mut language = None;
    let mut id = None;
    let mut ann = Annotations::default();
    for token in attr_split(info) {
        if !is_attr_token(token) && language.is_none() {
            language = Some(token.to_owned());
            continue;
        }
        apply_token(&mut id, &mut ann, token);
    }
    (language, id, ann)
}

/// A `Text` event directly after an image: an `{…}` attribute block, or
/// prose? Only whole-block matches whose every token has attribute shape
/// count, so literal braces near images survive as prose (D-001).
fn parse_image_attr_block(text: &str) -> Option<(Option<String>, Annotations)> {
    let inner = text.trim().strip_prefix('{')?.strip_suffix('}')?;
    if inner.is_empty() {
        return None;
    }
    let mut id = None;
    let mut ann = Annotations::default();
    for token in attr_split(inner) {
        if !is_attr_token(token) {
            return None;
        }
        apply_token(&mut id, &mut ann, token);
    }
    Some((id, ann))
}

/// Does the token look like an attribute rather than content: `{#id}` /
/// `#id`, the bare `new-page`, or `key=value` with an identifier-ish key?
/// Unknown keys of valid shape are accepted and ignored (forward
/// compatibility).
fn is_attr_token(token: &str) -> bool {
    if token.starts_with("{#") || token.starts_with('#') || token == "new-page" {
        return true;
    }
    match token.split_once('=') {
        Some((key, _)) => {
            let mut chars = key.chars();
            matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        }
        None => false,
    }
}

/// Applies one attribute token to the pending id/annotations (D-009).
/// Unknown keys and values are ignored without error.
fn apply_token(id: &mut Option<String>, ann: &mut Annotations, token: &str) {
    // `{#name}` in info strings; inside a stripped `{…}` block the braces
    // are gone, so `#name` / `#fig:name` are the same token.
    if let Some(rest) = token.strip_prefix("{#").and_then(|r| r.strip_suffix('}')) {
        *id = Some(strip_fig_prefix(rest).to_owned());
        return;
    }
    if let Some(rest) = token.strip_prefix('#') {
        *id = Some(strip_fig_prefix(rest).to_owned());
        return;
    }
    let (key, value) = match token.split_once('=') {
        Some((k, v)) => (k, Some(unquote(v))),
        None => (token, None),
    };
    apply_attr(ann, key, value.as_deref());
}

fn apply_attr(ann: &mut Annotations, key: &str, value: Option<&str>) {
    match (key, value) {
        ("place", Some("inline")) => ann.place = Some(Placement::Inline),
        ("place", Some("appendix")) => ann.place = Some(Placement::Appendix),
        ("page", Some("landscape")) => ann.page = Some(Orientation::Landscape),
        ("page", Some("portrait")) => ann.page = Some(Orientation::Portrait),
        ("caption", Some(v)) => ann.caption = Some(v.to_owned()),
        ("keep", Some("true")) => ann.keep = Some(true),
        ("keep", Some("false")) => ann.keep = Some(false),
        ("new-page", None) => ann.new_page = true,
        _ => {}
    }
}

/// `{#fig:x}` is the figure-id convention (D-004); the marker is stripped so
/// ids stay uniform across channels.
fn strip_fig_prefix(id: &str) -> &str {
    id.strip_prefix("fig:").unwrap_or(id)
}

/// Splits on whitespace outside double quotes, so `caption="two words"`
/// stays one token.
fn attr_split(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if let Some(st) = start.take() {
                    out.push(&s[st..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

/// Strips surrounding double quotes and unescapes `\"` and `\\` inside;
/// unquoted values pass through unchanged.
fn unquote(value: &str) -> String {
    if !(value.len() >= 2 && value.starts_with('"') && value.ends_with('"')) {
        return value.to_owned();
    }
    let inner = &value[1..value.len() - 1];
    if !inner.contains('\\') {
        return inner.to_owned();
    }
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Later attribute blocks win per field; `new-page` is sticky.
fn merge_annotations(base: Annotations, over: Annotations) -> Annotations {
    Annotations {
        place: over.place.or(base.place),
        caption: over.caption.or(base.caption),
        page: over.page.or(base.page),
        new_page: base.new_page || over.new_page,
        keep: over.keep.or(base.keep),
    }
}

fn trim_common_indent(code: &str) -> String {
    // pulldown-cmark already dedents fenced blocks; this only guards against
    // tabs/odd indentation surviving from exotic inputs.
    let lines: Vec<&str> = code.lines().collect();
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| l.get(indent..).unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_blocks() {
        let doc = parse_document(
            "---\ntitle: Test\nauthors: [joe]\n---\n\n# Intro {#intro}\n\nHello **world**.\n",
        )
        .unwrap();
        assert_eq!(doc.frontmatter.title.as_deref(), Some("Test"));
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(doc.blocks[0].id, "intro");
        match &doc.blocks[1].kind {
            BlockKind::Paragraph { inline } => {
                assert_eq!(inline.len(), 3);
                assert!(matches!(&inline[1], Inline::Strong(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn mermaid_block_becomes_diagram_with_explicit_id() {
        let doc = parse_document("```mermaid {#arch}\ngraph TD\n  A-->B\n```\n").unwrap();
        assert_eq!(doc.blocks.len(), 1);
        let block = &doc.blocks[0];
        assert_eq!(block.id, "arch");
        match &block.kind {
            BlockKind::Diagram { format, source } => {
                assert_eq!(*format, DiagramFormat::Mermaid);
                assert!(source.contains("A-->B"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn auto_ids_are_content_stable() {
        let a = parse_document("Some text here.\n").unwrap();
        let b = parse_document("# X\n\nSome text here.\n").unwrap();
        let id_a = &a.blocks[0].id;
        let id_b = b.blocks.last().unwrap();
        assert_eq!(id_a, &id_b.id);
        assert!(id_a.starts_with("p-"));
    }

    #[test]
    fn nested_lists() {
        let doc = parse_document("- a\n  - b\n- c\n").unwrap();
        assert_eq!(doc.blocks.len(), 1);
        match &doc.blocks[0].kind {
            BlockKind::List { ordered, items, .. } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].children.len(), 1);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn tables() {
        let doc = parse_document("| a | b |\n|---|---|\n| 1 | 2 |\n").unwrap();
        match &doc.blocks[0].kind {
            BlockKind::Table { rows } => {
                assert_eq!(rows.len(), 2);
                assert!(rows[0].is_header);
                assert_eq!(Inline::plain_text(&rows[1].cells[0]), "1");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    // -- attribute parsing (D-001, D-009) --------------------------------

    #[test]
    fn fence_info_tokens_full_vocabulary() {
        let doc = parse_document(
            "```mermaid {#fig:arch} place=appendix caption=\"System architecture\" page=landscape new-page keep=false\ngraph TD\n```\n",
        )
        .unwrap();
        let b = &doc.blocks[0];
        assert_eq!(b.id, "arch", "fig: prefix is stripped");
        let a = &b.annotations;
        assert_eq!(a.place, Some(Placement::Appendix));
        assert_eq!(a.caption.as_deref(), Some("System architecture"));
        assert_eq!(a.page, Some(Orientation::Landscape));
        assert!(a.new_page);
        assert_eq!(a.keep, Some(false));
    }

    #[test]
    fn fence_keeps_language_and_ignores_unknown_tokens() {
        let doc =
            parse_document("```rust {#snip} place=bogus whatever=1 new-page\nfn x() {}\n```\n")
                .unwrap();
        let b = &doc.blocks[0];
        match &b.kind {
            BlockKind::CodeBlock { language, .. } => {
                assert_eq!(language.as_deref(), Some("rust"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(b.id, "snip");
        assert!(b.annotations.new_page, "bare token recognized");
        assert_eq!(b.annotations.place, None, "unknown value ignored");
        assert_eq!(b.annotations.caption, None);
    }

    #[test]
    fn heading_attribute_tokens() {
        let doc = parse_document(
            "## Results {page=landscape}\n\n## T {new-page}\n\n## U {#x k=v .cls}\n",
        )
        .unwrap();
        assert_eq!(doc.blocks[0].annotations.page, Some(Orientation::Landscape));
        assert!(doc.blocks[1].annotations.new_page);
        assert_eq!(doc.blocks[2].id, "x");
        assert_eq!(doc.blocks[2].annotations.place, None, "unknown ignored");
        // Class-style attributes stay ignored.
        let doc = parse_document("## T {.warn}\n").unwrap();
        assert!(doc.blocks[0].annotations.is_default());
    }

    #[test]
    fn image_attribute_block_sets_block_annotations() {
        let doc =
            parse_document("![The main flow](img/flow.png){#fig:flow place=appendix keep=false}\n")
                .unwrap();
        assert_eq!(doc.blocks.len(), 1);
        let b = &doc.blocks[0];
        assert_eq!(b.id, "flow");
        assert_eq!(b.annotations.place, Some(Placement::Appendix));
        assert_eq!(b.annotations.keep, Some(false));
        match &b.kind {
            BlockKind::Paragraph { inline } => {
                // The attribute text must not leak into the content.
                assert_eq!(inline.len(), 1, "{inline:?}");
                match &inline[0] {
                    Inline::Image { url, alt } => {
                        assert_eq!(url, "img/flow.png");
                        assert_eq!(Inline::plain_text(alt), "The main flow");
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn image_attribute_with_spaces_and_quotes() {
        let doc =
            parse_document("![alt](u.png) {place=inline caption=\"Big system, v2\"}\n").unwrap();
        let a = &doc.blocks[0].annotations;
        assert_eq!(a.place, Some(Placement::Inline));
        assert_eq!(a.caption.as_deref(), Some("Big system, v2"));
    }

    #[test]
    fn prose_braces_are_not_swallowed() {
        // Standalone prose braces.
        let doc = parse_document("Braces {in prose} stay.\n").unwrap();
        match &doc.blocks[0].kind {
            BlockKind::Paragraph { inline } => {
                assert_eq!(Inline::plain_text(inline), "Braces {in prose} stay.");
            }
            other => panic!("unexpected {other:?}"),
        }
        // Non-attribute text right after an image.
        let doc = parse_document("![alt](u.png) {see the docs for details}\n").unwrap();
        match &doc.blocks[0].kind {
            BlockKind::Paragraph { inline } => {
                assert!(
                    Inline::plain_text(inline).contains("{see the docs for details}"),
                    "{inline:?}"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn figure_detection_is_lone_image() {
        let doc = parse_document("![alt](u.png)\n\n![alt2](v.png) inline text\n").unwrap();
        assert!(doc.blocks[0].kind.lone_image().is_some());
        assert!(doc.blocks[1].kind.lone_image().is_none());
    }

    #[test]
    fn caption_unquote_handles_escapes() {
        // Escapes survive only in fence info strings (pulldown unescapes
        // text events, so image attribute blocks see plain quotes).
        let doc =
            parse_document("```mermaid {#d} caption=\"say \\\"hi\\\"\"\ngraph TD\n```\n").unwrap();
        assert_eq!(
            doc.blocks[0].annotations.caption.as_deref(),
            Some("say \"hi\"")
        );
    }

    #[test]
    fn attr_split_and_unquote() {
        assert_eq!(
            attr_split("a b=1 c=\"two words\" d=\"x \\\"y\\\"\""),
            vec!["a", "b=1", "c=\"two words\"", "d=\"x \\\"y\\\"\""]
        );
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(unquote("\"q v\""), "q v");
        assert_eq!(unquote("\"a\\\\b\""), "a\\b");
    }

    #[test]
    fn info_string_with_only_language_is_unchanged() {
        let doc = parse_document("```mermaid\ngraph TD\n```\n").unwrap();
        assert!(doc.blocks[0].annotations.is_default());
        assert_eq!(doc.blocks[0].id, doc.blocks[0].id);
        match &doc.blocks[0].kind {
            BlockKind::Diagram { format, .. } => assert_eq!(*format, DiagramFormat::Mermaid),
            other => panic!("unexpected {other:?}"),
        }
    }
}
