#!/usr/bin/env bash
set -euo pipefail

DIST=dist
TARGET=wasm32-unknown-unknown

build_variant() {
  local features=$1
  local rustflags=$2
  local out_dir=$3

  echo "==> Building $out_dir..."
  RUSTFLAGS="$rustflags" cargo build --target $TARGET --profile instrument --no-default-features --features "$features"

  echo "==> Running wasm-bindgen ($out_dir)..."
  mkdir -p "$DIST/$out_dir"
  wasm-bindgen \
    --target web \
    --out-dir "$DIST/$out_dir" \
    --no-typescript \
    target/$TARGET/instrument/vello_bench2.wasm
}

# Parse filter from first argument (default: build all).
# Accepts: hybrid, cpu, simd, nosimd, hybrid-simd, hybrid-nosimd, cpu-simd, cpu-nosimd
FILTER="${1:-all}"

should_build() {
  local out_dir=$1
  [[ "$FILTER" == "all" ]] && return 0
  # Match if the filter is a substring of the variant name.
  [[ "$out_dir" == *"$FILTER"* ]] && return 0
  return 1
}

should_build hybrid-simd   && build_variant hybrid "-Ctarget-feature=+simd128" hybrid-simd
should_build hybrid-nosimd && build_variant hybrid ""                          hybrid-nosimd
should_build cpu-simd      && build_variant cpu    "-Ctarget-feature=+simd128" cpu-simd
should_build cpu-nosimd    && build_variant cpu    ""                          cpu-nosimd

cp web/index.html "$DIST/index.html"

LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || echo "<your-ip>")
echo "==> Serving at http://localhost:8080"
echo "==> On your tablet, open http://$LOCAL_IP:8080"
python3 -c "
import http.server, os

os.chdir('$DIST')

class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        super().end_headers()

http.server.HTTPServer(('0.0.0.0', 8080), Handler).serve_forever()
"
