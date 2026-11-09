// node bench.mjs <repeats> [engines] [case-indices] [output.json]
// Compile + render in isolated Node processes, alternating implementation order.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";

const repeats = Number(process.argv[2] ?? "9");
const engines = (process.argv[3] ?? "dagre,elk,tala").split(",");
const caseFile = process.env.CASES ?? new URL("./cases.json", import.meta.url);
const cases = JSON.parse(readFileSync(caseFile, "utf8"));
const indices = process.argv[4] ? process.argv[4].split(",").map(Number) : cases.map((_, i) => i);
const output = process.argv[5] ?? "bench-results.json";
// Wall time is the public API latency metric used by the WASM benchmark.
// Keep process-wide CPU samples too, including V8's background work.
const metric = process.env.METRIC ?? "wall";
const warmup = Number(process.env.WARMUP ?? "1"), settleMs = Number(process.env.SETTLE_MS ?? "0");
if (!Number.isInteger(repeats) || repeats < 1 || engines.some(e => !["dagre", "elk", "tala"].includes(e)) ||
    indices.some(i => !Number.isInteger(i) || !cases[i]) || !["wall", "cpu"].includes(metric) ||
    !Number.isInteger(warmup) || warmup < 1 || !Number.isFinite(settleMs) || settleMs < 0) throw Error("Invalid benchmark selection");
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const manifest = JSON.parse(execFileSync("uv", ["run", "--no-project", "python", "-c",
  "import json, sys; sys.path.insert(0, sys.argv[1]); import references; print(json.dumps({'d2': references.MANIFEST['d2']}))",
  fileURLToPath(new URL("..", import.meta.url))], {encoding: "utf8"}));
const goBytes = readFileSync(process.env.GO_WASM);
const infoStart = goBytes.indexOf("path\tgithub.com/d2lang/d2/d2js\n");
const infoEnd = goBytes.indexOf("build\tvcs.modified=false\n", infoStart);
if (infoStart < 0 || infoEnd < infoStart) throw Error("Missing clean Go WASM build provenance");
const goBuild = goBytes.subarray(infoStart, infoEnd + "build\tvcs.modified=false\n".length).toString();
if (!goBuild.includes(`vcs.revision=${manifest.d2.commit}`) || !goBytes.includes(manifest.d2.go_toolchain)) {
  throw Error("Go WASM source/toolchain does not match the reference pin");
}
const rustWasm = new URL("pkg/web/d2_rs_wasm_bg.wasm", pathToFileURL(`${process.env.RS_DIR}/`));
const before = {go: hash(goBytes), rust: hash(readFileSync(rustWasm)), cases: hash(readFileSync(caseFile))};
const median = values => {
  const xs = values.toSorted((a, b) => a - b), mid = Math.floor(xs.length / 2);
  return xs.length % 2 ? xs[mid] : (xs[mid - 1] + xs[mid]) / 2;
};
const run = (impl, engine, i) => JSON.parse(execFileSync(process.execPath,
  ["--max-old-space-size=8192", new URL("./run1.mjs", import.meta.url).pathname, impl, engine, String(i), "1"],
  { encoding: "utf8", maxBuffer: 1 << 20, timeout: 600000 }).trim().split("\n").pop());
const rows = [], summary = {};
const save = () => writeFileSync(output, JSON.stringify({
  source_commit: manifest.d2.commit, node: process.version, architecture: process.arch,
  measurement: "fresh process per sample; untimed warmup and optional settling; interleaved Go/Rust with alternating order; compile plus render; CPU/wall milliseconds; startup excluded",
  repeats, warmup, settle_ms: settleMs, primary_metric: metric, hashes: before, go_build: goBuild, summary, rows,
}, null, 1) + "\n");
for (const engine of engines) for (const i of indices) {
  const samples = {go: [], rs: []};
  for (let repeat = 0; repeat < repeats; repeat++) {
    for (const impl of repeat % 2 ? ["rs", "go"] : ["go", "rs"]) samples[impl].push(run(impl, engine, i));
  }
  const valid = Object.values(samples).every(xs => xs.length === repeats && xs.every(s =>
    !s.error && s.svgLen > 0 && s.medianMs > 0 && s.cpuMs > 0 && s.hashes.length === 1 && s.hashes[0] === s.svgSha));
  if (samples.go.some(s => s.version !== "v0.9.0")) throw Error("Go WASM version must be v0.9.0");
  const identical = valid && new Set(Object.values(samples).flat().map(s => s.svgSha)).size === 1;
  const times = Object.fromEntries(Object.entries(samples).map(([impl, xs]) => [impl, {
    wall_ms: median(xs.map(s => s.medianMs ?? Infinity)), cpu_ms: median(xs.map(s => s.cpuMs ?? Infinity)),
  }]));
  const row = {engine, name: cases[i].name, samples, identical, times,
    wall: valid ? times.rs.wall_ms / times.go.wall_ms : null,
    cpu: valid ? times.rs.cpu_ms / times.go.cpu_ms : null};
  rows.push(row); save();
  console.log(`${engine} ${row.name}: wall ${row.wall?.toFixed(3)}x CPU ${row.cpu?.toFixed(3)}x ${identical ? "identical" : "FAIL"}`);
}
for (const engine of engines) {
  const selected = rows.filter(r => r.engine === engine), valid = selected.filter(r => r.wall !== null);
  summary[engine] = {cases: selected.length, identical: selected.filter(r => r.identical).length,
    wall_median: valid.length ? median(valid.map(r => r.wall)) : null,
    cpu_median: valid.length ? median(valid.map(r => r.cpu)) : null,
    worst_wall: valid.length ? Math.max(...valid.map(r => r.wall)) : null,
    worst_cpu: valid.length ? Math.max(...valid.map(r => r.cpu)) : null};
}
if (before.go !== hash(readFileSync(process.env.GO_WASM)) || before.rust !== hash(readFileSync(rustWasm)) ||
    before.cases !== hash(readFileSync(caseFile))) throw Error("Benchmark inputs changed during capture");
save();
console.log(JSON.stringify(summary));
process.exitCode = rows.some(r => !r.identical || r[metric] > 1) ? 1 : 0;
