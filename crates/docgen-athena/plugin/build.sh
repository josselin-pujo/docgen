#!/usr/bin/env bash
# Build the docgen Athena component and pack both plugin bundles:
#   target/docgen.bundle            foundation: blob-native renderer (no fs)
#   target/docgen-workspace.bundle  companion: filesystem path flow (deps: docgen)
#
# Usage (from anywhere):  bash crates/docgen-athena/plugin/build.sh
# ATHENA_BIN overrides the host binary used for packing (default: `athena`
# on PATH; this dev machine uses the dist desktop build).
#
# The release link is the 12-16 min step; everything else is quick.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # repo root

cargo build -p docgen-athena --release --target wasm32-unknown-unknown
wasm-tools component new \
  target/wasm32-unknown-unknown/release/docgen_athena.wasm \
  -o target/docgen-athena.component.wasm
wasm-tools validate target/docgen-athena.component.wasm

cd crates/docgen-athena/plugin
# Prefer the host athena packer; fall back to the stdlib pack.py when the
# dist desktop binary can't pack (it silently exits when the app is down).
pack() {
  "${ATHENA_BIN:-athena}" --pack-bundle "$1" --out "$2" ||     python3 pack.py "$1" --out "$2"
}
pack pack-request.json ../../../target/docgen.bundle
pack companion/pack-request.json ../../../target/docgen-workspace.bundle
