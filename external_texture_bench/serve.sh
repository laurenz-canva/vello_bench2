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
python3 -c "
import http.server, os

os.chdir('dist')

class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

http.server.ThreadingHTTPServer(('$BIND_ADDR', 8081), Handler).serve_forever()
"
