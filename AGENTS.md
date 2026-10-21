# AGENTS.md — working in this repo

docgen: markdown + YAML frontmatter → PDF (embedded typst), DOCX/PPTX
(hand-rolled OOXML), mermaid + excalidraw rendered natively, structural
diff. Rust workspace, edition 2024, Windows host (Git Bash). The README is
the full story — architecture, measured import surfaces, roadmap. This
file is the operating manual: read it before building anything.

## Timing: do not burn 15 minutes by accident

`[profile.release]` is deliberately fat-LTO + `codegen-units=1` +
`opt-level=z` (measured −31% size, table in README "Release profile").

- **A full `cargo build --release` takes 12–16 minutes** — almost all of
  it the fat-LTO link. This includes `-p docgen-wasm --release` and the
  wasm-target builds in CI. Profile changes rebuild everything from zero.
- The iteration loop is **debug**: `cargo check`, `cargo clippy`, and
  `cargo run -p docgen-cli -- …` smoke tests. Never put a release build
  inside a debug loop, and never weaken the profile to "fix" slow release
  builds — the size win is the point.
- Release builds are for when size (or the wasm import surface) is the
  actual deliverable.

## Verify before declaring done

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace        # the real gate; a few minutes
```

CI (`.github/workflows/ci.yml`) runs exactly these plus wasm checks on
`wasm32-wasip1` + `wasm32-unknown-unknown`, the `wasm-tools` component
build, and cargo-fuzz target builds.

### Golden tests (byte-stability is a feature)

Every renderer crate (typst, mermaid, excalidraw, office) pins outputs by
SHA-256 (`tests/golden/`, plus an in-crate one in
`docgen-office/src/docx/mod.rs` for the parked diff). A hash mismatch
without an intentional output change is a **regression**. If you meant to
change output: `UPDATE_GOLDEN=1 cargo test -p <crate>`, and state that
output changed in your summary/commit.

### Office output gets checked by a human, in real Office

The user verifies decks and documents in the installed PowerPoint / Word /
LibreOffice — tests passing does not equal visually correct. When touching
OOXML emission, generate sample outputs (into `target/`, or `examples/`
when asked) and say what needs manual checking. Local render check:
`soffice --headless --convert-to pdf`; a PyMuPDF + python-pptx venv lives
at `/tmp/pdfvenv` (Git-Bash `/tmp` paths are invisible to Windows Python —
pass Windows paths).

## Hard rules (breaking these breaks the product)

1. **No I/O in library crates.** `&str`/`&[u8]` in, owned bytes out. Only
   `docgen-cli` touches the filesystem.
2. **No ambient state.** No clocks, no randomness, no env — outputs are
   byte-deterministic (that's what makes diffing and golden tests
   meaningful). Zip entries carry fixed timestamps (`1980-01-01`).
3. **Escape at every markup boundary** (typst lowering, XML emission).
   Author text can never change markup structure.
4. **Engines behind traits** (`DiagramRenderer`). The planned JS runtime is
   rquickjs — the user's own spike already cross-compiled it to wasm; take
   that as proven, don't re-litigate.
5. **Library crates must keep building for `wasm32-wasip1` and
   `wasm32-unknown-unknown`.** Mermaid's `math` feature is the one thing
   that drags wasm-bindgen host imports: `docgen-mermaid` default-features
   must stay off in the wasm path — and `default-features = false` has to
   be set on the **workspace-root** `[workspace.dependencies]` entry or it
   silently doesn't apply. `docgen-wasm` registers a deterministic
   `getrandom` stub (`custom` feature) — that's how the module stays
   import-free; `tests/imports.rs` asserts it (artifact-driven: builds the
   cdylib first or the tests skip).

## Parked: the DOCX diff (2026-10 decision)

`render_diff_docx` + `document::RevisionMode` are implemented, tested and
golden-pinned, but **deliberately unexposed**: `pub(crate)` +
`allow(dead_code)` (so CI's wasm checks still compile it), no CLI flag
(`docgen diff` is pptx-only; `--format docx` errors with a "parked"
message), no WIT export. Do not re-expose or "wire it up" — the un-park
condition is a designed **bidirectional update** story (how a
Word-reviewed DOCX flows back into the markdown source); that needs
brainstorming with the user first. `RevisionIdentity` stays `pub` because
the pptx diff API takes it. Details: README roadmap, M2 section.

## Excalidraw: raw JSON is first-class

Every excalidraw capability must work end-to-end from raw `.excalidraw`
JSON (standalone files and fenced ` ```excalidraw ` blocks), independent
of any future mermaid→excalidraw conversion. That conversion (rquickjs +
`mermaid-to-excalidraw`) is deliberately on the back burner — don't
promote it on your own initiative.

## Version/API gotchas (differ from common training data)

- **quick-xml 0.42**: attributes via `push_attribute((k, v))`; `Writer`
  has no `write_all` (use `Event::Text`); bind
  `let local = e.local_name(); let name = local.as_ref();` — a standalone
  `.as_ref()` let drops the temporary; string readers' attribute values
  are already `Cow<str>` (no decode step).
- **DrawingML (pptx)**: `b`/`i`/`sz`/etc. are **attributes** of `a:rPr`,
  never children; child order fill → `a:latin` → `a:hlinkClick`.
  graphicFrames take `p:xfrm`, shapes take `a:xfrm` (mixing them up is
  what triggered PowerPoint's repair dialog). Tables need `<a:tblGrid>`
  around `a:gridCol`s; placeholder shapes must not carry `txBox="1"`.
  LibreOffice leaks fonts across runs — stamp an explicit font on every
  run.
- **DOCX rows**: every `w:tr` gets `w:cantSplit` (LibreOffice renders a
  phantom bordered row when a `w:del` row sits at a page split);
  `w:trPr` children in schema order: cantSplit → tblHeader → ins/del.
- **OOXML rel ids**: rId100–105 reserved for fixed parts; media starts at
  rId200 (a collision made drawings resolve to styles.xml → blank
  images).
- **merman**: prepend `%%{init: {"htmlLabels": false}}%%` (rasterizers
  drop `<foreignObject>` labels); its CSS hardcodes edge-label plate
  opacity 0.5 — fixed via a scoped-CSS postprocessor, see README M1 notes.
  `mermaid math` (`$$…$$` labels) renders through HTML too, so it stays
  literal text in docgen; true math needs a browserless lowering upstream.
- **vendored crates** (`third-party/` + `[patch.crates-io]`, each with a
  `VENDORED.md`): `merman-export` (`register_raster_fonts` — without it
  the wasm raster fontdb is empty and mermaid PNG labels are silently
  dropped; docgen-mermaid seeds the typst-assets set before the first
  PNG) and `ratex-unicode-font` (system-fonts/web-sys gated off for
  wasm32 — what made the katex chain import-free). Drop both when
  upstream ships equivalents; keep their license texts when updating.
- **pulldown-cmark 0.13**: header rows are `Tag::TableHead`.
- **typst 0.15**: `Library` at crate root (+`LibraryExt`), `Source::new`
  needs a real `FileId`, `typst-layout` provides `PagedDocument`.
- **docgen-core**: derived `PartialEq` on `BlockKind` compares content —
  variant-only checks need a `std::mem::discriminant`-based
  `same_variant()`.
- **edition 2024**: e.g. `#[unsafe(no_mangle)]`.
- **wasmparser/wasmtime**: keep wasmtime at 48 (49 needs rustc ≥ 1.96).

## Conventions

- Commit subjects: short imperative, area-prefixed (`pptx: …`,
  `excalidraw: …`, `M5: …`). Don't commit unless asked.
- Example artifacts live in `examples/`; scratch outputs go to `target/`.
