#!/usr/bin/env bash
# Build the docgen Athena component and pack the docgen bundle.
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
"${ATHENA_BIN:-athena}" --pack-bundle pack-request.json --out ../../../target/docgen.bundle
