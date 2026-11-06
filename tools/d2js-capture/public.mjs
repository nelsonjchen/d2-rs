// SPDX-License-Identifier: MPL-2.0
// Public d2js API fixtures for crates/d2-rs-wasm (tools/fixtures/renderexport-d2js.toml),
// ported from the v0.9.0-era capture scripts in .reference/
// (public-graph-{inheritance,objects,root}, public-render-options,
// sequence-span-export, root-structured-graph, request-unicode,
// codec-contracts). Each family replays a committed request list
// (tools/d2js-capture/inputs) against the pinned d2js and writes the fixture
// in the layout the readers expect. Every call is followed by a health probe,
// as in the originals, and a failed probe aborts the capture.
//
//   node public.mjs <stage dir> <inputs dir> <commit>
import {readFileSync, writeFileSync, mkdirSync} from "node:fs";
import {dirname, join} from "node:path";
import {canonicalGoStack, publicClient, rawApi} from "./d2js.mjs";

const [stage, inputs, commit] = process.argv.slice(2);
const seed = (name) => JSON.parse(readFileSync(join(inputs, name), "utf8"));
const PREFIX = "crates/d2-rs-wasm/tests/fixtures/";
function write(name, value) {
  const path = join(stage, PREFIX + name);
  mkdirSync(dirname(path), {recursive: true});
  writeFileSync(path, JSON.stringify(value, (_key, item) => (Object.is(item, -0) ? JSON.rawJSON("-0") : item), 2) + "\n");
}
const errorOf = (e) => ({name: e?.name ?? null, message: canonicalGoStack(e?.message ?? String(e)), code: e?.code ?? null});

const d2 = await publicClient();
await d2.ready;
const api = await rawApi();

async function healthyCompile() {
  const healthy = await d2.compile("a -> b\n", {layout: "dagre"});
  if (healthy?.diagram?.shapes?.length !== 2) throw Error("Incomplete probe Diagram");
}
function rawHealthy() {
  const good = JSON.parse(api.getParentID("a.b"));
  if (good.data !== "a" || good.error) throw new Error("Unhealthy runtime");
}

// public-graph-*/public-render-options capture.mjs: compile(input, options) through the public client.
async function compileCase(c) {
  const inputPath = c.inputPath ?? "index.d2", fs = {...c.files, [inputPath]: c.source};
  const options = {...c.options};
  const input = {fs, inputPath};
  let result = null, error = null;
  try { result = await d2.compile(input, options); } catch (e) { error = errorOf(e); }
  await healthyCompile();
  return {request: {fs, inputPath, options: c.options}, result, error};
}

try {
  for (const [family, file] of [["inheritance", "public-graph-inheritance-source-v090.json"], ["class-context", "public-graph-class-context-source-v090.json"],
                                ["data", "public-graph-data-source-v090.json"]]) {
    const cases = [];
    for (const c of seed(`graph-${family}.json`)) {
      const {request, result, error} = await compileCase(c);
      cases.push({name: c.name, request, expected: result, error});
    }
    write(file, {upstream_commit: commit, source_capture: "tools/d2js-capture/public.mjs (" + family + ")", cases});
  }

  for (const [family, file] of [["objects", "public-graph-objects-source-v090.json"], ["edge-storage", "public-graph-edge-storage-source-v090.json"]]) {
    const cases = [];
    for (const c of seed(`graph-${family}.json`)) {
      const {request, result, error} = await compileCase(c);
      if (error) throw Error(`${c.name}: ${error.message}`);
      cases.push({name: c.name, request, graph: result.graph});
    }
    write(file, {upstream_commit: commit, source_capture: "tools/d2js-capture/public.mjs (" + family + ")", cases});
  }

  {
    const cases = [];
    for (const c of seed("graph-roots.json")) {
      const {result, error} = await compileCase({name: c.name, source: c.source, options: {layout: c.engine}});
      if (error) throw Error(`${c.name}: ${error.message}`);
      const boards = [];
      const walk = (graph, diagram, path) => {
        boards.push({path, ast: graph.ast, root: graph.root, target_root: diagram.root});
        for (const kind of ["layers", "scenarios", "steps"]) for (const child of graph[kind] || []) {
          const target = (diagram[kind] || []).find((d) => d.name === child.name);
          walk(child, target, (path ? path + "." : "") + kind + "." + child.name);
        }
      };
      walk(result.graph, result.diagram, "");
      const row = {name: c.name, source: c.source, engine: c.engine, boards};
      if (/\/(ordinary|nested)$/.test(c.name)) row.references = {objects: result.graph.objects.map((o) => o.references), edges: result.graph.edges.map((o) => o.references)};
      cases.push(row);
    }
    write("public-graph-roots-source-v090.json", {upstream_commit: commit, source_capture: "tools/d2js-capture/public.mjs (roots)", cases});
  }

  {
    const cases = [];
    for (const c of seed("render-options.json")) {
      const {result, error} = await compileCase(c);
      if (error) throw Error(`${c.name}: ${error.message}`);
      const requestWire = JSON.stringify({fs: {"index.d2": c.source}, inputPath: "index.d2", options: c.options});
      cases.push({name: c.name, request: JSON.parse(requestWire), requestWire, renderOptions: result.renderOptions, diagram: result.diagram});
    }
    write("public-render-options-source-v090.json", {upstream_commit: commit, source_capture: "tools/d2js-capture/public.mjs (render-options)", scope: "Actual JSON.stringify CompileRequest wire bytes; capture preserves response negative zero.", cases});
  }

  {
    const cases = [];
    for (const c of seed("sequence-span.json")) {
      const {result, error} = await compileCase({name: c.name, source: c.source, options: {layout: c.engine}});
      if (error) throw Error(`${c.name}: ${error.message}`);
      cases.push({name: c.name, source: c.source, engine: c.engine, diagram: result.diagram, graph: result.graph});
    }
    write("sequence-span-export-source-v090.json", {source_commit: commit, cases});
  }

  // root-structured-graph capture-source-phases.cjs: raw compile, then raw render of the compiled target.
  {
    const cases = [];
    for (const c of seed("root-structured.json")) {
      const response = JSON.parse(api.compile(JSON.stringify({fs: {"index.d2": c.source}, inputPath: "index.d2", options: {layout: c.engine ?? "dagre"}})));
      const render = response.data ? JSON.parse(api.render(JSON.stringify({diagram: response.data.diagram, options: response.data.renderOptions}))) : null;
      const healthy = JSON.parse(api.compile(JSON.stringify({fs: {"index.d2": "a -> b\n"}, inputPath: "index.d2", options: {layout: "dagre"}})));
      if (healthy.error || healthy.data.diagram.shapes.length !== 2) throw new Error("Runtime did not recover");
      cases.push({name: c.name, source: c.source, engine: c.engine, shape: c.shape, response, render_error: render?.error ?? null, render_success: !!render?.data});
    }
    write("root-structured-source-v090.json", {source_commit: commit, scope: "All original source inputs: complete compile responses and the raw render outcome of every compiled target. Standard public diagnostic JSON.", cases});
  }

  // request-unicode capture.cjs (raw getELKGraph) and capture-public.mjs (public compile).
  {
    const requests = seed("request-unicode.json"), cases = [], public_cases = [];
    for (const c of requests) {
      const raw = api.getELKGraph(c.request);
      cases.push({name: c.name, request: c.request, expected: JSON.parse(raw), source_raw: raw});
      rawHealthy();
    }
    for (const c of requests.filter((c) => /^(source|missing-path)-/.test(c.name))) {
      let parsed;
      try { parsed = JSON.parse(c.request); } catch { continue; }
      let result, error = null, resolved = false;
      try { result = await d2.compile(parsed); resolved = true; } catch (e) { error = {...errorOf(e), stack: e?.stack ?? null}; }
      if (await d2.decode("AwA=") !== "") throw new Error("Runtime unhealthy");
      public_cases.push({name: c.name, result: result ?? null, resolved, error, request: c.request});
    }
    write("request-unicode-source-v090.json", {source_commit: commit, cases, public_cases});
  }

  // codec-contracts capture.cjs: raw decode of the generated grid.
  {
    const cases = [];
    for (const c of JSON.parse(readFileSync(process.env.D2RS_URL_CODEC_CASES, "utf8"))) {
      const raw = api.decode(c.encoded);
      let response = JSON.parse(raw);
      if (response.data) response = response.data;
      cases.push({...c, expected: response, source_raw: raw});
      rawHealthy();
    }
    write("url-codec-source-v090.json", {commit, cases});
  }
} finally {
  await d2.dispose();
}

// public-option-boundaries capture.mjs / write-fixture.mjs (and its
// font-checksum-controls): every request three times in one fresh runtime, so
// font-registry state carries across requests and repeats. Font options are
// committed as {"$upstream_font_base64": <upstream path>, "set_bytes": {offset: byte}}.
const upstream = process.env.D2RS_UPSTREAM;
function expandFont(value) {
  if (!value || typeof value !== "object" || !("$upstream_font_base64" in value)) return value;
  const bytes = readFileSync(join(upstream, value.$upstream_font_base64));
  for (const [offset, byte] of Object.entries(value.set_bytes || {})) bytes[Number(offset)] = byte;
  return bytes.toString("base64");
}
const scope = "Complete355 original standard Node requests repeated three times in one runtime, including font registry partial writes and probe failures. Go panic stacks remain literal; only their first lines are a separate fault-class check.";
for (const [inputFile, file] of [["option-boundaries.json", "public-option-boundaries-source-v090.json"], ["font-registry.json", "public-font-registry-source-v090.json"]]) {
  const cases = seed(inputFile).map((c) => c.options ? {...c, options: Object.fromEntries(Object.entries(c.options).map(([k, v]) => [k, expandFont(v)]))} : c);
  const client = await publicClient();
  await client.ready;
  const repeats = [];
  try {
    for (let repeat = 0; repeat < 3; repeat++) {
      const rows = [];
      for (const c of cases) {
        const inputPath = c.inputPath ?? "index.d2", fs = {...c.files, [inputPath]: c.source};
        const options = {...c.options};
        for (const key of c.typedFonts || []) options[key] = new Uint8Array(options[key]);
        const input = c.input ?? {fs, inputPath};
        let result = null, error = null;
        try { result = (await client.compile(input, options)).diagram; } catch (e) { error = errorOf(e); }
        let healthy = null, healthyError = null;
        try { healthy = (await client.compile("a -> b\n", {layout: "dagre"})).diagram; } catch (e) { healthyError = canonicalGoStack(e?.message ?? String(e)); }
        if (!healthyError && healthy?.shapes?.length !== 2) throw Error("Incomplete probe Diagram");
        rows.push({name: c.name, result, error, healthy, healthyError});
      }
      repeats.push(rows);
    }
  } finally {
    await client.dispose();
  }
  const requests = cases.map((c) => {
    const inputPath = c.inputPath ?? "index.d2", input = c.input ?? {fs: {...c.files, [inputPath]: c.source}, inputPath};
    const options = {...c.options};
    for (const key of c.typedFonts || []) options[key] = new Uint8Array(options[key]);
    return {name: c.name, request_json: JSON.stringify({...input, options: {...options, ...input.options}})};
  });
  const path = join(stage, PREFIX + file);
  mkdirSync(dirname(path), {recursive: true});
  writeFileSync(path, JSON.stringify({upstream_commit: commit, scope, requests, repeats}, null, 2) + "\n");
}
console.log("d2js public fixtures written");
process.exit(0);
