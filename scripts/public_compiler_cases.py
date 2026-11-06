"""Request generators for the public d2compiler fixtures (tools/fixtures/langlibelk-language-public.toml).

Recovered from the 2026-10-16 probes (.reference/link-security-validation-20261016
prepare-matrix.py/prepare-extra.py, graph-measurement-state-20261016
prepare-matrix.py) and, where only the request lists survived, rewritten as
the small matrices those lists enumerate. Every request is
{hex, path, utf16, files_hex[, measured, original_index]}; each source text is
emitted with UTF16Pos false then true.
"""
from __future__ import annotations

import itertools
import json

# Two upstream d2compiler TestCompile sources the v0.9.0 probes kept as
# witnesses. Their file names keep the v0.9.0 test positions they were mined
# from; the name is only a label that appears in diagnostics.
UPSTREAM_LINK_WITNESSES = [
    ('pinned/d2compiler/compile_test.go#L2088C12-field-text.d2', 'x: {link: https://not-google.com; tooltip: https://google.com}'),
    ('pinned/d2compiler/compile_test.go#L2114C12-field-text.d2', 'x -> y: https://google.com {link: https://not-google.com }'),
]
UPSTREAM_CLASS_WITNESSES = [
    ('pinned/d2compiler/compile_test.go#L54C21-reader.d2', 'classes: { x: { class: x } }\na.class: x'),
    ('pinned/d2compiler/compile_test.go#L3448C10-field-text.d2',
     'classes: {\n  dragon_ball: {\n    label: ""\n    shape: circle\n    style.fill: orange\n  }\n  path: {\n'
     '    label: "then"\n    style.stroke-width: 4\n  }\n}\nnostar: { class: [dragon_ball, path] }'),
]


def _both(text: str, path: str, **extra) -> list[dict]:
    return [dict(hex=text.encode().hex(), path=path, utf16=utf16, files_hex={}, **extra) for utf16 in (False, True)]


def _witness(text: str, path: str) -> list[dict]:
    # Key order of the mined-corpus rows these came from.
    return [dict(hex=text.encode().hex(), path=path, files_hex={}, utf16=utf16) for utf16 in (False, True)]


def link_security() -> list[dict]:
    """Label/tooltip/link security product, identity/early-return controls, upstream witnesses."""
    combinations = [['label: "https://example.com/x"', 'tooltip: plain', 'link: target'],
                    ['label: plain', 'tooltip: "https://example.com/x"', 'link: target'],
                    ['label: "https://a/x"', 'tooltip: "https://b/x"', 'link: target'],
                    ['label: "mailto:a@b"', 'tooltip: "//host/x"', 'link: target'],
                    ['label: "http://[bad]/x"', 'tooltip: "https://h/x"', 'link: ""'],
                    ['label: "https://h/x"', 'tooltip: "<&x>"', 'link: target'],
                    ['label: "https://h/x"', 'tooltip: "https://b/x"'],
                    ['label: "http://h/x#%ZZ"', 'tooltip: "http://h/x#%ZZ"', 'link: target']]
    orders = [lambda v: v, lambda v: list(reversed(v)), lambda v: v[1:] + v[:1]]
    suffixes = ['', '; width: invalid; class: missing', '; shape: invalid; direction: invalid; near: ".."',
                '; style.opacity: -1; height: 42', '; icon: "%ZZ" { near: invalid; style.opacity: -1 }; top: 0',
                '; icon: okay {style}; left: 0', '; label: plain; tooltip: plain; grid-gap: 2',
                '; constraint: 3; grid-rows: 0; grid-columns: 2']
    contexts = [lambda b: 'x: {' + b + '}', lambda b: 'x: "https://initial/x" {' + b + '}', lambda b: b,
                lambda b: 'a -> b: "https://initial/x" {' + b + '}',
                lambda b: 'a -> b: {source-arrowhead: "https://initial/x" {' + b + '}}',
                lambda b: 'a -> b: {target-arrowhead: {' + b + '}}',
                lambda b: 'classes: { C: {' + b + '}}; x: {class:C}',
                lambda b: 'classes: { C: {' + b + '}}; a -> b: {class:C}',
                lambda b: 'classes: { C: {' + b + '}}; x: {class:C; label:plain; tooltip:plain}',
                lambda b: 'layers: {L: {x: {' + b + '}}}', lambda b: 'vars: {d2-legend: {x: {' + b + '}}}',
                lambda b: 's: {shape:sequence_diagram; actor; group: {actor: {' + b + '}}}']
    requests = []
    for combo, order, suffix, context in itertools.product(combinations, orders, suffixes, contexts):
        requests += _both(context('; '.join(order(combo)) + suffix), 'security.d2')
    for text in ['classes:{A:{link:target};B:{label:"https://h/x"}};x:{class:[A;B;A];width:1}',
                 'classes:{A:{source-arrowhead:{link:target}};B:{source-arrowhead:{label:"https://h/x"}}};a->b:{class:[A;B];source-arrowhead:{width:1}}',
                 's:{shape:sequence_diagram;actor:{link:target};group:{actor:{label:"https://h/x";width:1}}}',
                 'x:{link:target}; X:{label:"https://h/x";width:1}',
                 'x:{label:"https://h/x";link:target; WIDTH: 1; HEIGHT: 2}',
                 'x:{label:"https://h/x";link:target; label:{near:invalid};width:1}',
                 'a->b:{source-arrowhead: "https://h/x" {link:target};source-arrowhead.width:1}']:
        requests += _both(text, 'security.d2')
    for text in ['"https://h/x":{link:target;width:1}', 'a->shape:{label:"https://h/x";link:target}',
                 'classes:{C:{source-arrowhead:{label:"https://h/x";link:target}}};a->b:{class:C;source-arrowhead:plain;source-arrowhead.width:1}',
                 'classes:{C:{source-arrowhead:{link:target}}};a->b:{class:C;source-arrowhead:"https://h/x";source-arrowhead.width:1}',
                 'classes:{C:{link:target}};s:{shape:sequence_diagram;actor:{class:C};group:{actor:{label:"https://h/x";width:1}}}',
                 'classes:{C:{label:"https://h/x";link:target}};x:{class:C;label:null;tooltip:"https://h/x"}',
                 'x:{label:"https://h/x";link:target; WIDTH: invalid}', 'x:{label:"https://h/x";link:target; LABEL: plain}',
                 'x:{label:"https://h/x";link:target; tooltip:{near:invalid}}',
                 'x:{label:"https://h/x";link:target; icon:okay {style};width:2}',
                 'x:{label:"https://h/x";link:target; icon:okay {style:bad};width:2}',
                 'x:{label:"https://h/x";link:target; icon:"%ZZ" {near:invalid;style.opacity:-1};width:2}',
                 'label:"https://h/x";link:target;layers:{L:{label:plain;width:1}}',
                 's:{shape:sequence_diagram; group:{ actor->other }; actor:{label:"https://h/x";link:target}; group.actor:{width:1}}',
                 's:{shape:class;row:{label:"https://h/x";link:target;constraint:3}}',
                 's:{shape:sql_table;row:{label:"https://h/x";link:target;constraint:3}}',
                 'a->b:{label:"https://h/x";link:target;source-arrowhead:{width:1};width:2}',
                 'a->b:{source-arrowhead:{label:"https://h/x";link:target};target-arrowhead:{width:1};width:2}',
                 'classes:{A:{source-arrowhead:{label:"https://h/x";link:target}};B:{source-arrowhead:plain}};a->b:{class:[A;B];source-arrowhead:{width:1}}',
                 'x:{label:"https://h/x";link:target;Shape:wrong;vars:unused;constraint:[];width:1}']:
        requests += _both(text, 'extra.d2')
    for path, text in UPSTREAM_LINK_WITNESSES:
        requests += _witness(text, path)
    return requests


def class_applications() -> list[dict]:
    """Class application order, quoting, list separators, cycles and label languages."""
    requests = []
    for path, text in UPSTREAM_CLASS_WITNESSES:
        requests += _witness(text, path)
    two = 'classes: {a: {style.stroke: red}; b: {style.stroke: blue}}; '
    for text in [two + 'x.class: [a; b; a]', two + 'x -> y: {class: [a; b; a]}',
                 'classes: {a: {shape: circle}; b: {shape: square}}; x.class: [a; b; a]',
                 'classes: {a: {style.stroke: red}}; x.class: "a"', 'classes: {a: {style.stroke: red}}; x.class: ["a"]',
                 'classes: {a: {class: a}}; x.class: a', 'classes: {a: {class: b}; b: {class: a}}; x.class: a',
                 'classes: {a: {class: a}}', 'classes: {a: {class: a}}; x -> y: {class: a}',
                 'x.class: [{z}]', 'x.class: [[a]]', 'classes: {a: {style.opacity: 2}}; x.class: [a; {z}; a]',
                 two + 'x.class: [a, b]', two + 'x -> y: {class: [a, b]}', two + 'x.class: [a, z]',
                 'classes: {"a, b": {style.stroke: red}; a: {style.stroke: blue}; b: {style.stroke: green}}; x.class: [a, b]',
                 two + 'x.class: [a,\xa0b]', two + 'x.class: [a,\u2003b]', two + 'x.class: [a,\x85b]',
                 two + 'x: {class: [a; b; a]; style.stroke: green}', two + 'layers: {l: {x.class: [a; b; a]}}',
                 two + 'scenarios: {s: {x.class: [a; b; a]}}', 'classes: {a: {class: [a; a]}}; x.class: [a; a]']:
        requests += _both(text, 'classes.d2')
    base = ('classes: {a: {label: A; shape: circle; style: {stroke: red; fill: yellow; opacity: 0.3; stroke-width: 1; font-color: black}}; '
            'b: {label: B; shape: square; style: {stroke: blue; fill: green; opacity: 0.7; stroke-width: 3; font-color: white}}}; ')
    for order in ['a; b; a', 'b; a; b', 'a; a', 'b; b', 'a; b; b; a; b', 'A; b; a', '"a"', 'a', 'b']:
        for target in ['x: {class: [%s]}', 'x -> y: {class: [%s]}']:
            requests += _both(base + target % order, 'class-order.d2')
    md = 'classes: {a: {label: |md\n**A**\n|}; b: {label: B}}\n'
    for text in [md + 'x: {class: [a; b]}', md + 'x -> y: {class: [a; b]}', md + 'x: |md\nP\n| {class: b}',
                 md + 'x -> y: |md\nP\n| {class: b}', 'classes: {a: {label: |md\n**A**\n|; shape: code}}\nx.class: a',
                 'classes: {a: {label: |md\n**A**\n|; shape: code}}\nx -> y: {class: a}',
                 'classes: {a: {shape: code; label: |md\n**A**\n|}}\nx.class: a',
                 'classes: {a: {shape: code; label: |md\n**A**\n|}}\nx -> y: {class: a}',
                 'classes: {a: {label: A}}; x -> y: P {class: a}', 'classes: {a: {link: https://example.com}}; x -> y: {class: a}',
                 'classes: {a: {link: https://example.com; label: ""}}; x -> y: {class: a}',
                 'classes: {a: {label: A}}; x -> y: P {class: a; label: D}']:
        requests += _both(text, 'class-language.d2')
    return requests


def _measured(texts: list[str], path: str) -> list[dict]:
    return [r for i, text in enumerate(texts) for r in _both(text, path, measured=True, original_index=i)]


def measurement_boundary() -> list[dict]:
    """Label line-break/shape/font matrix plus class/sql_table row bodies, then class row order."""
    labels = ['b\rb', 'long\rb', 'b\rlong', '\nabc', '\n\nabc', 'abc\n', 'abc\n\n', 'abc\n \n', 'abc\n\nxyz', '\n \nabc',
              '\r', '\n', ' \n ', 'a\r\nb', 'a\n\rb', '']
    texts = []
    for label, shape, font, size in itertools.product(labels, ['rectangle', 'code'], ['sans', 'mono'], [12, 16, 28]):
        texts.append('x: ' + json.dumps(label) + ' {shape:' + shape + ';style.font-size:' + str(size)
                     + (' ;style.font:mono' if font == 'mono' else '') + '}')
    for shape, size, body in itertools.product(
            ['class', 'sql_table'], [16, 20, 30],
            ['m(); f: f', 'm(): "m()"; f: f', 'm(): ""; f: ""', 'm(): before {label: after}; f: before {label: after}',
             'm(){label: "m()"}; f: other {label:f}', 'a: int {constraint:[null; primary_key; PRIMARY_KEY; foreign_key; unique]}', '']):
        texts.append('x: "" {shape:' + shape + ';style.font-size:' + str(size) + ';' + body + '}')
    rows = ['x:{shape:class;m();"":""}', 'x:{shape:class;"":"";m()}', 'x:{shape:class;m();f:""}', 'x:{shape:class;f:"";m()}']
    return _measured(texts, 'matrix.d2') + _measured(rows, 'rows.d2')


def class_headers() -> list[dict]:
    """Class-shape header label language, italic and font-size roles."""
    labels = ['"Italic Alphabet\\nClass Header"'] + [f'|{tag}\n  Italic Alphabet\n  Class Header\n|' for tag in ['md', 'go', 'latex', 'text']]
    texts = []
    for label, italic, font, size in itertools.product(labels, ['false', 'true'], ['sans', 'mono'], [12, 20, 30]):
        texts.append(f'x: {label} {{shape:class;style.font-size:{size};style.italic:{italic}'
                     + (' ;style.font:mono' if font == 'mono' else '') + ';f:int;m()}')
    texts += ['classes:{C:{label:|' + tag + '\n  Header\n|}};x:{class:C;shape:class;label:""}' for tag in ['md', 'go', 'latex', 'text']]
    return _measured(texts, 'header.d2')

