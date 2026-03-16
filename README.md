## WebGL Benchmark

**[Live Site](https://laurenz-canva.github.io/vello_bench2/)**

A browser-based benchmark tool for Vello Hybrid's WebGL2 renderer. Two modes:

- **Interactive** -- tweak parameters in real-time, observe FPS.
- **Benchmark** -- automated suite with warmup calibration, vsync-independent timing, and comparison reports.

## Running

Run with SIMD enabled (recommended):

```
RUSTFLAGS=-Ctarget-feature=+simd128 cargo run -- --package vello_bench2 --release
```

Scalar (non-SIMD) build:

```
cargo run -- --package vello_bench2 -- --release
```

This builds the wasm and starts a local dev server automatically.
