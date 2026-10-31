// One measurement in a fresh process: node run1.mjs <go|rs> <engine> <case-index> <reps>
// Prints one JSON line: startup ms, per-call compile+render ms, peak RSS,
// wasm linear memory, and the SVG's sha256.
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import * as fs from "node:fs";

const require = createRequire(import.meta.url);
const [impl, engine, caseIndex, repsArg] = process.argv.slice(2);
const reps = Number(repsArg);
const cases = JSON.parse(readFileSync(process.env.CASES ?? new URL("./cases.json", import.meta.url), "utf8"));
const { name, script } = cases[Number(caseIndex)];
const GO_WASM = process.env.GO_WASM, GO_EXEC = process.env.GO_EXEC, RS_DIR = process.env.RS_DIR;

async function loadGo() {
  globalThis.require = require;
  globalThis.fs = fs;
  require(GO_EXEC);
  const go = new globalThis.Go();
  const ready = new Promise((resolve) => { globalThis.onWasmInitialized = resolve; });
  const { instance } = await WebAssembly.instantiate(readFileSync(GO_WASM), go.importObject);
  go.run(instance);
  await ready;
  const d2 = globalThis.d2;
  const call = (fn, req) => {
    const res = JSON.parse(d2[fn](JSON.stringify(req)));
    if (res.error) throw new Error(`${fn}: ${res.error.message}`);
    return res.data;
  };
  return {
    memory: () => instance.exports.mem?.buffer.byteLength ?? 0,
    run(src) {
      const compiled = call("compile", { fs: { index: src }, options: { layout: engine } });
      const b64 = call("render", { diagram: compiled.diagram, options: compiled.renderOptions });
      return Buffer.from(b64, "base64").toString("utf8");
    },
  };
}

async function loadRs() {
  const runtime = require(`${RS_DIR}/src/node.cjs`);
  const { D2 } = require(`${RS_DIR}/src/d2-api.cjs`);
  const d2 = new D2(() => runtime);
  await d2.ready;
  const raw = require(`${RS_DIR}/pkg/node/d2_rs_wasm.cjs`);
  return {
    memory: () => (raw.__wasm?.memory ?? raw.memory)?.buffer?.byteLength ?? 0,
    async run(src) {
      const compiled = await d2.compile({ fs: { index: src } }, { layout: engine });
      const out = await d2.render(compiled.diagram, compiled.renderOptions);
      return typeof out === "string" ? out : out?.svg ?? JSON.stringify(out);
    },
  };
}

const t0 = performance.now();
const api = impl === "go" ? await loadGo() : await loadRs();
const startupMs = performance.now() - t0;
let svg, error = null;
const times = [];
try {
  svg = await api.run(script); // warm-up, also the compared output
  for (let i = 0; i < reps; i++) {
    const s = performance.now();
    await api.run(script);
    times.push(performance.now() - s);
  }
} catch (e) {
  error = String(e.message || e).slice(0, 300);
}
if (svg && process.env.DUMP) fs.writeFileSync(process.env.DUMP, svg);
times.sort((a, b) => a - b);
console.log(JSON.stringify({
  impl, engine, name, startupMs, error,
  medianMs: times.length ? times[Math.floor(times.length / 2)] : null,
  maxRssKb: process.resourceUsage().maxRSS,
  wasmMemoryBytes: api.memory(),
  svgSha: svg ? createHash("sha256").update(svg).digest("hex").slice(0, 16) : null,
  svgLen: svg ? svg.length : 0,
}));
process.exit(0);
