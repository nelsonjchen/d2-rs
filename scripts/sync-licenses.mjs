// SPDX-License-Identifier: MPL-2.0
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const check = process.argv.includes('--check');
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--format-version', '1'], { cwd: root, maxBuffer: 16 * 1024 * 1024 }));
const read = (path) => readFileSync(path, 'utf8');
const normalizeLicense = (content) => content.replace(/\r\n?/g, '\n').trim().replace(/[ \t]+$/gm, '');
// A small number of crates publish SPDX license metadata but omit the
// corresponding text file from their registry package. Keep audited license
// fallbacks here so packaged notices remain complete and reproducible instead
// of silently dropping a license.
const mitLicenseText = (copyright) => `${copyright}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;
const licenseTextFallbacks = new Map([
  ['alloc-stdlib@0.2.4', { text: `Copyright (c) 2016 Dropbox, Inc.
All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.` }],
  ['block2@0.6.2', {
    text: mitLicenseText('Copyright (c) Mads Marquart <mads@marquart.dk>'),
    source: 'https://github.com/madsmtm/objc2/blob/block2-0.6.2/LICENSE.md',
    note: 'The upstream license declaration identifies block2 as MIT-licensed; the attribution comes from the published crate metadata.',
  }],
  ['objc2@0.6.4', {
    text: mitLicenseText('Copyright (c) Mads Marquart <mads@marquart.dk>'),
    source: 'https://github.com/madsmtm/objc2/blob/objc2-0.6.4/LICENSE.md',
    note: 'The upstream license declaration identifies objc2 as MIT-licensed; the attribution comes from the published crate metadata.',
  }],
  ['objc2-encode@4.1.0', {
    text: mitLicenseText('Copyright (c) Mads Marquart <mads@marquart.dk>'),
    source: 'https://github.com/madsmtm/objc2/blob/objc2-encode-4.1.0/LICENSE.md',
    note: 'The upstream license declaration identifies objc2-encode as MIT-licensed; the attribution comes from the published crate metadata.',
  }],
  ['dispatch2@0.3.1', {
    text: mitLicenseText('Copyright (c) Mads Marquart <mads@marquart.dk> and Mary <mary@mary.zone>'),
    source: 'https://github.com/madsmtm/objc2/blob/dispatch2-0.3.1/LICENSE.md',
    note: 'The upstream SPDX expression permits Zlib, Apache-2.0, or MIT. This distribution elects the MIT option; attribution comes from the published crate metadata.',
  }],
]);
function save(path, content) {
  if (!check) return writeFileSync(path, content);
  if (read(path) !== content) throw new Error(`Outdated license file: ${relative(root, path)}; run node scripts/sync-licenses.mjs`);
}
function licenseFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return licenseFiles(path);
    // r-efi publishes its MIT grant and copyright notices in AUTHORS.
    const authorLicense = entry.isFile() && /^authors$/i.test(entry.name)
      && read(path).includes('Permission is hereby granted, free of charge');
    return entry.isFile() && (/license|^(copying|notice|copyright)/i.test(entry.name) || authorLicense) ? [path] : [];
  }).sort();
}
let notices = '# Rust dependency notices\n\nGenerated by `node scripts/sync-licenses.mjs` from Cargo.lock and the\ninstalled crate sources. This includes build-time and target-specific\ndependencies; not every dependency is linked into every artifact.\n\n';
for (const pkg of metadata.packages.filter((p) => p.source).sort((a, b) => `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`, 'en'))) {
  const dir = dirname(pkg.manifest_path);
  const files = licenseFiles(dir);
  const fallback = licenseTextFallbacks.get(`${pkg.name}@${pkg.version}`);
  if (!files.length && !fallback) throw new Error(`No license text found for ${pkg.name}@${pkg.version}`);
  notices += `## ${pkg.name} ${pkg.version}\n\nLicense: ${pkg.license ?? 'see text below'}\n\nSource: https://crates.io/crates/${pkg.name}/${pkg.version}\n\n`;
  if (fallback) {
    const fallbackText = typeof fallback === 'string' ? fallback : fallback.text;
    notices += `### LICENSE (registry fallback)\n\n`;
    if (typeof fallback === 'object' && fallback.source) notices += `Upstream license declaration: ${fallback.source}\n\n`;
    if (typeof fallback === 'object' && fallback.note) notices += `${fallback.note}\n\n`;
    notices += `\`\`\`text\n${normalizeLicense(fallbackText)}\n\`\`\`\n\n`;
  }
  for (const path of files) notices += `### ${relative(dir, path).replaceAll('\\', '/')}\n\n\`\`\`text\n${normalizeLicense(read(path))}\n\`\`\`\n\n`;
}
notices = `${notices.trimEnd()}\n`;
save(join(root, 'DEPENDENCY_NOTICES.md'), notices);
for (const id of metadata.workspace_members) {
  const pkg = metadata.packages.find((p) => p.id === id);
  // Cargo also includes path dependencies below vendor/ as workspace members.
  // Those retain their own upstream license files; do not overwrite them with MPL.
  if (relative(root, pkg.manifest_path).split(/[\\/]/)[0] === 'vendor') continue;
  if (pkg.license !== 'MPL-2.0') throw new Error(`Unexpected license for ${pkg.name}`);
}
// Crates are not published separately: LICENSE, THIRD_PARTY_NOTICES.md,
// SOURCE.md and the generated DEPENDENCY_NOTICES.md live once at the root.
// The npm package in crates/d2-rs-wasm is the one distributed artifact built
// from a crate directory, so it gets generated (gitignored) copies.
const wasmDir = join(root, 'crates', 'd2-rs-wasm');
for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md', 'SOURCE.md']) save(join(wasmDir, name), read(join(root, name)));
save(join(wasmDir, 'DEPENDENCY_NOTICES.md'), notices);
console.log(check ? 'License metadata and packaged notices are current.' : 'Updated dependency notices and the WASM package copies.');
