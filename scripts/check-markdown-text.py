#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-text-merge`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-text-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-text-merge`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
from references import ROOT
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-text-oracle/main.go'
module, _ = prepare_module('markdown-text-oracle', harness)
binary = build(module)
requests = []
def segment(start,stop,padding=0): return {'Start':start,'Stop':stop,'Padding':padding,'ForceNewline':True}
def text(seg,flags): return {'kind':'Text','segment':seg,'raw':bool(flags&1),'soft_line_break':bool(flags&2),'hard_line_break':bool(flags&4)}
code={'kind':'CodeSpan','children':[]}
for source in [b'abc',b'a\nc']:
    for left_flags in range(8):
        for right_flags in range(8):
            for padding in [0,1]:
                for start in [1,2]:
                    left=text(segment(0,2,2),left_flags);right=text(segment(start,3,padding),right_flags)
                    base={'Source':source.hex(),'Children':[left],'Index':0,'Right':right,'Segment':right['segment']}
                    requests.append({**base,'Action':'merge'})
                    requests.append({**base,'Action':'append','Right':code})
                    requests.append({**base,'Action':'replace','Right':code,'Children':[left,code],'Index':1})
for children in [[],[code]]:
    requests.append({'Source':'616263','Children':children,'Index':0,'Right':code,'Segment':segment(0,3),'Action':'append'})
requests.append({'Source':'616263','Children':[code,code],'Index':1,'Right':code,'Segment':segment(0,3),'Action':'replace'})
requests.append({'Source':'616263','Children':[text(segment(0,2),0)],'Index':0,'Right':code,'Segment':segment(0,3),'Action':'merge'})
requests=list({json.dumps(q,sort_keys=True):q for q in requests}.values())
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/text-merge.json', requests, results)
print(f'{len(requests)} Goldmark text merge cases captured')
