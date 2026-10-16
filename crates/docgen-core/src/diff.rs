//! Id-anchored structural diff between two [`Document`]s.
//!
//! Block ids are the join key: a block that keeps its id and its content is
//! unchanged; same id but different content is a modify; same content but a
//! different position is a move; ids that vanish/appear are
//! removals/additions. Blocks whose ids were auto-derived from content
//! (hash) change id on every edit — for those, a similarity pass pairs a
//! removal with a nearby addition of the same family into a single
//! "changed" (fuzzy) instead of remove+add noise.
//!
//! The [`DiffPlan`] produced here is what output backends consume: docx
//! turns it into native tracked changes (`w:ins`/`w:del`) plus comments.

use std::collections::HashMap;
use std::hash::Hash;

use similar::{Algorithm, capture_diff_slices};

use crate::model::{Block, BlockKind, DiagramFormat, Document, Inline};

#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// Minimum text similarity (0.0–1.0) for pairing a removed and an added
    /// block of the same family into one [`BlockChange::Changed`] marked
    /// `fuzzy`. Explicitly-id'd blocks never need this.
    pub fuzzy_threshold: f64,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            fuzzy_threshold: 0.4,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentDiff {
    pub changes: Vec<BlockChange>,
}

impl DocumentDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut added = 0;
        let mut removed = 0;
        let mut changed = 0;
        let mut moved = 0;
        for c in &self.changes {
            match c {
                BlockChange::Added { .. } => added += 1,
                BlockChange::Removed { .. } => removed += 1,
                BlockChange::Moved { .. } => moved += 1,
                BlockChange::Changed { .. } => changed += 1,
            }
        }
        if self.changes.is_empty() {
            return "no changes".to_string();
        }
        let mut parts = Vec::new();
        if added > 0 {
            parts.push(format!("{added} added"));
        }
        if removed > 0 {
            parts.push(format!("{removed} removed"));
        }
        if changed > 0 {
            parts.push(format!("{changed} changed"));
        }
        if moved > 0 {
            parts.push(format!("{moved} moved"));
        }
        parts.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockChange {
    Added {
        block: Block,
        at: usize,
    },
    Removed {
        block: Block,
        at: usize,
    },
    Moved {
        id: String,
        from: usize,
        to: usize,
    },
    Changed {
        id: String,
        at: usize,
        old: Block,
        new: Block,
        content: ContentChange,
        fuzzy: bool,
    },
}

/// What changed inside a block whose id survived.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentChange {
    /// Word/run-level text changes (paragraphs, headings).
    Runs(Vec<RunOp>),
    /// Line-level changes (code blocks).
    Lines(Vec<LineOp>),
    /// Item-level changes in lists (item text flattened).
    Items(Vec<LineOp>),
    /// Row-level changes in tables (row text flattened).
    Rows(Vec<LineOp>),
    /// Diagram source replaced.
    Source {
        format: DiagramFormat,
        old: String,
        new: String,
    },
    /// Structural replacement (kind changed, block quote, thematic break).
    Replaced,
}

/// A styled run of text: the unit of word-level diffs. Formatting flags are
/// inherited through nesting, so `**bold word**` is one bold run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct Run {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub text: String,
    /// Link target when the run came from inside a link (D-006 internal
    /// links substrate); `None` for plain text. Stored as written —
    /// `#fragment` targets are internal anchors.
    #[doc(hidden)]
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOp {
    Equal(Run),
    Delete(Run),
    Insert(Run),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineOp {
    Equal(String),
    Delete(String),
    Insert(String),
}

/// Diffs two documents.
pub fn diff_documents(old: &Document, new: &Document) -> DocumentDiff {
    diff_documents_with(old, new, &DiffOptions::default())
}

pub fn diff_documents_with(old: &Document, new: &Document, opts: &DiffOptions) -> DocumentDiff {
    let old_pos: HashMap<&str, usize> = old
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.as_str(), i))
        .collect();
    let new_pos: HashMap<&str, usize> = new
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.as_str(), i))
        .collect();

    let mut changes = Vec::new();

    // Content changes for surviving ids; removals for vanished ids.
    let mut removed: Vec<(usize, &Block)> = Vec::new();
    for (i, ob) in old.blocks.iter().enumerate() {
        if let Some(&j) = new_pos.get(ob.id.as_str()) {
            let nb = &new.blocks[j];
            if !same_variant(&ob.kind, &nb.kind) {
                changes.push(BlockChange::Changed {
                    id: ob.id.clone(),
                    at: j,
                    old: ob.clone(),
                    new: nb.clone(),
                    content: ContentChange::Replaced,
                    fuzzy: false,
                });
            } else {
                let content = content_change(ob, nb);
                if !is_noop(&content) {
                    changes.push(BlockChange::Changed {
                        id: ob.id.clone(),
                        at: j,
                        old: ob.clone(),
                        new: nb.clone(),
                        content,
                        fuzzy: false,
                    });
                }
            }
        } else {
            removed.push((i, ob));
        }
    }

    // Additions; moved (same id + equal content, different position).
    let mut added: Vec<(usize, &Block)> = Vec::new();
    for (j, nb) in new.blocks.iter().enumerate() {
        if let Some(&i) = old_pos.get(nb.id.as_str()) {
            let ob = &old.blocks[i];
            if same_variant(&ob.kind, &nb.kind) && is_noop(&content_change(ob, nb)) && i != j {
                changes.push(BlockChange::Moved {
                    id: nb.id.clone(),
                    from: i,
                    to: j,
                });
            }
        } else {
            added.push((j, nb));
        }
    }

    // Fuzzy pairing of removals and additions of the same family.
    let mut used_removed = vec![false; removed.len()];
    let mut used_added = vec![false; added.len()];
    for (ri, (_old_at, ob)) in removed.iter().enumerate() {
        if used_removed[ri] {
            continue;
        }
        let mut best: Option<(f64, usize)> = None;
        for (ai, (_new_at, nb)) in added.iter().enumerate() {
            if used_added[ai] || family_of(&ob.kind) != family_of(&nb.kind) {
                continue;
            }
            let ratio = similarity(&plain_text(ob), &plain_text(nb));
            if ratio >= opts.fuzzy_threshold && best.map(|(b, _)| ratio > b).unwrap_or(true) {
                best = Some((ratio, ai));
            }
        }
        if let Some((_, ai)) = best {
            used_removed[ri] = true;
            used_added[ai] = true;
            let (_new_at, nb) = added[ai];
            changes.push(BlockChange::Changed {
                id: nb.id.clone(),
                at: _new_at,
                old: (*ob).clone(),
                new: nb.clone(),
                content: content_change(ob, nb),
                fuzzy: true,
            });
        }
    }
    for (ri, (at, ob)) in removed.iter().enumerate() {
        if !used_removed[ri] {
            changes.push(BlockChange::Removed {
                block: (*ob).clone(),
                at: *at,
            });
        }
    }
    for (ai, (at, nb)) in added.iter().enumerate() {
        if !used_added[ai] {
            changes.push(BlockChange::Added {
                block: (*nb).clone(),
                at: *at,
            });
        }
    }

    DocumentDiff { changes }
}

/// Ordered emission plan for output backends: walks the NEW document, with
/// removed old blocks anchored where they used to sit.
#[derive(Debug)]
pub struct DiffPlan {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone)]
pub enum Step {
    /// Emit as-is.
    Keep(Block),
    /// Emit fully marked as inserted.
    Add(Block),
    /// Emit old content as deleted, new content as inserted; equal parts
    /// flow through unmarked at word/line/item/row granularity.
    Modify {
        old: Block,
        new: Block,
        content: ContentChange,
        fuzzy: bool,
    },
    /// Emit as-is at the new position (moves get a comment, not marks).
    Move { block: Block, from: usize },
    /// Emit old content fully marked as deleted.
    Remove(Block),
}

/// Builds the ordered plan from a diff of `old` → `new`.
pub fn plan_diff(old: &Document, new: &Document) -> DiffPlan {
    let opts = DiffOptions::default();
    plan_diff_with(old, new, &opts)
}

pub fn plan_diff_with(old: &Document, new: &Document, opts: &DiffOptions) -> DiffPlan {
    let old_by_id: HashMap<&str, &Block> = old.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
    let new_by_id: HashMap<&str, &Block> = new.blocks.iter().map(|b| (b.id.as_str(), b)).collect();

    // Interleave removals correctly by LCS-ing the id sequences.
    let old_ids: Vec<&str> = old.blocks.iter().map(|b| b.id.as_str()).collect();
    let new_ids: Vec<&str> = new.blocks.iter().map(|b| b.id.as_str()).collect();
    let ops = capture_diff_slices(Algorithm::Patience, &old_ids, &new_ids);

    let mut steps: Vec<Step> = Vec::new();
    let mut handled: std::collections::HashSet<&str> = Default::default();

    for (tag, old_range, new_range) in ops.iter().map(|op| op.as_tag_tuple()) {
        match tag {
            similar::DiffTag::Equal => {
                for k in old_range.clone() {
                    let oid = old_ids[k];
                    let nid = new_ids[new_range.start + (k - old_range.start)];
                    let ob = old_by_id[oid];
                    let nb = new_by_id[nid];
                    if same_variant(&ob.kind, &nb.kind) {
                        let content = content_change(ob, nb);
                        if is_noop(&content) {
                            steps.push(Step::Keep(nb.clone()));
                        } else {
                            steps.push(Step::Modify {
                                old: ob.clone(),
                                new: nb.clone(),
                                content,
                                fuzzy: false,
                            });
                        }
                    } else {
                        steps.push(Step::Modify {
                            old: ob.clone(),
                            new: nb.clone(),
                            content: ContentChange::Replaced,
                            fuzzy: false,
                        });
                    }
                    handled.insert(nid);
                }
            }
            similar::DiffTag::Delete | similar::DiffTag::Replace => {
                for i in old_range.clone() {
                    // If the id still exists in the new document it sits
                    // outside the LCS there too and is handled by the
                    // Insert walk (move or move+modify). Otherwise it is a
                    // genuine removal, anchored here.
                    let oid = old_ids[i];
                    if !new_by_id.contains_key(oid) {
                        steps.push(Step::Remove(old_by_id[oid].clone()));
                        handled.insert(oid);
                    }
                }
                if tag == similar::DiffTag::Replace {
                    for j in new_range.clone() {
                        let nid = new_ids[j];
                        if handled.contains(nid) {
                            continue;
                        }
                        let nb = new_by_id[nid];
                        steps.push(emit_new_block(old, old_by_id.get(nid).copied(), nb));
                        handled.insert(nid);
                    }
                }
            }
            similar::DiffTag::Insert => {
                for j in new_range.clone() {
                    let nid = new_ids[j];
                    if handled.contains(nid) {
                        continue;
                    }
                    let nb = new_by_id[nid];
                    steps.push(emit_new_block(old, old_by_id.get(nid).copied(), nb));
                    handled.insert(nid);
                }
            }
        }
    }

    // Fuzzy pass: merge Remove+Add pairs of the same family into Modify.
    let threshold = opts.fuzzy_threshold;
    let mut merged: Vec<Step> = Vec::new();
    let mut adds: Vec<usize> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        if let Step::Add(_) = step {
            adds.push(i);
        }
    }
    let mut consumed: std::collections::HashSet<usize> = Default::default();
    for (i, step) in steps.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        if let Step::Remove(ob) = step {
            let mut best: Option<(f64, usize)> = None;
            for &ai in &adds {
                if consumed.contains(&ai) {
                    continue;
                }
                if let Step::Add(nb) = &steps[ai]
                    && family_of(&ob.kind) == family_of(&nb.kind)
                {
                    let ratio = similarity(&plain_text(ob), &plain_text(nb));
                    if ratio >= threshold && best.map(|(b, _)| ratio > b).unwrap_or(true) {
                        best = Some((ratio, ai));
                    }
                }
            }
            if let Some((_, ai)) = best {
                consumed.insert(ai);
                if let Step::Add(nb) = &steps[ai] {
                    merged.push(Step::Modify {
                        old: ob.clone(),
                        new: nb.clone(),
                        content: content_change(ob, nb),
                        fuzzy: true,
                    });
                }
                continue;
            }
        }
        merged.push(step.clone());
    }

    DiffPlan { steps: merged }
}

/// Classifies a block that appears at an unaligned (non-LCS) position in the
/// new document: addition if the id is new, move if content-equal, otherwise
/// a move with modifications.
fn emit_new_block(old: &Document, ob: Option<&Block>, nb: &Block) -> Step {
    match ob {
        Some(ob) if same_variant(&ob.kind, &nb.kind) => {
            let content = content_change(ob, nb);
            if is_noop(&content) {
                Step::Move {
                    block: nb.clone(),
                    from: old.blocks.iter().position(|b| b.id == nb.id).unwrap_or(0),
                }
            } else {
                Step::Modify {
                    old: ob.clone(),
                    new: nb.clone(),
                    content,
                    fuzzy: false,
                }
            }
        }
        Some(ob) => Step::Modify {
            old: ob.clone(),
            new: nb.clone(),
            content: ContentChange::Replaced,
            fuzzy: false,
        },
        None => Step::Add(nb.clone()),
    }
}

/// Same block *variant* (paragraph, heading, diagram, ...) regardless of
/// content — `BlockKind`'s derived `PartialEq` compares content too, which
/// is exactly what a "did the content change" check must not use.
pub fn same_variant(a: &BlockKind, b: &BlockKind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

fn is_noop(content: &ContentChange) -> bool {
    match content {
        ContentChange::Runs(ops) => ops.iter().all(|op| matches!(op, RunOp::Equal(_))),
        ContentChange::Lines(ops) | ContentChange::Items(ops) | ContentChange::Rows(ops) => {
            ops.iter().all(|op| matches!(op, LineOp::Equal(_)))
        }
        ContentChange::Source { old, new, .. } => old == new,
        ContentChange::Replaced => false,
    }
}

/// Computes the intra-block change for two blocks with the same kind.
pub fn content_change(old: &Block, new: &Block) -> ContentChange {
    match (&old.kind, &new.kind) {
        (
            BlockKind::Heading { inline: a, .. } | BlockKind::Paragraph { inline: a },
            BlockKind::Heading { inline: b, .. } | BlockKind::Paragraph { inline: b },
        ) => ContentChange::Runs(diff_runs(a, b)),
        (BlockKind::CodeBlock { code: a, .. }, BlockKind::CodeBlock { code: b, .. }) => {
            ContentChange::Lines(diff_lines(a, b))
        }
        (BlockKind::Diagram { format, source: a }, BlockKind::Diagram { source: b, .. }) => {
            ContentChange::Source {
                format: *format,
                old: a.clone(),
                new: b.clone(),
            }
        }
        (BlockKind::List { items: a, .. }, BlockKind::List { items: b, .. }) => {
            let ta = a.iter().map(item_text).collect::<Vec<_>>();
            let tb = b.iter().map(item_text).collect::<Vec<_>>();
            ContentChange::Items(diff_string_slices(&ta, &tb))
        }
        (BlockKind::Table { rows: a }, BlockKind::Table { rows: b }) => {
            let ta = a.iter().map(row_text).collect::<Vec<_>>();
            let tb = b.iter().map(row_text).collect::<Vec<_>>();
            ContentChange::Rows(diff_string_slices(&ta, &tb))
        }
        _ => ContentChange::Replaced,
    }
}

/// Word/run-level diff of inline content.
pub fn diff_runs(old: &[Inline], new: &[Inline]) -> Vec<RunOp> {
    let a = flatten_runs(old, RunFlags::default());
    let b = flatten_runs(new, RunFlags::default());
    run_ops(&ops_on(&a, &b), &a, &b)
}

fn ops_on<T: Eq + Hash>(
    a: &[T],
    b: &[T],
) -> Vec<(
    similar::DiffTag,
    std::ops::Range<usize>,
    std::ops::Range<usize>,
)> {
    capture_diff_slices(Algorithm::Patience, a, b)
        .iter()
        .map(|op| op.as_tag_tuple())
        .collect()
}

fn run_ops(
    ops: &[(
        similar::DiffTag,
        std::ops::Range<usize>,
        std::ops::Range<usize>,
    )],
    a: &[Run],
    b: &[Run],
) -> Vec<RunOp> {
    let mut out = Vec::new();
    for (tag, old_range, new_range) in ops {
        match tag {
            similar::DiffTag::Equal => {
                for r in old_range.clone() {
                    out.push(RunOp::Equal(a[r].clone()));
                }
            }
            similar::DiffTag::Delete => {
                for r in old_range.clone() {
                    out.push(RunOp::Delete(a[r].clone()));
                }
            }
            similar::DiffTag::Insert => {
                for r in new_range.clone() {
                    out.push(RunOp::Insert(b[r].clone()));
                }
            }
            similar::DiffTag::Replace => {
                for r in old_range.clone() {
                    out.push(RunOp::Delete(a[r].clone()));
                }
                for r in new_range.clone() {
                    out.push(RunOp::Insert(b[r].clone()));
                }
            }
        }
    }
    out
}

/// Line-level diff (code blocks, list items, table rows).
pub fn diff_lines(old: &str, new: &str) -> Vec<LineOp> {
    let a: Vec<String> = old.lines().map(String::from).collect();
    let b: Vec<String> = new.lines().map(String::from).collect();
    diff_string_slices(&a, &b)
}

fn diff_string_slices(a: &[String], b: &[String]) -> Vec<LineOp> {
    let ops = ops_on(a, b);
    let mut out = Vec::new();
    for (tag, old_range, new_range) in &ops {
        match tag {
            similar::DiffTag::Equal => {
                for r in old_range.clone() {
                    out.push(LineOp::Equal(a[r].clone()));
                }
            }
            similar::DiffTag::Delete => {
                for r in old_range.clone() {
                    out.push(LineOp::Delete(a[r].clone()));
                }
            }
            similar::DiffTag::Insert => {
                for r in new_range.clone() {
                    out.push(LineOp::Insert(b[r].clone()));
                }
            }
            similar::DiffTag::Replace => {
                for r in old_range.clone() {
                    out.push(LineOp::Delete(a[r].clone()));
                }
                for r in new_range.clone() {
                    out.push(LineOp::Insert(b[r].clone()));
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RunFlags {
    bold: bool,
    italic: bool,
    strike: bool,
}

/// Flattens inline content into styled runs — the same flattening the diff
/// uses, so fresh and diffed documents produce identical runs.
pub fn flatten_runs_pub(inlines: &[Inline]) -> Vec<Run> {
    flatten_runs(inlines, RunFlags::default())
}

fn flatten_runs(inlines: &[Inline], flags: RunFlags) -> Vec<Run> {
    let mut out = Vec::new();
    let push = |run: Run, out: &mut Vec<Run>| {
        if run.text.is_empty() {
            return;
        }
        // Word granularity: split plain text on whitespace, keeping the
        // trailing space attached, so diffs read like word diffs.
        if run.code || !run.text.contains(char::is_whitespace) {
            out.push(run);
            return;
        }
        for chunk in run.text.split_inclusive(char::is_whitespace) {
            if !chunk.is_empty() {
                out.push(Run {
                    bold: run.bold,
                    italic: run.italic,
                    strike: run.strike,
                    code: run.code,
                    text: chunk.to_string(),
                    link: run.link.clone(),
                });
            }
        }
    };
    for inline in inlines {
        match inline {
            Inline::Text(t) => push(
                Run {
                    bold: flags.bold,
                    italic: flags.italic,
                    strike: flags.strike,
                    code: false,
                    text: t.clone(),
                    link: None,
                },
                &mut out,
            ),
            Inline::Code(t) => push(
                Run {
                    code: true,
                    text: t.clone(),
                    ..Default::default()
                },
                &mut out,
            ),
            Inline::Strong(inner) => flatten_into(
                inner,
                RunFlags {
                    bold: true,
                    ..flags
                },
                &mut out,
            ),
            Inline::Emphasis(inner) => flatten_into(
                inner,
                RunFlags {
                    italic: true,
                    ..flags
                },
                &mut out,
            ),
            Inline::Strikethrough(inner) => flatten_into(
                inner,
                RunFlags {
                    strike: true,
                    ..flags
                },
                &mut out,
            ),
            Inline::Link { url, text } => {
                // The runs keep their styling and additionally carry the
                // link target (D-006).
                let start = out.len();
                flatten_into(text, flags, &mut out);
                for run in &mut out[start..] {
                    run.link = Some(url.clone());
                }
            }
            Inline::Image { alt, .. } => flatten_into(alt, flags, &mut out),
            Inline::SoftBreak => push(
                Run {
                    text: " ".into(),
                    ..Default::default()
                },
                &mut out,
            ),
            Inline::HardBreak => push(
                Run {
                    text: "\n".into(),
                    ..Default::default()
                },
                &mut out,
            ),
        }
    }
    out
}

fn flatten_into(inlines: &[Inline], flags: RunFlags, out: &mut Vec<Run>) {
    out.extend(flatten_runs(inlines, flags));
}

fn item_text(item: &crate::model::ListItem) -> String {
    let mut s = Inline::plain_text(&item.content);
    for child in &item.children {
        s.push(' ');
        s.push_str(&plain_text(child));
    }
    s
}

fn row_text(row: &crate::model::TableRow) -> String {
    row.cells
        .iter()
        .map(|c| Inline::plain_text(c))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn plain_text(block: &Block) -> String {
    match &block.kind {
        BlockKind::Heading { inline, .. } | BlockKind::Paragraph { inline } => {
            Inline::plain_text(inline)
        }
        BlockKind::CodeBlock { code, .. } => code.clone(),
        BlockKind::Diagram { source, .. } => source.clone(),
        BlockKind::List { items, .. } => items.iter().map(item_text).collect::<Vec<_>>().join(" "),
        BlockKind::Table { rows } => rows.iter().map(row_text).collect::<Vec<_>>().join(" "),
        BlockKind::BlockQuote { blocks } => {
            blocks.iter().map(plain_text).collect::<Vec<_>>().join(" ")
        }
        BlockKind::ThematicBreak => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Text,
    Code,
    Diagram,
    List,
    Table,
    Quote,
    Break,
}

fn family_of(kind: &BlockKind) -> Family {
    match kind {
        BlockKind::Heading { .. } | BlockKind::Paragraph { .. } => Family::Text,
        BlockKind::CodeBlock { .. } => Family::Code,
        BlockKind::Diagram { .. } => Family::Diagram,
        BlockKind::List { .. } => Family::List,
        BlockKind::Table { .. } => Family::Table,
        BlockKind::BlockQuote { .. } => Family::Quote,
        BlockKind::ThematicBreak => Family::Break,
    }
}

/// Word-level similarity ratio in 0.0–1.0 (`2·matched / (len_a + len_b)`),
/// computed on whitespace-free word sequences so spacing can't inflate it.
fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.trim().is_empty() || b.trim().is_empty() {
        return 0.0;
    }
    let aw: Vec<&str> = a.split_whitespace().collect();
    let bw: Vec<&str> = b.split_whitespace().collect();
    let matched: usize = ops_on(&aw, &bw)
        .iter()
        .filter(|(t, _, _)| matches!(t, similar::DiffTag::Equal))
        .map(|(_, old, _)| old.len())
        .sum();
    2.0 * matched as f64 / (aw.len() + bw.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_document;

    #[test]
    fn identical_documents_have_no_changes() {
        let d = parse_document("# A\n\nHello.\n").unwrap();
        let diff = diff_documents(&d, &d);
        assert!(diff.is_empty(), "{:?}", diff.changes);
    }

    #[test]
    fn changed_paragraph_same_id() {
        let old = parse_document("# T {#t}\n\nHello world.\n").unwrap();
        let new = parse_document("# T {#t}\n\nHello brave world.\n").unwrap();
        let diff = diff_documents(&old, &new);
        assert_eq!(diff.changes.len(), 1, "{:?}", diff.changes);
        match &diff.changes[0] {
            // Auto-id'd paragraph: shows up as a fuzzy-paired change carrying
            // the NEW block's id.
            BlockChange::Changed {
                id, content, fuzzy, ..
            } => {
                assert!(id.starts_with("p-"), "{id}");
                assert!(fuzzy, "expected fuzzy pairing");
                match content {
                    ContentChange::Runs(ops) => {
                        let deleted: String = ops
                            .iter()
                            .filter_map(|o| match o {
                                RunOp::Delete(r) => Some(r.text.clone()),
                                _ => None,
                            })
                            .collect();
                        let inserted: String = ops
                            .iter()
                            .filter_map(|o| match o {
                                RunOp::Insert(r) => Some(r.text.clone()),
                                _ => None,
                            })
                            .collect();
                        assert!(deleted.is_empty() || deleted.contains("world"), "{deleted}");
                        assert!(inserted.contains("brave"), "{inserted}");
                        assert!(ops.iter().any(|o| matches!(o, RunOp::Equal(_))));
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn added_and_removed() {
        let old = parse_document("# Keep {#k}\n\nHello world again.\n").unwrap();
        let new = parse_document("# Keep {#k}\n\nHello brave world again.\n").unwrap();
        let diff = diff_documents(&old, &new);
        // The paragraphs share enough words to be fuzzy-paired into one
        // change instead of remove+add noise.
        assert_eq!(diff.changes.len(), 1, "{:?}", diff.changes);
        assert!(matches!(
            &diff.changes[0],
            BlockChange::Changed { fuzzy: true, .. }
        ));
    }

    #[test]
    fn dissimilar_blocks_are_not_fuzzy_paired() {
        let old = parse_document("The quick brown fox jumps over the lazy dog.\n").unwrap();
        let new =
            parse_document("Integration tests validate module contracts end to end.\n").unwrap();
        let diff = diff_documents(&old, &new);
        assert_eq!(diff.changes.len(), 2, "{:?}", diff.changes);
        assert!(
            diff.changes
                .iter()
                .any(|c| matches!(c, BlockChange::Removed { .. }))
        );
        assert!(
            diff.changes
                .iter()
                .any(|c| matches!(c, BlockChange::Added { .. }))
        );
    }

    #[test]
    fn moved_block() {
        let old = parse_document("# A {#a}\n\n# B {#b}\n").unwrap();
        let new = parse_document("# B {#b}\n\n# A {#a}\n").unwrap();
        let diff = diff_documents(&old, &new);
        assert_eq!(diff.changes.len(), 2, "{:?}", diff.changes);
        assert!(
            diff.changes
                .iter()
                .all(|c| matches!(c, BlockChange::Moved { .. }))
        );
    }

    #[test]
    fn code_lines_diff() {
        let old = parse_document("```rust\nfn a() {}\nfn b() {}\n```\n").unwrap();
        let new = parse_document("```rust\nfn a() {}\nfn c() {}\n```\n").unwrap();
        let diff = diff_documents(&old, &new);
        match &diff.changes[0] {
            BlockChange::Changed { content, .. } => match content {
                ContentChange::Lines(ops) => {
                    assert!(ops.iter().any(|o| matches!(o, LineOp::Equal(_))));
                    assert!(
                        ops.iter()
                            .any(|o| matches!(o, LineOp::Delete(d) if d.contains("fn b")))
                    );
                    assert!(
                        ops.iter()
                            .any(|o| matches!(o, LineOp::Insert(i) if i.contains("fn c")))
                    );
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn plan_orders_steps_by_new_document() {
        let old = parse_document("# Keep {#k}\n\nGone away now.\n\n# Tail {#t}\n").unwrap();
        let new = parse_document("# Keep {#k}\n\nGone away today.\n\n# Tail {#t}\n").unwrap();
        let plan = plan_diff(&old, &new);
        let kinds: Vec<&str> = plan
            .steps
            .iter()
            .map(|s| match s {
                Step::Keep(_) => "keep",
                Step::Add(_) => "add",
                Step::Modify { .. } => "modify",
                Step::Move { .. } => "move",
                Step::Remove(_) => "remove",
            })
            .collect();
        // "Gone away now." vs "Gone away today." fuzzy-pair into one modify.
        assert_eq!(kinds, vec!["keep", "modify", "keep"], "{kinds:?}");
    }

    #[test]
    fn table_rows_diff() {
        let old = parse_document("| a | b |\n|---|---|\n| 1 | 2 |\n").unwrap();
        let new = parse_document("| a | b |\n|---|---|\n| 9 | 2 |\n").unwrap();
        let diff = diff_documents(&old, &new);
        match &diff.changes[0] {
            BlockChange::Changed { content, .. } => match content {
                ContentChange::Rows(ops) => {
                    assert!(
                        ops.iter()
                            .any(|o| matches!(o, LineOp::Delete(d) if d.contains("1 | 2")))
                    );
                    assert!(
                        ops.iter()
                            .any(|o| matches!(o, LineOp::Insert(i) if i.contains("9 | 2")))
                    );
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    // -- annotation neutrality (D-008) -----------------------------------

    #[test]
    fn annotations_alone_are_not_a_change() {
        // Same visible content and same ids (explicit, so the join key is
        // constant); only attribute annotations differ.
        let plain = parse_document(
            "# Results {#r}\n\nIntro text.\n\n```mermaid {#d}\ngraph TD\nA-->B\n```\n\n![Shot](img/x.png)\n",
        )
        .unwrap();
        let annotated = parse_document(
            "# Results {#r page=landscape new-page}\n\nIntro text.\n\n```mermaid {#d} place=appendix caption=\"Diagram\" keep=false\ngraph TD\nA-->B\n```\n\n![Shot](img/x.png){place=appendix}\n",
        )
        .unwrap();
        let diff = diff_documents(&plain, &annotated);
        assert!(
            diff.is_empty(),
            "annotations must be diff-invisible: {diff:?}"
        );
        let diff = diff_documents(&annotated, &plain);
        assert!(diff.is_empty(), "symmetric: {diff:?}");
    }

    #[test]
    fn annotations_do_not_disturb_content_diffs() {
        let old =
            parse_document("# R {#r}\n\nHello world.\n\n```mermaid {#d}\nA-->B\n```\n").unwrap();
        let new_plain =
            parse_document("# R {#r}\n\nHello brave world.\n\n```mermaid {#d}\nA-->C\n```\n")
                .unwrap();
        let new_ann = parse_document(
            "# R {#r page=landscape}\n\nHello brave world.\n\n```mermaid {#d} place=appendix\nA-->C\n```\n",
        )
        .unwrap();
        let a = diff_documents(&old, &new_plain);
        let b = diff_documents(&old, &new_ann);
        // Same change set (ids, positions, kinds) — only the embedded block
        // payloads may carry annotations.
        let shape = |d: &DocumentDiff| {
            d.changes
                .iter()
                .map(|c| match c {
                    BlockChange::Added { at, .. } => format!("add@{at}"),
                    BlockChange::Removed { at, .. } => format!("rem@{at}"),
                    BlockChange::Moved { id, from, to } => format!("move {id} {from}->{to}"),
                    BlockChange::Changed {
                        id, at, content, ..
                    } => {
                        format!("chg {id}@{at} {content:?}")
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(shape(&a), shape(&b));
        assert_eq!(a.summary(), b.summary());
    }

    #[test]
    fn flatten_runs_carries_link_targets() {
        let runs = flatten_runs_pub(&[Inline::Link {
            url: "#x".into(),
            text: vec![Inline::Text("a b".into())],
        }]);
        assert_eq!(runs.len(), 2);
        assert!(
            runs.iter().all(|r| r.link.as_deref() == Some("#x")),
            "{runs:?}"
        );
        let plain = flatten_runs_pub(&[Inline::Text("plain".into())]);
        assert!(plain.iter().all(|r| r.link.is_none()));
    }
}
