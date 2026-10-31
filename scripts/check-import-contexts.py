#!/usr/bin/env python3
"""Compare import reference/glob graph copying and subsequent mutation isolation.

Fixture generator for recipe `lang-import-contexts` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes import-contexts.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
go = parser_oracle()
requests=[{'mode':'ir_import','parts':[p.encode().hex() for p in parts],'in_key':quoted,'hex':'','options':'(a -> b)[2]','path':f'{glob}:{mode}:{target}'}for parts in [[],['a'],['layers','one']]for quoted in [False,True]for glob in ['none','empty','nil','shared']for mode in ['ordinary','import']for target in ['map','field']]
reference = call(go, requests)
write(fixture('import-contexts.json'), {'requests': requests, 'reference': reference}, compact=True)
print(f'{len(requests)} import context requests captured')
