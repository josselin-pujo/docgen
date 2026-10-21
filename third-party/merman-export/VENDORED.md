# Vendored: merman-export 0.8.0-alpha.7

Source: crates.io `merman-export@0.8.0-alpha.7`, license MIT OR Apache-2.0
(both license texts kept in this directory). Patched via
`[patch.crates-io]` in the workspace root; drop the patch entry when an
upstream merman release ships an equivalent API.

## Local changes

`src/lib.rs` — added `register_raster_fonts()` plus a module-level
`FONTDB`/`EXTRA_RASTER_FONTS` pair: font bytes registered before the first
PNG/JPEG/PDF export are loaded into the shared raster font database on top
of `load_system_fonts()`. No behavior change without registration.

`Cargo.toml` — added an empty `[workspace]` table (inert for the path
dependency; lets the vendored crate build/test standalone).

Why: `load_system_fonts()` is a no-op on `wasm32-unknown-unknown`, so the
component's font database is empty and usvg silently drops every text run —
mermaid PNG exports rendered shapes without any label text. docgen registers
the typst-assets fonts (already in the binary for the PDF path) through this
API. See the patch comment in the workspace root `Cargo.toml`.

Standalone test run (workspace crates can't test path deps in place):

```bash
cargo test --manifest-path third-party/merman-export/Cargo.toml --features png
```
