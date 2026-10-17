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

If they are missing, build them — see *Build* at the bottom.

## Run

```
docgen render INPUT.md --format pdf|docx|pptx [--out FILE] [--template T]
docgen diff OLD.md NEW.md [--format pptx] [--author NAME] [--date ISO]
docgen diagram INPUT --format svg|png [--out FILE]
```

- `render` — the main path: markdown + frontmatter → document. Images
  resolve relative to the source file; missing ones render as visible
  placeholders with a warning.
- `diff` — structural diff of two revisions rendered as a PPTX with
  revision chips, a removed-content slide and a summary. (`--format
  docx` exists in the libraries but is parked pending a round-trip
  design.)
- `diagram` — a standalone `.mmd`/mermaid file or raw `.excalidraw` JSON
  (detected by extension, then content) → SVG or PNG.

Warnings print to stderr: missing/unreadable images, diagrams no
renderer handled, figures scaled below 50% to fit the page, and — on
pptx — page/media directives the frozen pptx backend ignores.

## Documents: frontmatter

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

Unknown keys are preserved and passed through to templates (typst
`meta`).

## Figures, captions, placement

A **lone-image paragraph** (`![alt](url)` alone in a paragraph) or a
fenced mermaid/excalidraw block is *media*. Media with a non-empty
**caption** is a numbered figure ("Figure N", document-wide).

```markdown
![System architecture](img/arch.png){#fig:arch place=appendix keep=false}
```

- Alt text is the default caption; `caption="…"` overrides it (diagrams
  only get a caption through `caption=`). Uncaptioned media renders
  unnumbered and stays out of the index.
- `{#fig:name}` (fences, images) and `{#name}` (headings) set block ids;
  internal links `[text](#section-id)` and `[text](#fig:name)` become
  clickable in PDF and DOCX.
- `place=inline|appendix` overrides the `media.placement` default per
  item. Appendix-placed figures leave a "Figure N" link at their original
  position and render in a back-matter appendix.
- `keep=false` lets a figure split across pages (default: figures keep
  image and caption together).
- Figures and diagrams auto-scale to fit the page (width **and**
  height); a deterministic warning reports scales below 50%.

## Page control

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
  explicit `invalid attribute syntax` error (older builds silently
  dropped the id instead — check for that if a link fails on one).

## Per-format capability

| Capability | PDF | DOCX | PPTX |
|---|---|---|---|
| Figures, captions, numbering | ✓ | ✓ | media as plain images |
| Appendix + figures index | ✓ with page numbers | ✓ links only (no page numbers) | ✗ |
| `new-page`, `keep=`, `page=` | ✓ | ✓ | ✗ — ignored **with a warning** |
| Internal links | ✓ | ✓ | external only |
| `--template` | typst template | .docx template | .pptx template |

PPTX is frozen at status quo. Note: page controls in PDF apply to the
built-in template only — a custom typst template owns its page setup.

## Diagrams

- `mermaid` fenced blocks and files render natively (math labels
  supported in the CLI build).
- Raw `.excalidraw` JSON is first-class: standalone files and fenced
  ` ```excalidraw ` blocks, independent of any conversion.

## New in 0.3.1 (2026-10-16)

The two defects found during the 2026-10-16 end-to-end exercise are
fixed:

- **Strikethrough renders.** `~~text~~` now works in PDF as in
  DOCX/PPTX (it used to abort the PDF render).
- **Fence attribute containers work.** `{#fig:x caption="…"}` on a
  fence honours the id and the caption (it used to drop the id and
  corrupt the caption); unterminated containers now error explicitly
  instead of failing silently later.

A render failing with `label <fig-…> does not exist` still means a
fence id did not apply — check its spelling and that every `{#…`
container is terminated.

## Build

`scripts/build.sh` (bash; run from anywhere) builds both binaries from
the repo and drops them next to itself:

```bash
bash crates/docgen-cli/skill/scripts/build.sh
```

Needs a Rust toolchain. The musl binary requires a real musl C
toolchain (typst's `stacker` pulls `psm`, whose assembly needs
`musl-gcc`): on a Linux x86_64 host it builds natively; from Windows the
script builds inside the default **WSL distro** (needs `cargo` and
`musl-gcc` there — on Arch: `pacman -S rust musl`). Both are `--release`
builds with fat LTO — expect **12–16 minutes each on a cold tree**; the
profile is the point (small, deterministic binaries), don't "fix" it.

## Notes

- Output is byte-deterministic: same input + same version → identical
  bytes (zip entries carry fixed timestamps; no clocks, no randomness).
- `docgen render` never touches anything but the input path, the output
  path, and images relative to the input.
