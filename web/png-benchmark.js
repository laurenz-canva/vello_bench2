const METHODS = [
  { id: "browser", label: "Browser canvas.toBlob", shortLabel: "canvas.toBlob", color: "#22d3ee" },
  { id: "png", label: "png + miniz_oxide", shortLabel: "png / miniz", color: "#a78bfa" },
  { id: "zlib", label: "png + zlib-rs", shortLabel: "png / zlib-rs", color: "#fbbf24" },
];

const RESOLUTIONS = [
  { id: "960x540", label: "960 × 540", width: 960, height: 540 },
  { id: "1920x1080", label: "1920 × 1080", width: 1920, height: 1080 },
  { id: "3600x2025", label: "3600 × 2025", width: 3600, height: 2025 },
];

const REAL_DESIGNS = [
  { id: "transparent", label: "Design 2", detail: "Transparent design", hasAlpha: true },
  { id: "opaque", label: "Design 3", detail: "Opaque design", hasAlpha: false },
];

const SYNTHETIC_KINDS = [
  { id: "flat", label: "Flat color", detail: "Highly compressible solid color" },
  { id: "ui", label: "UI shapes", detail: "Repeated tiles, borders, and accents" },
  { id: "gradient", label: "Gradient", detail: "Smooth color variation" },
  { id: "noise", label: "Photo-like noise", detail: "Correlated high-frequency detail" },
];

const html = String.raw;

export function installPngBenchmark({ encodePngDefault, loadZlibEncoder }) {
  const root = document.createElement("div");
  root.className = "png-bench-page-root";
  root.innerHTML = html`
    <main class="png-bench-standalone">
      <section class="png-bench-panel" aria-labelledby="png-bench-title">
        <header class="png-bench-header">
          <div>
            <p class="png-bench-eyebrow">Temporary benchmark</p>
            <h2 id="png-bench-title">PNG encoding performance</h2>
            <p>Browser API versus two <code>png</code> DEFLATE backends.</p>
          </div>
          <a class="png-bench-back" href="./">← Playground</a>
        </header>
        <div class="png-bench-toolbar">
          <label class="png-bench-workload-field">Workload
            <select class="png-bench-workload">
              <option value="designs" selected>Downloaded designs</option>
              <option value="synthetic">Synthetic matrix</option>
            </select>
          </label>
          <label>Resolution
            <select class="png-bench-resolution">
              <option value="960x540" selected>960 × 540</option>
              <option value="1920x1080">1920 × 1080</option>
              <option value="3600x2025">3600 × 2025</option>
              <option value="all">All resolutions</option>
            </select>
          </label>
          <label>Warmups
            <select class="png-bench-warmups">
              <option value="1">1 iteration</option>
              <option value="2" selected>2 iterations</option>
              <option value="3">3 iterations</option>
              <option value="5">5 iterations</option>
            </select>
          </label>
          <label>Time budget
            <select class="png-bench-budget">
              <option value="500">0.5 seconds</option>
              <option value="1000">1 second</option>
              <option value="2000" selected>2 seconds</option>
              <option value="5000">5 seconds</option>
            </select>
          </label>
          <button class="png-bench-run" type="button">Run benchmark</button>
          <span class="png-bench-status">Ready</span>
        </div>
        <p class="png-bench-note">
          Decoded once · RGB8 opaque / RGBA8 transparent · warmups excluded · sample standard deviation.
        </p>
        <div class="png-bench-results">
          <div class="png-bench-empty">Choose a workload and run the benchmark.</div>
        </div>
      </section>
    </main>
  `;
  document.body.append(root);

  const run = root.querySelector(".png-bench-run");
  const workload = root.querySelector(".png-bench-workload");
  const resolution = root.querySelector(".png-bench-resolution");
  const warmups = root.querySelector(".png-bench-warmups");
  const budget = root.querySelector(".png-bench-budget");
  const status = root.querySelector(".png-bench-status");
  const results = root.querySelector(".png-bench-results");

  let activeRun = 0;
  let zlibEncoderPromise;
  run.addEventListener("click", async () => {
    const runId = ++activeRun;
    setBusy(true, run, workload, resolution, warmups, budget);
    results.innerHTML = '<div class="png-bench-empty">Preparing benchmark cases…</div>';
    try {
      const warmupCount = Number(warmups.value);
      const timeBudgetMs = Number(budget.value);
      const definitions = buildCaseDefinitions(workload.value, resolution.value);
      zlibEncoderPromise ??= loadZlibEncoder();
      const zlibEncoder = await zlibEncoderPromise;
      const allResults = [];

      for (let caseIndex = 0; caseIndex < definitions.length; caseIndex++) {
        if (runId !== activeRun) return;
        const definition = definitions[caseIndex];
        status.textContent = `${caseIndex + 1}/${definitions.length} · ${definition.label} · preparing`;
        const image = await buildCase(definition);
        try {
          const encoders = makeEncoders(image, encodePngDefault, zlibEncoder);
          const samples = {};
          const byteSizes = {};
          // Rotate the first method between cases to reduce order bias.
          const orderedMethods = METHODS.map(
            (_, index) => METHODS[(index + caseIndex) % METHODS.length],
          );

          for (const method of orderedMethods) {
            for (let iteration = 0; iteration < warmupCount; iteration++) {
              status.textContent = `${caseIndex + 1}/${definitions.length} · ${image.label} · ${method.label} · warmup ${iteration + 1}/${warmupCount}`;
              await encoders[method.id]();
              await nextFrame();
            }

            const measurement = await measureForBudget(
              encoders[method.id],
              timeBudgetMs,
              (count, measuredMs) => {
                status.textContent = `${caseIndex + 1}/${definitions.length} · ${image.label} · ${method.label} · ${count} runs · ${(measuredMs / 1000).toFixed(1)}s`;
              },
            );
            samples[method.id] = measurement.samples;
            byteSizes[method.id] = measurement.byteLength;
          }

          allResults.push(summarizeCase(image, samples, byteSizes));
          renderResults(results, allResults);
        } finally {
          image.bitmap.close();
        }
      }
      status.textContent = `Complete · ${allResults.length} cases`;
    } catch (error) {
      console.error("PNG benchmark failed", error);
      status.textContent = "Failed";
      results.innerHTML = `<div class="png-bench-error">${escapeHtml(errorMessage(error))}</div>`;
    } finally {
      if (runId === activeRun) setBusy(false, run, workload, resolution, warmups, budget);
    }
  });
}

function setBusy(busy, run, ...controls) {
  run.disabled = busy;
  for (const control of controls) control.disabled = busy;
  run.textContent = busy ? "Running…" : "Run benchmark";
}

function buildCaseDefinitions(workload, selectedResolution) {
  const resolutions = selectedResolution === "all"
    ? RESOLUTIONS
    : RESOLUTIONS.filter(resolution => resolution.id === selectedResolution);
  if (resolutions.length === 0) throw new Error(`Unknown resolution: ${selectedResolution}`);

  if (workload === "designs") {
    return resolutions.flatMap(resolution => REAL_DESIGNS.flatMap(design => [1, 2].map(page => ({
      id: `${design.id}-${resolution.id}-page-${page}`,
      label: `${design.label} · Page ${page}`,
      detail: design.detail,
      hasAlpha: design.hasAlpha,
      width: resolution.width,
      height: resolution.height,
      sourceUrl: `./assets/png-benchmark/${design.id}/${resolution.id}-page-${page}.png`,
    }))));
  }

  if (workload === "synthetic") {
    return resolutions.flatMap(resolution => SYNTHETIC_KINDS.flatMap(kind => [false, true].map(hasAlpha => ({
      id: `${kind.id}-${hasAlpha ? "transparent" : "opaque"}-${resolution.id}`,
      label: `${kind.label} · ${hasAlpha ? "Transparent" : "Opaque"}`,
      detail: kind.detail,
      kind: kind.id,
      hasAlpha,
      width: resolution.width,
      height: resolution.height,
    }))));
  }

  throw new Error(`Unknown workload: ${workload}`);
}

async function buildCase(definition) {
  return definition.sourceUrl == null
    ? buildSyntheticCase(definition)
    : buildRealDesignCase(definition);
}

async function buildRealDesignCase(definition) {
  const response = await fetch(definition.sourceUrl, { cache: "force-cache" });
  if (!response.ok) throw new Error(`Could not load ${definition.sourceUrl}: HTTP ${response.status}`);
  const encodedSource = await response.arrayBuffer();
  const bitmap = await createImageBitmap(new Blob([encodedSource], { type: "image/png" }));

  try {
    if (bitmap.width !== definition.width || bitmap.height !== definition.height) {
      throw new Error(
        `${definition.label} is ${bitmap.width}×${bitmap.height}; expected ${definition.width}×${definition.height}`,
      );
    }
    const rgba = readBitmapPixels(bitmap);
    if (!definition.hasAlpha) assertOpaque(rgba, definition.label);
    const pixels = definition.hasAlpha ? rgba : rgbaToRgb(rgba);
    await nextFrame();
    return { ...definition, pixels, bitmap, sourceBytes: encodedSource.byteLength };
  } catch (error) {
    bitmap.close();
    throw error;
  }
}

async function buildSyntheticCase(definition) {
  const rgba = generatePixels(
    definition.kind,
    definition.width,
    definition.height,
    definition.hasAlpha,
  );
  const imageData = new ImageData(
    new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength),
    definition.width,
    definition.height,
  );
  const bitmap = await createImageBitmap(imageData);
  const pixels = definition.hasAlpha ? rgba : rgbaToRgb(rgba);
  await nextFrame();
  return { ...definition, pixels, bitmap, sourceBytes: null };
}

function readBitmapPixels(bitmap) {
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (context == null) throw new Error("Canvas 2D is unavailable");
  context.drawImage(bitmap, 0, 0);
  const imageData = context.getImageData(0, 0, bitmap.width, bitmap.height);
  const rgba = new Uint8Array(imageData.data);
  canvas.width = 0;
  canvas.height = 0;
  return rgba;
}

function assertOpaque(rgba, label) {
  for (let offset = 3; offset < rgba.length; offset += 4) {
    if (rgba[offset] !== 255) {
      throw new Error(`${label} contains transparency but is configured as RGB8`);
    }
  }
}

function rgbaToRgb(rgba) {
  const rgb = new Uint8Array((rgba.length / 4) * 3);
  for (let source = 0, target = 0; source < rgba.length; source += 4, target += 3) {
    rgb[target] = rgba[source];
    rgb[target + 1] = rgba[source + 1];
    rgb[target + 2] = rgba[source + 2];
  }
  return rgb;
}

function makeEncoders(image, encodePngDefault, encodePngZlibRs) {
  return {
    browser: () => encodeWithBrowser(image.bitmap, image.hasAlpha),
    png: () => encodePngDefault(
      image.pixels,
      image.width,
      image.height,
      image.hasAlpha,
    ).byteLength,
    zlib: () => encodePngZlibRs(
      image.pixels,
      image.width,
      image.height,
      image.hasAlpha,
    ).byteLength,
  };
}

async function encodeWithBrowser(bitmap, hasAlpha) {
  // Mirrors Canva master:
  // web/src/ui/publish/menu/download/download_flow/lightspeed_export/impl/
  // lightspeed_local_exporter.ts::encodeBitmapImpl.
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const context = canvas.getContext("2d", { alpha: hasAlpha });
  if (context == null) throw new Error("Canvas 2D is unavailable");
  context.drawImage(bitmap, 0, 0);
  const blob = await new Promise(resolve => canvas.toBlob(resolve, "image/png", 1));
  canvas.width = 0;
  canvas.height = 0;
  if (blob == null) throw new Error("canvas.toBlob returned no PNG");
  return blob.size;
}

async function measureForBudget(encode, timeBudgetMs, onProgress) {
  const samples = [];
  let measuredMs = 0;
  let lastYieldAtMs = 0;
  let byteLength = 0;

  while (measuredMs < timeBudgetMs || samples.length === 0) {
    const startedAt = performance.now();
    byteLength = await encode();
    const duration = performance.now() - startedAt;
    samples.push(duration);
    measuredMs += duration;

    // Synchronous WASM encoding can otherwise prevent mobile browsers from painting status
    // updates or handling their own housekeeping for the full benchmark duration.
    if (measuredMs - lastYieldAtMs >= 100 || measuredMs >= timeBudgetMs) {
      onProgress(samples.length, measuredMs);
      await nextFrame();
      lastYieldAtMs = measuredMs;
    }
  }

  return { samples, byteLength };
}

function generatePixels(kind, width, height, hasAlpha) {
  const pixels = new Uint8Array(width * height * 4);
  let randomState = 0x9e3779b9;
  const randomByte = () => {
    randomState ^= randomState << 13;
    randomState ^= randomState >>> 17;
    randomState ^= randomState << 5;
    return randomState & 255;
  };

  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const offset = (y * width + x) * 4;
      let alpha = 255;
      if (kind === "flat") {
        pixels[offset] = 31;
        pixels[offset + 1] = 41;
        pixels[offset + 2] = 55;
        alpha = 144;
      } else if (kind === "gradient") {
        pixels[offset] = Math.round((x / (width - 1)) * 255);
        pixels[offset + 1] = Math.round((y / (height - 1)) * 255);
        pixels[offset + 2] = Math.round(((x + y) / (width + height - 2)) * 180 + 40);
        alpha = Math.round(32 + (x / (width - 1)) * 223);
      } else if (kind === "ui") {
        const tileSize = Math.max(8, Math.round(width / 16));
        const tileX = Math.floor(x / tileSize);
        const tileY = Math.floor(y / tileSize);
        const border = x % tileSize < 2 || y % tileSize < 2;
        const accent = (tileX + tileY * 3) % 7 === 0;
        pixels[offset] = accent ? 34 : border ? 71 : 15;
        pixels[offset + 1] = accent ? 211 : border ? 85 : 23;
        pixels[offset + 2] = accent ? 238 : border ? 105 : 42;
        alpha = (tileX + tileY) % 5 === 0 ? 96 : 255;
      } else {
        // Correlated noise resembles image detail more closely than independent RGBA noise.
        const wave = 36 * Math.sin(x * 0.031) + 28 * Math.cos(y * 0.027);
        const grain = randomByte() - 128;
        const base = Math.max(0, Math.min(255, 128 + wave + grain * 0.72));
        pixels[offset] = base;
        pixels[offset + 1] = Math.max(0, Math.min(255, base + (randomByte() - 128) * 0.35));
        pixels[offset + 2] = Math.max(0, Math.min(255, 220 - base / 2 + (randomByte() - 128) * 0.3));
        alpha = 48 + (randomByte() % 208);
      }
      pixels[offset + 3] = hasAlpha ? alpha : 255;
    }
  }
  return pixels;
}

function summarizeCase(image, samples, byteSizes) {
  return {
    label: image.label,
    width: image.width,
    height: image.height,
    hasAlpha: image.hasAlpha,
    methods: METHODS.map(method => {
      const values = samples[method.id];
      const average = values.reduce((sum, value) => sum + value, 0) / values.length;
      const variance = values.length > 1
        ? values.reduce((sum, value) => sum + (value - average) ** 2, 0) / (values.length - 1)
        : 0;
      return {
        ...method,
        average,
        standardDeviation: Math.sqrt(variance),
        bytes: byteSizes[method.id],
      };
    }),
  };
}

function renderResults(container, cases) {
  container.innerHTML = cases.map(image => html`
    <div class="png-bench-table-wrap">
      <table class="png-bench-table">
        <caption>
          <strong>${image.label}</strong>
          <span>${image.width} × ${image.height} · ${image.hasAlpha ? "RGBA8" : "RGB8"}</span>
        </caption>
        <thead><tr><th>Encoder</th><th>Average</th><th>Std dev</th><th>PNG size</th></tr></thead>
          <tbody>
            ${image.methods.map(method => html`
              <tr>
                <td class="png-bench-encoder"><i style="background:${method.color}"></i>${method.shortLabel}</td>
                <td>${formatMs(method.average)}</td>
                <td>${formatMs(method.standardDeviation)}</td>
                <td>${formatBytes(method.bytes)}</td>
              </tr>`).join("")}
          </tbody>
      </table>
    </div>`).join("");
}

function formatMs(value) {
  return `${formatMsValue(value)} ms`;
}

function formatMsValue(value) {
  return value < 10 ? value.toFixed(2) : value.toFixed(1);
}

function formatBytes(value) {
  return value >= 1024 * 1024
    ? `${(value / (1024 * 1024)).toFixed(2)} MiB`
    : `${(value / 1024).toFixed(1)} KiB`;
}

function nextFrame() {
  return new Promise(resolve => requestAnimationFrame(resolve));
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function escapeHtml(value) {
  const element = document.createElement("span");
  element.textContent = value;
  return element.innerHTML;
}
