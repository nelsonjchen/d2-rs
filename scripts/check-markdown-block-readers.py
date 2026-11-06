#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-block-reader`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-reader-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-block-reader`; `cargo test -p d2-rs-text --features oracle-fixtures`
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
    [{'Kind':'advance','N':1}]*8,
    [{'Kind':'rune'}]*8,
    [{'Kind':'line'}]*4,
    [{'Kind':'eol'},{'Kind':'advance','N':1}]*3,
    [{'Kind':'spaces'}]*3,
    [{'Kind':'blank'},{'Kind':'line'}]*3,
    [{'Kind':'padding','Padding':2},{'Kind':'advance','N':1},{'Kind':'advance','N':2}],
    [{'Kind':'advance-padding','N':1,'Padding':2},{'Kind':'spaces'}],
    [{'Kind':'save'},{'Kind':'line'},{'Kind':'restore'},{'Kind':'advance','N':1}],
    [{'Kind':'line'},{'Kind':'reset'},{'Kind':'rune'}],
]
def segment(start,stop,padding=0,newline=False):
    return {'Start':start,'Stop':stop,'Padding':padding,'ForceNewline':newline}
sources = [b'a\nb\nc\n', b'> a\n> b\n', b'\tA\n  Z!', '😀x\néz'.encode(), b'a\xffb\nc', b'a\x80b', b'  \n  X', b'']
for source_bytes in sources:
    n=len(source_bytes)
    sets=[[],[segment(0,n)]]
    if n>=3:
        sets += [[segment(0,2),segment(3,n)], [segment(1,3,2),segment(3,n,1,True)], [segment(0,0),segment(0,n)], [segment(0,1,0,True),segment(2,n,0,True)]]
    for segments in sets:
        for peek in [False,True]:
            for ops in sequences:
                requests.append({'Source':source_bytes.hex(),'Block':True,'Segments':segments,'PeekLine':peek,'Operations':ops})
        if segments:
            for start in [segments[0]['Start'],segments[-1]['Start']]:
                for stop in sorted({start,n}):
                    requests.append({'Source':source_bytes.hex(),'Block':True,'Segments':segments,'PeekLine':True,'Operations':[{'Kind':'value','Segment':segment(start,stop,3,True)}]})
results = []
completed_requests = []
timeouts = []
for request in requests:
    try:
        output = subprocess.run([str(binary)], input=json.dumps([request]).encode(), stdout=subprocess.PIPE, check=True, timeout=0.5)
    except subprocess.TimeoutExpired:
        timeouts.append(request)
        continue
    result = json.loads(output.stdout)
    assert len(result) == 1
    completed_requests.append(request)
    results.append(result[0])
from markdown_closure_cases import closure_requests, code_span_requests
closure_cases = closure_requests(block=True) + code_span_requests(block=True)
closure_results = json.loads(subprocess.run([str(binary)], input=json.dumps(closure_cases).encode(), stdout=subprocess.PIPE, check=True, timeout=30).stdout)
assert len(closure_results) == len(closure_cases)
completed_requests += closure_cases
results += closure_results
requests = completed_requests
# Oracle timeouts are not parity passes: keep their complete requests in the
# fixture (`oracle_timeouts`) so the exclusion stays visible.
write_request_fixture('crates/d2-rs-text/tests/fixtures/block-reader.json', requests, results,
                      oracle_timeouts=timeouts, timeout_seconds=0.5)
if timeouts:
    print(f'{len(timeouts)} block-reader requests timed out after 0.5 s (kept in oracle_timeouts)')
print(f'{len(requests)} Goldmark reader requests captured')
