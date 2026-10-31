// SPDX-License-Identifier: MPL-2.0
// Source-compile/render and raw-argument d2js fixtures for crates/d2-rs-wasm
// (recipe `d2js-sources`, tools/fixtures/renderexport-d2js.toml), ported from
// the v0.9.0-era captures in .reference/ (native-target-icons-c4, legend-target,
// target-row-collections-state, public-diagnostic-json, api-animation,
// api-multiboard, svg-responsive, engine-options/capture-error-types.mjs,
// sequence-api-faults/capture-public.mjs). Request lists are the committed
// seeds in tools/d2js-capture/inputs; two families are derived from other
// recipes' artifacts (target-sdk error rows from `d2js-target`, sequence
// panics from `d2lib-sequence-targets`). Every family writes the layout its
// readers expect; nothing is normalized except recovered-panic stacks
// (canonicalGoStack), whose readers compare only the first line.
//
//   node sources.mjs <stage dir> <inputs dir> <fixtures dir> <commit>
import {readFileSync, writeFileSync, mkdirSync} from "node:fs";
import {dirname, join} from "node:path";
import {canonicalGoStack, publicClient, rawApi} from "./d2js.mjs";

const [stage, inputs, fixtures, commit] = process.argv.slice(2);
const seed = (name) => JSON.parse(readFileSync(join(inputs, name), "utf8"));
const artifact = (logical) => JSON.parse(readFileSync(join(fixtures, logical), "utf8"));
const PREFIX = "crates/d2-rs-wasm/tests/fixtures/";
function write(name, value) {
  const path = join(stage, PREFIX + name);
  mkdirSync(dirname(path), {recursive: true});
  writeFileSync(path, JSON.stringify(value, null, 2) + "\n");
}
const errorOf = (e) => ({name: e?.name ?? null, message: canonicalGoStack(e?.message ?? String(e)), code: e?.code ?? null});
// The public client rejects compiler failures with the diagnostics JSON; the
// target fixtures keep their joined messages, as target-shape-arrays does.
function compileMessage(e) {
  const message = e?.message ?? String(e);
  let diagnostics = null;
  try { diagnostics = JSON.parse(message); } catch {}
  return Array.isArray(diagnostics) ? diagnostics.map((d) => d.errmsg).join("\n") : canonicalGoStack(message);
}
const ATTRIBUTES = ["width", "height", "viewBox", "preserveAspectRatio"];
// The same opening-tag regex projection the JS readers apply.
const attributesOf = (opening, names = ATTRIBUTES) => opening === undefined ? null
  : Object.fromEntries(names.map((n) => [n, opening.match(new RegExp(`${n}="([^"]*)"`))?.[1] ?? null]));
const transitionsOf = (svg) => (svg.match(/animation: d2Transition-/g) || []).length;

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
async function compileTarget(row) {
  try {
    const compiled = await d2.compile({fs: {...row.files, [row.path]: row.source}, inputPath: row.path}, {layout: row.layout ?? "dagre"});
    return {diagram: compiled.diagram, error: null};
  } catch (e) {
    return {diagram: null, error: compileMessage(e)};
  } finally {
    await healthyCompile();
  }
}

try {
  // native-target-icons-c4 / legend-target / target-diagram-expanded: complete targets.
  {
    const cases = [];
    for (const row of seed("target-icons-c4.json")) {
      const {diagram, error} = await compileTarget(row);
      if (error) throw Error(`${row.name}: ${error}`);
      cases.push({...row, diagram});
    }
    write("target-icons-c4-source-v090.json", {source_commit: commit, source: "d2js public compile of each committed source (tools/d2js-capture/sources.mjs)", cases});
  }
  {
    const cases = [];
    for (const row of seed("legend-target.json")) cases.push({...row, ...await compileTarget(row)});
    write("legend-target-source-v090.json", {source_commit: commit, source: "d2js public compile of each committed legend source; classes/SQL, labels/styles/themes/boards, hidden endpoints and host sequence scope", cases});
  }
  {
    const cases = [];
    for (const row of seed("target-diagram-expanded.json")) {
      const result = await compileTarget(row);
      cases.push({...row, ...result, panic: !!result.error?.startsWith("panic recovered:")});
    }
    write("target-diagram-expanded-source-v090.json", {source_commit: commit, source: "d2js public compile; root/config/board and connection field controls", cases});
  }

  // Shape-array projections of successful compiles.
  for (const [file, seedFile, scope, project] of [
    ["class-row-target-source-v090.json", "class-row-target.json",
     "Original successful class field/method arrays: name, type/return, visibility, underline, ordering; not whole target/API parity.",
     ({id, fields, methods}) => ({id, fields, methods})],
    ["target-row-collections-source-v090.json", "target-row-collections.json",
     "Successful API controls: empty/one-sided class row arrays, rectangle, empty/populated SQL. Complete fields/methods; column null/presence/count only.",
     ({id, fields, methods, columns}) => ({id, fields, methods, columnCount: columns === null ? null : columns.length})],
  ]) {
    const cases = [];
    for (const row of seed(seedFile)) {
      const compiled = await d2.compile(row.source, {layout: "dagre"});
      await healthyCompile();
      cases.push({source: row.source, shapes: compiled.diagram.shapes.map(project)});
    }
    write(file, {source_commit: commit, scope, cases});
  }
  {
    const cases = [];
    for (const row of seed("sql-column-target.json")) {
      const {diagram, error} = await compileTarget(row);
      cases.push(error ? {...row, error} : {...row, shapes: diagram.shapes});
    }
    write("sql-column-target-source-v090.json", {source_commit: commit, source: "d2js public compile of SQL table sources", scope: "Complete SQL shape outcomes and exact color-rejection errors. Whole diagrams/other shapes unqualified.", cases});
  }

  // public-diagnostic-json capture.mjs: compiler failures through the public facade.
  {
    const cases = [];
    for (const row of seed("public-compiler-diagnostics.json")) {
      const inputPath = row.inputPath || "index.d2";
      let error = null;
      try { await d2.compile({fs: {...row.files, [inputPath]: row.source}, inputPath}, {layout: "dagre", ...row.options}); }
      catch (e) { error = errorOf(e); }
      if (!error) throw Error(`${row.name}: public compile must reject`);
      await healthyCompile();
      cases.push({...row, error, diagnostics: JSON.parse(error.message)});
    }
    write("public-compiler-diagnostics-source-v090.json", {upstream_commit: commit, scope: "Unmodified Go WASM through the original standard Node facade. Complete Error name/message/code including ordered ranges.", cases});
  }

  // api-animation / api-multiboard capture-source.mjs: each seed omitted and at scale 1.25.
  for (const [file, seedFile, multi] of [["animation-single-board-source-v090.json", "animation-single-board.json", false],
                                         ["animation-multi-board-source-v090.json", "animation-multi-board.json", true]]) {
    const rows = [];
    for (const c of seed(seedFile)) {
      const {diagram} = await d2.compile(c.input, c.compile_options);
      for (const scale of [undefined, 1.25]) {
        const options = {...c.options, ...(scale === undefined ? {} : {scale})};
        const svg = await d2.render(diagram, options);
        const openings = svg.match(/<svg\b[^>]*>/g) || [];
        const row = {name: c.name + (scale === undefined ? "-omitted" : "-scaled"), input: c.input, compile_options: c.compile_options, options};
        if (multi) Object.assign(row, {type: typeof svg, value: openings.length ? null : svg});
        Object.assign(row, {attributes: attributesOf(openings[0]), inner_attributes: attributesOf(openings[1]),
                            transitions: transitionsOf(svg), hash: svg.match(/d2Transition-(d2-\d+)-/)?.[1] ?? null});
        rows.push(row);
      }
    }
    write(file, rows);
  }

  // svg-responsive compile-render-scales.mjs: root sizing per render scale; one compile per name.
  {
    const rows = [], compiled = new Map();
    for (const c of seed("svg-scale-render.json")) {
      if (!compiled.has(c.name)) compiled.set(c.name, await d2.compile(c.input, c.compile_options));
      const svg = await d2.render(compiled.get(c.name).diagram, c.options);
      rows.push({...c, attributes: attributesOf(svg.match(/<svg\b[^>]*>/)?.[0]), transitions: transitionsOf(svg)});
    }
    write("svg-scale-render-source-v090.json", rows);
    // The source-render scale presence contract: the "ordinary" a -> b rows.
    write("svg-scale-presence-source-v090.json", rows.filter((r) => r.name === "ordinary").map((r) => ({
      options: r.options, svg_attributes: Object.fromEntries(["viewBox", "width", "height"].map((n) => [n, r.attributes[n]])),
    })));
  }

  // engine-options capture-error-types.mjs: typed public errors and a healthy follow-up.
  {
    const sdk = artifact(PREFIX + "target-sdk-source-v090.json");
    const cases = sdk.cases.filter((c) => c.error !== undefined).map((c) => ({name: c.name, operation: "render", diagram: c.diagram, options: c.options}));
    for (const options of [{pad: 1.5}, {themeID: "bad"}, {ascii: "yes"}, {target: 2}, {noXMLTag: "x"}, {scale: "x"}]) cases.push({name: "typed-" + cases.length, operation: "render", diagram: {}, options});
    for (const source of ["a: {", "a ->", "a: {shape: nonexistent}"]) cases.push({name: "compile-" + cases.length, operation: "compile", source, options: {}});
    for (const c of cases) {
      let error = null, success = false;
      try {
        if (c.operation === "render") await d2.render(structuredClone(c.diagram), c.options);
        else await d2.compile(c.source, c.options);
        success = true;
      } catch (e) {
        error = {type: typeof e, is_error: e instanceof Error, name: e?.name ?? null, message: canonicalGoStack(e?.message ?? String(e)), code: e?.code ?? null};
      }
      const followup = await d2.render({}, {noXMLTag: true});
      if (typeof followup !== "string" || !followup.startsWith("<svg")) throw Error("Unhealthy followup");
      c.expected = {side: "source", name: c.name, success, error, healthy_followup: true};
    }
    write("public-error-types-source-v090.json", {source_commit: commit, cases});
  }

  // sequence-api-faults capture-public.mjs / capture-node.mjs: the actor-label sources whose
  // d2lib compile panicked, through the raw and the public API, each followed by a healthy call.
  {
    const sources = seed("sequence-actor-label-policy.json");
    const records = artifact("tools/d2lib-sequence-targets/actor-label-policy.json");
    const cases = [];
    for (const [i, c] of sources.entries()) {
      if (!records[i].panic) continue;
      const request = JSON.stringify({fs: {"index.d2": c.source}, inputPath: "index.d2", options: {layout: "dagre"}});
      const raw = JSON.parse(api.compile(request));
      if (!raw.error) throw Error(`${c.name}: raw compile did not fail`);
      rawHealthy();
      let error = null;
      try { await d2.compile({fs: {"index.d2": c.source}, inputPath: "index.d2"}, {layout: "dagre"}); } catch (e) { error = errorOf(e); }
      await healthyCompile();
      cases.push({name: c.name, source: c.source, source_raw_error: {message: canonicalGoStack(raw.error.message), code: raw.error.code}, source_public_error: error});
    }
    write("sequence-api-fault-source-v090.json", {source_commit: commit, scope: "Sequence sources whose d2lib.Compile panics: raw no-data error, public Error rejection and healthy same-instance continuation. Go backtraces canonicalized; only first lines are compared.", cases});
  }

  // Raw GetELKGraph request contracts (raw JSON-string API).
  {
    const cases = [];
    for (const c of seed("elk-request.json")) {
      const raw = api.getELKGraph(c.request);
      cases.push({name: c.name, request: c.request, expected: JSON.parse(raw), source_raw: raw});
      rawHealthy();
    }
    write("elk-request-source-v090.json", {source_commit: commit, source_method: "d2wasm.GetELKGraph", cases});
  }

  // svg-responsive browser-sdk.mjs legacy string-argument boundary: every JS value kind.
  {
    const values = {
      undefined: undefined, null: null, true: true, false: false, zero: 0, one: 1, negative: -1, fraction: 0.5,
      small: 1e-7, large: 1e21, "negative-zero": -0, nan: NaN, infinity: Infinity, string: "a.b", empty: "",
      object: {}, array: [], symbol: Symbol("x"), function: () => {},
    };
    const cases = [];
    for (const method of ["getParentID", "getObjOrder", "getELKGraph"]) for (const tag of ["omitted", ...Object.keys(values)]) {
      const args = tag === "omitted" ? [] : [values[tag]];
      cases.push({name: `${method}/${tag}`, method, tag, expected: JSON.parse(api[method](...args))});
      rawHealthy();
    }
    write("legacy-string-arguments-source-v090.json", {commit, cases});
  }
} finally {
  await d2.dispose();
}
process.exit(0);
