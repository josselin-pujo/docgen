# docgen

A single-binary documentation renderer and differ, written in Rust. Markdown
in; PDF, DOCX and PPTX out; diagrams rendered natively; diffs expressed in the
target format's own revision machinery (today: PPTX revision decks — the DOCX
tracked-changes diff is implemented but parked, see the roadmap) instead of
side-by-side approximations.

The libraries are strictly **in-memory**: strings and byte buffers in, owned
bytes out — no filesystem, no network, no clocks — so the same code runs as a
CLI, inside CI, and in a wasm runtime (`wasm32-wasip1/2` are the supported
targets; a browser `wasm32-unknown-unknown` build of the render core is a
roadmap item).

## Status (M5 — working today)

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
  Fresh rendering with styles/numbering/tables/images and template filling;
  `toc: true` frontmatter synthesizes a static, internally linked `Contents`
  (heading titles as bookmarks + hyperlinks — no field codes, so Word shows
  no field-update/security prompt). Inline images (PNG/JPEG/GIF) embed from
  paths relative to the source file; unresolvable ones stay visible as
  `[image not embedded: …]` markers.
  Diff rendering as native Word tracked changes is implemented and tested
  (equal content unmarked, deletions `w:del`/`w:delText`, insertions
  `w:ins` at word/line/item/row granularity, anchored comments on moves
  and diagram replacements) but **parked behind no public surface** until
  bidirectional updates are designed — see the roadmap.
- **Mermaid → SVG/PNG** via [`merman`](https://github.com/Latias94/merman)
  (headless Rust Mermaid, ELK layout). Diagram sources are automatically
  forced to native SVG `<text>` labels (`htmlLabels: false`) because
  browser-less consumers (typst/usvg, rasterizers) drop `<foreignObject>`
  labels silently. Edge-label plates default to opaque white (invisible on
  white pages, masks the edge line that would otherwise strike through the
  label); `EdgeLabelBackground::Transparent` opts out for colored page
  backgrounds. merman hardcodes `opacity: 0.5` on those plates (parroting
  mermaid.js), which lets half the line bleed through — docgen pipes a
  `ScopedCssPostprocessor` through merman's SVG pipeline to restore full
  opacity in both the SVG and the raster path.
- **PDF** via embedded typst 0.15 against a custom in-memory `World`:
  fonts compiled in from `typst-assets` (Libertinus + NewCM math + DejaVu
  mono), deterministic output (`World::today` returns `None` — document
  dates come from frontmatter). `toc: true` prepends a `Contents` outline
  (typst `#outline`). Inline images (PNG/JPEG/GIF/SVG) render from paths
  relative to the source; an image alone in its paragraph becomes a centered
  figure. Per-page PNG rendering via `typst-render`.
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
- **PPTX output** (`docgen-office::pptx`): PresentationML decks from the
  same document model — title slide from frontmatter, one slide per H1/H2
  with lead/bullets/lists, tables as bordered DrawingML tables with shaded
  headers, code blocks in content-sized monospace boxes, diagrams as
  centered PNG pictures, hyperlinks as live slide rels. Output is
  byte-deterministic (same fixed-timestamp zip discipline as docx) and
  opens cleanly in PowerPoint/LibreOffice/python-pptx.
- **PPTX templates**: `docgen render doc.md --format pptx --template
  corporate.pptx` keeps the template's masters/layouts/theme and appends
  generated slides bound to its best-matching layouts via placeholders,
  so template typography and bullet styles apply. Placeholder geometry is
  parsed from the layouts (with slide-size-proportional fallbacks), and
  the template's theme minor font is stamped on body runs.
- **PPTX diffs**: PresentationML has no native revision marks, so
  `docgen diff old.md new.md --format pptx` renders the new revision with
  small colored chips on changed slides (NEW/UPDATED/MOVED), a "Removed in
  this revision" slide, and a closing revision summary.
- **Excalidraw, natively** (`docgen-excalidraw`): raw `.excalidraw` JSON
  is a first-class input — as standalone files
  (`docgen diagram scene.excalidraw -o out.svg|--format png`) and as
  ` ```excalidraw ` code blocks in markdown. A permissive serde model
  round-trips scenes losslessly (unknown fields preserved); rendering is
  fully native — roughr (seeded per element, so output is
  byte-deterministic) for the hand-drawn geometry, bundled Excalifont /
  Virgil / Liberation / Cascadia TTFs shaped into SVG paths (no font
  machinery needed downstream), data-URL images embedded, `raster`
  feature PNGs via resvg for the office backends. Diffing is
  element-id-anchored: added/removed/changed elements with the changed
  *fields* enumerated, volatile bookkeeping ignored — surfaced as a
  detailed Word comment when an externally-edited scene changes. Arrow
  `startBinding`/`endBinding` are honored at render time (`fixedPoint`,
  focus, gap against rectangle/ellipse/diamond perimeters), so scenes
  edited outside the app — elements moved without updating arrow points —
  still render with arrows attached.
- **Wasm component** (`docgen-wasm`): the whole pipeline — parse,
  mermaid + excalidraw diagrams, typst PDF, DOCX/PPTX writers — is
  exported through a WIT world (`crates/docgen-wasm/wit/docgen.wit`:
  `render-pdf`, `render-docx`, `render-pptx`, `render-diagram-svg`,
  `version`) that **imports nothing**. Build the core module
  for `wasm32-unknown-unknown` (zero host imports — merman's katex chain
  is feature-gated off for this crate only, and `getrandom` gets a
  deterministic custom backend) and componentize with
  `wasm-tools component new`; `cargo test -p docgen-wasm` asserts the
  import surface. CI (`.github/workflows/ci.yml`) runs fmt/clippy/tests,
  wasm checks on both targets, the component build, and fuzz-target
  builds; outputs are pinned by golden SHA-256 tests per crate.
- **CLI**:
  - `docgen render doc.md [-o out] [--format pdf|docx|pptx] [--template ...]`
  - `docgen diff old.md new.md [-o out] [--format pptx] [--author A] [--date D]`
  - `docgen diagram flow.mmd | scene.excalidraw [-o out.svg|--format png]`

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
| `docgen-office` | DOCX/PPTX output + templates; pptx revision decks; docx tracked-changes diff parked (tested, unexposed) | ✅ wasip1 |
| `docgen-wasm` | WIT component exporting every renderer; imports nothing | ✅ unknown-unknown |
| `docgen-cli` | the only crate that touches the filesystem | n/a |

## Release profile (measured, 2026-10)

Host binary (`docgen.exe`, all engines + fonts statically linked), one
variable at a time:

| Config | Size | Δ |
|---|---|---|
| thin LTO, `strip="symbols"` (old default) | 79,483,904 B | — |
| — `strip="none"` | 79,403,008 B | strip is a no-op on PE (~0.1%, noise) |
| fat LTO + `codegen-units=1` + strip | 73,014,272 B | −8.1% |
| + `opt-level="z"` | **54,558,720 B** | −25.3% more (−31% total) |

So the shipped profile is `lto = true`, `codegen-units = 1`,
`opt-level = "z"`, `strip = "symbols"`: `z` clears a 10%-worth-it bar with
room to spare. The size is dominated by code + embedded font assets, not
symbols — stripping was already configured and buys nothing on Windows.

**Build-time cost (important):** fat LTO + one codegen unit mean a full
release build — including the wasm release builds — takes **12–16
minutes**, almost all of it the LTO link, vs seconds for `cargo check`.
Iterate with debug builds; only build release when size is the actual
deliverable. `z` also trades runtime speed for size; revisit if typst
compile latency ever matters interactively. The wasm cdylib shares this
profile and shrinks with it.

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

**Parked:** the DOCX tracked-changes diff (`docgen-office`'s
`render_diff_docx` + `document::RevisionMode`) is implemented, unit- and
golden-tested, but has no CLI flag, no wasm export and no public library
entry point. It comes back once **bidirectional updates** are designed —
how a DOCX that was reviewed in Word (changes accepted/rejected, comments
resolved) flows back into the markdown source, instead of the current
one-way markdown → DOCX trip. Exposing the one-way output first would
risk locking in an API/UX the round trip can't honor.

- Bidirectional update design (the unexposure condition above).
- Word comment threading (`commentsExtended.xml`) and resolved states.
- `w:moveFrom`/`w:moveTo` for moves (currently: comment only — Word's move
  markup is finicky across consumers).
- Frontmatter diff coverage (title/author changes currently render
  unmarked).
- Merge split runs before placeholder substitution in templates.

### M3 — PPTX ✅ (shipped; refinements below)

- Speaker-notes diff channel: put per-slide change details into notes
  slides instead of (only) the summary slide.
- Layout-aware content overflow: shrink long bullet lists to fit instead
  of spilling to "(cont.)" slides at a fixed line budget.
- Template mode: honor per-placeholder font overrides from layout
  `lstStyle` (currently one theme-wide minor font is stamped).
- SVG images in decks (PowerPoint `svgBlip` + PNG fallback) instead of
  PNG-only, for crisper projection.

### M4 — Excalidraw ✅ (shipped; refinements below)

- Mermaid → Excalidraw conversion (`@excalidraw/mermaid-to-excalidraw`
  in the rquickjs runtime) — still the planned approach; raw JSON input
  (shipped) is deliberately independent of it.
- Text: rich text (per-span formatting), containers with auto-grow, and
  `frame` elements are not rendered yet.
- Roundness fidelity for diamonds/ellipses; freedraw pressure-variable
  stroke width.

(Implementation note: the shipped renderer uses `ab_glyph` for font
shaping rather than the originally sketched `cosmic-text` — lighter,
wasm-safe, and text becomes SVG paths either way.)

### M5 — wasm packaging & CI ✅ (shipped; refinements below)

Shipped exactly as sketched above: WIT world importing nothing (see the
status section), zero-import `wasm32-unknown-unknown` core (katex chain
feature-gated off for the wasm crate; deterministic `getrandom` custom
backend), `wasm-tools component new` componentization, import-surface
assertion tests, per-crate golden hashes, cargo-fuzz targets
(frontmatter splitter, typst emitter), and a four-job GitHub Actions
workflow (test / wasm / component / fuzz-build).

- Real host smoke-run: a tiny wasmtime runner invoking the component
  end-to-end in CI.
- Browser shell (wasm-bindgen) reusing the same libraries.
- Publish the component (registry artifact) once there is a consumer.

(Deferred with intent: mermaid → excalidraw conversion via the rquickjs
runtime — raw excalidraw JSON input covers the spec'd need.)
