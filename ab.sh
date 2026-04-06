#!/usr/bin/env bash
#
# A/B benchmark: build both the pinned ("control") and a local ("treatment")
# version of Vello, then serve them side-by-side with a one-click toggle.
#
#   ./ab.sh ~/repos/vello            # build & serve
#   ./ab.sh ~/repos/vello --global   # bind to 0.0.0.0
#
set -euo pipefail

DIST=dist
TARGET=wasm32-unknown-unknown
RUSTFLAGS_SIMD="-Ctarget-feature=+simd128"

# ── Parse arguments ───────────────────────────────────────────────────────────

VELLO_PATH=""
BIND_ADDR="127.0.0.1"

for arg in "$@"; do
  case "$arg" in
    --global) BIND_ADDR="0.0.0.0" ;;
    *)
      if [[ -z "$VELLO_PATH" ]]; then
        VELLO_PATH="$arg"
      else
        echo "Error: unexpected argument '$arg'" >&2
        exit 1
      fi
      ;;
  esac
done

if [[ -z "$VELLO_PATH" ]]; then
  echo "Usage: $0 <path-to-local-vello> [--global]" >&2
  exit 1
fi

VELLO_PATH=$(cd "$VELLO_PATH" && pwd)

# ── Validate the local Vello checkout ─────────────────────────────────────────

if [[ ! -f "$VELLO_PATH/Cargo.toml" ]]; then
  echo "Error: $VELLO_PATH/Cargo.toml not found" >&2
  exit 1
fi

if ! grep -q '\[workspace\]' "$VELLO_PATH/Cargo.toml"; then
  echo "Error: $VELLO_PATH/Cargo.toml has no [workspace] — is this a Vello checkout?" >&2
  exit 1
fi

# Discover all vello_* crates anywhere in the workspace tree.
PATCH_CRATES=()
while IFS= read -r cargo_toml; do
  crate_dir=$(dirname "$cargo_toml")
  # `grep` may find no match (e.g. workspace-only Cargo.toml); don't let
  # pipefail kill the script.
  crate_name=$(grep '^name' "$cargo_toml" 2>/dev/null | head -1 | sed 's/.*= *"\(.*\)"/\1/' || true)
  if [[ -n "$crate_name" && "$crate_name" == vello_* ]]; then
    PATCH_CRATES+=("$crate_name|$crate_dir")
  fi
done < <(find "$VELLO_PATH" -name Cargo.toml -not -path '*/target/*' -not -path '*/.git/*')

if [[ ${#PATCH_CRATES[@]} -eq 0 ]]; then
  echo "Error: no vello_* crates found under $VELLO_PATH" >&2
  exit 1
fi

echo "==> Found ${#PATCH_CRATES[@]} vello crates to patch:"
for entry in "${PATCH_CRATES[@]}"; do
  echo "    ${entry%%|*} -> ${entry#*|}"
done

# ── Cleanup handler ───────────────────────────────────────────────────────────

cleanup() {
  echo "==> Restoring Cargo.toml and Cargo.lock..."
  git checkout -- Cargo.toml Cargo.lock 2>/dev/null || true
}
trap cleanup EXIT

# ── Build helpers ─────────────────────────────────────────────────────────────

build_variant() {
  local out_dir=$1

  echo "==> Building $out_dir..."
  RUSTFLAGS="$RUSTFLAGS_SIMD" cargo build --target $TARGET --profile instrument

  echo "==> Running wasm-bindgen ($out_dir)..."
  mkdir -p "$DIST/$out_dir"
  wasm-bindgen \
    --target web \
    --out-dir "$DIST/$out_dir" \
    --no-typescript \
    target/$TARGET/instrument/vello_bench2.wasm
}

# Remove stale serve.sh directories so index.html detects ab.sh mode correctly.
rm -rf "$DIST/simd" "$DIST/nosimd"

# ── Build control (current Cargo.toml, pinned git rev) ───────────────────────

build_variant control

# ── Patch Cargo.toml for treatment ────────────────────────────────────────────

{
  echo ""
  echo "[patch.'https://github.com/linebender/vello']"
  for entry in "${PATCH_CRATES[@]}"; do
    name="${entry%%|*}"
    path="${entry#*|}"
    # Remove trailing slash for cleanliness.
    path="${path%/}"
    echo "$name = { path = \"$path\" }"
  done
} >> Cargo.toml

echo "==> Patched Cargo.toml for treatment build"

# ── Build treatment (local Vello) ─────────────────────────────────────────────

build_variant treatment

# ── Restore Cargo.toml (trap will also fire, but be explicit) ─────────────────

cleanup
trap - EXIT

# ── Copy HTML pages ───────────────────────────────────────────────────────────

# Orchestrator page (interleaved A/B) becomes the landing page.
cp web/ab.html "$DIST/index.html"

# Worker pages loaded by the orchestrator's iframes.
cp web/worker.html "$DIST/control/worker.html"
cp web/worker.html "$DIST/treatment/worker.html"

# Also copy the original index.html into each variant for standalone use.
cp web/index.html "$DIST/control/index.html"
cp web/index.html "$DIST/treatment/index.html"

# ── Serve ─────────────────────────────────────────────────────────────────────

echo "==> Serving at http://localhost:8080"
if [[ "$BIND_ADDR" == "0.0.0.0" ]]; then
  LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || echo "<your-ip>")
  echo "==> On your tablet, open http://$LOCAL_IP:8080"
fi
python3 -c "
import http.server, os

os.chdir('$DIST')

class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

http.server.HTTPServer(('$BIND_ADDR', 8080), Handler).serve_forever()
"
