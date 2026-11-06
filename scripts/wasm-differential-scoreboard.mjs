// SPDX-License-Identifier: MPL-2.0
// Each runtime runs in its own process; failed or incomplete jobs never match.
import { spawnSync } from 'node:child_process';
import { readFileSync, mkdirSync, writeFileSync, statfsSync, readdirSync } from 'node:fs';
import { dirname, resolve, join, relative } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
import { createHash } from 'node:crypto';
import assert from 'node:assert/strict';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const arg = (name, fallback) => { const i = args.indexOf(name); return i < 0 ? fallback : args[i + 1]; };
const goPackage = resolve(arg('--go-package', resolve(root, 'oracle-cache/reports/wasm-differential/go-package/index.js')));
const rustPackage = resolve(arg('--rust-package', resolve(root, 'crates/d2-rs-wasm/src/node.cjs')));
const env = {PATH: '/usr/local/bin:/usr/bin:/bin', HOME: '/nonexistent', NO_COLOR: '1', LANG: 'C.UTF-8', LC_ALL: 'C.UTF-8', TZ: 'UTC', GOMAXPROCS: '2'};
const hash = value => createHash('sha256').update(value).digest('hex');
export const normalizeSvg = value => value.replace(/(data-d2-version|d2Version)="[^"]*"/g, '$1=""');
export const summarize = value => { const text = JSON.stringify(value); return text === undefined ? '<absent>' : text.slice(0, 600); };
export function firstDifference(a, b, path = []) {
  if (isDeepStrictEqual(a, b)) return null;
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    for (const key of [...new Set([...Object.keys(a), ...Object.keys(b)])].sort()) {
      if (!Object.hasOwn(a, key) || !Object.hasOwn(b, key)) return [...path, key];
      const found = firstDifference(a[key], b[key], [...path, key]);
      if (found) return found;
    }
  }
  return path;
}
export const atPath = (obj, path) => path.reduce((x, key) => x?.[key], obj);
async function invoke(D2, job) {
  const d2 = new D2();
  try {
    await d2.ready;
    if (job.probe) return {version: await d2.version()};
    let compiled;
    try { compiled = await d2.compile({fs: {index: job.source}}, job.options); }
    catch (error) { return {compile: {error: String(error.message ?? error)}}; }
    let render;
    try {
      const value = await d2.render(compiled.diagram, compiled.renderOptions);
      render = {value: typeof value === 'string' ? normalizeSvg(value) : value};
    } catch (error) { render = {error: String(error.message ?? error)}; }
    return {compile: {value: compiled}, render};
  } finally { await d2.dispose(); }
}
function packageHashes(entry, rust) {
  const paths = [];
  function walk(directory) {
    for (const item of readdirSync(directory, {withFileTypes: true})) {
      const path = join(directory, item.name);
      if (item.isDirectory()) walk(path);
      else if (item.isFile()) paths.push(path);
    }
  }
  const base = rust ? resolve(dirname(entry), '..') : dirname(entry);
  if (rust) { walk(dirname(entry)); walk(join(base, 'pkg/node')); }
  else walk(base);
  return Object.fromEntries(paths.sort().map(path => [relative(base, path), hash(readFileSync(path))]));
}
function command(program, argv) {
  const result = spawnSync(program, argv, {encoding: 'utf8', maxBuffer: 128 * 1024 * 1024});
  if (result.error || result.status !== 0) throw Error(`${program}: ${result.error || result.stderr}`);
  return result.stdout.trim();
}
if (args[0] === '--child') {
  const job = JSON.parse(readFileSync(0, 'utf8'));
  const module = await import(pathToFileURL(args[1] === 'go' ? goPackage : rustPackage));
  const D2 = module.D2 ?? module.default?.D2;
  process.stdout.write(JSON.stringify(await invoke(D2, job)));
  process.exitCode = 0;
} else if (args[0] === '--self-test') {
  assert.deepEqual(firstDifference({'a.b': {x: 1}}, {'a.b': {x: 2}}), ['a.b', 'x']);
  assert.equal(atPath({'a.b': {x: 1}}, ['a.b', 'x']), 1);
  assert.deepEqual(firstDifference({}, {a: 1}), ['a']);
  assert.deepEqual(firstDifference(1, 2), []);
  assert.equal(firstDifference({a: 1}, {a: 1}), null);
  assert.ok(summarize({huge: 'x'.repeat(10000)}).length <= 600);
  assert.equal(normalizeSvg('<svg d2Version="v1" data-d2-version="v2"/>'), '<svg d2Version="" data-d2-version=""/>');
  console.log('WASM harness self-tests passed');
} else if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const cap = Number(arg('--cap', '2000'));
  const offset = Number(arg('--offset', '0'));
  const seed = Number(arg('--seed', '20261102'));
  const timeout = Number(arg('--timeout', '30000'));
  const output = resolve(arg('--output', resolve(root, 'oracle-cache/reports/wasm-differential/latest.json')));
  if (!Number.isInteger(cap) || cap < 1 || !Number.isInteger(offset) || offset < 0 || !Number.isInteger(seed) || !Number.isFinite(timeout) || timeout < 1) throw Error('Invalid cap, offset, seed or timeout');
  const manifest = JSON.parse(readFileSync(join(root, 'reference-manifest.json')));
  const upstream = resolve(root, manifest.d2.checkout);
  const goCli = resolve(arg('--go', join(root, manifest.d2.oracle)));
  const cliVersion = command(goCli, ['--version']);
  const cliBuild = command('go', ['version', '-m', goCli]);
  // go version -m does not understand wasm containers. Read its embedded
  // module-info block and verify the version through the running public API.
  const wasmBytes = readFileSync(join(dirname(goPackage), 'd2.wasm'));
  const infoStart = wasmBytes.indexOf('path\tgithub.com/d2lang/d2/d2js\n');
  const infoEnd = wasmBytes.indexOf('build\tvcs.modified=false\n', infoStart);
  if (infoStart < 0 || infoEnd < infoStart) throw Error('Missing Go WASM build provenance');
  const wasmBuild = wasmBytes.subarray(infoStart, infoEnd + 'build\tvcs.modified=false\n'.length).toString();
  function packageVersion(engine) {
    const child = spawnSync(process.execPath, [fileURLToPath(import.meta.url), '--child', engine, '--go-package', goPackage, '--rust-package', rustPackage], {input: JSON.stringify({probe: true}), encoding: 'utf8', timeout, maxBuffer: 1024 * 1024, env});
    if (child.error || child.status !== 0) throw Error(`Version probe failed: ${child.error || child.stderr}`);
    return JSON.parse(child.stdout).version;
  }
  const goWasmVersion = packageVersion('go'), rustWasmVersion = packageVersion('rust');
  const upstreamHead = command('git', ['-C', upstream, 'rev-parse', 'HEAD']);
  if (goWasmVersion !== 'v0.9.0' || cliVersion !== 'v0.9.0' || upstreamHead !== manifest.d2.commit ||
      !cliBuild.includes('github.com/d2lang/d2/lib/version.Version=v0.9.0') || [cliBuild, wasmBuild].some(build => !build.includes(`vcs.revision=${manifest.d2.commit}`) || !build.includes('vcs.modified=false'))) throw Error('Go CLI and WASM must be clean pinned builds stamped v0.9.0');
  const arch = {x64: 'amd64', arm64: 'arm64'}[process.arch];
  if (!cliBuild.includes(`GOARCH=${arch}`) || !cliBuild.split('\n')[0].includes(manifest.d2.go_toolchain)) throw Error('Go CLI architecture or toolchain mismatch');
  const provenance = {source_head: command('git', ['-C', root, 'rev-parse', 'HEAD']), upstream_head: upstreamHead,
    go: {version: cliVersion, sha256: hash(readFileSync(goCli)), build: cliBuild, wasm_build: wasmBuild, wasm_version: goWasmVersion, package: goPackage, files: packageHashes(goPackage, false)},
    rust: {package: rustPackage, version: rustWasmVersion, source_head: command('git', ['-C', resolve(arg('--rust-source-root', resolve(dirname(rustPackage), '../../..'))), 'rev-parse', 'HEAD']), files: packageHashes(rustPackage, true)}};
  const generator = spawnSync('uv', ['run', '--no-project', 'python', resolve(root, 'scripts/wasm-vs-go/corpus.py'), '--seed', String(seed), '--cap', String(cap)], {encoding: 'utf8', maxBuffer: 128 * 1024 * 1024});
  if (generator.status !== 0 || generator.error) throw Error(`Corpus generation failed: ${generator.stderr || generator.error}`);
  const jobs = JSON.parse(generator.stdout).slice(offset), rows = [], clusters = {};
  if (!jobs.length) throw Error('No jobs selected by cap and offset');
  let bytes = Buffer.byteLength(generator.stdout);
  function run(engine, job) {
    const remaining = 1024 ** 3 - bytes;
    if (remaining <= 0) return {failure: 'budget-exceeded'};
    const child = spawnSync(process.execPath, [fileURLToPath(import.meta.url), '--child', engine, '--go-package', goPackage, '--rust-package', rustPackage], {input: JSON.stringify(job), encoding: 'utf8', timeout, maxBuffer: Math.min(128 * 1024 * 1024, Math.max(1, Math.floor(remaining / 2))), env});
    bytes += Buffer.byteLength(child.stdout || '') + Buffer.byteLength(child.stderr || '');
    if (bytes > 1024 ** 3 || child.error?.code === 'ENOBUFS') return {failure: 'budget-exceeded'};
    if (child.error || child.status !== 0) return {failure: child.error?.code === 'ETIMEDOUT' ? 'timeout' : 'process-failure', error: String(child.error || child.stderr).slice(0, 1000)};
    try {
      const value = JSON.parse(child.stdout);
      if (!value || !value.compile || !(value.compile.error || value.compile.value && value.render && (typeof value.render.value === 'string' || value.render.error))) throw Error('incomplete compile/render response');
      return {value, stderr: child.stderr};
    } catch (error) { return {failure: 'invalid-output', error: String(error)}; }
  }
  function compare(job) {
    const go = run('go', job), rust = run('rust', job);
    const row = {name: job.name, mutation: job.mutation, options: job.options, sourceSha256: hash(job.source)};
    if (go.failure || rust.failure) return {...row, outcome: go.failure || rust.failure, go, rust, source: job.source};
    const difference = firstDifference(go, rust);
    row.outcome = difference ? 'mismatch' : 'match';
    row.stage = go.value.compile.error ? 'compile-error' : go.value.render.error ? 'render-error' : 'rendered';
    if (difference) Object.assign(row, {difference, go: summarize(atPath(go, difference)), rust: summarize(atPath(rust, difference)), source: job.source});
    return row;
  }
  for (const job of jobs) {
    const disk = statfsSync(dirname(root));
    if (disk.bavail * disk.bsize < 30 * 1024 ** 3 || bytes >= 1024 ** 3) { rows.push({outcome: 'budget-exceeded'}); break; }
    let row = compare(job);
    if (row.outcome === 'timeout') { const firstAttempt = row; row = compare(job); row.first_attempt = firstAttempt; }
    if (row.outcome === 'mismatch') { const key = JSON.stringify(row.difference); clusters[key] = (clusters[key] || 0) + 1; }
    rows.push(row);
    if (rows.length % 25 === 0 || row.outcome !== 'match') console.error(`${rows.length}/${jobs.length} ${row.outcome} ${JSON.stringify(row.difference || '')}`);
  }
  if (!isDeepStrictEqual(provenance.go.files, packageHashes(goPackage, false)) ||
      !isDeepStrictEqual(provenance.rust.files, packageHashes(rustPackage, true))) rows.push({outcome: 'provenance-changed', error: 'Package bytes changed during capture'});
  // Bound line reduction by both calls and elapsed time. Error clusters are
  // distinguished by their first line, so unrelated panics stay separate.
  const witnesses = [];
  const signatures = new Set();
  for (const row of rows.filter(row => row.outcome === 'mismatch')) {
    const signature = JSON.stringify([row.difference, row.go?.split('\\n')[0], row.rust?.split('\\n')[0]]);
    if (signatures.has(signature) || witnesses.length >= 8) continue;
    signatures.add(signature);
    let lines = row.source.split(/(?<=\n)/), attempts = 0, width = Math.max(1, Math.floor(lines.length / 2));
    const deadline = Date.now() + 15000;
    while (width && attempts < 20 && Date.now() < deadline) {
      let changed = false;
      for (let i = 0; i < lines.length && attempts < 20 && Date.now() < deadline; i += width) {
        const candidate = [...lines.slice(0, i), ...lines.slice(i + width)];
        const reduced = compare({name: row.name, source: candidate.join(''), mutation: 'minimized', options: row.options});
        attempts++;
        if (reduced.outcome === 'mismatch' && JSON.stringify([reduced.difference, reduced.go?.split('\\n')[0], reduced.rust?.split('\\n')[0]]) === signature) {
          lines = candidate; changed = true; break;
        }
      }
      if (!changed) width = Math.floor(width / 2);
    }
    witnesses.push({difference: row.difference, options: row.options, source: lines.join(''), attempts});
  }
  const summary = {};
  for (const row of rows) summary[row.outcome] = (summary[row.outcome] || 0) + 1;
  mkdirSync(dirname(output), {recursive: true});
  const report = JSON.stringify({provenance, budget_bytes: 1024 ** 3, file_limit_bytes: 128 * 1024 ** 2, captured_bytes: bytes, seed, cap, offset, timeout, summary, clusters, witnesses, rows}, null, 2) + '\n';
  if (Buffer.byteLength(report) > 128 * 1024 * 1024 || bytes + Buffer.byteLength(report) > 1024 ** 3) {
    writeFileSync(output, JSON.stringify({provenance, seed, cap, timeout, summary: {'budget-exceeded': 1}, error: 'report exceeds capture budget'}) + '\n');
    process.exitCode = 1;
  } else { writeFileSync(output, report); process.exitCode = rows.some(row => row.outcome !== 'match') ? 1 : 0; }
  console.log(JSON.stringify({summary, clusters}, null, 2));
}
