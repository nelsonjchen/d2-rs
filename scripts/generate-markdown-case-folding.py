#!/usr/bin/env python3
"""Translate the pinned Goldmark case-folding data without host Unicode tables."""
import re
from references import REFERENCE, ROOT, digest
source = REFERENCE / 'go-mod/github.com/yuin/goldmark@v1.8.5/util/unicode_case_folding.gen.go'
text = source.read_text()
def values(name):
    body = re.search(r'var ' + name + r' = .*?\{(.*?)\}', text).group(1)
    return [int(v.strip(), 0) for v in body.split(',') if v.strip()]
keys = values('_unicodeCaseFoldingFrom')
targets = values('_unicodeCaseFoldingTo')
lengths = [int(v, 16) for v in re.search(r'var _unicodeCaseFoldingToIndex  = "(.*?)"', text).group(1).split('\\x')[1:]]
assert len(keys) == len(lengths) == 1530
out = ['// SPDX-License-Identifier: MPL-2.0 AND MIT', '// Copyright (c) 2019 Yusuke Inuzuka', '// Generated from Goldmark v1.8.5 util/unicode_case_folding.gen.go.', '// Source SHA-256: ' + digest(source), 'pub(super) static FOLDINGS: &[(u32, &[u8])] = &[']
offset = 0
for key,n in zip(keys,lengths):
    data = ''.join(chr(c) for c in targets[offset:offset+n]).encode()
    out.append('    ('+str(key)+', &['+', '.join(str(b) for b in data)+']),')
    offset += n
assert offset == len(targets)
out.append('];')
(ROOT/'crates/d2-rs-text/src/markdown/case_folding.rs').write_text('\n'.join(out)+'\n')
