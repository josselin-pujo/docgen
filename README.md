# docgen

A single-binary documentation renderer and differ, written in Rust. Markdown
in; PDF, DOCX and PPTX out; diagrams rendered natively; diffs expressed in the
target format's own revision machinery (Word tracked changes, ...) instead of
side-by-side approximations.

The libraries are strictly **in-memory**: strings and byte buffers in, owned
bytes out — no filesystem, no network, no clocks — so the same code runs as a
CLI, inside CI, and in a wasm runtime (`wasm32-wasip1/2` are the supported
targets; a browser `wasm32-unknown-unknown` build of the render core is a
roadmap item).

## Status (M2 — working today)

- **Markdown + rich YAML frontmatter** → typed document model
  (`docgen-core`): headings, paragraphs, inline styles (bold/italic/code/
  links/images), ordered & unordered lists (nested), tables, block quotes,
  code blocks.
- **Stable block ids** — explicit `{#id}` attributes on headings and fenced
  blocks (` ```mermaid {#arch} `), heading slugs, or content hashes
  otherwise. These are the join key for structural diffing.
- **Structural diff** (`docgen-core::diff`): id-anchored classification
  (added / removed / moved / changed), LCS interleaving of removals at their
  old positions, word/run-level inline diffs, line/item/row-level diffs for
  code/lists/tables, diagram source changes, and a similarity pass that
  fuzzy-pairs edited auto-id'd paragraphs into single changes instead of
  remove+add noise.
- **DOCX output** (`docgen-office`): hand-rolled OOXML (zip + XML, all in
  memory, byte-deterministic output — fixed zip timestamps, no clocks).
  Fresh rendering with styles/numbering/tables/images, template filling,
  and **diff rendering as native Word tracked changes**: equal content
  flows through unmarked, deletions are `w:del`/`w:delText`, insertions
  `w:ins`, at word/line/item/row granularity; paragraph and table-row
  marks; anchored Word comments on moved blocks and diagram replacements;
  revision author/date attributed deterministically.
- **Mermaid → SVG/PNG** via [`merman`](https://github.com/Latias94/merman)
  (headless Rust Mermaid, ELK layout). Diagram sources are automatically
  forced to native SVG `<text>` labels (`htmlLabels: false`) because
  browser-less consumers (typst/usvg, rasterizers) drop `<foreignObject>`
  labels silently. Edge-label plates default to opaque white (invisible on
  white pages, masks the edge line that would otherwise strike through the
  label); `EdgeLabelBackground::Transparent` opts out for colored page
  backgrounds.
- **PDF** via embedded typst 0.15 against a custom in-memory `World`:
  fonts compiled in from `typst-assets` (Libertinus + NewCM math + DejaVu
  mono), deterministic output (`World::today` returns `None` — document
  dates come from frontmatter). Per-page PNG rendering via `typst-render`.
- **Custom typst templates**: pass a `.typ` file as the main document shell
  (`docgen render doc.md --template shell.typ`, or
  `TypstRenderer::with_template`). Every compile exposes two virtual
  modules the template can use:
  - `#import "docgen-meta.typ": meta` — the frontmatter as a dictionary
    (`title`, `subtitle`, `authors`, `date`, `version`, `doc_id`, plus
    every unknown YAML key, nested values included);
  - `#include "docgen-body.typ"` — the rendered document blocks.
  Diagram images land in `assets/*` inside the same virtual filesystem.
  See `examples/template.typ` for a worked example (title band, running
  header, page numbering). Typst errors are reported with virtual file +
  line (e.g. `docgen-meta.typ:4`).
- **DOCX templates**: `docgen render doc.md --format docx --template
  house-style.docx` fills any `.docx`: `{{title}}`-style placeholders from
  frontmatter, and a `{{docgen:body}}` marker paragraph replaced by the
  body (direct formatting — no dependency on the template's styles).
  Diagram images are appended to `word/media/` with fresh rel ids.
- **CLI**:
  - `docgen render doc.md [-o out] [--format pdf|docx] [--template ...]`
  - `docgen diff old.md new.md [-o out.docx] [--author A] [--date D]`
  - `docgen diagram flow.mmd [-o out.svg|--format png]`

```bash
cargo run -p docgen-cli -- render examples/sample.md   # → examples/sample.pdf
cargo run -p docgen-cli -- example-style 2>/dev/null   # (see examples/)
```

## Workspace layout

| Crate | Role | wasm |
|---|---|---|
| `docgen-core` | document model, markdown+frontmatter parsing, stable ids, `DiagramRenderer` trait | ✅ wasip1 |
| `docgen-typst` | model → typst markup → PDF/PNG via in-memory `World` | ✅ wasip1 |
| `docgen-mermaid` | `DiagramRenderer` impl backed by merman (SVG + PNG) | ✅ wasip1, ✅ unknown-unknown |
| `docgen-office` | DOCX output + revision-aware diff (tracked changes, comments, templates) | ✅ wasip1 |
| `docgen-wasm` | wasm embedding surface: C-ABI probe today, WIT component in M5 | n/a |
| `docgen-cli` | the only crate that touches the filesystem | n/a |

## How IOless is the wasm story, really?

Measured, not guessed (`crates/docgen-wasm` builds a cdylib that runs the
full parse → diagram → typst pipeline; import sections parsed from the
release artifacts):

- **wasm32-wasip1**: compiles and links. The module imports **15 functions**
  from `wasi_snapshot_preview1` (`random_get`, `environ_*`, `clock_time_get`,
  `fd_close/read/write`, `fd_fdstat_get`, `fd_filestat_get`,
  `fd_prestat_get/dir_name`, `path_open`, `path_filestat_get`, `proc_exit`,
  `sched_yield`). None of them is ever *used* for IO by docgen — they are
  references std's startup/panic/misc paths keep alive. A host can satisfy
  all fifteen with trivial stubs (empty environ/args, EBADF on fds, host
  PRNG).
- **wasm32-unknown-unknown**: compiles and links with **zero WASI imports**.
  The only imports are 5 wasm-bindgen placeholder functions
  (`__wbindgen_throw`, externref table ops, describe, jspi poll) pulled by
  merman's math-label font fallback chain (`ratex-svg → system-fonts →
  web-sys`), which is a hard dep of merman's `math` feature. A browser host
  gets these for free; a bare embedder writes 5 one-line stubs — or drops
  merman's math feature / gets `system-fonts` made optional upstream for a
  completely import-free module.

**Consequence for packaging:** a small custom WIT surface is not just
possible, it is the right shape. Plan (M5): a `world docgen { export
render-pdf: func(markdown: string, ...) -> result<list<u8>, string>; ... }`
component, built from the unknown-unknown (or wasip1) core with
`wasm-tools component new` — importing nothing, exporting docgen. The full
WASI preview1 surface is never needed.

## Design rules

1. **No I/O in libraries.** Inputs are `&str`/`&[u8]`, outputs owned. The CLI
   (or a wasm shell) owns files.
2. **No ambient state.** No clocks, no random, no env: renders are
   deterministic and diffable byte-for-byte.
3. **Engines behind traits.** `DiagramRenderer` today; a JS-runtime trait
   (rquickjs native, already proven to cross-compile to wasm in a prior
   spike) for Excalidraw later.
4. **Escaping at the boundary.** All author text is escaped when lowered to
   typst (and will be for OOXML), so content can never change markup
   structure.

## Licensing notes

- typst, typst-assets fonts (Libertinus OFL, NewCM OFL, DejaVu – free),
  merman core (MIT OR Apache-2.0).
- merman's default layout engine is **ELK, EPL-2.0** (weak copyleft):
  distributing a binary that includes it requires keeping EPL notices and
  source provenance for the ELK part. If that is unacceptable, build merman
  with `default-features = false` + `complete-svg` (Cytoscape layout) — the
  `docgen-mermaid` feature surface will expose this as a Cargo feature.

## Roadmap

### M2 — structural diff + DOCX ✅ (shipped; refinements below)

- Word comment threading (`commentsExtended.xml`) and resolved states.
- `w:moveFrom`/`w:moveTo` for moves (currently: comment only — Word's move
  markup is finicky across consumers).
- Frontmatter diff coverage (title/author changes currently render
  unmarked).
- Merge split runs before placeholder substitution in templates.

### M3 — PPTX

- Slide-oriented projection of the document model (one section per slide,
  or a template deck with id-keyed placeholder slides).
- Structural diff as slide-level add/remove/replace; native pptx
  "versioning" is not dependable, so diffs will be change-marked slides +
  a summary slide, not revision marks.

### M4 — Excalidraw

- Parse native `.excalidraw` JSON into the model (serde types for the
  element schema).
- Native renderer to SVG using `roughr` (Rust port of Rough.js) +
  `cosmic-text` for label metrics with embedded Excalifont — no browser, no
  DOM shims, wasm-safe. Precedent: moona3k's `excalidraw-export` proved a
  DOM-free roughjs renderer matches the web output closely.
- Structural diff on element ids: externally-edited JSON is merged by
  element id (replace changed elements, add new ones) rather than
  regenerated wholesale.
- Mermaid → Excalidraw via `@excalidraw/mermaid-to-excalidraw` inside the
  JS-runtime trait (rquickjs native/wasm). This package drags in the real
  mermaid.js parser, so it stays an optional feature.

### M5 — wasm packaging & CI

- Turn `docgen-wasm` into the real embedding surface: a WIT component with a
  custom world (`render-pdf`, `render-diagram`, later `diff-docx`) importing
  nothing (see "How IOless is the wasm story, really?" above), plus an
  optional wasm-bindgen shell for browsers reusing the same libraries.
- CI jobs: `cargo check --target wasm32-wasip1` and
  `--target wasm32-unknown-unknown` for every library crate; import-surface
  assertion on the built component (fail if new host imports appear).
- Golden-file tests for PDF/DOCX outputs; fuzz the frontmatter splitter
  and the typst escaper.
