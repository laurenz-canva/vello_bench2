## WebGL Benchmark

A browser-based benchmark tool for Vello Hybrid's WebGL2 renderer. Two modes:

- **Interactive** -- tweak parameters in real-time, observe FPS.
- **Benchmark** -- automated suite with warmup calibration, vsync-independent timing, and comparison reports.

## Running

Run with SIMD enabled (recommended):

```
RUSTFLAGS=-Ctarget-feature=+simd128 cargo run --release
```

Scalar (non-SIMD) build:

```
cargo run --release
```

This builds the wasm and starts a local dev server automatically.
