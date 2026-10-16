# Decisions

Decision log for code-workflow rounds in this repo. One entry per
decision, numbered sequentially, never reused; superseding entries point
forward. Current round source: `docs/notes/large-diagrams-and-page-control.md`
(brainstorm, converged 2026-10-16).

## D-001: Markdown extension syntax — attributes on existing blocks
- Status: accepted
- Context: page/keep/orientation/placement controls and figure options need a markdown surface; we own the parser (pulldown-cmark 0.13 single-pass, `docgen-core/src/parse.rs`, no pre/post passes).
- Decision: express controls as key=value and bare tokens in (a) heading attribute blocks `## Results {page=landscape}`, (b) fence info-strings ` ```mermaid {#fig:arch} place=appendix ` `, (c) image attribute blocks `![alt](url){#fig:x}` (image attrs are new parsing — title/id are discarded today at `parse.rs:325-336`). Store parsed values in a new per-block annotations field; never overload `Block.id` (id is the diff join key and the diagram asset key).
- Rationale: pulldown already parses heading attributes (only id consumed today, `parse.rs:86-100`) and `parse_info` already tokenizes info-strings (`parse.rs:383-399`) — smallest parser diff; controls attach to the exact object they modify.
- Alternatives rejected: Pandoc fenced divs — needs a brand-new parser pre-pass (none exist) and duplicates what headings already express; HTML comments — raw HTML is discarded by the parser today and the syntax is invisible, unscoped magic; a dedicated ` ```page ` fence — fake code block competing with diagram fences for the "what lives in a fence" mental model.

## D-002: Page-control vocabulary — breaks + keeps
- Status: accepted
- Context: no break/keep logic exists in any emitter; DOCX heading styles already carry keepNext/keepLines (`docgen-office/src/docx/parts.rs:54-94`).
- Decision: implement `new-page` (start this block on a fresh page) and keep-together for figure/diagram blocks (default ON for figures; `keep=false` relaxes). Book-style rules (chapter starts new/odd page) and a standalone "break between two arbitrary paragraphs" directive are deferred.
- Rationale: the two controls that directly serve large diagrams; breaks are almost always wanted *before* a block, which `new-page` on that block covers.
- Alternatives rejected: breaks only — defers the keep work large diagrams need; full book semantics — no use case, larger Office-verification surface.

## D-003: Media placement — frontmatter knobs + per-item override
- Status: accepted
- Context: brief wants memoir-style media appendices, selected per document via frontmatter.
- Decision: document default in frontmatter `media.placement: inline|appendix` (default inline) with per-figure `place=inline|appendix` attribute override; `media.index: true` generates a back-matter Figures list.
- Rationale: an all-or-nothing switch punishes mixed-size media (small diagrams should stay inline in an appendix-mode document); presets freeze bundling decisions before the knobs have been lived with.
- Alternatives rejected: document-kind presets (`kind: memoir`) — revisit as sugar later once knobs are proven; document-level switch only — no escape hatch.

## D-004: Figures get full identity machinery
- Status: accepted
- Context: index/appendix links are meaningless without identity; the typst emitter already has a "paragraph that is exactly one image → centered 92%" proto-figure heuristic (`docgen-typst/src/emit.rs:213-248`).
- Decision: a lone-image paragraph is a figure: alt text is the caption (Pandoc-style), `{#fig:…}` sets the id; diagram figures get captions via the `caption="…"` fence attribute; figures auto-number document-wide (Figure N); index/appendix entries carry caption text.
- Alternatives rejected: ids + links only — the index becomes a bare link list; appendix grouping without per-figure targets — no index possible, imprecise links.

## D-005: Orientation — geometry-only, per-format capability gate
- Status: accepted
- Context: brief wants landscape pages for large diagrams, but only where rendering is proper (reader never turns head/screen to read).
- Decision: orientation always means a real page-geometry change; content and text are never rotated. PDF/typst: native scoped `#set page`. DOCX: mid-body `w:sectPr` + `w:orient="landscape"` (new multi-section plumbing; today one hardcoded trailing sectPr exists, `docgen-office/src/docx/document.rs:1474-1476`). PPTX: per-slide orientation is impossible in OOXML (deck-wide `sldSz` only) → directives ignored **with a warning**, no new pptx plumbing.
- Rationale: the brief's own "only if it renders properly" rule decides pptx; a visible warning beats silent degradation.
- Alternatives rejected: rotating content on portrait pages (LaTeX-style sideways figures) — banned by the no-head-turning requirement; silent pptx ignore — surprises; auto-orientation from diagram aspect ratio — non-deterministic surprise, explicit opt-in only.

## D-006: Cross-cutting v1 boundaries
- Status: accepted
- Context: the feature touches three renderers with very different capabilities; scope must be fenced.
- Decision: (1) page controls/orientation work in the typst **default template only** (custom templates own their page setup — documented limitation). (2) Figures-index page numbers: PDF yes (typst computes layout); DOCX links-only (page numbers would need Word field codes); PPTX none. (3) No new WIT/athena exports. (4) Internal body links are new plumbing in all renderers (DOCX flattening `Run` has no link field and drops link URLs — `docgen-core/src/diff.rs:129-135`; pptx links are external-only; typst emits no labels) — this plumbing is the shared substrate for placement links, the figures index, and future citations.
- Rationale: keeps the wasm surface stable and scopes per-format cliffs honestly.
- Alternatives rejected: Word TOC/figure field codes for the DOCX index — field-update dependency, heavier; pptx slide-jump links / backup-slide appendix — moot under the pptx freeze (D-005, D-012).

## D-007: Auto-shrink MVP is PDF-only
- Status: accepted
- Context: oversized diagrams; portable shrink is hard.
- Decision: typst figures/diagrams scale to fit the page content box (width *and* height), with a deterministic warning when the scale factor falls below 0.5 (compile-time constant). DOCX keeps its current width clamp (`docgen-office/src/docx/document.rs:64-81`; DOCX output is destined for human editing). PPTX moot (frozen).
- Rationale: typst layout is ours, so fit is exact and deterministic; DOCX/PPTX layout belongs to the viewer.
- Alternatives rejected: identical shrink across formats — layouts differ, cannot be honest; doing nothing — tall diagrams already overflow.

## D-008: Diff integration postponed
- Status: accepted
- Context: annotations and appendix relocation reshape office output; the structural diff (pptx-only CLI surface) joins on block ids.
- Decision: in v1, page/keep/placement annotations are invisible metadata to the markdown structural diff; a dedicated deep diff pass is postponed.
- Rationale: user call — no current need; revisit when diff work resumes.
- Alternatives rejected: modeling annotations as diff-visible content — destabilizes diffs for no present benefit.

## D-009: Naming scheme
- Status: accepted
- Context: naming explicitly delegated to the agent by the user ("I'm bad at naming things").
- Decision: frontmatter gets one nested map — `media: { placement: inline|appendix, index: bool }` — riding the existing `extra` → renderer/typst-`meta` path. Attribute tokens: `{#fig:arch}` (explicit id, existing channel), `place=inline|appendix`, `caption="…"`, `page=landscape|portrait`, `new-page` (bare token = true), `keep=true|false`.
- Rationale: one namespace groups the related knobs and keeps the flat frontmatter clean (`toc:` stays the lone flat precedent); tokens are short and reuse the existing `{#id}` channel; figures keep-together by default is the sane default.
- Alternatives rejected: flat `media-placement`-style keys — collision-prone as the set grows; per-concern top-level keys — sprawl.

## D-010: Section/orientation semantics
- Status: accepted
- Context: small semantics left open at brainstorm convergence; user-confirmed 2026-10-16.
- Decision: (1) the media appendix inherits the document's orientation; figures opt into landscape individually via `page=landscape` — no implicit landscape appendix. (2) A heading's `page=` spans its section, ending at the next heading of level ≤ its own. (3) A figure with `page=` always gets its own page (a geometry change forces a break anyway; in DOCX it becomes its own section). (4) Typst warns when a figure's fit scale falls below the 0.5 floor.
- Rationale: explicit beats implicit; matches typst's natural break-on-geometry-change behavior.
- Alternatives rejected: implicit landscape appendix — surprising; heading `page=` scoped to a single page — fights section structure.

## D-011: Citations
- Status: deferred
- Context: brief asks to plan for citations but not implement (no use case yet).
- Decision: reserve the inline syntax namespace `[@key]` (Pandoc; collides with nothing docgen parses today) and build the back-matter machinery as a generic ordered registry (figures index first; bibliography slot later). Note for that future work: the no-IO rule means bibliography data must enter through the caller boundary the way image bytes do.
- Rationale: reservations are cheap now, expensive to retrofit.
- Alternatives rejected: designing citation syntax now — speculative.

## D-013: `page=` is orientation-only — no forced break on same orientation
- Status: accepted (amends D-010 (3))
- Context: user feedback 2026-10-16 after seeing the samples: a figure carrying `page=` for the orientation it already sits in still forced a page break (typst) and its own section (DOCX), so relocating media reshuffled layout even with no geometry change involved.
- Decision: a `page=` directive only takes effect when the orientation differs from the current section — then it starts a fresh page (typst page set rule / DOCX own section) as before. Same-orientation `page=` is a no-op: figures flow inline, and keep-together handles page fits. `new-page` remains the explicit break mechanism.
- Rationale: layout stability when figures move — breaks belong to `new-page` and keep-together; orientation belongs to `page=`.
- Alternatives rejected: keeping "always own page" — the directive leaked into layout regardless of geometry; dropping own-sections for differing orientations — would lose the geometry change itself.

## D-012: pptx output deprecation
- Status: deferred
- Context: user stated 2026-10-16 they are considering deprecating pptx output altogether, counting on DOCX export + human copy-paste for deck-shaped needs.
- Decision: not decided here. This feature treats pptx as frozen: current behavior plus warnings on directives it cannot honor (orientation, placement, index). When the user decides, record it in the README roadmap as its own item.
- Rationale: deprecation changes the roadmap broadly; it deserves its own decision.
- Alternatives rejected: deciding deprecation inside this feature — scope creep.
