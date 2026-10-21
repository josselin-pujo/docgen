# Vendored: ratex-unicode-font 0.1.14

Source: crates.io `ratex-unicode-font@0.1.14` (RaTeX), license MIT. The
upstream .crate ships no license text, so the standard MIT notice is added
here as `LICENSE-MIT`. Patched via `[patch.crates-io]` in the workspace
root; drop the patch entry when upstream makes the system-fonts dependency
optional.

## Local changes

- `Cargo.toml`: the `system-fonts` dependency is moved to
  `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.
- `src/lib.rs`: the `system_fonts` import and the locale-aware discovery
  block in `discover_system_font()` are `#[cfg(not(target_arch = "wasm32"))]`
  gated (marked `VENDORED PATCH` in both files). On wasm32, discovery falls
  through to the hard-coded-path/fontdb steps, which find nothing without a
  filesystem; every caller already handles the resulting `None`.

Why: `system-fonts` → web-sys → wasm-bindgen is the only thing that pulled
host imports onto docgen's import-free `wasm32-unknown-unknown` component,
and it is what forced merman's math feature (mermaid math labels) to stay
disabled for wasm builds. See the patch comment in the workspace root
`Cargo.toml`.
