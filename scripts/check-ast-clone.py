#!/usr/bin/env python3
"""Compare pinned import-cache AST copy contents, identities and edit isolation.

Fixture generator for recipe `lang-ast-clone` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ast-clone.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
from language_parse_inputs import public_parse_inputs
from references import ROOT
from upstream_inputs import e2e_inputs
go = parser_oracle()
inputs = public_parse_inputs()
inputs += [p.read_bytes() for p in sorted((ROOT / 'tests/d2').glob('*.d2')) + e2e_inputs()]
inputs += [s.encode() for s in ['', 'a: [null; true; false; suspend; 3/4; 0x1.fp2; {} ; []]', 'a: "pre${b.c}post"', 'a: @../b.c', 'a.*.${var}.**: x', '(a -> b)[2].label: x', 'a: |\n\u2000x\n|']]
requests = [{'mode': 'ast_clone', 'hex': b.hex(), 'path': 'clone.d2'} for b in dict.fromkeys(inputs)]
reference = call(go, requests)
write(fixture('ast-clone.json'), {'requests': requests, 'reference': reference}, compact=True)
print(f'{len(requests)} AST clone requests captured')
