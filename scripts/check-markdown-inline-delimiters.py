#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-inline-delimiter`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-inline-delimiter-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-inline-delimiter`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
from references import ROOT
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-inline-delimiter-oracle/main.go'
module, _ = prepare_module('markdown-inline-delimiter-oracle', harness)
binary = build(module)
from itertools import product
requests=[]
for prefix,marker,length,suffix,kind,padding in product([b'',b' ',b'a',b'!',b'~','😀'.encode(),b'\n'],[b'*',b'_',b'~',b'!'],range(1,5),[b'x',b' ',b'!',b'\xff'],['emphasis','strike'],[0,2]):
    data=prefix+marker*length+suffix
    requests.append({'Source':data.hex(),'Segments':[{'Start':0,'Stop':len(data),'Padding':padding,'ForceNewline':True}],'Calls':[{'Kind':kind,'Advance':len(prefix)+padding}]})
for data,calls in [
    (b'*x_y~z~',[('emphasis',0),('emphasis',1),('strike',1),('strike',1)]),
    (b'~~x~~',[('strike',0),('strike',1)]),
    (b'~~~x',[('strike',0),('strike',1)]),
    (b'__x**',[('emphasis',0),('emphasis',1)]),
    (b'~x~y*z*',[('strike',0),('strike',1),('emphasis',1),('emphasis',1)]),
]:
    requests.append({'Source':data.hex(),'Segments':[{'Start':0,'Stop':len(data),'Padding':0,'ForceNewline':False}],'Calls':[{'Kind':kind,'Advance':advance} for kind,advance in calls]})
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/inline-delimiter.json', requests, results)
print(f'{len(requests)} Goldmark inline delimiter cases captured')
