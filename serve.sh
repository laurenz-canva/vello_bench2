#!/usr/bin/env bash
set -euo pipefail

DIST=dist

echo "==> Building SIMD variant..."
RUSTFLAGS="-Ctarget-feature=+simd128" cargo build --target wasm32-unknown-unknown --release

echo "==> Running wasm-bindgen (SIMD)..."
mkdir -p "$DIST/simd"
wasm-bindgen \
  --target web \
  --out-dir "$DIST/simd" \
  --no-typescript \
  target/wasm32-unknown-unknown/release/vello_bench2.wasm

echo "==> Building no-SIMD variant..."
RUSTFLAGS="" cargo build --target wasm32-unknown-unknown --release

echo "==> Running wasm-bindgen (no-SIMD)..."
mkdir -p "$DIST/nosimd"
wasm-bindgen \
  --target web \
  --out-dir "$DIST/nosimd" \
  --no-typescript \
  target/wasm32-unknown-unknown/release/vello_bench2.wasm

cp web/index.html "$DIST/index.html"

echo "==> Serving at http://localhost:8080"
python3 -m http.server 8080 --directory "$DIST"
