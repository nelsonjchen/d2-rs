#!/usr/bin/env python3
"""Qualify glob matching, traversal and identity paths against pinned Go.

Fixture generator for recipe `lang-ir-globs` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes ir-pattern.json and ir-globs.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
import re
go = parser_oracle()


def capture(name, requests):
    reference = call(go, requests)
    write(fixture(f'{name}.json'), dict(requests=requests, reference=reference), compact=True)
    print(f'{name}: {len(requests)} requests captured')

texts = [b'', b'a', b'ab', b'aba', b'ababa', b'abcabc', b'aaabbbccc', b'*',
         b'foo.bar', b'ABC', b'fooBAR', b'\xff', b'a\xffb', b'\xc0\x80',
         'İ'.encode(), 'i'.encode(), 'K'.encode(), b'K', 'Σςσ'.encode(),
         'ß'.encode(), 'ẞ'.encode(), 'Éx'.encode(), 'e\u0301x'.encode()]
# Exercise the exact reserved inventory, both reserved and case-variant spelling.
keywords = call(go, None, 'reserved-keywords')
texts += [s.encode() for key in sorted(keywords) for s in (key, key.upper(), key.title())]
patterns = [[], [b''], [b'*'], [b'a'], [b'A'], [b'ab'], [b'*', b'a'],
            [b'a', b'*'], [b'*', b'a', b'*', b'b'], [b'*', b'ab', b'*', b'a'],
            [b'*', b'', b'*'], [b'*', b'', b'*', b'', b'*'],
            [b'a', b'*', b'b', b'*', b'c'], [b'*', b'*'], [b'', b''],
            [b'*', b'\xff', b'*'], [b'\xff'], ['İ'.encode()], ['K'.encode()],
            [b'*', 'İ'.encode(), b'*'], [b'*', 'K'.encode(), b'*']]
texts += [x.encode() for x in ['style','Style','STYLE','vars','Vars','classes','Classes',
          'layers','Layers','scenarios','steps','shape','label','near','width','height']]
requests = [dict(mode='ir_pattern', hex=t.hex(), parts=[p.hex() for p in pattern])
            for t in texts for pattern in patterns]
# Import every anchor case from the pinned internal test table, retaining names.
anchor_source = (upstream() / 'd2ir/pattern_internal_test.go').read_text()
anchors = re.findall(r'\{name: ("[^"\n]*"), text: ("[^"\n]*"), pattern: \[\]string\{([^}]*)\}, want: (true|false)\}', anchor_source)
assert len(anchors) == 10, 'Upstream anchor table changed'
for name, text, parts, want in anchors:
    requests.append(dict(mode='ir_pattern', name=json.loads(name),
                         hex=json.loads(text).encode().hex(),
                         parts=[part.encode().hex() for part in json.loads('[' + parts + ']')]))
capture('ir-pattern', requests)

paths = [[], ['a'], ['a','b'], ['style','x'], ['Style','x'], ['classes','x'],
         ['vars','x'], ['Vars','x'], ['layers','one','x'], ['Layers','one','x'],
         ['scenarios','two','x'], ['steps','three','vars'], ['shape','x'],
         ['a','style','x'], ['a','classes','x'], ['a','vars','x'],
         ['a','layers','one','x'], ['layers','vars','x'],
         ['layers','one','steps','two','x'], ['layers','one','classes','x']]
requests = [dict(mode='ir_glob', parts=[p.encode().hex() for p in path],
                 in_key=quoted, options=edge, path='fixture.d2')
            for path in paths for quoted in [False, True]
            for edge in ['', '(a -> b)[2]']]
capture('ir-globs', requests)
