#!/usr/bin/env python3
"""Resolved edge queries/deletions, reference-linked removal and board-base copies.

Fixture generator for recipe `lang-ir-deletion` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-deletion.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
go = parser_oracle()
def name(s,quoted=False):return {'hex':s.encode().hex(),'quoted':quoted}
def op(kind,**kw):return {'op':kind,'map':2,**kw}
def append(s,**kw):return op('append',name=name(s),**kw)
queries=[[name(s)]for s in ['scope','dive','style','layers','LAYERS','scenarios','steps','a','b','gone']]+[[name('style'),name('target')]]
edge_queries=['a -> b','(a -> b)[0]','(a -> b)[1]','b <- a','scope.a -> scope.b','scope.dive.a -> scope.dive.b','dive.a -> dive.b','_.a -> _.b','_._.a -> _._.b','_.a -> b','a -> scope.b','c -> d','e -> f','g -> h']
cases=[]
setup=[op('append_edge',map=m,key=f'(a -> b)[{i}]') for m in range(3) for i in range(2)]+[op('append_edge',map=0,key='a -> scope.b'),op('append_edge',map=1,key='a -> dive.b')]
for m,key in [(0,'scope.dive.a -> scope.dive.b'),(0,'scope.a -> scope.b'),(2,'_._.a -> _._.b'),(1,'_.a -> b'),(2,'_.a -> b'),(0,'_.a -> b'),(0,'missing.a -> missing.b'),(2,'(b <- a)[1]'),(2,'')]:
 cases.append(setup+[op('delete_edge',map=m,key=key),op('delete_edge',map=m,key=key)])
for board in [None,'layers','scenarios','steps','LAYERS']:
 for repeated in [False,True]:
  for nil in [False,True]:
   ops=[append('gone')]+[op('append_edge',map=0,key=k)for k in ['a -> b','c -> d','e -> f','g -> h']]
   if board:ops.append(op('rename',id=0,name=name(board)))
   ops.append(op('ref_field',id=2,index=7,name={'nil':nil}))
   if repeated:ops.append(op('ref_field',id=2,index=7,name={'nil':nil}))
   ops += [op('ref_edge',id=i,index=7,name={'nil':nil})for i in range(4)]
   ops.append(op('delete_field',path=['gone']));cases.append(ops)
cases.append([append('gone'),op('ref_field',id=2,index=0),op('append_edge',key='a -> b'),op('ref_edge',id=0,index=0),op('replace_context',index=0),op('append_edge',key='c -> d'),op('ref_edge',id=1,index=0),op('delete_field',path=['gone'])])
for holder in ['style','STYLE','other']:
 for quoted in [False,True]:
  cases.append([op('append',name=name(holder,quoted),child=True),op('delete_field',path=[holder,'target'])])
cases.append([append('style',child=True),append('style',child=True),op('delete_field',map=4,path=['target'])])
for parent in [False,True]:
 ops=[append('steps'),append('a'),op('append',name=name('LAYERS',True)),append('scenarios'),append('layers'),append('b')]
 ops += [op('append_edge',key=k)for k in ['a -> b','c -> d','e -> f']]
 ops += [op('ref_field',id=4,index=0),op('ref_field',id=6,index=1),op('ref_edge',id=0,index=0),op('ref_edge',id=1,index=1),op('ref_edge',id=2,index=0)]
 ops += [op('base',child=parent),op('base',child=parent),op('base_nil',child=parent)]
 cases.append(ops)
requests=[]
for ops in cases:
 for operation in ops:
  if 'path'in operation:operation['path']=[p.encode().hex()for p in operation['path']]
 data={'operations':ops,'queries':queries,'edges':edge_queries,'detailed':True}
 requests.append({'mode':'ir_maps','hex':json.dumps(data).encode().hex()})
reference = call(go, requests)
write(fixture('ir-deletion.json'), {'requests': requests, 'reference': reference}, compact=True)
print(f'{len(requests)} IR deletion requests captured')
