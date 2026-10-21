---
name: docgen-cli
description: Render markdown + YAML frontmatter to PDF, DOCX and PPTX, and mermaid/excalidraw sources to SVG/PNG with the bundled docgen CLI — deterministic, offline, no runtime dependencies. Static binaries for Windows x64 and Linux x64 (musl).
---

# docgen-cli

The `docgen` command-line tool turns markdown (with YAML frontmatter)
into finished documents, and diagram sources into images. Byte-
deterministic output, no network access, no runtime dependencies — the
Linux build is a static musl binary, the Windows build needs nothing but
Windows.

Binaries live in this skill's `scripts/`:

- `scripts/docgen.exe` — Windows x86_64 (MSVC)
- `scripts/docgen-linux` — Linux x86_64, static (musl)

## References

Read before authoring — each is self-contained and grounded in what this
exact binary supports:

- [references/markdown-primer.md](references/markdown-primer.md) — the
  complete markdown dialect: frontmatter, blocks, inline styles,
  attribute syntax (`{#id …}`, `page=`, `keep=`), figures and captions,
  ids and internal links. Read when authoring or reviewing any document.
- [references/mermaid-primer.md](references/mermaid-primer.md) — every
  supported mermaid diagram type with snippets, plus docgen integration
  (SVG-text labels, edge-label plates, captions, sizing). Read before
  writing mermaid.
- [references/excalidraw-primer.md](references/excalidraw-primer.md) —
  raw `.excalidraw` JSON as a first-class input: element and field
  coverage, bindings, copy-paste element templates, authoring
  conventions. Read before generating scenes.
- [references/typst-templates.md](references/typst-templates.md) —
  authoring custom PDF templates: the `meta`/body contract, the hermetic
  world's constraints, a typst language tour, diagnostics. Read before
  writing a `--template` file.
- [templates/](templates/) — five ready-to-use templates (classic
  `report`, two-column `paper`, `design-doc`, `handbook`, `memo`),
  package-free and verified against this binary. Use directly or as
  starting points.

## Run

```
docgen render INPUT.md --format pdf|docx|pptx [--out FILE] [--template T]
docgen diff OLD.md NEW.md [--format pptx] [--author NAME] [--date ISO]
docgen diagram INPUT --format svg|png [--out FILE]
```

- `render` — the main path: markdown + frontmatter → document. Images
  resolve relative to the source file; missing ones render as visible
  placeholders with a warning. `--template` takes a typst template
  (PDF), a `.docx` with `{{placeholders}}` + a `{{docgen:body}}` marker
  paragraph (DOCX), or a `.pptx` whose masters/layouts/theme style the
  generated slides (PPTX).
- `diff` — structural diff of two revisions rendered as a PPTX with
  revision chips, a removed-content slide and a summary (pptx only).
- `diagram` — a standalone `.mmd`/mermaid file or raw `.excalidraw` JSON
  (detected by extension, then content) → SVG or PNG.

Warnings print to stderr: missing/unreadable images, diagrams no
renderer handled, figures scaled below 50% to fit the page, and — on
pptx — page/media directives the pptx backend ignores. Surface them.

## Documents in one screen

Frontmatter:

```yaml
---
title: …            # cover page / document metadata
subtitle: …
authors: [a, b]
date: 2026-10-16
version: 1.2
doc_id: stable-id   # stable identity for diffing revisions
toc: true           # Contents (PDF) / linked static TOC (DOCX)
media:
  placement: inline # or appendix — media moves to a figures appendix
  index: true       # add a "List of Figures" back-matter section
---
```

Unknown keys are preserved and passed through to PDF templates (typst
`meta`); DOCX/PPTX templates see the scalar keys as `{{title}}`-style
placeholders.

Figures: a **lone-image paragraph** or a fenced mermaid/excalidraw block
is *media*; with a non-empty caption it becomes a numbered figure.

```markdown
![System architecture](img/arch.png){#fig:arch place=appendix keep=false}
```

- Alt text is the default caption; `caption="…"` overrides it (diagrams
  only get a caption through `caption=`). Uncaptioned media renders
  unnumbered and stays out of the index.
- `{#fig:name}` (fences, images) and `{#name}` (headings) set block ids;
  internal links `[text](#section-id)` and `[text](#fig:name)` become
  clickable in PDF and DOCX. A render error `label <fig-…> does not
  exist` means a link targets an id no block set — check the spelling
  and that every `{#…` container is terminated.
- `place=inline|appendix` overrides the `media.placement` default per
  item. Appendix-placed figures leave a "Figure N" link at their
  original position and render in a back-matter appendix.
- `keep=false` lets a figure split across pages (default: figures keep
  image and caption together).
- Figures and diagrams auto-scale to fit the page (width **and**
  height); a deterministic warning reports scales below 50%.

Page control:

- `new-page` — start the block on a fresh page (heading or fence token).
- `page=landscape|portrait` — real page-geometry change, never rotated
  content. On a heading it spans the section (until the next heading of
  level ≤ its own); on a figure it applies to that figure alone. It is
  **orientation-only**: when the orientation already matches, nothing
  breaks — figures flow inline. In DOCX a differing `page=` figure
  becomes its own section.
- Attributes ride three syntax channels: heading blocks
  (`## Results {page=landscape}`), fence info strings
  (```` ```mermaid {#fig:arch} place=appendix ````), and image attribute
  blocks (`![alt](url){#fig:x}`). Unknown tokens are ignored without
  error; literal braces in prose are left alone.
- **The `{#id …}` container works on all three channels** — including
  fences, where it may hold several attributes across its whitespace
  (```` ```mermaid {#fig:arch caption="System"} ````). Every container
  must be terminated: an unterminated `{#…` fails the render with an
  explicit `invalid attribute syntax` error.

## Per-format capability

| Capability | PDF | DOCX | PPTX |
|---|---|---|---|
| Figures, captions, numbering | ✓ | ✓ | media as plain images |
| Appendix + figures index | ✓ with page numbers | ✓ links only (no page numbers) | ✗ |
| `new-page`, `keep=`, `page=` | ✓ | ✓ | ✗ — ignored **with a warning** |
| Internal links | ✓ | ✓ | external only |
| `--template` | typst template | .docx template | .pptx template |

Note: page controls in PDF apply to the built-in template only — a
custom typst template owns its page setup
([references/typst-templates.md](references/typst-templates.md)).

## Diagrams

- `mermaid` fenced blocks and files render natively — supported types
  and integration details:
  [references/mermaid-primer.md](references/mermaid-primer.md).
- Raw `.excalidraw` JSON is first-class: standalone files and fenced
  ` ```excalidraw ` blocks, independent of any conversion —
  [references/excalidraw-primer.md](references/excalidraw-primer.md).

## Notes

- Output is byte-deterministic: same input + same version → identical
  bytes (zip entries carry fixed timestamps; no clocks, no randomness).
- `docgen render` never touches anything but the input path, the output
  path, and images relative to the input.
