#!/bin/sh
set -eu

DIST=dist
TARGET=wasm32-unknown-unknown
BUILD_PROFILE=release
RUSTFLAGS_SIMD="-Ctarget-feature=+simd128"

FILTER=all
BIND_ADDR=127.0.0.1
SERVE_BROTLI_WASM=0

build_variant() {
  rustflags=$1
  out_dir=$2

  echo "==> Building $out_dir..."
  RUSTFLAGS="$rustflags" cargo build --lib --target "$TARGET" --profile "$BUILD_PROFILE"

  echo "==> Running wasm-bindgen ($out_dir)..."
  mkdir -p "$DIST/$out_dir"
  wasm-bindgen \
    --target web \
    --out-dir "$DIST/$out_dir" \
    --no-typescript \
    "target/$TARGET/$BUILD_PROFILE/vello_bench2.wasm"

  echo "==> Building PNG zlib-rs benchmark helper ($out_dir)..."
  RUSTFLAGS="$rustflags" cargo build \
    --manifest-path png_zlib_bench/Cargo.toml \
    --target "$TARGET" \
    --profile "$BUILD_PROFILE"

  echo "==> Running wasm-bindgen for PNG zlib-rs helper ($out_dir)..."
  mkdir -p "$DIST/$out_dir/png-zlib"
  wasm-bindgen \
    --target web \
    --out-dir "$DIST/$out_dir/png-zlib" \
    --no-typescript \
    "png_zlib_bench/target/$TARGET/$BUILD_PROFILE/vello_png_zlib_bench.wasm"
}

should_build() {
  out_dir=$1
  if [ "$FILTER" = all ]; then
    return 0
  fi
  case "$out_dir" in
    "$FILTER") return 0 ;;
    *) return 1 ;;
  esac
}

copy_svg_assets() {
  if ! command -v brotli >/dev/null 2>&1; then
    echo "Error: brotli is required to package compressed assets" >&2
    exit 1
  fi

  mkdir -p "$DIST/assets"
  for asset in assets/*.svg; do
    out="$DIST/assets/$(basename "$asset").br"
    if [ -f "$out" ]; then
      continue
    fi
    brotli -q 11 -c "$asset" > "$out"
  done
}

compress_wasm_assets() {
  if ! command -v brotli >/dev/null 2>&1; then
    echo "Error: brotli is required to compress Wasm assets" >&2
    exit 1
  fi

  find "$DIST" -type f -name '*.wasm' -exec sh -c 'brotli -q 11 -c "$1" > "$1.br"' _ {} \;
}

while [ $# -gt 0 ]; do
  case "$1" in
    --global)
      BIND_ADDR=0.0.0.0
      shift
      ;;
    --debug)
      BUILD_PROFILE=instrument
      shift
      ;;
    --brotli-wasm)
      SERVE_BROTLI_WASM=1
      shift
      ;;
    *)
      FILTER=$1
      shift
      ;;
  esac
done

rm -rf "$DIST/control" "$DIST/treatment"
should_build simd && build_variant "$RUSTFLAGS_SIMD" simd
should_build nosimd && build_variant "" nosimd
cp web/index.html "$DIST/index.html"
cp web/styles.css "$DIST/styles.css"
cp web/png-benchmark.js "$DIST/png-benchmark.js"

copy_svg_assets
if [ "$SERVE_BROTLI_WASM" = 1 ]; then
  echo "==> Compressing Wasm assets..."
  compress_wasm_assets
fi

echo "==> Serving at http://localhost:8080"
if [ "$BIND_ADDR" = "0.0.0.0" ]; then
  LOCAL_IP=$(python3 -c 'import socket; sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); sock.connect(("1.1.1.1", 80)); print(sock.getsockname()[0]); sock.close()' 2>/dev/null || echo "<your-ip>")
  echo "==> On your tablet, open http://$LOCAL_IP:8080"
fi
python3 -c "
import http.server, os, urllib.parse

os.chdir('$DIST')
serve_brotli_wasm = '$SERVE_BROTLI_WASM' == '1'

class Handler(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if serve_brotli_wasm:
            url_path = urllib.parse.urlparse(self.path).path
            if url_path.endswith('.wasm') and 'br' in self.headers.get('Accept-Encoding', ''):
                br_path = self.translate_path(url_path + '.br')
                if os.path.isfile(br_path):
                    self.path = url_path + '.br'
                    self._serving_brotli_wasm = True
        super().do_GET()

    def do_HEAD(self):
        if serve_brotli_wasm:
            url_path = urllib.parse.urlparse(self.path).path
            if url_path.endswith('.wasm') and 'br' in self.headers.get('Accept-Encoding', ''):
                br_path = self.translate_path(url_path + '.br')
                if os.path.isfile(br_path):
                    self.path = url_path + '.br'
                    self._serving_brotli_wasm = True
        super().do_HEAD()

    def guess_type(self, path):
        if getattr(self, '_serving_brotli_wasm', False):
            return 'application/wasm'
        return super().guess_type(path)

    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        if getattr(self, '_serving_brotli_wasm', False):
            self.send_header('Content-Encoding', 'br')
            self.send_header('Vary', 'Accept-Encoding')
        if self.path.startswith('/assets/'):
            self.send_header('Cache-Control', 'public, max-age=31536000, immutable')
        else:
            self.send_header('Cache-Control', 'no-store')
        super().end_headers()

http.server.ThreadingHTTPServer(('$BIND_ADDR', 8080), Handler).serve_forever()
"
