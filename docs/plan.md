# Plan: page control, media appendix, orientation, figures

Decisions governing this plan: `docs/decisions.md` D-001 – D-012.
Repo operating rules (build timing, golden tests, verification gates):
`AGENTS.md` at the repo root.

## Goal

Markdown-authored documents gain page/keep controls, per-page and
per-section orientation, and a media-placement system — inline by
default, optional memoir-style appendix plus figures index — built on
captioned, numbered figures. First-class in PDF (typst) and DOCX;
pptx is frozen at current behavior and warns on directives it cannot
honor. Citations are reserved (namespace + registry) but not
implemented. Done = all tasks below verified, `cargo fmt`/`clippy -D
warnings`/`cargo test --workspace` green, golden hashes updated only
intentionally (each output change stated), and the user has manually
verified DOCX output in real Word and LibreOffice.

## Non-goals

- Citation implementation — `[@key]` namespace reserved only (D-011).
- Book-style odd/even page rules; standalone break-between-paragraphs
  directive (D-002 deferrals).
- Any new pptx capability: no slide-jump links, backup-slide appendix,
  agenda index, or per-slide orientation (D-005, D-012).
- Word TOC/figure field codes — the DOCX figures index is links-only
  (D-006).
- Cross-format identical auto-shrink; any DOCX shrink change (D-007).
- Diff integration for annotations or relocated media (D-008).
- New WIT/athena exports (D-006).
- Page-control support in custom typst templates — default template
  only (D-006).
- Auto-orientation from diagram aspect ratio (D-005).

## Tasks

- [x] T1: docgen-core — annotations model
  Add a per-block annotations payload (placement, caption, page
  orientation, new-page, keep) to the block model, separate from
  `Block.id` — id keeps its meaning as diff join key + diagram asset
  key, unchanged.
  - done when: model + accessors exist; all existing docgen-core tests
    pass unchanged; docgen-core still compiles for `wasm32-wasip1` and
    `wasm32-unknown-unknown`.
- [ ] T2: docgen-core — attribute parsing
  Consume pulldown heading attributes beyond id (classes/attrs ignored
  today, `parse.rs:86-100`); extend `parse_info` (`parse.rs:383-399`)
  with key=value + bare tokens; parse `{…}` attribute blocks after
  inline images (title/id discarded today, `parse.rs:325-336`);
  recognize lone-image paragraphs as figures (alt text = caption).
  - done when: unit tests cover every token form (`{#fig:x}`, `place=`,
    `caption=`, `page=`, `new-page`, `keep=`), image attributes, and
    figure detection; unknown tokens are ignored without error
    (forward-compatible); prose containing literal braces is not
    swallowed.
- [ ] T3: docgen-core — frontmatter `media` namespace
  Parse `media: { placement, index }` (typed; defaults inline/false),
  flowing through the existing `extra` → renderer path.
  - done when: unit tests cover defaults, explicit values, and absent
    map; the typst `meta` export still carries the values.
- [ ] T4: docgen-core — figure numbering + placement resolution
  Pure pass over `Document`: assign Figure N in document order, resolve
  effective placement (frontmatter default + per-item override
  per D-003), and produce a back-matter plan (figures-index entries,
  appendix figure order per D-010).
  - done when: unit tests cover mixed inline/appendix, per-item
    overrides, numbering, and empty cases.
- [ ] T5: docgen-core — diff neutrality
  Annotations and figure metadata are invisible to the markdown
  structural diff.
  - done when: diff output is byte-identical with and without
    annotations on the same content (D-008).
- [x] T6: typst — figures + internal links
  Emit figure captions/numbers, labels on figures, and fragment links
  for appendix-mode inline references and the figures index (body
  internal links are new — typst emits no labels today).
  - done when: a smoke PDF (via `docgen-cli`) shows working internal
    links and captioned numbered figures; golden hashes updated
    intentionally (`UPDATE_GOLDEN=1`) with the output change stated.
- [x] T7: typst — page controls + orientation
  `new-page` → `#pagebreak()`; keeps → non-breakable blocks; `page=` →
  scoped `#set page` geometry change, section spans per D-010 (default
  template only per D-006).
  - done when: smoke PDF contains genuinely landscape pages with
    upright text (never rotated content); goldens updated
    intentionally.
- [x] T8: typst — fit-to-page + shrink warning
  Scale figures/diagrams to the page content box (width and height);
  deterministic warning when scale < 0.5 (D-007, D-010).
  - done when: an oversized tall diagram renders fully inside the
    page; a deliberately huge one triggers the warning; goldens
    updated intentionally.
- [ ] T9: typst — back matter
  Media appendix (captioned figures, orientation inherited from the
  document per D-010) and a figures index with page numbers when
  `media.index: true`.
  - done when: appendix-mode smoke PDF has appendix figures with
    working links from the inline positions; the index lists captions
    with correct page numbers; goldens updated intentionally.
- [ ] T10: docx — internal links + bookmarks
  Add link support to the flattened `Run` (`docgen-core/src/diff.rs`
  Run + docx emission; body links render as plain text today) and
  `w:hyperlink w:anchor` + `w:bookmarkStart/End` on figures (the
  pattern already exists in the TOC path, `document.rs:547-591`).
  - done when: an appendix-mode DOCX has clickable inline references
    to figure bookmarks; opens correctly in Word and LibreOffice (user
    verifies); goldens updated intentionally.
- [ ] T11: docx — captions, keeps, page breaks
  Figure captions + numbering; keep-together paragraph properties;
  `new-page` → explicit page break.
  - done when: smoke DOCX shows captioned figures and clean breaks
    (user verifies in Word/LibreOffice); goldens updated
    intentionally.
- [ ] T12: docx — multi-section orientation
  Mid-body `w:sectPr` with `w:orient="landscape"` and matching margins
  for `page=` sections/figures (single trailing sectPr today,
  `document.rs:1474-1476`); existing per-`w:tr` cantSplit rules
  untouched.
  - done when: a mixed-orientation DOCX opens without repair prompts
    in Word and LibreOffice (user verifies); landscape pages are real
    geometry; goldens updated intentionally.
- [ ] T13: docx — back matter
  Media appendix + links-only figures index (no page numbers per
  D-006).
  - done when: smoke DOCX has the appendix with working anchor links
    (user verifies rendering); goldens updated intentionally.
- [ ] T14: pptx — warn on unsupported directives
  Detect placement/orientation/index usage and emit a warning; zero
  output change.
  - done when: the warning appears on a CLI run exercising the
    directives; pptx golden hashes are byte-identical (no update).
- [ ] T15: workspace verification + samples
  `cargo fmt --all`; `cargo clippy --workspace --all-targets --
  -D warnings`; `cargo test --workspace`; generate sample PDF + DOCX
  outputs into `target/` exercising every feature; list what needs
  manual Office verification.
  - done when: all three gates green in one run; samples exist; the
    summary states every intentional golden change.

## Risks & unknowns

- **OOXML multi-section correctness** (T12) — highest-risk item; Word
  repair dialogs have bitten this repo before (graphicFrame/xfrm
  mixup). Early check: emit a minimal mixed-orientation DOCX before
  building further DOCX work on top, and have the user open it in Word
  + LibreOffice immediately.
- **Image `{…}` parsing ambiguity** (T2) — `![a](u "title"){#id}` vs
  trailing prose braces. Early check: unit tests with braces in
  surrounding prose.
- **Typst label/link semantics** (T6) — fragment links to labels need
  exact emission discipline. Early check: two-figure smoke doc before
  the back-matter tasks.
- **Golden churn volume** — many renderers change at once. Mitigation:
  update goldens per task (`UPDATE_GOLDEN=1`) and state each output
  change with that task, never one big re-baseline at the end.
- **pptx deprecation landing mid-plan** (D-012) — would drop T14 only;
  it is deliberately last and independent.
- **`Block.id` overload creep** (T1) — never smuggle annotations into
  id strings; that would corrupt the diff join key.

## Gaps found during execution

(none yet — appended by `/execute` when the plan proves insufficient)
