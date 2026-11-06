#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-context`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-context-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-context`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
from references import ROOT, digest
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-context-oracle/main.go'
module, trace_dep = prepare_module('markdown-context-oracle', harness, copy_goldmark=True)
original=trace_dep/'ast/inline.go'
original_hash=digest(original)
instrumented=original.read_text()
marker='func NewTextSegment(v textm.Segment) *Text {'
assert instrumented.count(marker)==1
instrumented=instrumented.replace(marker,'func oracleOriginalNewTextSegment(v textm.Segment) *Text {')
instrumented+='\nvar OracleNewTextHook func(Node)\nfunc NewTextSegment(v textm.Segment) *Text {n:=oracleOriginalNewTextSegment(v);if OracleNewTextHook!=nil {OracleNewTextHook(n)};return n}\n'
patched=module/'inline.go.txt';patched.write_text(instrumented)
binary = build(module, overlay={str(original): str(patched)})
from itertools import product,permutations
requests=[]
def segment(start,stop):return {'Start':start,'Stop':stop,'Padding':1,'ForceNewline':True}
def text_node(start,stop,soft):return {'kind':'Text','segment':segment(start,stop),'raw':True,'soft_line_break':soft,'hard_line_break':True}
def delimiter(start,length):return {'kind':'Delimiter','segment':segment(start,start+length),'can_open':True,'can_close':True,'length':length,'original_length':max(1,length),'char':42,'previous_delimiter':None,'next_delimiter':None}
for a,b,c,soft,gap in product([0,1,2],[0,2],[0,3],[False,True],[0,1]):
    nodes=[{'kind':'Container'},text_node(0,2-gap,soft),delimiter(2,a),text_node(4,6-gap,soft),delimiter(6,b),text_node(8,10-gap,soft),delimiter(10,c),text_node(12,13,False)]
    sequences=[[{'Kind':'remove','ID':i},{'Kind':'clear','Bottom':None}] for i in [2,4,6]]
    sequences += [[{'Kind':'clear','Bottom':bottom},{'Kind':'clear','Bottom':None}] for bottom in [None,1,2,3,4,6]]
    sequences += [[{'Kind':'remove','ID':i} for i in order] for order in permutations([2,4,6])]
    for ops in sequences:requests.append({'Nodes':nodes,'Parents':[None,0,0,0,0,0,0,0],'Operations':ops})
# Delimiters in nested parents exercise AST-sibling versus delimiter-list walks.
for length,soft,bottom in product([0,1,2],[False,True],[None,1,2,3,4,5]):
    nodes=[{'kind':'Container'},{'kind':'Container'},delimiter(0,length),text_node(2,4,soft),delimiter(4,length),delimiter(6,length)]
    requests.append({'Nodes':nodes,'Parents':[None,0,1,1,0,0],'Operations':[{'Kind':'clear','Bottom':bottom},{'Kind':'clear','Bottom':None}]})
context_only_cases=len(requests)
def process_case(lengths,chars,flags,bottom=None):
    nodes=[{'kind':'Container'}];parents=[None];offset=0
    for length,char,(can_open,can_close) in zip(lengths,chars,flags):
        d=delimiter(offset,length);d['char']=ord(char);d['can_open']=can_open;d['can_close']=can_close
        nodes.append(d);parents.append(0);offset+=length
        nodes.append(text_node(offset,offset+1,False));parents.append(0);offset+=1
    requests.append({'Nodes':nodes,'Parents':parents,'Operations':[{'Kind':'process','Bottom':bottom},{'Kind':'process','Bottom':None}]})
for left,right,char,can_close,can_open in product(range(1,7),range(1,7),'*_~',[False,True],[False,True]):
    process_case([left,right],char*2,[(True,can_close),(can_open,True)])
for lengths,chars,middle,bottom in product([(1,1,1),(1,2,3),(2,1,2),(2,2,2),(3,3,3),(3,1,2),(4,2,4)],['***','___','~~~','*_*','*~*','_*_','~*~'],[(True,True),(True,False),(False,True)],[None,1,2,3,5]):
    process_case(lengths,chars,[(True,False),middle,(False,True)],bottom)
# Mixed longer runs exercise nested construction and unmatched cleanup.
state=0x753da1
def draw(n):
    global state
    state^=(state<<13)&0xffffffff;state^=state>>17;state^=(state<<5)&0xffffffff
    return state%n
for _ in range(128):
    process_case([1+draw(5) for _ in range(5)],''.join('*_~'[draw(3)] for _ in range(5)),[(bool(draw(2)),bool(draw(2))) for _ in range(5)])
import copy
typed_nil_requests=[]
for q in requests:
    if any(op['Kind']=='process' for op in q['Operations']):
        q=copy.deepcopy(q)
        q['Operations'][0]['TypedNil']=True
        typed_nil_requests.append(q)
requests.extend(typed_nil_requests)
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/context.json', requests, results)
assert digest(original)==original_hash
print(f'{len(requests)} Goldmark delimiter-context sequences captured')
