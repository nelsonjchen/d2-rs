#!/usr/bin/env python3
"""IR NewEdgeIDs, Copy aliasing and Match, including reversed arrows and Unicode folding.

Fixture generator for recipe `lang-ir-edges` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-edges.json to the
staged output. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, fixture, parser_oracle, write

go = parser_oracle()
keys=['a -- b','b -- a','A -- B','a -> b','b <- a','b -> a','a <- b','a <-> b','b <-> a','(a -> b)[0]','(a -> b)[1]','(a -> b)[*]','a --* b','a.K -> b','a.K -> b','ß -> x','SS -> x','ẞ -> x','ſ -> x','S -> x','İ -> x','i -> x','a -> b -> c','a','"A" -> b','a.(b -> c)[2].style: {fill: red}']
parts=[k.encode().hex() for k in keys]
requests=[{'mode':'ir_edges','hex':key,'parts':parts} for key in parts]
values=[s.encode() for s in ['', 'a','A','ß','ẞ','SS','s','S','ſ','K','k','K','İ','i','I','ı','Σ','σ','ς','世界','é','É']]+[b'\xff',b'\xfe',b'\xe2\x82',b'\xef\xbf\xbd',b'\xff\xff']
for a in values:
 for b in values:requests.append({'mode':'ir_edge_match','parts':[a.hex(),b'endpoint'.hex(),b.hex(),b'endpoint'.hex()]})
reference = call(go, requests)
assert len(reference) == len(requests)
write(fixture('ir-edges.json'), {"requests": requests, "reference": reference})
print(f"{len(requests)} ir-edges.json requests captured")
