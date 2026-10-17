# SBOM & license analysis — Apache-2.0 readiness

Generated 2026-10-17 against the `Cargo.lock` (v4) as of commit f5f2850 +
working tree. **Verdict: yes — the project can be licensed under Apache-2.0.**
548 packages total: 543 fully Apache-compatible permissive, 4 MPL-2.0
(acceptable weak copyleft, see below), 1 EPL-2.0 (`merman-elk-layered` — the
one real decision to make, see below), 0 unknown licenses. Bundled fonts
impose attribution duties only. Repo-level to-dos at the bottom.

## Artifacts & method

| File | What |
|---|---|
| `sbom.cdx.json` | CycloneDX 1.5 SBOM — all 548 packages with purl, license expression, SHA-256 (from `Cargo.lock`), dependency graph |
| `dependency-inventory.tsv` | flat audit table: name, version, license, verdict, scope, targets, checksum |
| `target/sbom/gen_sbom.py` | deterministic generator (sorted output, no timestamps) |

Reproduce with:

```bash
cargo metadata --format-version 1 --locked > target/sbom/meta-host.json
cargo metadata --format-version 1 --locked --filter-platform wasm32-wasip1 > target/sbom/meta-wasip1.json
cargo metadata --format-version 1 --locked --filter-platform wasm32-unknown-unknown > target/sbom/meta-wasm32.json
python target/sbom/gen_sbom.py
```

Graph resolved for all three shipped targets (host Windows, `wasm32-wasip1`,
`wasm32-unknown-unknown`); each package records which targets it reaches.
Scope: **runtime** = reachable via normal deps (447 pkgs), **build-only** =
only via build scripts (13), **dev-only** = tests/benches only (88, never
shipped). No npm/JS dependencies exist (the athena plugin's `broker.js` is
hand-rolled); the planned rquickjs runtime is **not yet** a dependency.

## The one finding: `merman-elk-layered` is EPL-2.0

> **Decision 2026-10-17: keep ELK (option 1).** The cytoscape experiment
> (option 2) was executed and reverted: `manatee` (MIT OR Apache-2.0)
> compiles for all targets and passes the full gate, but its layout
> ignores `flowchart TB`/`LR` direction for subgraph placement —
> flowcharts regress visibly (`target/elkcmp/` has the before/after
> renders; sequence output was byte-identical, class/state/er
> equivalent). The EPL-2.0 obligations are handled instead: license text
> vendored at `third-party/EPL-2.0.txt`, called out in the root `NOTICE`.
> If the flowchart regression is ever fixed upstream, option 2 removes
> the last non-permissive dependency from the tree.

- Path: `docgen-mermaid` (features `["all-diagrams","svg","layout-elk","png"]`)
  → `merman-render` → `merman-layout-elk` (MIT OR Apache-2.0) →
  **`merman-elk-layered` (EPL-2.0)** — runtime, all three targets.
- EPL-2.0 is a *weak* copyleft (file-level, like MPL): it is fine to use as an
  **unmodified dependency** of an Apache-2.0 product (EPL-2.0 §3
  "separate agreements"), but the EPL terms ride along on that component:
  include the EPL-2.0 text (the crate ships `LICENSES/EPL-2.0.txt`), state
  modifications (none — you consume the crates.io crate as-is), and accept its
  patent-grant terms. The Apache Software Foundation classifies EPL-2.0 as
  Category B ("binary dependency, don't merge source") — as a dependency this
  is workable; it is the only package in the tree that isn't plain permissive.
- Options:
  1. **Keep it** (zero code change): redistribute the EPL-2.0 text with
     binaries + note it in third-party notices. Done.
  2. **Drop `layout-elk`** from `docgen-mermaid`'s merman features: merman's
     non-ELK layout (`layout-cytoscape`, part of `merman-render`, MIT OR
     Apache-2.0) would make the entire tree permissive. **This changes
     rendered diagram layout** → golden tests need `UPDATE_GOLDEN=1` and
     outputs need re-verification in real renders. A quality decision, not a
     legal one.
  3. Ask merman upstream whether the ELK port could be dual-licensed.

## Weak copyleft, acceptable as-is: MPL-2.0 ×4

`cssparser`, `cssparser-macros`, `dtoa-short`, `selectors` — reach the
binaries via `lol_html` (BSD-3, Cloudflare's HTML rewriter) in `merman-core`.
MPL-2.0 is file-level copyleft: fine as separate dependencies alongside
Apache-2.0; list them in third-party notices.

## Dual licenses — permissive branch available (non-issues)

| Crate | Expression | Chosen branch |
|---|---|---|
| `ittapi`, `ittapi-sys` | GPL-2.0-only OR BSD-3-Clause | BSD-3-Clause — and **dev-only** (wasmtime test tooling), never shipped |
| `r-efi` | MIT OR Apache-2.0 OR LGPL-2.1-or-later | MIT — build-only |

No other package in the lockfile has a copyleft option on any branch.

## License distribution (29 distinct expressions, 548 packages)

| Count | Expression |
|---|---|
| 231 | MIT OR Apache-2.0 |
| 88 | MIT |
| 44 | Apache-2.0 OR MIT |
| 31 | Apache-2.0 WITH LLVM-exception (wasmtime/cranelift/wasm-tools stack) |
| 28 | Apache-2.0 (incl. the whole typst stack) |
| 27 | Unicode-3.0 (icu4x used by typst) |
| 26 | MIT/Apache-2.0 (informal dual = OR) |
| 25 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (rustix, wasi, wasm-*) |
| 5 | BSD-3-Clause (tiny-skia, lol_html, zstd-*) |
| 5 | Unlicense OR MIT |
| 4 | MPL-2.0 (see above) |
| 4 | Unlicense/MIT (= OR) |
| 3 | BSD-2-Clause |
| 3 | BSD-3-Clause OR Apache-2.0 |
| 3 | Zlib |
| 3 | Zlib OR Apache-2.0 OR MIT |
| 2 | Apache-2.0 OR BSL-1.0 |
| 2 | BSD-2-Clause OR Apache-2.0 OR MIT |
| 2 | GPL-2.0-only OR BSD-3-Clause (dev-only, see above) |
| 2 | MIT OR Apache-2.0 OR Zlib |
| 2 | MIT OR Zlib OR Apache-2.0 |
| 1 each | (Apache-2.0 OR MIT) AND BSD-3-Clause · (MIT OR Apache-2.0) AND Unicode-3.0 · 0BSD OR CC0-1.0 · 0BSD OR MIT OR Apache-2.0 · Apache-2.0/MIT · BSD-2 OR MIT OR Apache-2.0 · **EPL-2.0** · MIT OR Apache-2.0 OR LGPL-2.1-or-later |

All AND-combinations pair permissive licenses only. `wasmtime` 48 keeps its
LLVM exception — compatible with Apache-2.0 distribution.

## Bundled fonts — attribution duties (not blockers)

### `typst-assets` 0.15.1 (feature `fonts`; embedded in every binary, incl. wasm)

Per its NOTICE (which contains all license texts and must be redistributed
with binaries):

| Asset | License |
|---|---|
| Libertinus Serif | SIL OFL 1.1 (Reserved Font Names) |
| NewComputerModern (except NewCM10-Regular), DejaVu | GUST Font License 1.0 |
| **NewCM10-Regular.otf** | **GPL-3.0-or-later + Font Exception + Distribution Exception** — the FE keeps generated PDFs/documents out of GPL scope; the DE explicitly permits bundling with non-GPL software. Precedent: typst's own official binaries are Apache-2.0 and embed this exact font set. |
| Foxit base-14 `.pfb` (PDF core fonts) | BSD-3-Clause (PDFium/Google) |
| ICC profiles, extracted data tables | CC0-1.0 |

README currently says "NewCM OFL, DejaVu – free" — imprecise (NewCM10-Regular
is GPL+FE+DE; the bundled DejaVu copies are declared GFL in the NOTICE).
Worth correcting.

### `docgen-excalidraw` assets (see `crates/docgen-excalidraw/assets/FONTS.md`)

Excalifont (MIT), Virgil (**OFL 1.1** — the upstream `excalidraw/virgil`
repo licenses it OFL with © Ellinor Rapp, RFN "Virgil"; FONTS.md previously
said Apache-2.0 and was corrected 2026-10-17), Nunito (OFL 1.1), Cascadia
(OFL 1.1), Liberation Sans (OFL 1.1). All are converted/derived from the
woff2 subsets in the `@excalidraw/excalidraw` npm package; OFL
modified-version rules apply (keep license text with the fonts, respect
Reserved Font Names). License texts are vendored in
`crates/docgen-excalidraw/assets/LICENSES/`; the Excalifont MIT text there
is the standard template — paste the exact upstream text from
excalifont.com when reachable.

## Shipping as Apache-2.0 — done 2026-10-17

1. Root [`LICENSE`](../../LICENSE) — full Apache-2.0 text;
   `workspace.package.license` switched from `MIT OR Apache-2.0` to
   `Apache-2.0` (all member crates inherit it).
2. Root [`NOTICE`](../../NOTICE) — project notice + third-party callouts
   (EPL-2.0, MPL-2.0, both font sets, LLVM-exception note) pointing at the
   vendored texts.
3. `third-party/` — `EPL-2.0.txt` (from the merman-elk-layered crate),
   `MPL-2.0.txt` (from cssparser), `NOTICE.typst-assets` (verbatim copy,
   contains OFL/GFL/GPL+FE+DE/BSD-PDFium/CC0 texts for the embedded fonts).
4. `crates/docgen-excalidraw/assets/LICENSES/` — `OFL-1.1.txt`,
   `MIT.txt`, and a README with per-font copyright/RFN attribution;
   `FONTS.md` links to it.
5. Keep `sbom.cdx.json` + `dependency-inventory.tsv` in the repo and
   regenerate per release (three `cargo metadata` dumps + `gen_sbom.py`).

## Not covered here

CI-time tools (`wasm-tools`, cargo-fuzz/cargo-nextest, wasm-bindgen-cli) are
build infrastructure, not shipped artifacts. If a `fuzz/` target is added to
the workspace, its deps (`libfuzzer-sys`, `arbitrary` — MIT OR Apache-2.0)
stay dev-scope.
