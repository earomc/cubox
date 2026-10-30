#!/usr/bin/env bash
# Builds the web version into dist/ (or the directory given as first argument).
# Needs wasm-bindgen-cli in the version of the wasm-bindgen crate in Cargo.lock:
#   cargo install wasm-bindgen-cli --version <that version> --locked
# wasm-opt (binaryen) is used when available.
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-dist}"

locked=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
installed=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [ "$installed" != "$locked" ]; then
  echo "need wasm-bindgen-cli $locked (found: ${installed:-none}):" >&2
  echo "  cargo install wasm-bindgen-cli --version $locked --locked" >&2
  exit 1
fi

cargo build --release --locked --lib --target wasm32-unknown-unknown

rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out/pkg" \
  target/wasm32-unknown-unknown/release/cubox.wasm
# wasm-opt takes the wasm features (simd etc.) from the module, an old binaryen may not know them
if ! command -v wasm-opt >/dev/null; then
  echo "wasm-opt not found, skipping optimization" >&2
elif wasm-opt -O3 -o "$out/pkg/opt.wasm" "$out/pkg/cubox_bg.wasm"; then
  mv "$out/pkg/opt.wasm" "$out/pkg/cubox_bg.wasm"
else
  echo "wasm-opt failed, keeping the unoptimized wasm" >&2
  rm -f "$out/pkg/opt.wasm"
fi
cp web/* "$out/"
echo "built $out ($(du -h "$out/pkg/cubox_bg.wasm" | cut -f1) wasm)"
