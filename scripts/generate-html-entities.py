#!/usr/bin/env python3
"""Extract pinned Go html entity data, excluding commented-out entries."""
from pathlib import Path
import re, hashlib
root=Path(__file__).resolve().parents[1]
source=root/'.reference/go-mod/golang.org/toolchain@v0.0.1-go1.27.0.darwin-arm64/src/html/entity.go'
text=source.read_text();entries=[]
for name,value in re.findall(r'^\s*"([^"]+)":\s*(.+),\s*$',text,re.M):
    chars=re.findall(r"'\\[uU]([0-9A-Fa-f]+)'",value)
    assert len(chars) in (1,2),(name,value)
    entries.append((name,chars))
assert len(entries)==2229,len(entries)
lines=['// SPDX-License-Identifier: BSD-3-Clause','// Generated from Go 1.27.0 html/entity.go; Go Authors notice in THIRD_PARTY_NOTICES.md.',f'// Source SHA256: {hashlib.sha256(source.read_bytes()).hexdigest()}','pub(super) static ENTITIES: &[(&[u8], &str)] = &[']
for name,chars in sorted(entries):
    escaped=''.join('\\u{'+c.lower().lstrip('0')+'}' for c in chars)
    lines.append(f'    (b"{name}", "{escaped}"),')
lines+= ['];']
(root/'crates/d2-rs-language/src/compiler/import_urls/entities.rs').write_text('\n'.join(lines)+'\n')
