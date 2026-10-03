//! Markdown → [`Document`] via pulldown-cmark.
//!
//! Extensions enabled: tables, strikethrough, heading attributes (`{#id}`),
//! task lists. Fenced blocks tagged `mermaid` / `excalidraw` (optionally with
//! an `{#id}` attribute in the info string) become [`BlockKind::Diagram`].

use std::collections::VecDeque;

use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

use crate::frontmatter;
use crate::ids::{fnv1a64, slug, IdAllocator};
use crate::model::{
    Block, BlockKind, DiagramFormat, Document, Frontmatter, Inline, ListItem, TableRow,
};
use crate::Result;

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
}

impl ParseCtx {
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
                Event::End(end) => {
                    if Some(end) == stop {
                        break;
                    }
                }
                Event::Rule => blocks.push(self.mk_block(BlockKind::ThematicBreak, None)),
                // Stray content outside any block (rare with commonmark).
                Event::Text(t) if stop.is_none() => {
                    if !t.trim().is_empty() {
                        blocks.push(self.mk_block(
                            BlockKind::Paragraph {
                                inline: vec![Inline::Text(t.into_string())],
                            },
                            None,
                        ));
                    }
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
                let inline = self.parse_inline(events, TagEnd::Paragraph);
                Some(self.mk_block(BlockKind::Paragraph { inline }, None))
            }
            Tag::Heading { level, id, .. } => {
                let end = TagEnd::Heading(level);
                let inline = self.parse_inline(events, end);
                let level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                let wanted = id
                    .map(|s| s.into_string())
                    .unwrap_or_else(|| slug(&Inline::plain_text(&inline)));
                Some(self.mk_block(
                    BlockKind::Heading { level, inline },
                    Some(wanted),
                ))
            }
            Tag::BlockQuote(_) => {
                let blocks = self.parse_blocks(events, Some(TagEnd::BlockQuote(None)));
                Some(self.mk_block(BlockKind::BlockQuote { blocks }, None))
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
                let (language, explicit_id) = match &kind {
                    CodeBlockKind::Fenced(info) => parse_info(info),
                    CodeBlockKind::Indented => (None, None),
                };
                let kind = match language
                    .as_deref()
                    .and_then(DiagramFormat::from_language)
                {
                    Some(format) => BlockKind::Diagram {
                        format,
                        source: trim_common_indent(&code),
                    },
                    None => BlockKind::CodeBlock { language, code },
                };
                Some(self.mk_block(kind, explicit_id))
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
                Some(self.mk_block(BlockKind::Table { rows }, None))
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
                    content.extend(self.parse_inline(events, TagEnd::Paragraph));
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

    fn inline_from_event(
        &mut self,
        events: &mut VecDeque<Event>,
        event: Event,
    ) -> Option<Inline> {
        match event {
            Event::Text(t) => Some(Inline::Text(t.into_string())),
            Event::Code(t) => Some(Inline::Code(t.into_string())),
            Event::SoftBreak => Some(Inline::SoftBreak),
            Event::HardBreak => Some(Inline::HardBreak),
            Event::Start(Tag::Strong) => Some(Inline::Strong(
                self.parse_inline(events, TagEnd::Strong),
            )),
            Event::Start(Tag::Emphasis) => Some(Inline::Emphasis(
                self.parse_inline(events, TagEnd::Emphasis),
            )),
            Event::Start(Tag::Strikethrough) => Some(Inline::Strikethrough(
                self.parse_inline(events, TagEnd::Strikethrough),
            )),
            Event::Start(Tag::Link { dest_url, .. }) => Some(Inline::Link {
                url: dest_url.into_string(),
                text: self.parse_inline(events, TagEnd::Link),
            }),
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                let _ = (link_type, title, id);
                Some(Inline::Image {
                    url: dest_url.into_string(),
                    alt: self.parse_inline(events, TagEnd::Image),
                })
            }
            Event::InlineHtml(_) => None,
            _ => None,
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

    fn mk_block(&mut self, kind: BlockKind, wanted_id: Option<String>) -> Block {
        let id = match wanted_id {
            Some(w) => self.ids.allocate(w),
            None => {
                let prefix = kind_prefix(&kind);
                let hash = fnv1a64(&format!("{kind:?}"));
                self.ids.allocate(format!("{prefix}-{hash}"))
            }
        };
        Block { id, kind }
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

/// ```` ```mermaid {#arch} ```` → (Some("mermaid"), Some("arch"))
fn parse_info(info: &str) -> (Option<String>, Option<String>) {
    let mut language = None;
    let mut id = None;
    for token in info.split_whitespace() {
        if let Some(rest) = token.strip_prefix("{#") {
            if let Some(name) = rest.strip_suffix('}') {
                id = Some(name.to_owned());
                continue;
            }
        }
        if language.is_none() {
            language = Some(token.to_owned());
        }
    }
    (language, id)
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
        + if code.ends_with('\n') { "" } else { "" }
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
}
