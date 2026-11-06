#!/usr/bin/env python3
"""Ordinary IR copies: AST isolation, retained references, ownership and counters.

Fixture generator for recipe `lang-ir-copy` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-copy.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
go = parser_oracle()
requests = []
for parts in [[], ['a'], ['layers', 'one', 'x'], ['vars', 'a']]:
    for quoted in [False, True]:
        for scalar in ['', '"x${a.b}z"', '3/4', '|\n\u2000x\n|']:
            requests.append({'mode': 'ir_copy', 'parts': [p.encode().hex() for p in parts], 'in_key': quoted, 'hex': scalar.encode().hex(), 'options': '(a -> b)[3]', 'path': 'copy.d2'})
reference = call(go, requests)
write(fixture('ir-copy.json'), {'requests': requests, 'reference': reference}, compact=True)
print(f'{len(requests)} IR copy requests captured')
