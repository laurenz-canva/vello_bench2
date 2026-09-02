## Vello interactive playground

**[Live Site](https://laurenz-canva.github.io/vello_bench2/)**

An interactive browser playground for exploring Vello rendering scenes and their real-time performance.

## Running

Run a single SIMD-enabled build:

```sh
RUSTFLAGS=-Ctarget-feature=+simd128 cargo run -- --package vello_bench2 --release
```

Or build both SIMD and scalar variants, then serve them with the runtime toggle:

```sh
./serve.sh
```

Open http://localhost:8080. The script requires a matching `wasm-bindgen-cli` installation.

The temporary PNG benchmark is available from the button in the lower-right corner. `serve.sh`
also builds its small companion WASM module separately so the default `png`/miniz_oxide and
`png`/zlib-rs backends do not get unified by Cargo features.

Useful options:

```sh
./serve.sh --debug
./serve.sh --brotli-wasm
./serve.sh --global
```
