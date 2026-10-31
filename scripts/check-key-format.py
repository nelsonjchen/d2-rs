#!/usr/bin/env python3
"""Canonical identifier spelling with mixed AST quote forms and custom specials.

Fixture generator for recipe `lang-key-format` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes key-format.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, write
go = parser_oracle()
values=['','a','a_b','A0','_','null','NULL','nUlL','suspend','unsuspend','true','a b',' a','a ','a.b','a-b','a--b','a-','-a','a\\b','a\nb','a\rb','a\tb','a"b',"a'b","'a",'"a','|a','&a','$a','${a}','*','**','***','a: b','世界','é','\u00a0a','a\u2000','x\x00y','FİLL','İCON']
reserved=sorted(call(go,None,'reserved-keywords'))
values += [v for key in reserved for v in [key,key.upper()]]
values=[v.encode() for v in values]+[b'\xff',b'\xe2\x82',b' x\xff ',b'\xff--x']
requests=[{'mode':'format_keypath','parts':[],'specials':None}]
for a in values:
 for b in values:requests.append({'mode':'format_keypath','parts':[a.hex(),b.hex(),a.hex()],'specials':None})
for specials in ['#;\n\\{}[]\'"|:.-<>*&()@&x','#;\n\\{}[]\'"|:.-<>*&()@&N','#;\n\\{}[]\'"|:.-<>*&()@&_','a012','','�']:
 for parts in [[b'x',b'NULL',b'a_b',b'A0'],[b'\xff',b'\xe2\x82',b'NULL'],[b'a--b',b'@x',b'&a']]:
  requests.append({'mode':'format_keypath','parts':[p.hex() for p in parts],'specials':specials})
reference=call(go,requests)
write(fixture('key-format.json'),{'requests':requests,'reference':reference})
print(f'{len(requests)} key-path formatting requests captured')
