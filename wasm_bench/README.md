# Vello core browser benchmarks

This temporary entry point in the existing `vello_bench2` package mirrors the default
(non-`EXTENDED`) benchmarks from `sparse_strips/vello_bench` and runs them in a browser. It compares
Vello immediately before the `fearless_simd` 0.4 to 0.7 bump with the PR head, both with and without
Wasm SIMD enabled.

The four generated builds are:

- before / scalar
- before / SIMD128
- after / scalar
- after / SIMD128

Run and serve them with:

```sh
./wasm_bench/serve.sh
```

Then open <http://localhost:8081>. Use `--global` to listen on all interfaces for testing a phone or
tablet on the same network. `--build-only` and `--serve-only` are also available.

Each benchmark gets a fixed 100 ms warm-up followed by one 500 ms measurement window for each
build. There is no iteration calibration or repeated sampling.

The root package links directly to `../vello_2`. The script temporarily switches that checkout to
each revision and restores the original branch/commit when it finishes or is interrupted. It also
backs up and restores the root `Cargo.lock`, builds offline, and reuses one Cargo target directory
per SIMD mode. It refuses to start if `vello_2` has tracked or untracked changes. Override the
defaults with `VELLO_REPO`, `BEFORE_REV`, or `AFTER_REV` environment variables.
