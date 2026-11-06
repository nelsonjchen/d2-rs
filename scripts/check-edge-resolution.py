#!/usr/bin/env python3
"""Direct relative-edge resolution, including identities and upstream failures.

Fixture generator for recipe `lang-edge-resolution` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes edge-resolution.json to the
staged output. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, fixture, parser_oracle, write

go = parser_oracle()
keys=['a -> b','_.a -> b','a -> _.b','_._.a -> _.b','_._._.a -> b','_ -> _','_._ -> _._','_ -> _.a','_ -> _._.a','"_".a -> _.b','a.b -> A.c','*.a -> *.b','"*".a -> "*".b','ſ.x -> s.y','(_._.a <-> _._.b)[3]','(a.b -> a.c)[*]','|\n\u2000x\n|.a -> |\n\u2000x\n|.b']
requests=[{'mode':'ir_resolve','parts':[p.encode().hex() for p in ['scope','dive','inner'][:depth]],'in_key':quoted,'hex':key.encode().hex()} for depth in range(4) for quoted in [False,True] for key in keys]
reference = call(go, requests)
assert len(reference) == len(requests)
write(fixture('edge-resolution.json'), {"requests": requests, "reference": reference}, compact=True)
print(f"{len(requests)} edge-resolution.json requests captured")
