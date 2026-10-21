---
name: render-doc
description: Render markdown to PDF/DOCX/PPTX and mermaid/excalidraw diagrams to SVG/PNG, fully in-process — blob refs, no shell, no filesystem required. Supports figures with captions and numbering, media appendices with a figures index, page breaks/keeps, and landscape/portrait sections.
---

# render-doc

`docgen_compose` renders documents and diagrams inside this session — a
wasm component runs the real docgen pipeline (the same markdown, YAML
frontmatter, and diagram support as the `docgen` CLI, without its diff
features). No executables, no shell. The tool is blob-native: refs in,
refs out — it works the same in sessions with or without a workspace
filesystem.

## Call

```
docgen_compose({
  action: "render" | "diagram",
  format: "pdf" | "docx" | "pptx",   // action: render
  // or   "svg" | "png",             // action: diagram
  content: "<inline markdown>",      // short sources
  // or:  source: { ref: "sha256:…" },  // the source as a blob ref
  assets: [{ name: "img/logo.png", ref: "sha256:…" }],  // images, by markdown URL
  kind: "mermaid" | "excalidraw",    // diagrams; inferred when omitted
})
```

Exactly one of `content` / `source`. Warnings come back with the
artifact: missing image bytes, diagrams no renderer handled, and — on
PDF — figures scaled below 50% to fit the page. Surface them.

- **`render`** — markdown with YAML frontmatter → PDF, Word document, or
  slide deck. `mermaid` and `excalidraw` fenced blocks render natively.
  `toc: true`
  frontmatter adds a Contents outline (PDF) or a static, linked table of
  contents (DOCX).
- **`diagram`** — a mermaid source or raw excalidraw JSON → SVG or PNG.

## Math

`$…$` inline and a `$$…$$` display paragraph are typeset **in PDF** —
LaTeX-ish notation: Greek, fracs, roots, sub/superscripts, sums/integrals
with under/over limits in display math, matrices (`pmatrix`), `cases`,
`aligned` multi-line equations, `\left(…\right)`, accents, `\text`.
Unknown commands degrade to visible upright text rather than failing the
render. In DOCX/PPTX (and inside mermaid labels) math stays **literal
source text** — those targets have no equation typesetter.

## Documents: frontmatter

```yaml
---
title: …            # cover page / document metadata
subtitle: …
authors: [a, b]
date: 2026-10-16
version: 1.2
toc: true           # Contents (PDF) / linked static TOC (DOCX)
media:
  placement: inline # or appendix — media moves to a figures appendix
  index: true       # add a "List of Figures" back-matter section
---
```

## Figures, captions, placement

A **lone-image paragraph** (`![alt](url)` alone in a paragraph) or a
fenced mermaid/excalidraw block is *media*. Media with a non-empty
**caption** is a numbered figure ("Figure N", document-wide).

```markdown
![System architecture](img/arch.png){#fig:arch place=appendix keep=false}
```

The `{#id …}` container form works everywhere — image attribute blocks
**and** fences. On a fence the container may hold several attributes
across its whitespace: ```` ```mermaid {#fig:arch caption="System
architecture"} ````. The container must be **terminated** — an
unterminated `{#…` fails the render with an explicit
`invalid attribute syntax` error.

- Alt text is the default caption; `caption="…"` overrides it (diagrams
  only get a caption through `caption=`). Uncaptioned media renders
  unnumbered and stays out of the index.
- `{#fig:name}` sets the block id and the link anchor (`#fig:name` or
  `#fig-name` both link to it).
- `place=inline|appendix` overrides the `media.placement` default per
  item. Appendix-placed figures leave a "Figure N" link at their original
  position and render in a back-matter appendix.
- `keep=false` lets a figure split across pages (default: figures keep
  image and caption together).
- Figures and diagrams auto-scale to fit the page (width **and** height);
  a warning reports scales below 50%. A render error `label <fig-…>
  does not exist` means a link targets an id no block set — check the
  fence id spelling and that every `{#…` container is terminated.

## Page control

- `new-page` — start the block on a fresh page (heading or fence token).
- `page=landscape|portrait` — real page-geometry change, never rotated
  content. On a heading it spans the section (until the next heading of
  level ≤ its own); on a figure it applies to that figure alone. It is
  **orientation-only**: when the orientation already matches, nothing
  breaks — figures flow inline. In DOCX a differing `page=` figure
  becomes its own section.
- Internal links: `[text](#section-id)` and `[text](#fig:name)` become
  clickable links in PDF and DOCX.

## Per-format capability

| Capability | PDF | DOCX | PPTX |
|---|---|---|---|
| Figures, captions, numbering | ✓ | ✓ | media as plain images |
| Appendix + figures index | ✓ with page numbers | ✓ links only (no page numbers) | ✗ |
| `new-page`, `keep=`, `page=` | ✓ | ✓ | ✗ — ignored **with a warning** |
| Internal links | ✓ | ✓ | external only |
| `$…$` / `$$…$$` math | ✓ typeset | literal text | literal text |

PPTX ignores directives it cannot honor and reports them in the
result's warnings.

## Filesystem sessions

When the `docgen-workspace` companion is installed (`docgen_workspace`
appears in the tool table), prefer it for workspace files — one call does
the whole trip, paths in, file out:

```
docgen_workspace({
  action: "render", format: "pdf",
  source: "docs/arch.md",
  assets: [{ name: "img/logo.png", path: "docs/img/logo.png" }],  // name = URL in the md
  out: "docs/build/arch.pdf",          // optional
})
```

Its siblings mint/materialize any blob for hand-offs: `docgen_file_to_blob
{path}` → ref, `docgen_blob_to_file {ref, path}`. Surfacing a written
file as a card: `load_artifact {path, media_type}` — pass `media_type`
for anything but PDF/PNG (the host's sniffer cannot identify OOXML or
SVG): `image/svg+xml` (SVG),
`application/vnd.openxmlformats-officedocument.wordprocessingml.document`
(DOCX) /
`application/vnd.openxmlformats-officedocument.presentationml.presentation`
(PPTX). The `docgen_compose` card carries the type itself in blob
sessions.

## Flow (blob sessions, no filesystem)

1. Compose the markdown and pass it inline as `content` (or chain a blob
   ref from another tool as `source: {ref}`).
2. Call `docgen_compose`; the result reports the artifact ref and the
   artifact is surfaced to the transcript as a card automatically.
3. For images, pass `assets` — `name` must be the **exact URL the markdown
   uses**. A URL without bytes renders as a visible placeholder and the
   result lists warnings; fix by adding the missing refs.

## Notes

- Prefer `content` for short sources; large sources should ride as blob
  refs so bytes never transit the tool call.
- Renders run under a 30 s / 256 MiB budget per call — split monster
  documents into sections.
- Renders are byte-deterministic: identical inputs (source + assets)
  yield the identical artifact digest — safe to cache or compare.
- The artifact is a content-addressed digest: pass it on
  (`load_artifact {digest}`, other tools), or materialize it to a file
  when the session has a workspace. With no filesystem it simply stays a
  blob — that is a complete result, not a failure.
- Rendering always uses the built-in document template; page controls and
  figures work as documented above. (A custom typst template would own
  its page setup — not exposed here.)
