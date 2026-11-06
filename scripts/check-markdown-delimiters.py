#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-delimiter`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-delimiter-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-delimiter`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import os
import subprocess
import sys
from references import ROOT
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-delimiter-oracle/main.go'
module, _ = prepare_module('markdown-delimiter-oracle', harness)
binary = build(module)
ranges=json.loads(subprocess.run([str(binary),'ranges'],stdout=subprocess.PIPE,check=True).stdout)
# The Unicode classifier table is committed source generated from this
# oracle. Regeneration verifies it; set D2RS_UPDATE_UNICODE_TABLE=1 to rewrite it.
table=ROOT/'crates/d2-rs-text/src/markdown/delimiter/unicode.rs'
table_text='// SPDX-License-Identifier: BSD-3-Clause\n// Copyright The Go Authors. Rust translation Copyright 2026 Nelson Chen.\n// Generated from Go 1.27 Unicode 17.0.0 and Goldmark v1.8.5 classifiers.\n// Regenerate: D2RS_UPDATE_UNICODE_TABLE=1 uv run --no-project python scripts/regen-fixtures.py --only goldmark-delimiter --force.\n// Bit 1: punctuation or symbol; bit 2: whitespace.\npub(super) const RANGES: &[(u32, u32, u8)] = &[\n'+''.join(f'    ({a}, {b}, {flags}),\n' for a,b,flags in ranges)+'];\n'
if os.environ.get('D2RS_UPDATE_UNICODE_TABLE') == '1':
    table.write_text(table_text)
elif table.read_text() != table_text:
    sys.exit(f'{table} is stale for the pinned Go/Goldmark classifiers; rerun with D2RS_UPDATE_UNICODE_TABLE=1')
requests=[]
def add(line,before,minimum=1,allowed='*_~'):
    requests.append({'Line':line.hex(),'Before':before,'Minimum':minimum,'Allowed':allowed})
points={0,9,10,11,12,13,32,65,95,127,128,255,256,0xfffd,0x10ffff}
for a,b,_ in ranges:points.update([max(0,a-1),a,b,min(0x10ffff,b+1)])
for point in sorted(points):
    for byte in [b'*',b'_',b'~']:
        add(byte+b'x',point)
        if not 0xd800<=point<=0xdfff:
            for before in [32,65,33]:add(byte+chr(point).encode(),before)
for byte in range(256):
    for marker in [b'*',b'_']:
        add(marker+bytes([byte]),65)
for before in [-2147483648,-257,-256,-247,-246,-243,-224,-1,0x110000,2147483647]:
    for marker in [b'*',b'_']:add(marker+b'x',before)
for marker in [b'*',b'_',b'~',b'!',b'a']:
    for length in range(1,7):
        for minimum in [-1,0,1,2,3,7]:
            for after in [b'',b'x',b' ',b'!',b'\x80',b'\xe2\x82']:
                add(marker*length+after,32,minimum)
requests=list({json.dumps(q,sort_keys=True):q for q in requests}.values())
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/delimiter.json', requests, results)
consumption_requests=[]
def delimiter(original,remaining,can_open,can_close,start):
    return {'can_open':can_open,'can_close':can_close,'length':remaining,'original_length':original,'char':42,'segment':{'Start':start,'Stop':start+remaining,'Padding':2,'ForceNewline':True}}
for left in range(1,9):
    for right in range(1,9):
        for left_close in [False,True]:
            for right_open in [False,True]:
                for remaining_left,remaining_right in sorted({(left,right),(max(0,left-1),max(0,right-1)),(1,1),(0,0)}):
                    for count in [-1,0,1,2,3]:
                        consumption_requests.append({'Opener':delimiter(left,remaining_left,True,left_close,4),'Closer':delimiter(right,remaining_right,right_open,True,20),'Count':count})
consumption_results=json.loads(subprocess.run([str(binary),'consume'],input=json.dumps(consumption_requests).encode(),stdout=subprocess.PIPE,check=True).stdout)
assert len(consumption_results)==len(consumption_requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/delimiter-consumption.json', consumption_requests, consumption_results)
print(f'{len(requests)} Goldmark delimiter scan and {len(consumption_requests)} consumption cases captured')
