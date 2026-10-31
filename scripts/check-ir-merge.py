#!/usr/bin/env python3
"""Compare complete pinned merge.go overlays and substitution expansion.

Fixture generator for recipe `lang-ir-merge` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-merge.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
go = parser_oracle()
def name(s):return {'hex':s.encode().hex()}
def op(kind,**kw):return {'op':kind,'map':2,**kw}
def field(s,map=2):return op('append',map=map,name=name(s))
def shape(id,kind):
 if kind=='none':return []
 if kind.startswith('map'):return [op('field_map',id=id,child=kind=='map_one')]
 return [op('field_array',id=id,key='3/4' if kind=='array_scalar' else '',child=kind=='array_map')]
kinds=['none','map_empty','map_one','array_empty','array_scalar','array_map'];cases=[]
for indexed in [False,True]:
 for direct in [False,True]:
  for i,b in enumerate(kinds):
   for j,o in enumerate(kinds):
    ops=[op('base_nil'),field('A'),field('a',3),op('field_primary',id=2,key='"base"'),op('ref_field',id=2,index=0),op('ref_field',id=3,index=1)]
    ops+=shape(2,b)+shape(3,o)
    if (i+j)%2:ops.append(op('field_primary',id=3,key=['null','"x${v}z"','0x1.fp2'][j%3]))
    ops.append(op('merge_field',id=2,index=3,child=indexed) if direct else op('merge_map',id=3,child=indexed))
    cases.append(ops)
for indexed in [False,True]:
 for direct in [False,True]:
  for b in ['none','empty','one']:
   for o in ['none','empty','one']:
    ops=[op('base_nil'),op('append_edge',key='(a -> b)[0]'),op('append_edge',key='(a -> b)[1]'),op('append_edge',map=3,key='(b <- a)[*]'),op('edge_primary',id=0,key='"base"'),op('edge_primary',id=2,key='"overlay"'),op('ref_edge',id=0,index=0),op('ref_edge',id=2,index=1)]
    if b!='none':ops.append(op('edge_map',id=0,child=b=='one'))
    if o!='none':ops.append(op('edge_map',id=2,child=o=='one'))
    ops.append(op('merge_edge',id=0,index=2,child=indexed) if direct else op('merge_map',id=3,child=indexed));cases.append(ops)
for indexed in [False,True]:
 cases.append([op('base_nil'),field('left'),field('placeholder'),field('right'),field('x',3),field('right',3),field('y',3),field('left',3),op('field_primary',id=6,key='true'),op('field_primary',id=8,key='false'),op('ref_field',id=5,index=0),op('ref_field',id=6,index=1),op('append_edge',key='a -> b'),op('append_edge',map=3,key='a -> c'),op('expand',id=3,index=3,child=indexed)])
 cases.append([op('base_nil'),field('left'),field('right'),field('left',3),op('field_primary',id=4,key='true'),op('expand',id=3,index=4,child=indexed)])
 cases.append([op('base_nil'),field('A'),field('a',3),op('field_primary',id=2,key='false'),op('merge_map',id=3,child=indexed)])
 cases.append([field('a'),op('field_primary',id=2,key='"self"'),op('field_array',id=2,child=True),op('ref_field',id=2,index=0),op('merge_field',id=2,index=2,child=indexed)])
 cases.append([op('append_edge',key='a -> b'),op('edge_primary',id=0,key='true'),op('ref_edge',id=0,index=0),op('merge_edge',id=0,index=0,child=indexed)])
 cases.append([op('base_nil'),op('append_edge',key='a -> b'),op('append_edge',map=3,key='a -> c'),op('edge_map',id=1,child=True),op('ref_edge',id=1,index=0),op('merge_map',id=3,child=indexed)])
 cases.append([field('a'),op('field_map',id=2,child=True),op('ref_field',id=2,index=0),op('merge_map',id=2,child=indexed)])
requests=[{'mode':'ir_maps','hex':json.dumps({'operations':ops,'queries':[[name(n)]for n in ['a','A','left','placeholder','right','x','y','inside']],'edges':['a -> b','(a -> b)[0]','(a -> b)[1]','a -> c'],'detailed':True}).encode().hex()}for ops in cases]
reference = call(go, requests)
write(fixture('ir-merge.json'), {'requests': requests, 'reference': reference}, compact=True)
print(f'{len(requests)} IR merge requests captured')
