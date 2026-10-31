// node bench.mjs <reps> [engines] [case-indices]; interleaves go/rs per case.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const reps = process.argv[2] ?? "5";
const engines = (process.argv[3] ?? "dagre,elk,tala").split(",");
const cases = JSON.parse(readFileSync(process.env.CASES ?? new URL("./cases.json", import.meta.url), "utf8"));
const indices = process.argv[4] ? process.argv[4].split(",").map(Number) : cases.map((_, i) => i);
const run = (impl, engine, i) => JSON.parse(execFileSync(process.execPath,
  ["--max-old-space-size=8192", new URL("./run1.mjs", import.meta.url).pathname, impl, engine, String(i), reps],
  { encoding: "utf8", maxBuffer: 1 << 26, timeout: 900000 }).trim().split("\n").pop());

const rows = [];
for (const engine of engines) for (const i of indices) {
  const go = run("go", engine, i), rs = run("rs", engine, i);
  const row = { engine, name: cases[i].name, go, rs,
    identical: !!go.svgSha && go.svgSha === rs.svgSha,
    wall: go.medianMs && rs.medianMs ? rs.medianMs / go.medianMs : null,
    rss: rs.maxRssKb / go.maxRssKb };
  rows.push(row);
  console.log(`${engine.padEnd(5)} ${cases[i].name.padEnd(34)} go ${String(go.medianMs?.toFixed(1)).padStart(8)}ms rs ${String(rs.medianMs?.toFixed(1)).padStart(8)}ms  x${row.wall?.toFixed(2)}  rss ${(go.maxRssKb/1024).toFixed(0)}/${(rs.maxRssKb/1024).toFixed(0)}MB  ${row.identical ? "identical" : "DIFF"}${go.error ? " goErr:" + go.error : ""}${rs.error ? " rsErr:" + rs.error : ""}`);
}
writeFileSync("bench-results.json", JSON.stringify(rows, null, 1));
const geo = (xs) => Math.exp(xs.reduce((a, x) => a + Math.log(x), 0) / xs.length);
for (const engine of engines) {
  const r = rows.filter((x) => x.engine === engine && x.wall);
  const med = (xs) => xs.sort((a, b) => a - b)[Math.floor(xs.length / 2)];
  console.log(`${engine}: wall median x${med(r.map((x) => x.wall)).toFixed(2)} geomean x${geo(r.map((x) => x.wall)).toFixed(2)}`
    + ` total x${(r.reduce((a, x) => a + x.rs.medianMs, 0) / r.reduce((a, x) => a + x.go.medianMs, 0)).toFixed(2)}`
    + ` rss median x${med(r.map((x) => x.rss)).toFixed(2)} identical ${rows.filter((x) => x.engine === engine && x.identical).length}/${rows.filter((x) => x.engine === engine).length}`);
}
