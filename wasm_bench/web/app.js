const buildSpecs = [
  ["before_nosimd", "./before/nosimd/vello_wasm_bench.js"],
  ["after_nosimd", "./after/nosimd/vello_wasm_bench.js"],
  ["before_simd", "./before/simd/vello_wasm_bench.js"],
  ["after_simd", "./after/simd/vello_wasm_bench.js"],
];

const builds = new Map();
const results = new Map();
let names = [];
let stopped = false;

const $ = (selector) => document.querySelector(selector);
const status = $("#status");
const tbody = $("#results");

function supportsWasmSimd() {
  return WebAssembly.validate(new Uint8Array([
    0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123,
    3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11,
  ]));
}

function formatNs(value) {
  if (value == null) return "—";
  if (value < 1_000) return `${value.toFixed(1)} ns`;
  if (value < 1_000_000) return `${(value / 1_000).toFixed(2)} µs`;
  return `${(value / 1_000_000).toFixed(2)} ms`;
}

function delta(before, after) {
  if (!before || !after) return null;
  return (after.median_ns / before.median_ns - 1) * 100;
}

function deltaCell(before, after) {
  const value = delta(before, after);
  if (value == null) return `<td class="pending">—</td>`;
  const className = value <= 0 ? "faster" : "slower";
  return `<td class="${className}">${value > 0 ? "+" : ""}${value.toFixed(1)}%</td>`;
}

function renderRow(index) {
  const row = tbody.children[index];
  const data = results.get(names[index]) ?? {};
  row.innerHTML = `
    <td>${names[index]}</td>
    <td>${formatNs(data.before_nosimd?.median_ns)}</td>
    <td>${formatNs(data.after_nosimd?.median_ns)}</td>
    ${deltaCell(data.before_nosimd, data.after_nosimd)}
    <td>${formatNs(data.before_simd?.median_ns)}</td>
    <td>${formatNs(data.after_simd?.median_ns)}</td>
    ${deltaCell(data.before_simd, data.after_simd)}
  `;
}

function createRows() {
  tbody.replaceChildren();
  names.forEach((name, index) => {
    const row = document.createElement("tr");
    row.innerHTML = `<td>${name}</td>${'<td class="pending">—</td>'.repeat(6)}`;
    row.addEventListener("dblclick", () => runOne(index));
    tbody.append(row);
  });
}

function nextPaint() {
  return new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
}

async function loadBuild(key, path) {
  const module = await import(path);
  await module.default();
  builds.set(key, { module, info: JSON.parse(module.build_info()) });
}

async function initialize() {
  const simd = supportsWasmSimd();
  $("#simd-support").textContent = simd ? "supported" : "not supported";
  $("#device").textContent = navigator.userAgent;
  $("#cores").textContent = navigator.hardwareConcurrency ?? "unknown";

  const specs = simd ? buildSpecs : buildSpecs.slice(0, 2);
  await Promise.all(specs.map(([key, path]) => loadBuild(key, path)));

  const manifests = [...builds.values()].map(({ module }) => JSON.parse(module.benchmark_names()));
  names = manifests[0];
  if (!manifests.every((manifest) => JSON.stringify(manifest) === JSON.stringify(names))) {
    throw new Error("The four builds expose different benchmark manifests");
  }

  const before = builds.get("before_nosimd").info.revision.slice(0, 9);
  const after = builds.get("after_nosimd").info.revision.slice(0, 9);
  $("#revisions").textContent = `${before} → ${after}`;
  createRows();
  status.textContent = simd ? "Ready — four builds loaded" : "Ready — scalar builds only";
  status.className = "status ready";
  $("#run").disabled = false;
}

async function runOne(index) {
  const row = tbody.children[index];
  row.classList.add("running");

  for (const [key] of buildSpecs) {
    if (stopped || !builds.has(key)) break;
    status.textContent = `${names[index]} — ${key.replace("_", " / ")}`;
    await nextPaint();
    const { module } = builds.get(key);
    const result = JSON.parse(module.run_benchmark(index));
    const byBuild = results.get(names[index]) ?? {};
    byBuild[key] = result;
    results.set(names[index], byBuild);
    renderRow(index);
  }

  row.classList.remove("running");
}

async function runAll() {
  stopped = false;
  $("#run").disabled = true;
  $("#stop").disabled = false;
  for (let index = 0; index < names.length && !stopped; index += 1) {
    await runOne(index);
    $("#progress").style.width = `${((index + 1) / names.length) * 100}%`;
  }
  status.textContent = stopped ? "Stopped" : "Complete";
  status.className = "status ready";
  $("#run").disabled = false;
  $("#stop").disabled = true;
  $("#csv").disabled = results.size === 0;
  $("#json").disabled = results.size === 0;
}

function download(name, type, content) {
  const link = document.createElement("a");
  link.href = URL.createObjectURL(new Blob([content], { type }));
  link.download = name;
  link.click();
  URL.revokeObjectURL(link.href);
}

function exportJson() {
  download("vello-wasm-bench.json", "application/json", JSON.stringify({
    userAgent: navigator.userAgent,
    hardwareConcurrency: navigator.hardwareConcurrency,
    builds: Object.fromEntries([...builds].map(([key, value]) => [key, value.info])),
    results: Object.fromEntries(results),
  }, null, 2));
}

function exportCsv() {
  const header = ["benchmark", "before_scalar_ns", "after_scalar_ns", "scalar_delta_percent", "before_simd_ns", "after_simd_ns", "simd_delta_percent"];
  const rows = names.map((name) => {
    const data = results.get(name) ?? {};
    return [
      name,
      data.before_nosimd?.median_ns ?? "",
      data.after_nosimd?.median_ns ?? "",
      delta(data.before_nosimd, data.after_nosimd) ?? "",
      data.before_simd?.median_ns ?? "",
      data.after_simd?.median_ns ?? "",
      delta(data.before_simd, data.after_simd) ?? "",
    ];
  });
  const csv = [header, ...rows].map((row) => row.map((cell) => JSON.stringify(cell)).join(",")).join("\n");
  download("vello-wasm-bench.csv", "text/csv", csv);
}

$("#run").addEventListener("click", runAll);
$("#stop").addEventListener("click", () => { stopped = true; });
$("#csv").addEventListener("click", exportCsv);
$("#json").addEventListener("click", exportJson);

initialize().catch((error) => {
  console.error(error);
  status.textContent = error.message;
  status.className = "status error";
});
