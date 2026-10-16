# Brainstorm notes — page control, media appendix, orientation, citations

- **Topic:** large diagrams & page control
- **Phase:** brainstorming (CONVERGED 2026-10-16 — P1–P6 and D7–D10 all
  decided; ready for `/distill`)
- **Inputs:** user brief (2026-10-16) + codebase grounding pass (Explore agent).

## The brief (as given)

1. **Page control management** — page/section jumps signalled in markdown,
   propagated into rendered documents.
2. **Flexible media references** — diagrams/images placed inline (default,
   unchanged) or in a **media appendix** (memoir style) with inline links
   out; optionally an **index postfix section** linking back to inline
   media. Selected by a frontmatter property tied to document type.
3. **Citations** — *planned, not implemented* (no use case yet); the
   linking/back-matter machinery must reserve room for them.
4. **Page orientation** — portrait/landscape for pages (or sections) so
   large/complex diagrams get canvas — **only** where the renderer can do
   it properly: real page-geometry change, upright text, never a page the
   reader must turn their head/screen to read.

Markdown/frontmatter extensions are in-bounds (we own the parser) if they
feel markdowny.

## Hard constraints (repo rules + discovered)

- Byte-deterministic outputs; golden tests pin renderer bytes → intended
  output changes must be stated in commits.
- No I/O in library crates (matters for a future bibliography data source —
  citation data must enter the way image bytes do, via the caller).
- Library crates build for both wasm targets; the WIT export surface is
  deliberate — new features decide explicitly whether they cross it.
- Office output is human-verified in real Word/PowerPoint/LibreOffice.
- `Block.id` is **overloaded today**: diff join key + diagram asset key +
  anchor source. New per-block properties must NOT ride `Block.id`.
- Typst custom templates (`--template`) fully own page setup — any
  orientation/page-control contract must define what custom templates are
  expected to honor (or features are default-template-only, stated).

## Codebase grounding (facts, from exploration)

- **Frontmatter**: `Frontmatter` (`docgen-core/src/model.rs:79-93`) —
  typed title/subtitle/authors/date/version/doc_id + `#[serde(flatten)]
  extra` (unknown keys preserved). `toc: true` (read from `extra`) is the
  only option that drives renderers today (docx `document.rs:1174-1179`;
  typst default template `emit.rs:56`). Full `extra` is exported to typst
  as the `meta` dict (`emit.rs:118-148`) — proven path for new options.
- **AST**: `BlockKind` = Heading/Paragraph/CodeBlock/Diagram/List/Table/
  BlockQuote/ThematicBreak (`model.rs:104-135`). Images are inline-only
  (`Inline::Image`, `model.rs:183`); image/link `title` and `id` are
  discarded at parse (`parse.rs:325-336`). No figure/caption node exists.
- **Annotation channels today**: heading attributes via pulldown-cmark
  `ENABLE_HEADING_ATTRIBUTES` — only `id` consumed (`parse.rs:86-100`);
  info strings via `parse_info` (`parse.rs:383-399`) — language + `{#name}`
  id token only. Both collapse into `Block.id`.
- **Id derivation**: explicit > slug (headings) > `<kind>-<fnv1a64>`;
  collisions de-duped (`ids.rs`).
- **Internal links**: typst — none (TOC is typst's `#outline`, headings
  carry no labels, `#link` has no fragment→label resolution). DOCX — TOC
  only: static `w:hyperlink w:anchor` + `w:bookmarkStart/End`
  (`document.rs:547-591`); **body inline links are plain text** (flattening
  `Run` has no link field, `diff.rs:129-135`). PPTX — external links only
  (`slide.rs:407-416`), no `ppaction://hlinksldjump` anywhere.
- **Page/break handling**: none in any emitter (no `#pagebreak`, no
  keepNext beyond static heading styles `parts.rs:54-94`).
- **Image sizing**: typst — "paragraph that is exactly one image" and
  diagrams → centered `#image(width: 92%)` (`emit.rs:213-248`) — a
  proto-figure heuristic already exists; other inline images flow at
  natural size. DOCX — clamp to content width EMU, no height clamp
  (`document.rs:64-81`). PPTX — aspect fit into 80% placeholder width
  (`slide.rs:913-951`); unrenderable diagrams → red error textbox.
- **Page geometry**: DOCX — one hardcoded trailing `w:sectPr` (A4
  portrait, 2cm margins) `document.rs:1474-1476`; no mid-body sectPr, no
  `w:orient`. PPTX — fixed 16:9 `sldSz` (`parts.rs:11-12`); template's
  sldSz honored with proportional scaling. Typst — default template
  `#set page(paper: "a4")` (`emit.rs:31`).
- **Parsing pipeline**: `parse_document` = frontmatter split → pulldown
  0.13 (tables, strikethrough, heading attributes, tasklists) → single
  tree-walk (`parse.rs:18-41`). **No pre/post passes.** Raw HTML blocks
  and inline HTML are *discarded* (`parse.rs:203-211`, `:337`) — so
  HTML-comment syntax would be dropped today. Footnotes skipped.
- **README roadmap**: nothing on page control, appendix, figures,
  captions, citations, orientation, or document kinds — greenfield.

## Design space

### A. Markdown syntax family (for page controls + per-item options)

- **A1 — attributes on existing blocks (heading `{...}` + fence
  info-strings).** e.g. `## Results {page=landscape}`, ` ```mermaid
  id=arch place=appendix ` . Pulldown already parses heading attrs (id
  consumed, classes/attrs ignored — consume them); `parse_info` extends to
  key=value tokens. No new block kinds; controls attach to the exact
  objects they modify. Needs a new per-block annotations field (not
  `Block.id`).
- **A2 — Pandoc-style fenced divs** (`::: {page=landscape}` … `:::`):
  general block scoping; requires a brand-new pre-pass (none exist);
  extra collision risk with diagram fences.
- **A3 — HTML comments** (`<!-- docgen: pagebreak -->`): zero visual
  footprint anywhere, but invisible magic, no scoping, and raw HTML is
  discarded today (new plumbing to *not* drop it). Feels like hiding
  semantics.
- **A4 — dedicated fence** (` ```page landscape ` `): explicit but a fake
  code block; competes with diagram fences for the "what's in a fence"
  mental model.

### B. Page-control vocabulary

- plain page break (before/after a block);
- keep-together / keep-with-next (matters for large diagrams near page
  ends; DOCX heading styles already keepNext);
- book semantics: chapter starts new page / odd (recto) page — typst has
  `pagebreak(to: "even"/"odd")` natively;
- scoped page properties (orientation, maybe margins/paper) — see C.

### C. Orientation per format (capability table — the "only if proper" gate)

| Format | Per-page orientation | Notes |
| --- | --- | --- |
| PDF (typst) | **Yes, native** | `#set page(...)` mid-flow auto-breaks; text upright; real landscape pages. Default template only unless custom-template contract defined. |
| DOCX | **Yes, via sectPr surgery** | Needs mid-body `w:sectPr` + `w:orient="landscape"` + per-section margin/header geometry; verify Word + LibreOffice rendering by hand. Multi-section is new plumbing (single trailing sectPr today). |
| PPTX | **No** | Deck-wide `sldSz` only; per-slide orientation is impossible in OOXML. Brief's own rule ⇒ don't rotate content, don't fake it: ignore directive + warn. |

Rule to write down: **orientation = page geometry change only; never
rotated content.** Text and captions stay upright everywhere.

### D. Media placement: modes, identity, captions, index

- Modes: inline (default, unchanged) · inline + index postfix (list of
  figures linking back) · appendix (memoir: media in back-matter section,
  inline anchor links out).
- Identity: figures need ids/numbers/captions for links + index to be
  meaningful. Pandoc idiom: `![Caption text](img.png){#fig:arch}` alone in
  a paragraph = figure. Diagrams: caption via fence attr.
- Per-format cliffs: page numbers in an index are computable only in
  typst (layout is ours); DOCX index would be links-only (no static page
  numbers without Word field codes); PPTX "appendix" ≈ backup slides +
  slide-jump links (both new plumbing: no internal links exist in any
  renderer's body text today).
- Frontmatter shape: raw knobs vs document-kind presets (`kind: memoir`
  bundling placement+index+citation slots).

### E. Citations (plan-only, reserve don't build)

- Reserve inline syntax namespace `[@key]` (Pandoc) — doesn't collide with
  anything docgen parses today.
- Reserve back-matter slots: the appendix/index machinery should be a
  general ordered back-matter registry (list of figures, list of
  diagrams, bibliography later) fed by the same anchor/link plumbing.
- No-IO rule: bibliography *data* must flow in via the caller boundary
  (like image bytes), never read by a library crate.

## Decisions

All six proposals (P1–P6) were **confirmed by the user in chat,
2026-10-16**, with amendments under P5 and follow-up decisions D7–D10
below.

### P1 — Syntax: attributes on existing blocks (design space A1)

- **Proposal:** key=value tokens in heading `{...}` attribute blocks and
  fence info-strings; consumed into a new per-block annotations field
  (headings/fences/figures), never `Block.id`.
- **Rationale:** pulldown already parses heading attributes (only id is
  consumed today — `parse.rs:86-100`); `parse_info` already tokenizes
  info-strings (`parse.rs:383-399`); controls attach to the exact object
  they modify; smallest parser diff; survives stock-markdown eyes.
- **Rejected:** fenced divs (new pre-pass, none exist; second way to say
  what headings already say); HTML comments (raw HTML discarded today,
  invisible magic, unscoped); dedicated page fence (fake code block,
  competes with diagram fences).
- **Open:** standalone "break here between two paragraphs" — v1 covers
  `pagebreak-before` on headings/figures/fences (breaks are ~always
  wanted before a block anyway); a standalone directive deferred until a
  real need appears.

### P2 — Page vocabulary: breaks + keep controls (B)

- **Proposal:** `pagebreak-before` (+ `-after` if cheap) and
  keep-together / keep-with-next on figure/diagram blocks.
- **Rationale:** directly serves large diagrams near page ends; DOCX
  heading styles already keepNext (pattern exists); typst has
  `block(breakable: false)` + `#pagebreak()`.
- **Rejected:** breaks only (defers the keep work the brief implies);
  book rules (odd/even chapter starts) — deferred, no stated use case.

### P3 — Placement: frontmatter knobs + per-item override (D)

- **Proposal:** document-level frontmatter keys (media placement default,
  figure-index toggle) + per-diagram `place=inline|appendix` fence attr.
- **Rationale:** presets freeze bundling before knobs are lived with;
  all-or-nothing switch punishes mixed-size media (small diagram wants to
  stay inline in an appendix-mode doc).
- **Rejected:** document-kind presets now (revisit as sugar later);
  doc-level switch only (no escape hatch).

### P4 — Figure identity: full machinery (D)

- **Proposal:** lone-image paragraph = figure with caption + `{#fig:…}`
  id + auto-numbering; diagrams get caption via fence attr;
  index/appendix entries carry caption text.
- **Rationale:** an index without captions/numbers is a bare link list —
  the brief's memoir deliverable needs identity; the typst "lone image =
  centered 92%" heuristic (`emit.rs:213-248`) is already proto-figure
  behavior.
- **Rejected:** ids+links only (weak index); appendix-grouping only (no
  index possible, imprecise links).

### P5 — Orientation: geometry-only, per-format capability gate (C)

- **Proposal:** rule written into the design: orientation = real
  page-geometry change; content/text never rotated. PDF: native typst.
  DOCX: multi-`sectPr` + `w:orient` (new plumbing, hand-verified in Word
  + LibreOffice). PPTX: directive ignored **with a warning** (per-slide
  orientation is impossible in OOXML; the brief's own rule decides).
- **Rationale:** "only if the rendering can do it properly."
- **Rejected:** rotating content on portrait pages (head-turning —
  banned by brief); silent pptx degradation (surprises are worse than
  warnings); auto-orientation from diagram aspect ratio (non-deterministic
  surprise; explicit opt-in only, possibly with a lint suggesting it).
- **Amended (user, 2026-10-16):** the user is considering **deprecating
  pptx output altogether** (docx export + human copy-paste would cover
  that case). Until that separate decision lands, this feature is
  **PDF + DOCX first-class**; pptx is frozen at current behavior and
  *warns* on any new directive it can't honor (orientation, appendix
  placement, figure index). No new pptx plumbing — no slide-jump links,
  backup-slide appendix, or agenda index. Deprecation itself remains an
  open question, tracked below.

### P6 — Cross-cutting defaults (v1 boundary)

- Custom typst templates: page controls/orientation are
  default-template-only in v1; documented as such.
- Index page numbers: PDF yes (typst computes layout); DOCX links-only
  (no static page numbers without field codes); PPTX index optional
  agenda-style slide, appendix ≈ backup slides with slide-jump links.
- Internal body links are new plumbing in all three renderers (docx
  flattening `Run` has no link field; pptx has no internal jump; typst
  has no label emission) — this is the shared substrate P3/P4/citations
  all sit on.
- WIT/athena surface: no v1 export of new controls (keep wasm surface
  stable); revisit when the plugin needs them.
- Citations: reserve `[@key]` namespace + generic back-matter registry;
  implement nothing.

### D7 — Auto-shrink scope (user, 2026-10-16)

- **Decision:** portable auto-shrink is hard; the MVP is **PDF only** —
  typst figures/diagrams scale to fit the page content box (width *and*
  height). DOCX keeps its current width clamp unchanged (DOCX output is
  destined for human editing); pptx is moot (frozen per P5 amendment).
- **Rationale:** typst layout is ours, so fit is exact and deterministic;
  DOCX/PPTX layout belongs to the viewer.
- **Rejected:** identical shrink across formats (layouts differ — can't
  be honest); doing nothing (oversized diagrams already overflow).

### D8 — Diff integration postponed (user, 2026-10-16)

- **Decision:** page/keep/placement annotations do not participate in
  the markdown structural diff in v1 (invisible metadata); the
  office-level diff is unaffected in practice (pptx-only, and pptx is
  frozen). A dedicated deep pass is explicitly postponed.
- **Rationale:** user call — no current need; revisit when diff work
  resumes.

### D9 — Naming (delegated to agent by user, 2026-10-16)

- **Frontmatter** — one nested namespace, flowing to renderers through
  the proven `extra` → typst `meta` path as a `media` dictionary:
  ```yaml
  media:
    placement: inline   # inline (default) | appendix
    index: false        # true → back-matter Figures list linking to figures
  ```
- **Attribute vocabulary** — heading `{...}`, fence info-strings, and
  image `{...}` (image attribute parsing is new; today title/id are
  discarded at `parse.rs:325-336`):
  | token | applies to | meaning |
  | --- | --- | --- |
  | `{#fig:arch}` | any block | explicit id (existing `{#name}` channel) |
  | `place=inline\|appendix` | figures, fences | per-item override of `media.placement` |
  | `caption="…"` | fences | figure caption (images: alt text is the caption, Pandoc-style) |
  | `page=landscape\|portrait` | headings, figures | orientation of that page/section |
  | `new-page` | any block | start this block on a fresh page (bare token = true) |
  | `keep=true\|false` | figures | keep-together; **default true for figures** |
- **Rationale:** a nested frontmatter map groups the related knobs and
  keeps the flat namespace clean (`toc:` stays the lone flat precedent);
  tokens are short and reuse the existing `{#id}` channel; figures
  keep-together by default because that is the sane default.
- **Rejected:** flat `media-placement`-style keys (collision-prone as the
  set grows); separate top-level keys per concern (sprawl).

### D10 — Small semantics (user-confirmed 2026-10-16)

1. **Appendix orientation follows the document default** (portrait);
   individual figures opt into landscape via `page=landscape` even inside
   the appendix — no implicit landscape appendix.
2. **Heading `page=` spans its section**: until the next heading of
   level ≤ its own.
3. **Figure `page=` implies its own page** (a geometry change forces a
   break in typst anyway; in DOCX it becomes its own section).
4. **Typst fit warns below a 0.5 scale factor** (deterministic compile
   constant), nudging the author toward an explicit landscape page.

## Open questions

1. **pptx deprecation** — user is *considering* it (2026-10-16); not yet
   decided. Separate from this feature (which freezes pptx at status quo
   + warnings). When decided, it deserves its own roadmap note.
2. (Implementation note, not a design question: image `{...}` attribute
   parsing is new — `Inline::Image` title/id discarded today.)

Both carry into distillation as context; neither blocks the plan.
