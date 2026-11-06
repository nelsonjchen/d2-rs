# SPDX-License-Identifier: MPL-2.0
"""Translate ordered MIME signatures and built-in extensions from the pinned Go oracle."""
import re
from pathlib import Path

root = Path(__file__).resolve().parents[1]
source = root / '.reference/go-mod/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.0.darwin-arm64/src'
sniff = (source / 'net/http/internal/sniff.go').read_text()
body = sniff.split('var sniffSignatures = []sniffSig{', 1)[1].split('\n}\n', 1)[0]
body = re.sub(r'//[^\n]*', '', body)
rows = []
pattern = r'htmlSig\("[^"]*"\)|&exactSig\{[^}]*\}|&maskedSig\{[^}]*\}|mp4Sig\{\}|textSig\{\}'
for match in re.finditer(pattern, body):
    entry = match.group()
    if entry.startswith('htmlSig'):
        rows.append('Signature::Html(b' + entry[8:-1] + ')')
    elif entry.startswith('&exactSig'):
        values = re.findall(r'"(?:\\.|[^"\\])*"', entry)
        assert len(values) == 2
        rows.append(f'Signature::Exact(b{values[0]}, {values[1]})')
    elif entry.startswith('&maskedSig'):
        mask = re.search(r'mask:\s*\[\]byte\(("(?:\\.|[^"\\])*")\)', entry)[1]
        pat = re.search(r'pat:\s*\[\]byte\(("(?:\\.|[^"\\])*")\)', entry)[1]
        ct = re.search(r'ct:\s*("[^"]*")', entry)[1]
        rows.append(f'Signature::Masked(b{mask}, b{pat}, {str("skipWS: true" in entry).lower()}, {ct})')
    elif entry == 'mp4Sig{}':
        rows.append('Signature::Mp4')
    else:
        rows.append('Signature::Text')
assert len(rows) == 51, len(rows)
extensions = re.findall(r'^\s*("\.[^"]*"):\s*("[^"]*"),', (source / 'mime/type.go').read_text(), re.M)
assert len(extensions) == 65, len(extensions)
output = '''// Copyright 2010, 2011 The Go Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause
// Generated from pinned Go 1.27.0 mime/type.go and net/http/internal/sniff.go.
// Regenerate with scripts/generate-http-mime-tables.py; see THIRD_PARTY_NOTICES.md.
use super::Signature;
pub(super) const SIGNATURES: &[Signature] = &[
'''
output += ''.join(f'    {row},\n' for row in rows) + '];\n'
output += 'pub(super) const EXTENSIONS: &[(&str, &str)] = &[\n'
output += ''.join(f'    ({key}, {value}),\n' for key, value in extensions) + '];\n'
(root / 'crates/d2-rs-render/src/image_mime_data.rs').write_text(output)
