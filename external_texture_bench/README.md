# Vello external texture run benchmark

A small, independent browser benchmark comparing Vello Hybrid's WebGL2 external-texture path with its regular image-paint path.

The benchmark answers how steady-state rendering changes with:

- the number of externally bound images/external-texture runs;
- the number of resident textures those images alternate across;
- a texture allocation dimension selected before each run.

Both strategies draw the same tiny rectangles from byte-identical premultiplied images. The images use the deterministic translucent radial-wave generator from `vello_bench2`'s image-paint rectangle workload (pixel-for-pixel identical at its native 64×64 size). By default, the generalized generator creates 16×16 images and draws them into 4×4 destination rectangles.

## Measurement boundaries

The following work is excluded from measured samples:

- deterministic CPU image generation;
- WebGL texture allocation and upload;
- creation of `WebGlTextureBindings`;
- initial renderer and texture warmup.

The benchmark accepts one allocation size and one resident-texture count per run. It prepares that texture pool one texture per animation frame. Logical images cycle through the pool; because the pool must contain at least two textures, adjacent images have different `TextureId`s and remain separate external-texture runs. To compare allocation sizes or resident-set sizes, run the benchmark separately for each configuration.

Each warmup or measured `requestAnimationFrame` callback submits exactly one Vello render. There is no calibration, repetition multiplier, or hidden workload amplification. The frame interval therefore represents one render of the current scene.

The benchmark runs every image-paint variant in ascending rect-count order, then runs the matching external-texture variants in the same order. Each variant gets a 0.5-second warmup immediately before its single timed run. Warmup and measurements are defined by elapsed wall-clock time rather than frame count, with a default measurement duration of one second per strategy.

Square images use `vello_bench2`'s seeded position/velocity model, speed scaling, and boundary bounce behavior. As in `vello_bench2`, each animated frame records the current scene before rendering, so scene recording contributes to the measured frame rate.

Each result row contains one rect count, image-paint FPS, external-texture FPS, and the external-texture FPS difference. Each variant runs exactly once. A difference of at least +10% is green and at most -10% is red. Once either strategy averages less than 20 FPS, the benchmark records that result and skips all higher rect counts. If the image-paint pass reaches the cutoff first, the external-texture pass only runs the rect counts reached by image paint.

The page pauses timing while hidden and rejects large interruption intervals.

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

The default run uses `1, 10, 50, 100, 200, 300, 450, 600, 800, 1000, 1400, 1800, 2400, 3000, 4000, 5000, 6000, 8000` rects alternating across two 16×16 images, drawn at 4×4 pixels. The benchmark allocates exactly the configured source-image pool for both strategies, with no separate texture-memory limit.

Interactive mode renders continuously and exposes live controls for image source, rect count, number of source images, image size, and rect size. Changing the image size or source-image pool destroys the old paired resources and prepares a new pool before rendering resumes.

## Interpretation

FPS is capped by the display refresh rate for workloads that comfortably finish within one refresh period. It becomes useful once a variant starts missing frames.

The external measurements include Vello scheduling, WebGL texture binding, and the corresponding tiny draw. This is not a direct timer around `gl.bindTexture`, because WebGL is asynchronous and Vello intentionally treats binding and drawing as one external-texture run.

Texture size is expected to be mostly irrelevant to the intrinsic binding operation. Changes across allocation sizes can reveal device-specific descriptor, residency, memory-pressure, or cache behavior. Texture creation and upload cost are intentionally not represented.
