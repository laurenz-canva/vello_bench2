const METHODS = [
  { id: "browser", label: "Browser canvas.toBlob", color: "#22d3ee" },
  { id: "png", label: "png + miniz_oxide", color: "#a78bfa" },
  { id: "zlib", label: "png + zlib-rs", color: "#fbbf24" },
];

const IMAGE_KINDS = [
  { id: "flat", label: "Flat color", detail: "Opaque, highly compressible" },
  { id: "ui", label: "UI shapes", detail: "Hard edges and transparency" },
  { id: "gradient", label: "Gradient", detail: "Smooth color variation" },
  { id: "noise", label: "Photo-like noise", detail: "High entropy and difficult to compress" },
];

const html = String.raw;

export function installPngBenchmark({ encodePngDefault, loadZlibEncoder }) {
  const root = document.createElement("div");
  root.innerHTML = html`
    <button class="png-bench-launcher" type="button">PNG benchmark</button>
    <div class="png-bench-backdrop" hidden>
      <section class="png-bench-panel" role="dialog" aria-modal="true" aria-labelledby="png-bench-title">
        <header class="png-bench-header">
          <div>
            <p class="png-bench-eyebrow">Temporary benchmark</p>
            <h2 id="png-bench-title">PNG encoding performance</h2>
            <p>Browser API versus identical <code>png</code> settings with two DEFLATE backends.</p>
          </div>
          <button class="png-bench-close" type="button" aria-label="Close benchmark">×</button>
        </header>
        <div class="png-bench-toolbar">
          <label>Image size
            <select class="png-bench-size">
              <option value="512">512 × 512</option>
              <option value="1024" selected>1024 × 1024</option>
              <option value="2048">2048 × 2048</option>
            </select>
          </label>
          <label>Measured runs
            <select class="png-bench-runs">
              <option value="3">3</option>
              <option value="5" selected>5</option>
              <option value="10">10</option>
            </select>
          </label>
          <button class="png-bench-run" type="button">Run benchmark</button>
          <span class="png-bench-status">Ready</span>
        </div>
        <p class="png-bench-note">
          Each case gets one unmeasured warmup. Browser timing mirrors Canva Lightspeed local export:
          create canvas, draw the bitmap, then await <code>toBlob("image/png", 1)</code>.
        </p>
        <div class="png-bench-results">
          <div class="png-bench-empty">Run the benchmark to compare encoders.</div>
        </div>
      </section>
    </div>
  `;
  document.body.append(root);

  const launcher = root.querySelector(".png-bench-launcher");
  const backdrop = root.querySelector(".png-bench-backdrop");
  const close = root.querySelector(".png-bench-close");
  const run = root.querySelector(".png-bench-run");
  const size = root.querySelector(".png-bench-size");
  const runs = root.querySelector(".png-bench-runs");
  const status = root.querySelector(".png-bench-status");
  const results = root.querySelector(".png-bench-results");

  const setOpen = open => {
    backdrop.hidden = !open;
    document.body.classList.toggle("png-bench-open", open);
  };
  launcher.addEventListener("click", () => setOpen(true));
  close.addEventListener("click", () => setOpen(false));
  backdrop.addEventListener("click", event => {
    if (event.target === backdrop) setOpen(false);
  });
  window.addEventListener("keydown", event => {
    if (event.key === "Escape" && !backdrop.hidden) setOpen(false);
  });

  let activeRun = 0;
  let zlibEncoderPromise;
  run.addEventListener("click", async () => {
    const runId = ++activeRun;
    setBusy(true, run, size, runs);
    results.innerHTML = '<div class="png-bench-empty">Preparing deterministic images…</div>';
    let cases = [];
    try {
      const dimension = Number(size.value);
      const iterationCount = Number(runs.value);
      zlibEncoderPromise ??= loadZlibEncoder();
      const zlibEncoder = await zlibEncoderPromise;
      cases = await buildCases(dimension);
      const allResults = [];

      for (let caseIndex = 0; caseIndex < cases.length; caseIndex++) {
        if (runId !== activeRun) return;
        const image = cases[caseIndex];
        status.textContent = `${image.label} · warmup`;
        const encoders = makeEncoders(image, encodePngDefault, zlibEncoder);
        for (const method of METHODS) await encoders[method.id]();

        const samples = Object.fromEntries(METHODS.map(method => [method.id, []]));
        const byteSizes = {};
        for (let iteration = 0; iteration < iterationCount; iteration++) {
          // Rotate the order so no encoder consistently benefits from running first.
          const orderedMethods = METHODS.map((_, index) => METHODS[(index + iteration) % METHODS.length]);
          for (const method of orderedMethods) {
            status.textContent = `${image.label} · ${iteration + 1}/${iterationCount} · ${method.label}`;
            await nextFrame();
            const startedAt = performance.now();
            const byteLength = await encoders[method.id]();
            samples[method.id].push(performance.now() - startedAt);
            byteSizes[method.id] = byteLength;
          }
        }
        allResults.push(summarizeCase(image, samples, byteSizes));
        renderResults(results, allResults, dimension);
      }
      status.textContent = "Complete";
    } catch (error) {
      console.error("PNG benchmark failed", error);
      status.textContent = "Failed";
      results.innerHTML = `<div class="png-bench-error">${escapeHtml(errorMessage(error))}</div>`;
    } finally {
      for (const image of cases) image.bitmap.close();
      if (runId === activeRun) setBusy(false, run, size, runs);
    }
  });
}

function setBusy(busy, run, size, runs) {
  run.disabled = busy;
  size.disabled = busy;
  runs.disabled = busy;
  run.textContent = busy ? "Running…" : "Run benchmark";
}

async function buildCases(dimension) {
  const cases = [];
  for (const kind of IMAGE_KINDS) {
    const pixels = generatePixels(kind.id, dimension, dimension);
    const imageData = new ImageData(new Uint8ClampedArray(pixels.buffer), dimension, dimension);
    const bitmap = await createImageBitmap(imageData);
    cases.push({ ...kind, width: dimension, height: dimension, pixels, bitmap });
    await nextFrame();
  }
  return cases;
}

function makeEncoders(image, encodePngDefault, encodePngZlibRs) {
  return {
    browser: () => encodeWithBrowser(image.bitmap),
    png: () => encodePngDefault(image.pixels, image.width, image.height).byteLength,
    zlib: () => encodePngZlibRs(image.pixels, image.width, image.height).byteLength,
  };
}

async function encodeWithBrowser(bitmap) {
  // Mirrors Canva master:
  // web/src/ui/publish/menu/download/download_flow/lightspeed_export/impl/
  // lightspeed_local_exporter.ts::encodeBitmapImpl.
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const context = canvas.getContext("2d");
  if (context == null) throw new Error("Canvas 2D is unavailable");
  context.drawImage(bitmap, 0, 0);
  const blob = await new Promise(resolve => canvas.toBlob(resolve, "image/png", 1));
  if (blob == null) throw new Error("canvas.toBlob returned no PNG");
  return blob.size;
}

function generatePixels(kind, width, height) {
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
      if (kind === "flat") {
        pixels[offset] = 31;
        pixels[offset + 1] = 41;
        pixels[offset + 2] = 55;
        pixels[offset + 3] = 255;
      } else if (kind === "gradient") {
        pixels[offset] = Math.round((x / (width - 1)) * 255);
        pixels[offset + 1] = Math.round((y / (height - 1)) * 255);
        pixels[offset + 2] = Math.round(((x + y) / (width + height - 2)) * 180 + 40);
        pixels[offset + 3] = 255;
      } else if (kind === "ui") {
        const tileX = Math.floor(x / Math.max(8, width / 16));
        const tileY = Math.floor(y / Math.max(8, height / 16));
        const border = x % Math.max(8, width / 16) < 2 || y % Math.max(8, height / 16) < 2;
        const accent = (tileX + tileY * 3) % 7 === 0;
        pixels[offset] = accent ? 34 : border ? 71 : 15;
        pixels[offset + 1] = accent ? 211 : border ? 85 : 23;
        pixels[offset + 2] = accent ? 238 : border ? 105 : 42;
        pixels[offset + 3] = (tileX + tileY) % 5 === 0 ? 144 : 255;
      } else {
        // Correlated noise resembles image detail more closely than independent RGBA noise.
        const wave = 36 * Math.sin(x * 0.031) + 28 * Math.cos(y * 0.027);
        const grain = randomByte() - 128;
        const base = Math.max(0, Math.min(255, 128 + wave + grain * 0.72));
        pixels[offset] = base;
        pixels[offset + 1] = Math.max(0, Math.min(255, base + (randomByte() - 128) * 0.35));
        pixels[offset + 2] = Math.max(0, Math.min(255, 220 - base / 2 + (randomByte() - 128) * 0.3));
        pixels[offset + 3] = 255;
      }
    }
  }
  return pixels;
}

function summarizeCase(image, samples, byteSizes) {
  return {
    id: image.id,
    label: image.label,
    detail: image.detail,
    methods: METHODS.map(method => {
      const sorted = [...samples[method.id]].sort((a, b) => a - b);
      return {
        ...method,
        median: percentile(sorted, 0.5),
        low: sorted[0],
        high: sorted[sorted.length - 1],
        bytes: byteSizes[method.id],
      };
    }),
  };
}

function renderResults(container, cases, dimension) {
  const megapixels = (dimension * dimension) / 1_000_000;
  container.innerHTML = cases.map(image => {
    const fastest = Math.min(...image.methods.map(method => method.median));
    const slowest = Math.max(...image.methods.map(method => method.median));
    return html`
      <article class="png-bench-case">
        <div class="png-bench-case-title">
          <div><h3>${image.label}</h3><p>${image.detail}</p></div>
          <span>${dimension}² RGBA8</span>
        </div>
        <div class="png-bench-chart">
          ${image.methods.map(method => {
            const width = Math.max(3, (method.median / slowest) * 100);
            const speedup = method.median / fastest;
            return html`
              <div class="png-bench-row">
                <div class="png-bench-method"><i style="background:${method.color}"></i>${method.label}</div>
                <div class="png-bench-track">
                  <div class="png-bench-bar" style="width:${width}%;background:${method.color}"></div>
                </div>
                <div class="png-bench-value">
                  <strong>${formatMs(method.median)}</strong>
                  <span>${(megapixels / (method.median / 1000)).toFixed(1)} MP/s · ${formatBytes(method.bytes)}</span>
                </div>
                <div class="png-bench-range">${speedup === 1 ? "fastest" : `${speedup.toFixed(2)}×`} · ${formatMs(method.low)}–${formatMs(method.high)}</div>
              </div>`;
          }).join("")}
        </div>
      </article>`;
  }).join("");
}

function percentile(sorted, value) {
  if (sorted.length === 1) return sorted[0];
  const position = (sorted.length - 1) * value;
  const lower = Math.floor(position);
  const remainder = position - lower;
  return sorted[lower] + (sorted[lower + 1] - sorted[lower]) * remainder;
}

function formatMs(value) {
  return value < 10 ? `${value.toFixed(2)} ms` : `${value.toFixed(1)} ms`;
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
