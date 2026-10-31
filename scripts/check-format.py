#!/usr/bin/env python3
"""Compare native AST formatting bytes and mutations to pinned D2.

Fixture generator for recipe `lang-format` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes format.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
from language_parse_inputs import public_parse_inputs
from references import ROOT
from upstream_inputs import e2e_inputs
go = parser_oracle()
inputs=public_parse_inputs()
inputs+= [p.read_bytes() for p in sorted((ROOT/'tests/d2').glob('*.d2'))+e2e_inputs()]
inputs+=[s.encode() for s in ['layers: {x: {a}}\na: b','layers: {}','layers: {x: {a}}','a: [x # hi\ny\n\n z]','a: "x${y}z"','a: {\n# c\nb: c\n\n d: e\n}','a: |md\n x||y\n|','a: null {}','STYLE: { STROKE: RED }','a: @../x/../y.d2.z']]
inline=call(go,None,'format-fixtures',str(upstream()/'d2format/format_test.go'))
inputs += [s.encode() for s in inline]
inputs=list(dict.fromkeys(inputs))
requests=[{'hex':data.hex(),'mode':'format','utf16':mode,'path':'input.d2'} for data in inputs for mode in [False,True]]
strings=["", "null", "NULL", "STYLE", "FİLL", "İCON", "DİRECTİON", "a--b", "a-b", "&a", "a&b", "a@b", "x${y}", "\"x", "a'b", "a\nb", "\t a \n b \n", "é😀", "|||", "x|éy", "xé|y", "a\\b"]+[chr(c) for c in range(128)]
for text in strings:
 for kind in ['single','double','unquoted','block']:
  for in_key in [False,True]:
   requests.append({'hex':text.encode().hex(),'mode':'format_'+kind,'utf16':False,'path':'','in_key':in_key,'options':'é' if kind=='block' else ''})
for text in ['a@b','a#b','a:b','a-b','null','x','a0_',"'$",'', 'ab\ncd']:
 for in_key in [False,True]:
  for specials in ['', 'x', 'a012', '#:$']:
   requests.append({'hex':text.encode().hex(),'mode':'format_unquoted','utf16':False,'path':'','in_key':in_key,'options':'','specials':specials})
reference = call(go, requests)
write(fixture('format.json'), {'requests': requests, 'reference': reference})
print(f'{len(requests)} format requests captured')
