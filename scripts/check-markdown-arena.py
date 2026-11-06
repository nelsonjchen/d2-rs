#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-arena`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-arena-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-arena`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
from references import ROOT
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-arena-oracle/main.go'
module, _ = prepare_module('markdown-arena-oracle', harness)
binary = build(module)
requests=[]
for kind in ['append','remove','clear','before','after','replace']:
    for parent in [0,1]:
        for target in ([None,2,3,4,5,6,7] if kind=='before' else [2,3,4,5,6,7]):
            for child in range(2,8):
                op={'Kind':kind,'Parent':parent,'Target':target,'Child':child}
                requests.append([op,{'Kind':'append','Parent':1-parent,'Target':None,'Child':child}])
# Mixed sequences retain original detached-node identities across later moves.
state=0x927acd
kinds=['append','remove','clear','before','after','replace']
def draw(n):
    global state
    state ^= (state<<13)&0xffffffff;state ^= state>>17;state ^= (state<<5)&0xffffffff
    return state%n
for _ in range(128):
    ops=[]
    for _ in range(16):
        kind=kinds[draw(len(kinds))]
        target=2+draw(6)
        if kind=='before' and draw(4)==0:target=None
        ops.append({'Kind':kind,'Parent':draw(2),'Target':target,'Child':2+draw(6)})
    requests.append(ops)
requests=list({json.dumps(q,sort_keys=True):q for q in requests}.values())
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/arena.json', requests, results)
print(f'{len(requests)} Goldmark node-mutation sequences captured')
