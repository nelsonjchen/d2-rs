// SPDX-License-Identifier: MPL-2.0
// Public d2js compile -> JSON round trip/mutate -> render captures for
// crates/d2-rs-render/tests/fixtures/public-target{,-precision}-v090.json
// (recipe render-d2js-public-target, tools/fixtures/renderexport-render-api.toml).
// Ported from the v0.9.0-era .reference/target-render-20261019/
// capture-source.mjs and capture-precision-source.mjs; the cases and the
// mutation code are unchanged. Writes raw {target:[...], precision:[...]}
// rows with the rendered SVG (or error); scripts/render_d2js_fixtures.py
// shapes them.
//
//   node render-public-target.mjs <out.json> <inputs-dir>
import {readFileSync, writeFileSync} from "node:fs";
import {join} from "node:path";
import {publicClient} from "./d2js.mjs";

const [out, inputs] = process.argv.slice(2);
const d2 = await publicClient();
const result = {target: [], precision: []};
try {
  const equation = readFileSync(join(inputs, "single-backslash-latex.d2"), "utf8");
  const cases = [["edge", "a -> b: message"], ["container", "box: {a -> b}; box -> outside"], ["class", "A: {shape: class; +field: int; method(): string}"], ["table", "T: {shape: sql_table; id: int {constraint: primary_key}; value: string}"], ["markdown", "a: |md\n # Heading\n - item\n|\na -> b"], ["equation", equation], ["sequence", "shape: sequence_diagram\na -> b: message\nb -> a: reply"], ["boards", "a -> b\nlayers: {alt: {c -> d}}"]];
  const mutations = [["clone", () => {}], ["translate", (d) => { for (const s of d.shapes || []) { s.pos.x += 37; s.pos.y -= 19; } for (const c of d.connections || []) for (const point of c.route || []) { point.x += 37; point.y -= 19; } }], ["paint", (d) => { for (const s of d.shapes || []) { s.fill = "#ffdd99"; s.stroke = "#902030"; s.color = "#123456"; s.opacity = 0.7; } for (const c of d.connections || []) { c.stroke = "#008800"; c.strokeWidth = 4; } }], ["label", (d) => { if (d.shapes?.length) { d.shapes[0].label = "edited"; d.shapes[0].labelWidth = 47; d.shapes[0].labelHeight = 24; d.shapes[0].fontSize = 20; } }]];
  for (const [name, input] of cases) {
    const compiled = await d2.compile(input, {layout: "dagre"});
    for (const [mutation, apply] of mutations) {
      const diagram = JSON.parse(JSON.stringify(compiled.diagram));
      apply(diagram);
      const options = {pad: 20, noXMLTag: true};
      const row = {name: name + "-" + mutation, input, diagram, options};
      try { row.svg = await d2.render(diagram, options); } catch (e) { row.error = String(e); }
      result.target.push(row);
    }
  }
  const base = result.target[0];
  for (const sign of [1, -1]) {
    const diagram = structuredClone(base.diagram), c = diagram.connections[0], x = sign * 100.000987654321;
    c.route = [{x, y: 0.000321}, {x, y: 220.000321}];
    c.isCurve = false;
    c.srcArrow = "none";
    c.dstArrow = "none";
    c.label = "";
    const name = sign === 1 ? "fractional-positive" : "fractional-negative", options = {pad: 20, noXMLTag: true};
    result.precision.push({name, input: base.input, diagram, options, svg: await d2.render(diagram, options),
                           expected_coordinate: (sign * 100.000987654321).toFixed(6)});
  }
} finally {
  await d2.dispose();
}
writeFileSync(out, JSON.stringify(result));
