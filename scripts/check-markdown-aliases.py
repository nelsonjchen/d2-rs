#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-aliases`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-alias-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-aliases`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
from references import ROOT
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-alias-oracle/main.go'
module, _ = prepare_module('markdown-alias-oracle', harness)
binary = build(module)
requests = []
for size in [0,1,2,3,7,8,16,31,255,256,511,512,1024,32760,32768]:
    data=(b'abcd'*((size+3)//4))[:size]
    for extra in [0,1,5]:
        for stop in sorted({size,max(0,size-1)}):
            for padding in [0,2]:
                for newline in [False,True]:
                    requests.append({'Source':data.hex() if size<=2048 else '', 'Size':size if size>2048 else 0,'Capacity':size+extra,'Segment':{'Start':0,'Stop':stop,'Padding':padding,'ForceNewline':newline}})
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/aliases.json', requests, results)
print(f'{len(requests)} Goldmark shared-source cases captured')
