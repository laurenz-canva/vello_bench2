#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
WORK_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
VELLO_REPO=${VELLO_REPO:-"$WORK_DIR/vello_2"}
BEFORE_REV=${BEFORE_REV:-"6d915226a9d1c313df102de06e1b086bc33575d9"}
AFTER_REV=${AFTER_REV:-"0ebec804371bdf76768c2eec8642e1028b582638"}
TARGET=wasm32-unknown-unknown
DIST="$SCRIPT_DIR/dist"
STAGING="$SCRIPT_DIR/dist-staging-$$"

BUILD=1
SERVE=1
BIND_ADDR=127.0.0.1
PORT=8081
ORIGINAL_REV=
ORIGINAL_BRANCH=
CHECKOUT_CHANGED=0
LOCK_SAVED=0
LOCK_WAS_MISSING=0
LOCK_BACKUP="$STAGING/Cargo.lock.original"

usage() {
  echo "Usage: $0 [--build-only | --serve-only] [--global] [--port PORT]"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --build-only)
      SERVE=0
      ;;
    --serve-only)
      BUILD=0
      ;;
    --global)
      BIND_ADDR=0.0.0.0
      ;;
    --port)
      shift
      if [ "$#" -eq 0 ]; then
        usage >&2
        exit 2
      fi
      PORT=$1
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  shift
done

restore_checkout() {
  if [ "$CHECKOUT_CHANGED" -eq 0 ]; then
    return
  fi

  echo "==> Restoring vello_2 checkout..."
  if [ -n "$ORIGINAL_BRANCH" ]; then
    git -C "$VELLO_REPO" -c core.fsmonitor=false switch "$ORIGINAL_BRANCH"
  else
    git -C "$VELLO_REPO" -c core.fsmonitor=false switch --detach "$ORIGINAL_REV"
  fi
  CHECKOUT_CHANGED=0
}

restore_lockfile() {
  if [ "$LOCK_SAVED" -eq 1 ]; then
    cp "$LOCK_BACKUP" "$PROJECT_DIR/Cargo.lock"
    rm -f "$LOCK_BACKUP"
    LOCK_SAVED=0
  elif [ "$LOCK_WAS_MISSING" -eq 1 ]; then
    rm -f "$PROJECT_DIR/Cargo.lock"
    LOCK_WAS_MISSING=0
  fi
}

cleanup() {
  status=$?
  trap - EXIT INT TERM
  restore_checkout || true
  restore_lockfile || true
  if [ -d "$STAGING" ]; then
    rm -rf "$STAGING"
  fi
  exit "$status"
}

trap cleanup EXIT INT TERM

check_tools() {
  for tool in cargo git rustc wasm-bindgen python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      echo "Error: required tool '$tool' was not found" >&2
      exit 1
    fi
  done

  bindgen_version=$(wasm-bindgen --version | awk '{print $2}')
  if [ "$bindgen_version" != "0.2.121" ]; then
    echo "Error: wasm-bindgen-cli 0.2.121 is required; found $bindgen_version" >&2
    exit 1
  fi
}

check_checkout() {
  if [ ! -d "$VELLO_REPO/.git" ]; then
    echo "Error: VELLO_REPO is not a Git checkout: $VELLO_REPO" >&2
    exit 1
  fi

  if [ -n "$(git -C "$VELLO_REPO" -c core.fsmonitor=false status --porcelain --untracked-files=all)" ]; then
    echo "Error: $VELLO_REPO has local changes; refusing to switch revisions" >&2
    exit 1
  fi

  git -C "$VELLO_REPO" cat-file -e "$BEFORE_REV^{commit}"
  git -C "$VELLO_REPO" cat-file -e "$AFTER_REV^{commit}"
  BEFORE_REV=$(git -C "$VELLO_REPO" rev-parse "$BEFORE_REV^{commit}")
  AFTER_REV=$(git -C "$VELLO_REPO" rev-parse "$AFTER_REV^{commit}")
  ORIGINAL_REV=$(git -C "$VELLO_REPO" rev-parse HEAD)
  ORIGINAL_BRANCH=$(git -C "$VELLO_REPO" symbolic-ref --quiet --short HEAD || true)
}

build_variant() {
  revision_label=$1
  revision=$2
  variant=$3
  rustflags=$4
  target_dir="$PROJECT_DIR/target/wasm-bench-$variant"
  out_dir="$STAGING/$revision_label/$variant"

  echo "==> Building $revision_label / $variant..."
  CARGO_TARGET_DIR="$target_dir" \
  CARGO_NET_OFFLINE=true \
  RUSTFLAGS="$rustflags" \
  VELLO_BENCH_REV="$revision" \
  VELLO_BENCH_VARIANT="$variant" \
    cargo build \
      --manifest-path "$PROJECT_DIR/Cargo.toml" \
      --package vello_bench2 \
      --bin vello_wasm_bench \
      --features wasm_bench \
      --target "$TARGET" \
      --release \
      --offline

  mkdir -p "$out_dir"
  wasm-bindgen \
    --target web \
    --out-dir "$out_dir" \
    --no-typescript \
    "$target_dir/$TARGET/release/vello_wasm_bench.wasm"
}

build_revision() {
  label=$1
  revision=$2
  fearless_simd_version=$3

  echo "==> Checking out $label revision $revision..."
  git -C "$VELLO_REPO" -c core.fsmonitor=false switch --detach "$revision"
  CHECKOUT_CHANGED=1

  echo "==> Pinning fearless_simd $fearless_simd_version in the temporary lock state..."
  if [ "$fearless_simd_version" = "0.7.0" ]; then
    cp "$LOCK_BACKUP" "$PROJECT_DIR/Cargo.lock"
  else
    python3 "$SCRIPT_DIR/pin_lock.py" "$PROJECT_DIR/Cargo.lock" "$fearless_simd_version"
  fi

  build_variant "$label" "$revision" nosimd ""
  build_variant "$label" "$revision" simd "-Ctarget-feature=+simd128"
}

if [ "$BUILD" -eq 1 ]; then
  check_tools
  check_checkout

  mkdir -p "$STAGING"
  if [ -f "$PROJECT_DIR/Cargo.lock" ]; then
    cp "$PROJECT_DIR/Cargo.lock" "$LOCK_BACKUP"
    LOCK_SAVED=1
  else
    LOCK_WAS_MISSING=1
  fi
  build_revision before "$BEFORE_REV" 0.4.0
  build_revision after "$AFTER_REV" 0.7.0

  cp "$SCRIPT_DIR/web/index.html" "$STAGING/index.html"
  cp "$SCRIPT_DIR/web/app.js" "$STAGING/app.js"
  cp "$SCRIPT_DIR/web/styles.css" "$STAGING/styles.css"

  restore_checkout
  restore_lockfile

  if [ -d "$DIST" ]; then
    rm -rf "$DIST"
  fi
  mv "$STAGING" "$DIST"
  echo "==> Generated four builds in $DIST"
fi

if [ "$SERVE" -eq 1 ]; then
  if [ ! -f "$DIST/index.html" ]; then
    echo "Error: no built harness found; run without --serve-only first" >&2
    exit 1
  fi
  echo "==> Serving http://$BIND_ADDR:$PORT"
  if [ "$BIND_ADDR" = "0.0.0.0" ]; then
    echo "==> Open this machine's LAN address on the device under test, port $PORT"
  fi
  exec python3 "$SCRIPT_DIR/server.py" "$DIST" "$BIND_ADDR" "$PORT"
fi
