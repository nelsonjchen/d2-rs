#!/usr/bin/env python3
"""Replay source-ordered IR map edits against pristine private lookup/mutation methods.

Fixture generator for recipe `lang-ir-maps` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-maps.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
go = parser_oracle()
def name(text, quoted=False):
    return {'hex': (text.encode() if isinstance(text, str) else text).hex(), 'quoted': quoted}
keywords = call(go, None, 'reserved-keywords')
pairs = [(name(k), name(k.upper())) for k in sorted(keywords)]
pairs += [(name(k), name(k, True)) for k in sorted(keywords)]
pairs += [(name(a), name(b)) for a, b in [('a', 'A'), ('K', 'K'), ('s', 'ſ'), ('FİLL', 'fill'), ('Σ', 'ς'), ('é', 'É'), ('', ''), (b'\xff', b'\xfe'), (b'\xe2\x82', b'\xff\xff')]]
requests = []
for a, b in pairs:
    operations = [
        {'op': 'append', 'name': a},
        {'op': 'insert', 'index': 0, 'name': b},
        {'op': 'append', 'name': a, 'child': True},
        {'op': 'rename', 'id': 2, 'name': name('renamed')},
        {'op': 'swap'},
        {'op': 'remove', 'index': 1},
        {'op': 'append', 'name': {'nil': True}},
        {'op': 'remove', 'index': 0},
        {'op': 'mark'},
        {'op': 'direct_append', 'name': name('renamed')},
        {'op': 'append_edge', 'key': '(a -> b)[0]'},
        {'op': 'append_edge', 'key': '(b <- a)[1]'},
        {'op': 'swap_edges'},
        {'op': 'reedge', 'id': 0, 'key': '(a -> c)[0]'},
        {'op': 'remove_edge', 'index': 0},
        {'op': 'append_edge', 'key': 'a -> b'},
        {'op': 'append_edge', 'key': 'K -> s'},
        {'op': 'append_edge', 'key': 'k -> ſ'},
        {'op': 'append_edge', 'key': ''},
        {'op': 'remove_edge', 'index': 4},
    ]
    for op in operations:
        op['map'] = 2
    queries = [[], [a], [b], [name('renamed')], [name('_')], [name('_'), a], [name('_', True)], [a, name('target')], [b, name('target')], [name('scope'), name('dive'), a], [name('dive'), b]]
    edges = ['a -> b', '(a -> b)[0]', '(a -> b)[1]', '(a -> b)[*]', 'b <- a', 'a -> c', 'k -> S', 'K -> ſ', 'a -- b', 'a <-> b']
    data = {'operations': operations, 'queries': queries, 'edges': edges}
    requests.append({'mode': 'ir_maps', 'hex': json.dumps(data).encode().hex()})
reference = call(go, requests)
write(fixture('ir-maps.json'), {'requests': requests, 'reference': reference}, compact=True)
print(f'{len(requests)} IR map requests captured')
