#!/usr/bin/env bash
# Installs wasm-opt from a binaryen release, newer than what Linux distributions ship (those can't
# read current wasm). Usage: install-wasm-opt.sh [directory for the wasm-opt link, default /usr/local/bin]
set -euo pipefail
version=132
dest="${1:-/usr/local/bin}"
case "$(uname -m)" in
  x86_64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) echo "no binaryen release for $(uname -m)" >&2; exit 1 ;;
esac
prefix="${BINARYEN_PREFIX:-/opt/binaryen}"
mkdir -p "$prefix" "$dest"
curl -sSfL "https://github.com/WebAssembly/binaryen/releases/download/version_$version/binaryen-version_$version-$arch-linux.tar.gz" \
  | tar -xz -C "$prefix" --strip-components=1
ln -sf "$prefix/bin/wasm-opt" "$dest/wasm-opt"
"$dest/wasm-opt" --version
