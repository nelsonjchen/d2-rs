// SPDX-License-Identifier: MPL-2.0
// ASCII renders of D2 sources through the pinned upstream d2js public client,
// for scripts/export_ascii_callers.py (recipe renderexport-ascii-callers).
// usage: node ascii-callers.mjs <requests.json> <outputs.json>
// requests: [{source, options}]; the same options go to compile and render.
// outputs:  [{output} | {error}] in request order.
import {readFileSync, writeFileSync} from "node:fs";
import {args, publicClient} from "./d2js.mjs";

const [requestsPath, outputsPath] = args();
const requests = JSON.parse(readFileSync(requestsPath, "utf8"));
const d2 = await publicClient();
await d2.ready;
const outputs = [];
try {
  for (const {source, options} of requests) {
    try {
      const {diagram} = await d2.compile(source, options);
      outputs.push({output: await d2.render(diagram, options)});
    } catch (e) {
      outputs.push({error: e?.message ?? String(e)});
    }
  }
} finally {
  await d2.dispose();
}
writeFileSync(outputsPath, JSON.stringify(outputs));
