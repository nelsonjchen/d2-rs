// SPDX-License-Identifier: MPL-2.0
// Raw d2target -> d2js render captures for the target fixture family
// (tools/fixtures/renderexport-target.toml). Ported from the v0.9.0-era
// capture scripts (.reference/target-render-20261019/capture-source.mjs,
// target-fonts, target-mono-fonts, target-paint, target-legend (+identities),
// ascii-target (source/routing/boundary), target-sdk (+inheritance) and
// wasm-target-boundary (options/terminal-appendix)). Every case starts from the
// eight base inputs below compiled by the pinned d2js with dagre; the mutation
// code is unchanged. Writes {cases:[{group,name,...,svg|error}]} as raw
// observations; scripts/d2js_target_fixtures.py shapes them into fixtures.
//
//   node target.mjs <out.json> <inputs-dir>
import {readFileSync, writeFileSync} from "node:fs";
import {join} from "node:path";
import {publicClient, rawApi} from "./d2js.mjs";

const [out, inputs] = process.argv.slice(2);
const clone = (v) => structuredClone(v);

async function base(d2) {
  const equation = readFileSync(join(inputs, "single-backslash-latex.d2"), "utf8");
  const cases = [["edge", "a -> b: message"], ["container", "box: {a -> b}; box -> outside"], ["class", "A: {shape: class; +field: int; method(): string}"], ["table", "T: {shape: sql_table; id: int {constraint: primary_key}; value: string}"], ["markdown", "a: |md\n # Heading\n - item\n|\na -> b"], ["equation", equation], ["sequence", "shape: sequence_diagram\na -> b: message\nb -> a: reply"], ["boards", "a -> b\nlayers: {alt: {c -> d}}"]];
  const mutations = [["clone", () => {}], ["translate", (d) => { for (const s of d.shapes || []) { s.pos.x += 37; s.pos.y -= 19; } for (const c of d.connections || []) for (const point of c.route || []) { point.x += 37; point.y -= 19; } }], ["paint", (d) => { for (const s of d.shapes || []) { s.fill = "#ffdd99"; s.stroke = "#902030"; s.color = "#123456"; s.opacity = 0.7; } for (const c of d.connections || []) { c.stroke = "#008800"; c.strokeWidth = 4; } }], ["label", (d) => { if (d.shapes?.length) { d.shapes[0].label = "edited"; d.shapes[0].labelWidth = 47; d.shapes[0].labelHeight = 24; d.shapes[0].fontSize = 20; } }]];
  const rows = [];
  for (const [name, input] of cases) {
    const compiled = await d2.compile(input, {layout: "dagre"});
    for (const [mutation, apply] of mutations) {
      const diagram = JSON.parse(JSON.stringify(compiled.diagram));
      apply(diagram);
      rows.push({name: name + "-" + mutation, input, diagram, options: {pad: 20, noXMLTag: true}});
    }
  }
  return rows;
}

function fonts(baseRows) {
  const rows = [];
  for (const row of baseRows.filter((x) => x.name.endsWith("-clone"))) for (const fontFamily of ["SourceSansPro", "SourceCodePro", "HandDrawn"]) for (const sketch of [false, true]) {
    const diagram = clone(row.diagram);
    diagram.fontFamily = fontFamily;
    rows.push({name: row.name.replace("-clone", "") + "-" + fontFamily + "-" + sketch, input: row.input, diagram, options: {pad: 20, noXMLTag: true, sketch}});
  }
  return rows;
}

function mono(baseRows) {
  const list = baseRows.filter((x) => x.name.endsWith("-clone")).map(clone);
  const original = list.find((r) => r.name === "markdown-clone");
  for (const isMono of [false, true]) {
    const row = clone(original);
    row.name = isMono ? "rich-mono-clone" : "rich-default-clone";
    const shape = row.diagram.shapes.find((s) => s.language === "markdown");
    Object.assign(shape, {label: "# Heading `code`\n\nNormal **bold** and *italic*, `inline` code.\n\n- item with **`bold code`**\n- second\n\n```text\nlet value = 123\nnext line\n```\n\n| Name | Value |\n| --- | --- |\n| `item` | **bold** |", fontFamily: isMono ? "MONO" : "DEFAULT", fontSize: 16, labelWidth: 400, labelHeight: 430, width: 430, height: 460});
    list.push(row);
  }
  const rows = [];
  for (const row of list) for (const fontFamily of ["SourceSansPro", "SourceCodePro", "HandDrawn"]) for (const monoFontFamily of ["SourceCodePro", "SourceSansPro", "HandDrawn"]) for (const sketch of [false, true]) {
    const diagram = clone(row.diagram);
    Object.assign(diagram, {fontFamily, monoFontFamily});
    rows.push({name: row.name.replace("-clone", "") + "-" + fontFamily + "-" + monoFontFamily + "-" + sketch, input: row.input, diagram, options: {pad: 20, noXMLTag: true, sketch}});
  }
  return rows;
}

function paint(baseRows) {
  const variants = [["default", {}, () => {}], ["terminal", {themeID: 300}, () => {}], ["terminal-dark", {themeID: 301}, () => {}], ["c4", {themeID: 303}, () => {}], ["palette", {themeID: 1}, () => {}], ["dark", {darkThemeID: 200}, () => {}], ["override", {themeOverrides: {n1: "#123456", n2: "#993344", b1: "#225588", b6: "#ffeedd"}}, () => {}], ["classes", {}, (d) => { d.shapes?.forEach((s, i) => (s.classes = ["custom-shape", "node-" + i])); d.connections?.forEach((c, i) => (c.classes = ["custom-edge", "edge-" + i])); }], ["label-fill", {}, (d) => { d.shapes?.forEach((s) => (s.labelFill = "#ffbbaa")); }], ["label-fill-theme", {darkThemeID: 200}, (d) => { d.shapes?.forEach((s) => (s.labelFill = "B2")); }], ["edge-label-fill", {}, (d) => { d.connections?.forEach((c) => (c.labelFill = "#ffbbaa")); }]];
  const rows = [];
  for (const row of baseRows.filter((x) => x.name.endsWith("-clone"))) for (const [suffix, opts, mutate] of variants) {
    const diagram = clone(row.diagram);
    mutate(diagram);
    rows.push({name: row.name.replace("-clone", "") + "-" + suffix, input: row.input, diagram, options: {pad: 20, noXMLTag: true, ...opts}});
  }
  return rows;
}

function legend(baseRows) {
  const by = Object.fromEntries(baseRows.filter((r) => r.name.endsWith("-clone")).map((r) => [r.name.replace("-clone", ""), r]));
  const shape = (label = "Node") => ({...clone(by.edge.diagram.shapes[0]), id: "legend-node", label});
  const conn = (label = "Relation") => ({...clone(by.edge.diagram.connections[0]), id: "legend-edge", label});
  const variants = {empty: () => ({}), hidden: () => ({label: "Hidden items", shapes: [{...shape(""), type: "unavailable"}], connections: [conn("")]}), shape: () => ({shapes: [shape()]}), connection: () => ({connections: [conn()]}), mixed: () => ({label: "A legend", shapes: [shape("Object"), {...shape("Other"), id: "legend-other", type: "oval"}], connections: [conn()]}), lower: () => ({shapes: [{...shape("MMMMMM iiiiii"), fontFamily: "mono"}]}), upper: () => ({shapes: [{...shape("MMMMMM iiiiii"), fontFamily: "MONO"}]}), structured: () => ({shapes: [{...clone(by.class.diagram.shapes[0]), label: "Class icon"}, {...clone(by.table.diagram.shapes[0]), label: "Table icon"}]}), effects: () => ({shapes: [{...shape("Effects"), multiple: true, "double-border": true, fillPattern: "dots", opacity: 0.6}]}), threeDee: () => ({shapes: [{...shape("3D"), "3d": true, fill: "#f0d0ff", stroke: "#771199"}]}), blend: () => ({shapes: [{...shape("Blend"), blend: true, opacity: 0.6, fill: "#ff6699"}]}), image: () => ({shapes: [{...shape("Image"), type: "image", icon: {Scheme: "data", Opaque: "image/svg+xml;base64," + Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#ee6633"/></svg>').toString("base64")}}]}), arrows: () => ({connections: [{...conn("Markers"), srcArrow: "diamond", dstArrow: "triangle", strokeWidth: 4, strokeDash: 3, fill: "#ddaa88", opacity: 0.6, borderRadius: 8}]}), discarded: () => ({connections: [{...conn("Route only"), srcLabel: {label: "ignored", labelWidth: 100, labelHeight: 20}, dstLabel: {label: "ignored"}, link: "https://example.invalid", tooltip: "ignored", icon: {Scheme: "https", Host: "example.invalid", Path: "/unused.svg"}}]})};
  const rows = [];
  for (const host of ["edge", "container", "class", "boards"]) for (const [tag, make] of Object.entries(variants)) for (const [mode, extra] of [["default", {}], ["sketch", {sketch: true}], ["terminal", {themeID: 300}]]) {
    const diagram = clone(by[host].diagram);
    diagram.legend = make();
    rows.push({name: host + "-" + tag + "-" + mode, input: by[host].input, diagram, options: {pad: 20, noXMLTag: true, ...extra}});
  }
  // capture-identities-source.mjs
  for (const host of ["edge", "container", "class", "boards"]) for (const kind of ["shape", "connection"]) for (const classes of [false, true]) {
    const original = rows.find((v) => v.name === `${host}-${kind}-default`), diagram = clone(original.diagram);
    const item = diagram.legend[kind === "shape" ? "shapes" : "connections"][0];
    delete item.id;
    if (classes) item.classes = ["custom", "other-class"];
    rows.push({name: `${host}-${kind}-no-id${classes ? "-classes" : ""}`, diagram, options: original.options});
  }
  return rows;
}

function ascii(baseRows) {
  const cases = [];
  for (const row of baseRows.filter((v) => v.name.endsWith("-clone"))) for (const mode of ["extended", "standard", "STANDARD", "unknown"]) for (const scale of [undefined, 0.5, 2]) cases.push({name: `${row.name}-${mode}-${scale ?? "unset"}`, diagram: row.diagram, options: {ascii: true, asciiMode: mode, ...(scale === undefined ? {} : {scale})}});
  const shape = clone(baseRows.find((v) => v.name === "edge-clone").diagram.shapes[0]);
  for (const type of ["rectangle", "square", "page", "hexagon", "person", "stored_data", "cylinder", "package", "parallelogram", "queue", "step", "callout", "document", "diamond", "class", "sql_table", "cloud", "circle", "oval", "text", "code", "image", "unrecognized"]) for (const [tag, change] of Object.entries({plain: {}, negative: {pos: {x: -27, y: -19}}, multiple: {multiple: true}, unicode: {label: "é🙂界"}, multiline: {label: "ab\nc"}, empty: {label: ""}, outside: {labelPosition: "OUTSIDE_TOP_CENTER"}, small: {width: 8, height: 9}})) for (const mode of ["extended", "standard"]) {
    const item = {...clone(shape), type, width: 140, height: 126, ...change};
    cases.push({name: `shape-${type}-${tag}-${mode}`, diagram: {shapes: [item], connections: []}, options: {ascii: true, asciiMode: mode}});
  }
  for (const [name, diagram] of [["empty", {}], ["nil-shapes", {shapes: null}], ["empty-shapes", {shapes: []}]]) for (const mode of ["extended", "standard"]) cases.push({name: name + "-" + mode, diagram, options: {ascii: true, asciiMode: mode}});
  // capture-routing-source.mjs
  const edge = cases.find((v) => v.name === "edge-clone-extended-unset").diagram;
  const routes = {vertical: [[29.5, 66], [29.5, 166]], right: [[56, 33], [130, 33], [130, 199], [56, 199]], left: [[3, 33], [-50, 33], [-50, 199], [3, 199]], diagonal: [[56, 33], [190, 199]], duplicates: [[29.5, 66], [29.5, 66], [29.5, 90], [29.5, 166]], backtrack: [[56, 33], [140, 33], [80, 33], [80, 199], [56, 199]]};
  for (const [tag, points] of Object.entries(routes)) for (const reverse of [false, true]) for (const [srcArrow, dstArrow] of [["none", "none"], ["none", "triangle"], ["diamond", "triangle"], ["", ""]]) for (const label of ["message", "é²🙂", "a\nb", ""]) for (const mode of ["extended", "standard"]) {
    const diagram = clone(edge), c = diagram.connections[0];
    c.route = points.map(([x, y]) => ({x, y}));
    c.srcArrow = srcArrow; c.dstArrow = dstArrow; c.label = label; c.labelPosition = "UNLOCKED_TOP";
    c.srcLabel = {label: "from é", labelWidth: 42, labelHeight: 14};
    c.dstLabel = {label: "to 🙂", labelWidth: 42, labelHeight: 14};
    if (reverse) { c.route.reverse(); [c.src, c.dst] = [c.dst, c.src]; }
    cases.push({name: `${tag}-${reverse}-${srcArrow || "empty"}-${dstArrow || "empty"}-${Buffer.from(label).toString("hex")}-${mode}`, diagram, options: {ascii: true, asciiMode: mode}});
  }
  // capture-boundary-source.mjs
  const rect = cases.find((v) => v.name === "shape-rectangle-plain-extended").diagram.shapes[0];
  for (const type of ["rectangle", "class", "sql_table", "hexagon"]) for (const width of [120, 140, 160]) for (const strokeWidth of [0, 1, 3]) for (const labelPosition of ["", "INSIDE_MIDDLE_CENTER", "BORDER_TOP_CENTER", "OUTSIDE_LEFT_MIDDLE"]) for (const mode of ["extended", "standard"]) {
    const s = {...clone(rect), type, width, strokeWidth, label: "é²🙂", labelWidth: 350, labelHeight: 98, labelPosition, fields: [{name: "é²", type: "界🙂abcdefgh", visibility: "private"}], methods: [{name: "méthod()", return: "🙂abcdefgh", visibility: "protected"}], columns: [{name: {label: "é²"}, type: {label: "界🙂abcdefgh"}, constraint: []}]};
    cases.push({name: `${type}-${width}-${strokeWidth}-${labelPosition || "unset"}-${mode}`, diagram: {shapes: [s]}, options: {ascii: true, asciiMode: mode}});
  }
  return cases;
}

function sdk(baseRows) {
  const cases = [];
  for (const c of baseRows) cases.push({name: "mutation-" + c.name, diagram: c.diagram, options: c.options});
  const options = [{}, {pad: 0}, {pad: 20, scale: 1.25, center: true}, {themeID: 200, darkThemeID: 100}, {forceAppendix: true, noXMLTag: true}, {sketch: true, pad: 20}, {animateInterval: 500, pad: 20}, {animateInterval: 500, pad: 20, noXMLTag: true, scale: 0.5}, {animateInterval: 500, target: "*", pad: 20}, {animateInterval: 500, target: "layers.alt.*", pad: 20, salt: "target-sdk"}];
  for (const c of baseRows.filter((c) => c.name.endsWith("-clone"))) for (let i = 0; i < options.length; i++) cases.push({name: `options-${c.name}-${i}`, diagram: c.diagram, options: options[i]});
  const boards = baseRows.find((c) => c.name === "boards-clone").diagram;
  for (const all of [false, true]) {
    const diagram = clone(boards);
    diagram.isFolderOnly = true; diagram.shapes = []; diagram.connections = [];
    if (all) for (const child of diagram.layers || []) { child.isFolderOnly = true; child.shapes = []; child.connections = []; }
    cases.push({name: "folder-" + all, diagram, options: {target: "*", animateInterval: 500, pad: 20}});
  }
  for (const diagram of [null, {}, {shapes: [], connections: []}]) cases.push({name: "empty-" + cases.length, diagram, options: {noXMLTag: true}});
  return cases;
}

function boundary(baseRows) {
  const rows = [];
  for (const row of baseRows.filter((x) => x.name.endsWith("-clone"))) for (const [tag, options, mutate] of [["default", {}, () => {}], ["size", {pad: 0, scale: 0.75, center: true, noXMLTag: true}, () => {}], ["salt", {pad: 10, salt: "raw-pipeline"}, () => {}], ["appendix", {pad: 20, forceAppendix: true, noXMLTag: true}, (d) => { d.shapes[0].link = "https://example.invalid/raw"; d.shapes[0].tooltip = "A tooltip"; }], ["ignored-compile-fields", {pad: 20, noXMLTag: true, layout: [], fontRegular: 42}, () => {}]]) {
    const diagram = clone(row.diagram);
    mutate(diagram);
    rows.push({name: "options-" + row.name.replace("-clone", "") + "-" + tag, diagram, options});
  }
  const boards = baseRows.find((r) => r.name === "boards-clone");
  rows.push({name: "options-selected-board", diagram: boards.diagram, options: {target: "layers.alt", pad: 20, noXMLTag: true}});
  const diagram = baseRows[0].diagram;
  for (const [tag, d, o] of [["missing", null, {}], ["invalid-diagram", 4, {}], ["pad-type", diagram, {pad: "x"}], ["pad-fraction", diagram, {pad: 1.5}], ["scale-type", diagram, {scale: "x"}], ["sketch-type", diagram, {sketch: 4}], ["target-missing", diagram, {target: "missing"}], ["target-all", diagram, {target: "*"}], ["target-parse", diagram, {target: "a ->"}]]) rows.push({name: "error-" + tag, diagram: d, options: o});
  // capture-terminal-appendix.mjs
  const appendix = rows.find((v) => v.name === "options-edge-appendix");
  for (const id of [300, 301]) { const c = clone(appendix); c.options.themeID = id; c.name = "terminal-appendix-" + id; rows.push(c); }
  { const c = clone(appendix); c.name = "appendix-without-rows"; c.diagram.shapes[0].tooltip = ""; rows.push(c); }
  return rows;
}

const d2 = await publicClient();
const api = await rawApi();
const result = {cases: [], inheritance: null};
try {
  const baseRows = await base(d2);
  const groups = {target: baseRows, fonts: fonts(baseRows), mono: mono(baseRows), paint: paint(baseRows), legend: legend(baseRows), ascii: ascii(baseRows), sdk: sdk(baseRows), boundary: boundary(baseRows)};
  for (const [group, rows] of Object.entries(groups)) for (const row of rows) {
    const record = {group, ...row};
    try { record.svg = await d2.render(clone(row.diagram), row.options); }
    catch (e) { record.error = e?.message ?? String(e); }
    if (group === "sdk") {
      const response = JSON.parse(api.render(JSON.stringify({diagram: row.diagram, options: row.options})));
      record.raw_data_null = response.data === null && !response.error;
    }
    result.cases.push(record);
  }
  // target-sdk capture-inheritance-source.mjs
  const input = "x -> y\nlayers: {numbers: {1 -> 2}}\n", compile_options = {target: "*"}, rows = [];
  const compiled = await d2.compile(input, compile_options);
  for (const options of [{}, {target: "*"}]) {
    try { rows.push({options, svg: await d2.render(compiled.diagram, options)}); }
    catch (e) { rows.push({options, error: e?.message ?? String(e)}); }
  }
  result.inheritance = {input, compile_options, rows};
  // Shape arrays: committed source requests compiled through the public client.
  result.shape_arrays = [];
  for (const row of JSON.parse(readFileSync(join(inputs, "target-shape-arrays.json"), "utf8"))) {
    const record = {...row};
    try {
      const compiled = await d2.compile({fs: {...row.files, [row.path]: row.source}, inputPath: row.path}, {layout: row.layout});
      record.shapes = compiled.diagram.shapes;
      record.diagram = compiled.diagram;
    } catch (e) {
      // The public client rejects with the diagnostics JSON; keep their messages.
      const message = e?.message ?? String(e);
      let diagnostics = null;
      try { diagnostics = JSON.parse(message); } catch {}
      record.error = Array.isArray(diagnostics) ? diagnostics.map((d) => d.errmsg).join("\n") : message;
    }
    result.shape_arrays.push(record);
  }
} finally {
  await d2.dispose();
}
writeFileSync(out, JSON.stringify(result) + "\n");
console.log("target observations", result.cases.length);
process.exit(0);
