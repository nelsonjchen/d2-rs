#!/usr/bin/env python3
"""IR tree parent/primary/map navigation, board kinds, vars and root reference.

Fixture generator for recipe `lang-ir-tree` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-tree.json to the
staged output. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, fixture, parser_oracle, write

go = parser_oracle()
paths=[[],['a'],['a','b'],['vars','x'],['Vars','x'],['layers','one','vars','x'],['Layers','one','x'],['scenarios','two','x'],['steps','three','vars'],['foo','scenarios','one','x'],['layers','vars','x'],['vars','layers','one','x'],['layers','one','steps','two','x']]
requests=[{'mode':'ir_tree','parts':[p.encode().hex() for p in path],'in_key':quoted}for path in paths for quoted in [False,True]]
reference = call(go, requests)
assert len(reference) == len(requests)
write(fixture('ir-tree.json'), {"requests": requests, "reference": reference})
print(f"{len(requests)} ir-tree.json requests captured")
