# Markdown primer

Everything the markdown dialect accepts, as docgen renders it today. The
dialect is CommonMark plus tables, strike­through, heading attributes and
task lists (pulldown-cmark). One `.md` file = optional YAML frontmatter,
then the body.

Read this when authoring or reviewing a document source. For diagrams see
[mermaid-primer.md](mermaid-primer.md) and
[excalidraw-primer.md](excalidraw-primer.md); for PDF page design see
[typst-templates.md](typst-templates.md).

## Frontmatter

A leading `---` fenced YAML block. LF or CRLF, optional BOM. Every key
below is optional.

```yaml
---
title: System Design            # cover title, PDF metadata
subtitle: Payment platform      # cover subtitle
authors: [J. Doe, R. Roe]       # array of strings
date: 2026-10-17                # any scalar; rendered verbatim as text
version: 1.2                    # free-form version label
doc_id: payment-design          # stable identity for diffing revisions
toc: true                       # Contents outline (PDF) / linked static TOC (DOCX)
media:
  placement: inline             # inline | appendix — where media renders
  index: true                   # add a "List of Figures" back-matter section
---
```

- **Unknown keys are kept** and reach PDF templates through the typst
  `meta` dictionary (nested maps, arrays, numbers, booleans all work) —
  see [typst-templates.md](typst-templates.md). DOCX/PPTX templates only
  see the known scalar keys via `{{title}}`-style placeholders.
- A malformed `media:` map (e.g. `placement: inlane`) fails the render
  with an explicit error — typos do not silently fall back to defaults.
- `media: placement: appendix` moves all figures to a back-matter
  appendix; each original position keeps a clickable "Figure N" link.
  `index: true` adds a "List of Figures" (PDF: with page numbers; DOCX:
  links only). Per-item `place=` overrides the default.

## Blocks

| Syntax | Rendering |
|---|---|
| `# H1` … `#### H4` (through H6) | headings; H1/H2 also structure PPTX slides |
| paragraph | plain text |
| `- item` / `1. item` | unordered/ordered lists, arbitrarily nested; ordered lists keep their start number |
| GFM pipe table | bordered table, first row is the header row; cells take full inline styling |
| ```` ```lang ```` fenced block | verbatim code block in a monospace style; `lang` is a cosmetic label |
| `> quoted` | block quote (quoted paragraphs inside) |
| `---` alone | horizontal rule |
| `![alt](url)` **alone in its paragraph** | media — see *Figures* below |
| ```` ```mermaid ```` / ```` ```excalidraw ```` | diagram media — see the diagram primers |

Math (`$…$` inline, `$$…$$` display paragraph) is **typeset in PDF**
(LaTeX-ish notation; unknown commands degrade to visible text). In DOCX
and PPTX it stays literal source text.

Not supported (silently or visibly ignored — do not use): raw HTML
(blocks and inline are dropped), footnotes,
task-list markers (`- [x]` renders the item text without a checkbox),
definition lists. Autolinks (`<https://…>`) work as normal links.

### Headings carry ids

Every heading gets a stable id used by internal links and diffing:

- an explicit `{#my-id}` attribute always wins;
- otherwise a slug of the text: lowercased ASCII alphanumerics, runs of
  other characters collapse to one `-` ("Hello, World!" → `hello-world`).
  Non-ASCII characters are dropped ("Étude" → `tude`); a slug-less
  heading becomes `section`;
- collisions get `-2`, `-3`, … suffixes.

`{#fig:name}` on any block: the `fig:` marker is stripped and the block
id becomes `name`, so the same id works as a heading id and as a figure
anchor.

## Inline styles

`**bold**`, `*italic*`, `~~strikethrough~~`, `` `code` `` (backticks are
honored inside inline code via doubled backticks), `[text](url)`,
`![alt](url)` images, hard breaks (backslash or two trailing spaces),
soft breaks join with a space.

- **Internal links**: `[text](#section-id)` and `[text](#fig:name)`
  (also `#fig-name`) become clickable links in PDF and DOCX. A link
  whose target does not exist is a **typst render error**
  (`label <…> does not exist`) — check spelling and that the target's
  `{#…}` container is terminated.
- **Images**: PNG/JPEG/GIF/SVG, resolved relative to the source file
  (CLI) or supplied as `assets` (wasm). Inline images sit in the
  paragraph flow; an image **alone in its paragraph** becomes media
  (below). SVG renders in PDF only — DOCX/PPTX use raster images.
  Missing bytes render as the linked alt text plus a warning, never as
  a blank.

## Figures, captions, placement

Media = a lone-image paragraph or a mermaid/excalidraw fence. Media
with a **non-empty caption** is a numbered figure ("Figure N",
document-wide, across inline and appendix placement).

```markdown
![System architecture](img/arch.png){#fig:arch place=appendix keep=false}
```

- Alt text is the default caption; `caption="…"` overrides it.
  Diagrams only get a caption through `caption=`.
- Uncaptioned media renders unnumbered (full width, centered for lone
  images) and stays out of the figures index.
- `place=inline|appendix` overrides the `media.placement` default per
  item. Appendix-placed figures leave a "Figure N" link at their
  original position.
- `keep=false` lets a figure split across pages (default: image and
  caption stay together).
- Figures auto-scale to fit the page content box (width **and**
  height); a deterministic warning fires below 50% of natural size —
  usually means a very tall diagram that wants its own landscape
  section. This auto-fit applies to the built-in PDF template only; a
  custom typst template owns its geometry (fixed 92% width then).

## Attribute syntax

Attributes ride three channels:

```markdown
## Results {page=landscape}                        ← heading attributes
```mermaid {#fig:arch caption="System"} place=inline ```   ← fence info string
![alt](img/arch.png){#fig:arch keep=false}         ← image attribute block
```

Vocabulary: `{#id}`, `place=inline|appendix`,
`page=landscape|portrait`, `keep=true|false`, `caption="…"`,
`new-page`. Unknown keys and values of valid shape are accepted and
ignored, so authoring is forward-safe.

Rules that matter in practice:

- On a fence or image, a `{#id …}` container may hold several
  attributes across its whitespace (```` ```mermaid {#fig:arch
  caption="System"} ````). Every container must be **terminated** — an
  unterminated `{#…` fails the render with an explicit
  `invalid attribute syntax` error.
- Attribute blocks after images are recognized only when *every* token
  inside the braces has attribute shape — `{see the docs for details}`
  after an image stays prose. Literal braces in plain prose are never
  touched.
- Values may be quoted (`caption="two words"`); `\"` and `\\` are the
  escapes inside quotes. Fence info strings keep escapes; heading and
  image attribute values are already unescaped by markdown parsing.

## Page control

- `new-page` — start the block on a fresh page (token on a heading or
  in a fence info string).
- `page=landscape|portrait` — a real page-geometry change, never
  rotated content. On a heading it spans the section, until the next
  heading of level ≤ its own. On a figure it applies to that figure
  alone, and it is **orientation-only**: when the orientation already
  matches the surroundings nothing breaks — the figure simply flows
  inline. In DOCX a differing `page=` figure becomes its own section.
- Both apply to PDF (built-in template) and DOCX; PPTX ignores them
  with a warning. `new-page` works in PDF too when the target
  orientation matches; it is the explicit break, `page=` is geometry.

## Per-format capability

| Capability | PDF | DOCX | PPTX |
|---|---|---|---|
| Figures, captions, numbering | ✓ | ✓ | media as plain images |
| Appendix + figures index | ✓ with page numbers | ✓ links only (no page numbers) | ✗ |
| `new-page`, `keep=`, `page=` | ✓ | ✓ | ✗ — ignored, warning |
| Internal links | ✓ | ✓ | external only |
| `--template` | typst template | .docx template | .pptx template |

PPTX output is slide-oriented: title slide from frontmatter, one slide
per H1/H2, bullets, tables, code boxes, centered diagram images. Page
controls and figure appendices do not exist there.

## Authoring checklist

- Put diagrams and images in their own paragraphs; caption what the
  reader should be able to reference (`{#fig:…}` + caption).
- Give sections you will link to explicit ids (`{#stable-id}`) — slugs
  change when the heading text changes.
- Landscape figures: put `page=landscape` on the figure, not a manual
  break + rotated image.
- Avoid HTML and footnotes — they are not modelled. `$math$` typesets in PDF only.
- Long documents: frontmatter `toc: true`, and consider
  `media: placement: appendix` for many large figures.
