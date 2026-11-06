// SPDX-License-Identifier: MPL-2.0

import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const crateDir = resolve(fileURLToPath(new URL("..", import.meta.url)));
const workspaceDir = resolve(crateDir, "../..");
const wasmPath = resolve(workspaceDir, "target/wasm32-unknown-unknown/release/d2_rs_wasm.wasm");
const webOutputDir = resolve(crateDir, "pkg/web");
const nodeOutputDir = resolve(crateDir, "pkg/node");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: workspaceDir,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("cargo", ["build", "--locked", "--release", "--target", "wasm32-unknown-unknown", "-p", "d2-rs-wasm"]);
mkdirSync(webOutputDir, { recursive: true });
mkdirSync(nodeOutputDir, { recursive: true });
run("wasm-bindgen", [wasmPath, "--target", "web", "--typescript", "--out-dir", webOutputDir]);
run("wasm-bindgen", [wasmPath, "--target", "nodejs", "--typescript", "--out-dir", nodeOutputDir]);
// The nodejs target is CommonJS. Keep the package itself ESM while giving
// Node's conditional export an extension that follows its module rules.
renameSync(resolve(nodeOutputDir, "d2_rs_wasm.js"), resolve(nodeOutputDir, "d2_rs_wasm.cjs"));

// Browser modules cannot import the CommonJS facade directly. Emit an ESM
// adapter from that single source of truth so Node and browser callers share
// the same D2 lifecycle and response projection.
const facadeSource = readFileSync(resolve(crateDir, "src/d2-api.cjs"), "utf8");
const commonJsExport = "module.exports = { D2 };\n";
if (!facadeSource.endsWith(commonJsExport)) {
  throw new Error("d2-api.cjs must end with the expected CommonJS export before generating the browser adapter");
}
writeFileSync(
  resolve(webOutputDir, "d2_api.js"),
  `${facadeSource.slice(0, -commonJsExport.length)}export { D2 };\nexport default { D2 };\n`,
);

// Preserve Go's argument-count/type boundary before wasm-bindgen's typed
// string glue. Passing a JS number directly to that glue can otherwise trap.
const stringArgumentMethods = ["getParentID", "getObjOrder", "getELKGraph", "encode", "decode"];
const adapter = readFileSync(resolve(crateDir, "src/go-js-string-args.cjs"), "utf8");
const adapterExport = "module.exports = { wrapGoStringArgument };\n";
if (!adapter.endsWith(adapterExport)) throw new Error("Unexpected Go string adapter export");
writeFileSync(resolve(webOutputDir, "go_js_string_args.js"),
  `${adapter.slice(0, -adapterExport.length)}export { wrapGoStringArgument };\n`);
const webBindings = resolve(webOutputDir, "d2_rs_wasm.js");
let webSource = readFileSync(webBindings, "utf8");
for (const name of stringArgumentMethods) {
  const declaration = `export function ${name}(`;
  if (webSource.split(declaration).length !== 2) throw new Error(`Missing unique binding ${name}`);
  webSource = webSource.replace(declaration, `function go_string_${name}(`);
  webSource += `\nexport const ${name} = wrapGoStringArgument(go_string_${name});\n`;
}
writeFileSync(webBindings, `import { wrapGoStringArgument } from "./go_js_string_args.js";\n${webSource}`);
const nodeBindings = resolve(nodeOutputDir, "d2_rs_wasm.cjs");
writeFileSync(nodeBindings, `${readFileSync(nodeBindings, "utf8")}\n` +
  `const { wrapGoStringArgument } = require("../../src/go-js-string-args.cjs");\n` +
  stringArgumentMethods.map(name => `module.exports.${name} = wrapGoStringArgument(module.exports.${name});`).join("\n") + "\n");

// A fresh binding closure per instance keeps token classes, finalizers,
// memory views and wasm-bindgen's heap independent across trap recovery.
// Retain the original generated modules for the explicit ./raw export.
let factorySource = readFileSync(webBindings, "utf8");
const imports = [];
const exports = [];
factorySource = factorySource.replace(/^import .+;$/gm, line => {
  imports.push(line);
  return "";
}).replace(/^export (class|function|const) (\w+)/gm, (_, kind, name) => {
  exports.push(name);
  return `${kind} ${name}`;
});
const initExport = "export { initSync, __wbg_init as default };";
if (!factorySource.includes(initExport)) throw new Error("Missing WASM initialization exports");
factorySource = factorySource.replace(initExport, "");
if (/^\s*(export|import)\b/m.test(factorySource)) throw new Error("Unrecognized WASM binding syntax");
writeFileSync(resolve(webOutputDir, "d2_runtime.js"),
  `${imports.join("\n")}\nexport function createRuntime() {\n${factorySource}\n` +
  `return { ${exports.join(", ")}, initSync, default: __wbg_init };\n}\n`);
writeFileSync(resolve(nodeOutputDir, "d2_runtime.cjs"),
  `module.exports = function createRuntime() {\nconst module = { exports: {} };\nconst exports = module.exports;\n` +
  `${readFileSync(nodeBindings, "utf8")}\nreturn module.exports;\n};\n`);
const recoverySource = readFileSync(resolve(crateDir, "src/recover-runtime.cjs"), "utf8");
const recoveryExport = "module.exports = { recoverRuntime };\n";
if (!recoverySource.endsWith(recoveryExport)) throw new Error("Unexpected runtime recovery export");
writeFileSync(resolve(webOutputDir, "recover_runtime.js"),
  `${recoverySource.slice(0, -recoveryExport.length)}export { recoverRuntime };\n`);
