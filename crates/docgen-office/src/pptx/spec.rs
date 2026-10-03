//! The markdown → slides layout algorithm: which blocks land on which
//! slide, and (in diff mode) which revision tag each slide carries.
//!
//! Rules of thumb:
//! - frontmatter becomes a title slide;
//! - every H1/H2 opens a content slide (deeper headings become bold
//!   bullets); the first paragraph after a heading is the unbulleted lead;
//! - bullets accumulate, overflowing onto "(cont.)" slides;
//! - tables, diagrams and code blocks get a slide of their own (sharing it
//!   with the heading and lead that precede them);
//! - in diff mode, slides pick up a colored tag from the worst change
//!   touching their blocks, removed content gets a summary slide, and the
//!   deck ends with a revision summary.

use std::collections::HashMap;

use docgen_core::diff::{DiffPlan, Step};
use docgen_core::model::{Block, BlockKind, Document, Frontmatter, Inline, ListItem, TableRow};

/// Rough bullet-line budget before a slide overflows onto a continuation.
const MAX_LINES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideKind {
    Title,
    Content,
}

#[derive(Debug, Clone)]
pub struct SlideSpec {
    pub kind: SlideKind,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub byline: Option<String>,
    pub elems: Vec<Elem>,
    /// Revision chip ("new"/"updated"/"moved"/"removed"), diff mode only.
    pub tag: Option<&'static str>,
}

impl SlideSpec {
    fn content(title: Option<String>) -> Self {
        Self {
            kind: SlideKind::Content,
            title,
            subtitle: None,
            byline: None,
            elems: Vec::new(),
            tag: None,
        }
    }

    fn line_count(&self) -> usize {
        let mut n = 0;
        for e in &self.elems {
            match e {
                Elem::Lead(_) | Elem::Quote(_) => n += 2,
                Elem::Bullets(b) => n += b.len(),
                Elem::Table(t) => n += t.len().max(2),
                Elem::Picture { .. } | Elem::Code { .. } => n += MAX_LINES,
            }
        }
        n
    }

    fn has_fullbleed(&self) -> bool {
        self.elems
            .iter()
            .any(|e| matches!(e, Elem::Table(_) | Elem::Picture { .. } | Elem::Code { .. }))
    }
}

#[derive(Debug, Clone)]
pub enum Elem {
    /// Unbulleted intro paragraph under the title.
    Lead(Vec<Inline>),
    Bullets(Vec<Bullet>),
    Table(Vec<TableRow>),
    /// Rendered diagram image, keyed by block id.
    Picture { block_id: String, alt: String },
    Code {
        language: Option<String>,
        code: String,
    },
    Quote(Vec<Inline>),
}

#[derive(Debug, Clone)]
pub struct Bullet {
    pub level: u8,
    pub ordered: bool,
    /// Explicit number for ordered items (rendered via buAutoNum startAt).
    pub number: u64,
    pub runs: Vec<Inline>,
}

/// Builds slide specs for a fresh render.
pub fn slides_for(doc: &Document) -> Vec<SlideSpec> {
    intro_slides(doc)
        .into_iter()
        .chain(Builder {
            slides: Vec::new(),
            current: None,
            last_heading: None,
            changes: &HashMap::new(),
        }
        .run(doc))
        .collect()
}

/// Builds slide specs for diff output: slides come from `new`, tagged by
/// the worst change affecting their blocks, followed by a removed-content
/// slide and a revision summary.
pub fn slides_for_diff(new: &Document, plan: &DiffPlan, identity: (&str, &str)) -> Vec<SlideSpec> {
    let mut changes: HashMap<String, &'static str> = HashMap::new();
    let mut removed: Vec<String> = Vec::new();
    for step in &plan.steps {
        match step {
            Step::Add(b) => tag_block(&mut changes, &b.id, "new"),
            Step::Remove(b) => removed.push(describe(&b.kind)),
            Step::Modify { old, new, .. } => {
                // Slides are built from `new`, but content-hash ids differ
                // between revisions — tag both sides.
                tag_block(&mut changes, &old.id, "updated");
                tag_block(&mut changes, &new.id, "updated");
            }
            Step::Move { block, .. } => tag_block(&mut changes, &block.id, "moved"),
            Step::Keep(_) => {}
        }
    }

    let mut slides = Builder {
        slides: intro_slides(new),
        current: None,
        last_heading: None,
        changes: &changes,
    }
    .run(new);

    if !removed.is_empty() {
        let bullets = removed
            .into_iter()
            .map(|what| Bullet {
                level: 0,
                ordered: false,
                number: 0,
                runs: vec![Inline::Text(what)],
            })
            .collect();
        let mut slide = SlideSpec::content(Some("Removed in this revision".to_string()));
        slide.tag = Some("removed");
        slide.elems.push(Elem::Bullets(bullets));
        slides.push(slide);
    }

    let (added, changed, moved) = plan_counts(plan);
    let mut summary = SlideSpec::content(Some("Revision summary".to_string()));
    let mut bullets = vec![
        summary_line(added, "added"),
        summary_line(changed, "changed"),
        summary_line(moved, "moved"),
        summary_line(removed_count(plan), "removed"),
    ];
    bullets.push(Bullet {
        level: 0,
        ordered: false,
        number: 0,
        runs: vec![Inline::Text(format!("by {} on {}", identity.0, identity.1))],
    });
    summary.elems.push(Elem::Bullets(bullets));
    slides.push(summary);

    slides
}

fn removed_count(plan: &DiffPlan) -> usize {
    plan.steps
        .iter()
        .filter(|s| matches!(s, Step::Remove(_)))
        .count()
}

fn summary_line(n: usize, what: &str) -> Bullet {
    let text = if n == 1 {
        format!("{n} block {what}")
    } else {
        format!("{n} blocks {what}")
    };
    Bullet {
        level: 0,
        ordered: false,
        number: 0,
        runs: vec![Inline::Text(text)],
    }
}

fn plan_counts(plan: &DiffPlan) -> (usize, usize, usize) {
    let mut added = 0;
    let mut changed = 0;
    let mut moved = 0;
    for step in &plan.steps {
        match step {
            Step::Add(_) => added += 1,
            Step::Modify { .. } => changed += 1,
            Step::Move { .. } => moved += 1,
            _ => {}
        }
    }
    (added, changed, moved)
}

fn tag_block(changes: &mut HashMap<String, &'static str>, id: &str, tag: &'static str) {
    let entry = changes.entry(id.to_string()).or_insert(tag);
    if rank(*entry) < rank(tag) {
        *entry = tag;
    }
}

fn rank(tag: &str) -> u8 {
    match tag {
        "new" => 3,
        "updated" => 2,
        "moved" => 1,
        _ => 0,
    }
}

fn describe(kind: &BlockKind) -> String {
    let label = match kind {
        BlockKind::Heading { level, .. } => return format!("heading (h{level})"),
        BlockKind::Paragraph { .. } => "paragraph",
        BlockKind::CodeBlock { .. } => "code block",
        BlockKind::Diagram { format, .. } => return format!("{} diagram", format.as_str()),
        BlockKind::List { .. } => "list",
        BlockKind::Table { .. } => "table",
        BlockKind::BlockQuote { .. } => "quote",
        BlockKind::ThematicBreak => "thematic break",
    };
    let text = match kind {
        BlockKind::Paragraph { inline } => Inline::plain_text(inline).trim().to_string(),
        _ => String::new(),
    };
    let snippet = char_slice(&text, 60);
    if snippet.is_empty() {
        label.to_string()
    } else {
        format!("{label}: {snippet}")
    }
}

pub fn char_slice(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

struct Builder<'a> {
    slides: Vec<SlideSpec>,
    current: Option<SlideSpec>,
    last_heading: Option<String>,
    changes: &'a HashMap<String, &'static str>,
}

impl<'a> Builder<'a> {
    fn run(mut self, doc: &Document) -> Vec<SlideSpec> {
        for block in &doc.blocks {
            self.block(block);
        }
        self.flush();
        self.slides
    }

    fn flush(&mut self) {
        if let Some(s) = self.current.take() {
            // Keep heading-only slides as section dividers, but drop fully
            // empty untitled ones.
            if !s.elems.is_empty() || s.title.is_some() {
                self.slides.push(s);
            }
        }
    }

    /// Starts a fresh content slide titled `title`, after flushing.
    fn open(&mut self, title: Option<String>, tag: Option<&'static str>) {
        self.flush();
        let mut s = SlideSpec::content(title);
        s.tag = tag;
        self.current = Some(s);
    }

    /// Overflow-aware bullet push: spills onto a "(cont.)" slide.
    fn bullet(&mut self, bullet: Bullet) {
        if self
            .current
            .as_ref()
            .is_some_and(|s| s.line_count() >= MAX_LINES && !s.has_fullbleed())
        {
            let prev = self.current.take().unwrap();
            let cont_title = prev
                .title
                .clone()
                .map(|t| if t.ends_with("(cont.)") { t } else { format!("{t} (cont.)") })
                .or_else(|| self.last_heading.clone());
            let tag = prev.tag;
            self.slides.push(prev);
            let mut cont = SlideSpec::content(cont_title);
            cont.tag = tag;
            self.current = Some(cont);
        }
        let s = self
            .current
            .get_or_insert_with(|| SlideSpec::content(self.last_heading.clone()));
        match s.elems.last_mut() {
            Some(Elem::Bullets(b)) => b.push(bullet),
            _ => s.elems.push(Elem::Bullets(vec![bullet])),
        }
    }

    fn block(&mut self, block: &Block) {
        let tag = self.changes.get(&block.id).copied();
        match &block.kind {
            BlockKind::Heading { level, inline } => {
                if *level <= 2 {
                    self.last_heading = Some(Inline::plain_text(inline));
                    let title = self.last_heading.clone();
                    self.open(title, tag);
                } else {
                    self.bullet(Bullet {
                        level: 0,
                        ordered: false,
                        number: 0,
                        runs: vec![Inline::Strong(inline.clone())],
                    });
                }
            }
            BlockKind::Paragraph { inline } => {
                let is_lead = self
                    .current
                    .as_ref()
                    .is_some_and(|s| s.elems.is_empty());
                if is_lead {
                    let s = self.current.as_mut().unwrap();
                    s.tag = s.tag.or(tag);
                    s.elems.push(Elem::Lead(inline.clone()));
                } else {
                    self.bullet(Bullet {
                        level: 0,
                        ordered: false,
                        number: 0,
                        runs: inline.clone(),
                    });
                }
            }
            BlockKind::List {
                ordered,
                start,
                items,
            } => {
                let s = self.current.get_or_insert_with(|| {
                    SlideSpec::content(self.last_heading.clone())
                });
                s.tag = s.tag.or(tag);
                self.list(items, *ordered, *start, 0);
            }
            BlockKind::Table { rows } => {
                let needs_own = self.current.as_ref().is_some_and(|s| {
                    s.has_fullbleed() || s.line_count() + rows.len().max(2) > MAX_LINES
                });
                if needs_own {
                    self.flush();
                }
                let s = self.current.get_or_insert_with(|| {
                    SlideSpec::content(self.last_heading.clone())
                });
                s.tag = s.tag.or(tag);
                s.elems.push(Elem::Table(rows.clone()));
                self.flush();
            }
            BlockKind::Diagram { .. } => {
                let needs_own = self.current.as_ref().is_some_and(|s| s.has_fullbleed());
                if needs_own {
                    self.flush();
                }
                let s = self.current.get_or_insert_with(|| {
                    SlideSpec::content(self.last_heading.clone())
                });
                s.tag = s.tag.or(tag);
                s.elems.push(Elem::Picture {
                    block_id: block.id.clone(),
                    alt: diagram_alt(&block.kind),
                });
                self.flush();
            }
            BlockKind::CodeBlock { language, code } => {
                let needs_own = self.current.as_ref().is_some_and(|s| s.has_fullbleed());
                if needs_own {
                    self.flush();
                }
                let s = self.current.get_or_insert_with(|| {
                    SlideSpec::content(self.last_heading.clone())
                });
                s.tag = s.tag.or(tag);
                s.elems.push(Elem::Code {
                    language: language.clone(),
                    code: code.clone(),
                });
                self.flush();
            }
            BlockKind::BlockQuote { blocks } => {
                let inline: Vec<Inline> = blocks
                    .iter()
                    .filter_map(|b| match &b.kind {
                        BlockKind::Paragraph { inline } => Some(inline.clone()),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                let s = self.current.get_or_insert_with(|| {
                    SlideSpec::content(self.last_heading.clone())
                });
                s.tag = s.tag.or(tag);
                s.elems.push(Elem::Quote(inline));
            }
            BlockKind::ThematicBreak => {
                // A thematic break separates sections: whatever follows
                // must not inherit the previous heading as its title.
                self.last_heading = None;
                self.flush();
            }
        }
    }

    fn list(&mut self, items: &[ListItem], ordered: bool, start: u64, level: u8) {
        for (i, item) in items.iter().enumerate() {
            self.bullet(Bullet {
                level,
                ordered,
                number: start + i as u64,
                runs: item.content.clone(),
            });
            for child in &item.children {
                match &child.kind {
                    BlockKind::List {
                        ordered: co,
                        start: cs,
                        items: ci,
                    } => self.list(ci, *co, *cs, (level + 1).min(4)),
                    BlockKind::Paragraph { inline } => self.bullet(Bullet {
                        level: (level + 1).min(4),
                        ordered: false,
                        number: 0,
                        runs: inline.clone(),
                    }),
                    _ => {}
                }
            }
        }
    }
}

fn diagram_alt(kind: &BlockKind) -> String {
    match kind {
        BlockKind::Diagram { format, .. } => format!("{} diagram", format.as_str()),
        _ => "diagram".to_string(),
    }
}

/// The opening title slide, unless the frontmatter carries nothing worth
/// showing (no title, subtitle or byline).
fn intro_slides(doc: &Document) -> Vec<SlideSpec> {
    let s = title_slide(&doc.frontmatter);
    if s.title.is_none() && s.subtitle.is_none() && s.byline.is_none() {
        return Vec::new();
    }
    vec![s]
}

fn title_slide(fm: &Frontmatter) -> SlideSpec {
    let mut byline_parts = Vec::new();
    if !fm.authors.is_empty() {
        byline_parts.push(fm.authors.join(", "));
    }
    if let Some(v) = &fm.version {
        byline_parts.push(format!("v{v}"));
    }
    if let Some(d) = &fm.date {
        byline_parts.push(d.clone());
    }
    SlideSpec {
        kind: SlideKind::Title,
        title: fm.title.clone(),
        subtitle: fm.subtitle.clone(),
        byline: if byline_parts.is_empty() {
            None
        } else {
            Some(byline_parts.join("  ·  "))
        },
        elems: Vec::new(),
        tag: None,
    }
}
