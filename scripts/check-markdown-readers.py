#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-reader`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-reader-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-reader`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
from references import ROOT
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-reader-oracle/main.go'
module, _ = prepare_module('markdown-reader-oracle', harness)
binary = build(module)
requests = []
sequences = [
    [{'Kind':'advance','N':1}]*12,
    [{'Kind':'rune'}]*8,
    [{'Kind':'line'}]*5,
    [{'Kind':'eol'},{'Kind':'advance','N':1}]*4,
    [{'Kind':'spaces'}]*4,
    [{'Kind':'blank'},{'Kind':'line'}]*3,
    [{'Kind':'padding','Padding':3},{'Kind':'advance','N':1},{'Kind':'advance','N':5},{'Kind':'line'}],
    [{'Kind':'advance-padding','N':1,'Padding':2},{'Kind':'spaces'},{'Kind':'eol'}],
    [{'Kind':'save'},{'Kind':'line'},{'Kind':'restore'},{'Kind':'advance','N':2}],
    [{'Kind':'line'},{'Kind':'reset'},{'Kind':'reset'}],
]
sources = [b'', b'a', b'abc', b'\n', b'\n\n', b'a\nb\nc', b' \t a\r\n\tZ', b' \n\t\nX', b'\v\fx', '😀é\n�!'.encode(), b'a\xffz', b'a\xe2\x82z', b'a\x80b']
for source_bytes in sources:
    for peek in [False,True]:
        for ops in sequences:
            requests.append({'Source':source_bytes.hex(),'PeekLine':peek,'Operations':ops})
for byte in range(256):
    for peek in [False,True]:
        requests.append({'Source':bytes([97,byte,120,10]).hex(),'PeekLine':peek,'Operations':[{'Kind':'rune'}]*5})
from markdown_closure_cases import closure_requests, code_span_requests
requests += closure_requests() + code_span_requests()
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/reader.json', requests, results)
print(f'{len(requests)} Goldmark reader requests captured')
