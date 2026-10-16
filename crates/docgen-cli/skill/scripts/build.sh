#!/usr/bin/env bash
# Build the docgen CLI binaries for this skill and drop them next to this
# script:
#   docgen.exe    x86_64-pc-windows-msvc (built on a Windows host only —
#                 MSVC does not cross-build)
#   docgen-linux  x86_64-unknown-linux-musl, static — needs a real musl C
#                 toolchain: built natively on a Linux host, or inside the
#                 default WSL distro from a Windows host (typst's `stacker`
#                 pulls `psm`, whose assembly needs musl-gcc; rust-lld is
#                 not enough).
#
# Usage (from anywhere):  bash crates/docgen-cli/skill/scripts/build.sh
# Requires a Rust toolchain on the host (and in the WSL distro for the WSL
# path). Both builds are --release with fat LTO — 12–16 minutes each on a
# cold tree (see README "Release profile"; that size win is deliberate).
set -euo pipefail
cd "$(dirname "$0")/../../../.."   # repo root (skill/scripts/ is four levels down)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

host=$(rustc -vV | awk '/host:/ {print $2}')

if [[ "$host" == *-windows-msvc ]]; then
  cargo build --release -p docgen-cli --target x86_64-pc-windows-msvc
  cp target/x86_64-pc-windows-msvc/release/docgen.exe "$SCRIPT_DIR/docgen.exe"
  echo "built $SCRIPT_DIR/docgen.exe"
fi

build_musl_wsl() {
  # Build inside the default WSL distro (its native filesystem — DrvFS is
  # slow), then copy the artifact out. Requires cargo + musl-gcc there.
  local win_root drive wsl_root tmp
  win_root=$(pwd -W 2>/dev/null || pwd)
  drive=$(printf '%s' "$win_root" | cut -c1 | tr 'A-Z' 'a-z')
  wsl_root="/mnt/$drive${win_root#?:}"
  tmp="$SCRIPT_DIR/.wsl-build.sh"
  cat > "$tmp" <<EOF
set -eu
# Non-login shells don't source the profile — rustup installs there.
. "\$HOME/.cargo/env" 2>/dev/null || export PATH="\$HOME/.cargo/bin:\$PATH"
command -v cargo >/dev/null || { echo "WSL distro has no cargo" >&2; exit 1; }
mkdir -p ~/docgen-musl && cd ~/docgen-musl
cp -f "$wsl_root/Cargo.toml" "$wsl_root/Cargo.lock" .
cp -rf "$wsl_root/crates" .
rustup target add x86_64-unknown-linux-musl >/dev/null 2>&1 || true
# Keep the WSL toolchain current enough for the dependency set (merman
# requires rustc ≥ 1.95).
rustup update stable >/dev/null 2>&1 || true
cargo build --release -p docgen-cli --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/docgen "$wsl_root/crates/docgen-cli/skill/scripts/docgen-linux"
EOF
  wsl.exe -e sh -c "sh ./crates/docgen-cli/skill/scripts/.wsl-build.sh"
  rm -f "$tmp"
}

case "$host" in
  *-linux-*)
    if [[ "$host" == x86_64-* ]]; then
      cargo build --release -p docgen-cli --target x86_64-unknown-linux-musl
      cp target/x86_64-unknown-linux-musl/release/docgen "$SCRIPT_DIR/docgen-linux"
      echo "built $SCRIPT_DIR/docgen-linux"
    else
      echo "host $host cannot build the x86_64 musl binary; run on x86_64 Linux or Windows+WSL" >&2
      exit 1
    fi
    ;;
  *-windows-msvc)
    if command -v wsl.exe >/dev/null; then
      build_musl_wsl
      echo "built $SCRIPT_DIR/docgen-linux (via WSL)"
    else
      echo "no WSL found: build the musl binary on a Linux x86_64 host" >&2
      exit 1
    fi
    ;;
  *)
    echo "host $host: build the musl binary on a Linux x86_64 host or Windows+WSL" >&2
    exit 1
    ;;
esac
