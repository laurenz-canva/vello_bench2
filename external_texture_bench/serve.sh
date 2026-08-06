#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
TARGET=wasm32-unknown-unknown
PROFILE=release
BIND_ADDR=127.0.0.1

if [ "${1:-}" = "--global" ]; then
  BIND_ADDR=0.0.0.0
fi

cd "$ROOT"
RUSTFLAGS="-Ctarget-feature=+simd128" cargo build --target "$TARGET" --profile "$PROFILE"
mkdir -p dist
wasm-bindgen \
  --target web \
  --out-dir dist \
  --no-typescript \
  "target/$TARGET/$PROFILE/external_texture_bench.wasm"
cp web/index.html dist/index.html

echo "Serving http://$BIND_ADDR:8081"
python3 -m http.server 8081 --bind "$BIND_ADDR" --directory dist

