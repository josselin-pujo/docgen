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
  local request="$1" out="$2"
  rm -f "$out"
  "${ATHENA_BIN:-athena}" --pack-bundle "$request" --out "$out" || true
  [ -f "$out" ] || python3 pack.py "$request" --out "$out"
  # Either packer can exit 0 without writing anything (the dist binary
  # silently no-ops when the app is down) — a missing or unsealed bundle
  # must fail the build, loudly, instead of being shipped.
  python3 - "$out" <<'PY'
import sqlite3, sys
con = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
version = con.execute("PRAGMA user_version").fetchone()[0]
if version != 1:
    sys.exit(f"pack: {sys.argv[1]} has user_version={version}, expected 1")
print(f"verified {sys.argv[1]} (user_version=1)")
PY
}
pack pack-request.json ../../../target/docgen.bundle
pack companion/pack-request.json ../../../target/docgen-workspace.bundle
