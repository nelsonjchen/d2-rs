#!/usr/bin/env python3
"""IR reference primary/index/AST selection, context equality and lazy dedup.

Fixture generator for recipe `lang-ir-references` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-references.json to the
staged output. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, fixture, parser_oracle, write

go = parser_oracle()
keys=['a.b: x','a.b: y','a.b: suspend','a.b: unsuspend','a.b','a -> b','a -> b -> c','(a -> b).style.fill: red','a.(b -> c).style: {fill: red}','*: suspend','*: unsuspend','*: null','*: true','*: [a]','*: [b]','(a -> b)[2].label: hi','a.b: label {shape: circle}','(a -> b -> c).style.fill: red']
parts=[k.encode().hex() for k in keys];requests=[{'mode':'ir_refs','hex':p,'parts':parts}for p in parts]
reference = call(go, requests)
assert len(reference) == len(requests)
write(fixture('ir-references.json'), {"requests": requests, "reference": reference})
print(f"{len(requests)} ir-references.json requests captured")
