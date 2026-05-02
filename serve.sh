#!/bin/sh
set -eu

DIST=dist
TARGET=wasm32-unknown-unknown
BUILD_PROFILE=release
RUSTFLAGS_SIMD="-Ctarget-feature=+simd128"

FILTER=all
BIND_ADDR=127.0.0.1
AB_VELLO_PATH=
CONTROL_REV=
SERVE_BROTLI_WASM=0

build_variant() {
  rustflags=$1
  out_dir=$2

  echo "==> Building $out_dir..."
  RUSTFLAGS="$rustflags" cargo build --target "$TARGET" --profile "$BUILD_PROFILE"

  echo "==> Running wasm-bindgen ($out_dir)..."
  mkdir -p "$DIST/$out_dir"
  wasm-bindgen \
    --target web \
    --out-dir "$DIST/$out_dir" \
    --no-typescript \
    "target/$TARGET/$BUILD_PROFILE/vello_bench2.wasm"
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
    brotli -f -q 11 "$asset" -o "$out"
  done
}

compress_wasm_assets() {
  if ! command -v brotli >/dev/null 2>&1; then
    echo "Error: brotli is required to compress Wasm assets" >&2
    exit 1
  fi

  find "$DIST" -type f -name '*.wasm' -exec brotli -f -q 11 {} \;
}

cleanup() {
  if [ -n "$AB_VELLO_PATH" ]; then
    echo "==> Restoring Cargo.toml and Cargo.lock..."
    git checkout -- Cargo.toml Cargo.lock 2>/dev/null || true
  fi
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
    --ab)
      if [ $# -lt 2 ]; then
        echo "Error: --ab requires a path to a local Vello checkout" >&2
        exit 1
      fi
      AB_VELLO_PATH=$2
      shift 2
      ;;
    --rev)
      if [ $# -lt 2 ]; then
        echo "Error: --rev requires a value" >&2
        exit 1
      fi
      CONTROL_REV=$2
      shift 2
      ;;
    --rev=*)
      CONTROL_REV=${1#--rev=}
      shift
      ;;
    *)
      FILTER=$1
      shift
      ;;
  esac
done

if [ -n "$AB_VELLO_PATH" ]; then
  AB_VELLO_PATH=$(cd "$AB_VELLO_PATH" && pwd)

  if [ ! -f "$AB_VELLO_PATH/Cargo.toml" ]; then
    echo "Error: $AB_VELLO_PATH/Cargo.toml not found" >&2
    exit 1
  fi

  if ! grep -q '\[workspace\]' "$AB_VELLO_PATH/Cargo.toml"; then
    echo "Error: $AB_VELLO_PATH/Cargo.toml has no [workspace]" >&2
    exit 1
  fi

  PATCH_FILE=$(mktemp)
  trap 'rm -f "$PATCH_FILE"; cleanup' EXIT HUP INT TERM

  find "$AB_VELLO_PATH" -name Cargo.toml -not -path '*/target/*' -not -path '*/.git/*' | while IFS= read -r cargo_toml; do
    crate_dir=$(dirname "$cargo_toml")
    crate_name=$(grep '^name' "$cargo_toml" 2>/dev/null | head -1 | sed 's/.*= *"\(.*\)"/\1/' || true)
    case "$crate_name" in
      vello_*|glifo)
        printf '%s|%s\n' "$crate_name" "$crate_dir" >> "$PATCH_FILE"
        ;;
    esac
  done

  if [ ! -s "$PATCH_FILE" ]; then
    echo "Error: no vello_* crates or glifo found under $AB_VELLO_PATH" >&2
    exit 1
  fi

  rm -rf "$DIST/simd" "$DIST/nosimd"

  if [ -n "$CONTROL_REV" ]; then
    echo "==> Overriding control rev to $CONTROL_REV"
    sed -i.bak -E \
      "s|(git = \"https://github.com/linebender/vello\", rev = \")([^\"]+)(\")|\1${CONTROL_REV}\3|g" \
      Cargo.toml
    rm -f Cargo.toml.bak
  fi

  build_variant "$RUSTFLAGS_SIMD" control

  {
    echo ""
    echo "[patch.'https://github.com/linebender/vello']"
    while IFS='|' read -r name path; do
      path=${path%/}
      echo "$name = { path = \"$path\" }"
    done < "$PATCH_FILE"
  } >> Cargo.toml

  echo "==> Patched Cargo.toml for treatment build"
  build_variant "$RUSTFLAGS_SIMD" treatment

  cleanup
  rm -f "$PATCH_FILE"
  trap - EXIT HUP INT TERM

  cp web/index.html "$DIST/index.html"
  cp web/ab_child.html "$DIST/control/ab_child.html"
  cp web/ab_child.html "$DIST/treatment/ab_child.html"
  cp web/index.html "$DIST/control/index.html"
  cp web/index.html "$DIST/treatment/index.html"
else
  rm -rf "$DIST/control" "$DIST/treatment"
  should_build simd && build_variant "-Ctarget-feature=+simd128" simd
  should_build nosimd && build_variant "" nosimd
  cp web/index.html "$DIST/index.html"
fi

copy_svg_assets
if [ "$SERVE_BROTLI_WASM" = 1 ]; then
  echo "==> Compressing Wasm assets..."
  compress_wasm_assets
fi

echo "==> Serving at http://localhost:8080"
if [ "$BIND_ADDR" = "0.0.0.0" ]; then
  LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || echo "<your-ip>")
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
