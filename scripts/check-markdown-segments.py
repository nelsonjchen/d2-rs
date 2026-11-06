#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-segments`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-segment-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-segments`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
from references import ROOT
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-segment-oracle/main.go'
module, _ = prepare_module('markdown-segment-oracle', harness)
binary = build(module)
requests = []
def add(data, start, stop, padding, newline):
    requests.append({'Source': data.hex(), 'Segment': {'Start': start, 'Stop': stop, 'Padding': padding, 'ForceNewline': newline}, 'Widths': [-1, 0, 1, 2, 3, 4, 5, 8]})
for byte in range(256):
    add(bytes([byte, 120, byte]), 0, 3, 0, True)
for data in [b'', b' ', b'\t', b'\n', b' \t x \r\n', b'\t\tX', b'\v\f x\v\f', '😀 z'.encode(), b'\xff \xfe']:
    for start in range(len(data) + 1):
        for stop in sorted({start, len(data)}):
            for padding in [0, 1, 3]:
                for newline in [False, True]:
                    add(data, start, stop, padding, newline)
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/segments.json', requests, results)
print(f'{len(requests)} Goldmark Segment cases captured')
