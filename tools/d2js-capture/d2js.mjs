// SPDX-License-Identifier: MPL-2.0
// Access to the pinned upstream d2js build produced by the `d2js-node-package`
// recipe (scripts/d2js_node_package.py). Two clients, as the original captures
// used them:
//   publicClient()  upstream's own Node `D2` class (worker thread, index.js)
//   rawApi()        the same d2.wasm instantiated in-process; globalThis.d2
//                   exposes the raw JSON-string functions registered by d2js/js.go.
// The package directory is $D2RS_D2JS_PACKAGE, else
// $D2RS_FIXTURES_DIR/tools/d2js-node/package.
import {readFileSync} from "node:fs";
import {join, resolve} from "node:path";
import {pathToFileURL} from "node:url";
import {webcrypto} from "node:crypto";

export function packageDir() {
  if (process.env.D2RS_D2JS_PACKAGE) return resolve(process.env.D2RS_D2JS_PACKAGE);
  const fixtures = process.env.D2RS_FIXTURES_DIR;
  if (!fixtures) throw new Error("D2RS_FIXTURES_DIR is unset: run through scripts/regen-fixtures.py");
  return join(resolve(fixtures), "tools/d2js-node/package");
}

export async function publicClient() {
  const {D2} = await import(pathToFileURL(join(packageDir(), "index.js")).href);
  return new D2();
}

let raw = null;
export async function rawApi() {
  if (raw) return raw;
  if (!globalThis.crypto) globalThis.crypto = webcrypto;
  const dir = packageDir();
  new Function(readFileSync(join(dir, "wasm_exec.js"), "utf8")).call(globalThis);
  const ready = new Promise((r) => (globalThis.onWasmInitialized = r));
  const go = new globalThis.Go();
  const instance = await WebAssembly.instantiate(readFileSync(join(dir, "d2.wasm")), go.importObject);
  go.run(instance.instance).catch((e) => {
    console.error(e);
    process.exit(2);
  });
  await ready;
  raw = globalThis.d2;
  return raw;
}

// Go runtime stacks inside recovered-panic messages carry goroutine numbers
// and argument/pointer words that change between runs. Readers compare only
// the first line of such messages, so captures keep the stack's frames and
// lines but write "goroutine N" and "0x?" for every hex word.
export function canonicalGoStack(message) {
  if (typeof message !== "string" || !/\ngoroutine \d+ \[/.test(message)) return message;
  return message.replace(/goroutine \d+ \[/g, "goroutine N [").replace(/0x[0-9a-f]+\??/g, "0x?");
}

// JSON with stable key order is the caller's job; this only writes.
export function args() {
  const [, , ...rest] = process.argv;
  return rest;
}
