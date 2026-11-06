#!/usr/bin/env python3
"""Compare native byte-input API against pristine d2parser.Parse.

Fixture generator for recipe `lang-public-parse` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes public-parse.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, write
from language_parse_inputs import public_parse_requests
go = parser_oracle()
requests=public_parse_requests()
reference=call(go,requests)
write(fixture('public-parse.json'),{'requests':requests,'reference':reference})
print(f'{len(requests)} public Parse requests captured')
