# Vello external texture run benchmark

A small, independent browser benchmark comparing Vello Hybrid's WebGL2 external-texture path with its regular image-paint path.

The benchmark answers how steady-state rendering changes with:

- the number of externally bound images/external-texture runs;
- the number of resident textures those images alternate across;
- a texture allocation dimension selected before each run.

Both strategies draw the same tiny rectangles from byte-identical premultiplied images. The images use the deterministic translucent radial-wave generator from `vello_bench2`'s image-paint rectangle workload (pixel-for-pixel identical at its native 64×64 size). The full image is scaled into a 4×4 destination by default, keeping fragment work small.

## Measurement boundaries

The following work is excluded from measured samples:

- deterministic CPU image generation;
- WebGL texture allocation and upload;
- creation of `WebGlTextureBindings`;
- initial renderer and texture warmup.

The benchmark accepts one allocation size and one resident-texture count per run. It prepares that texture pool one texture per animation frame. Logical images cycle through the pool; because the pool must contain at least two textures, adjacent images have different `TextureId`s and remain separate external-texture runs. To compare allocation sizes or resident-set sizes, run the benchmark separately for each configuration.

Each warmup or measured `requestAnimationFrame` callback submits exactly one Vello render. There is no calibration, repetition multiplier, or hidden workload amplification. The frame interval therefore represents one render of the current scene.

For each image-count variant, warmup frames run once for image paint and once for external textures. Both warmup blocks finish before any measurements; all requested paired measurements then run consecutively without another warmup.

Rectangles use `vello_bench2`'s seeded position/velocity model, speed scaling, and boundary bounce behavior. As in `vello_bench2`, each animated frame records the current scene before rendering. Scene recording therefore contributes to frame intervals, while the CPU-submit column times only `renderer.render`.

Each result row pairs image paint and external texture for the same image count and measurement number. Deltas are `external texture - image paint`; positive time deltas mean external textures are slower. Pair order alternates across repeated measurements to reduce systematic ordering bias.

The page pauses timing while hidden and rejects large interruption intervals. Multiple measurements are run for every variant.

## Build and serve

Requirements:

- the `wasm32-unknown-unknown` Rust target;
- `wasm-bindgen-cli` version compatible with the manifest;
- Python 3 for the small static server.

Build and serve locally:

```sh
sh serve.sh
```

Open from another device on the network:

```sh
sh serve.sh --global
```

Then open port `8081` on the host machine.

The default run uses `1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000` images alternating across two 64×64 textures. A configurable memory limit rejects unsafe configurations before allocating them.

## Interpretation

FPS and frame intervals are capped by the display refresh rate for workloads that comfortably finish within one refresh period. They become useful once a variant starts missing frames. The CPU-submit measurement can show smaller changes below that threshold, but it does not wait for asynchronous GPU completion.

The external measurements include Vello scheduling, WebGL texture binding, and the corresponding tiny draw. This is not a direct timer around `gl.bindTexture`, because WebGL is asynchronous and Vello intentionally treats binding and drawing as one external-texture run.

Texture size is expected to be mostly irrelevant to the intrinsic binding operation. Changes across allocation sizes can reveal device-specific descriptor, residency, memory-pressure, or cache behavior. Texture creation and upload cost are intentionally not represented.
