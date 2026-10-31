#!/usr/bin/env python3
"""IR AST reconstruction, formatter mutations and edge IDString output.

Fixture generator for recipe `lang-ir-ast` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-ast.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
go = parser_oracle()
requests=[]
for path in [['a'],['layers','one','x'],['steps','two','vars','x']]:
 for quoted in [False,True]:
  for edge in ['', '(STYLE -> İCON)[2]', '(a -- b)[*]', 'a <-> b']:
   requests.append({'mode':'ir_ast','parts':[p.encode().hex() for p in path],'in_key':quoted,'path':'origin-é.d2','options':edge,'hex':''})
for scalar in ["'x''y'",'"a${b}c"','true','false','null','0x1.fp2','3/4','|md\n  body\n|','|\n\u2000x\n|']:
 for quoted in [False,True]:requests.append({'mode':'ir_ast','parts':[p.encode().hex() for p in ['a','b']],'in_key':quoted,'path':'origin.d2','options':'(a -> b)[0]','hex':scalar.encode().hex()})
for endpoint in ['x','x"','x.a','x-','x-z','x--','x $','x\nnext']:
 requests.append({'mode':'ir_ast','parts':['61'],'in_key':False,'options':'|\n\u2000'+endpoint+'\n| -> b'})
reference = call(go, requests)
write(fixture('ir-ast.json'), {'requests': requests, 'reference': reference})
print(f'{len(requests)} IR AST requests captured')
