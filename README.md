## WebGL Benchmark

**[Live Site](https://laurenz-canva.github.io/vello_bench2/)**

A browser-based benchmark tool for Vello Hybrid's WebGL2 renderer. Two modes:

- **Interactive** -- tweak parameters in real-time, observe FPS.
- **Benchmark** -- automated suite with rAF-based measurement, 3 warmup frames, 15 measured frames, and comparison reports.

## Running

### Quick (single build)

Run with SIMD enabled (recommended):

```
RUSTFLAGS=-Ctarget-feature=+simd128 cargo run -- --package vello_bench2 --release
```

Scalar (non-SIMD) build:

```
cargo run -- --package vello_bench2 --release
```

### Full local server (SIMD toggle)

Builds both SIMD and non-SIMD variants and serves them with a toggle button in the top bar:

```
./serve.sh
```

Then open http://localhost:8080. Requires `wasm-bindgen-cli` (`cargo install wasm-bindgen-cli --version 0.2.114`).

### A/B testing a local Vello branch

Use the same server script, but pass a local Vello checkout:

```
./serve.sh --ab ~/repos/vello
```

This:

1. Builds the **control** variant using the git revision pinned in `Cargo.toml`.
2. Temporarily patches `Cargo.toml` to point at your local Vello checkout and builds the **treatment** variant. `Cargo.toml` and `Cargo.lock` are restored automatically afterwards.
3. Serves the normal dashboard at http://localhost:8080, with integrated A/B support enabled in the benchmark view.

In the browser:

1. Open `Benchmark`.
2. Select the benchmarks you want.
3. Click `Run A/B`.

Each selected benchmark is run in an interleaved order:

1. `control`
2. `treatment`
3. then the next benchmark

The active benchmark canvas is shown on screen while each side runs, so the A/B harness uses the same visible, rAF-driven benchmark approach as the normal benchmark page.

Use `--rev` to override the control git revision without editing `Cargo.toml`:

```
./serve.sh --ab ~/repos/vello --rev abc123def
```

Use `--global` to bind to `0.0.0.0` for testing on another device over the local network:

```
./serve.sh --ab ~/repos/vello --rev abc123def --global
```

A/B mode always builds with SIMD enabled. Use plain `./serve.sh` if you want the normal SIMD/non-SIMD toggle instead.
