#!/usr/bin/env python3
"""Input matrix of recipe `lang-edge-key-validation` (tools/fixtures/langlibelk-language-compiler.toml).

Writes [{name, source[, utf16]}] to $D2RS_RECIPE_OUTPUT: edge-map and
arrowhead-map keys (every reserved/style keyword plus case, Unicode and
custom spellings) quoted and unquoted, in edge, arrowhead, class and board
scopes; non-scalar values for the scalar keywords; integer spellings for the
integer keywords; and individual reference-order, class and legacy-position
witnesses. The hook fills in each row's d2compiler.Compile error.
"""
import json
import os
import sys

# Keyword -> a valid-looking value, so only the key spelling decides the outcome.
KEYWORDS = [
    ('label', 'hello'), ('shape', 'triangle'), ('icon', './icon.svg'), ('constraint', 'value'),
    ('tooltip', 'hello'), ('link', 'https://example.com'), ('near', 'top-left'), ('width', '10'),
    ('height', '10'), ('direction', 'right'), ('top', '10'), ('left', '10'), ('grid-rows', '1'),
    ('grid-columns', '1'), ('grid-gap', '10'), ('vertical-gap', '10'), ('horizontal-gap', '10'),
    ('class', 'missing'), ('vars', '{ x: value }'), ('source-arrowhead', 'value'),
    ('target-arrowhead', 'value'), ('classes', '{ x: { label: hi } }'), ('style', '{ fill: red }'),
    ('layers', '{ x: {} }'), ('scenarios', '{ x: {} }'), ('steps', '{ x: {} }'),
] + [(key, 'value') for key in [
    'opacity', 'stroke', 'fill', 'fill-pattern', 'stroke-width', 'stroke-dash', 'border-radius',
    'font', 'font-size', 'font-color', 'bold', 'italic', 'underline', 'text-transform', 'shadow',
    'multiple', 'double-border', '3d', 'animated', 'filled', 'custom', 'STYLE', 'Label', 'é']]

SCOPES = [
    ('edge', 'a -> b: {{ {} }}'),
    ('source-arrowhead', 'a -> b: {{ source-arrowhead: {{ {} }} }}'),
    ('target-arrowhead', 'a -> b: {{ target-arrowhead: {{ {} }} }}'),
    ('class-edge', 'classes: {{ c: {{ {} }} }}\na -> b: {{ class: c }}'),
    ('class-arrowhead', 'classes: {{ c: {{ target-arrowhead: {{ {} }} }} }}\na -> b: {{ class: c }}'),
    ('unused-class', 'classes: {{ c: {{ target-arrowhead: {{ {} }} }} }}\na -> b'),
    ('board-arrowhead', 'layers: {{ l: {{ a -> b: {{ target-arrowhead: {{ {} }} }} }} }}'),
]

REFERENCE_ORDER = [
    ('first-reference', 'a -> b: { custom: first }\n(a -> b)[0].custom: second'),
    ('last-arrow-reference', 'a -> b: { target-arrowhead.custom: first }\n(a -> b)[0].target-arrowhead.custom: second'),
    ('ordered', 'a -> b: { custom: x; target-arrowhead: { custom: x; style.fill: bad; shape: bad }; other: y; fill: red }'),
    ('implicit', 'a -> b: { target-arrowhead.custom.child: x }'),
    ('null-delete', 'a -> b: { target-arrowhead.custom: x; target-arrowhead.custom: null }'),
    ('unicode', '😀 -> 中: { target-arrowhead: { "label": hello; custom: x } }'),
]

BOUNDARY_KEYS = ['shape', 'width', 'height', 'direction', 'top', 'left', 'grid-rows', 'grid-columns', 'grid-gap',
                 'vertical-gap', 'horizontal-gap', 'link', 'near', 'vars', 'class', 'constraint', 'label', 'icon', 'tooltip']
BOUNDARY_FORMS = ['{}', '{}: {{}}', '{}: [a; b]', '{}: value {{ custom: x }}']

CLASSES = [
    ('class-first-ref', 'classes.c.custom: x\nclasses.c.custom: y\na -> b: { class: c }'),
    ('class-edge', 'classes: { a -> b }'),
    ('class-nested', 'n: { classes.c.custom: x }'),
    ('class-unused-unknown', 'classes.c.custom: x\na -> b'),
    ('class-quoted-inherit', 'classes: { c: { "class": missing } }\na -> b: { class: c }'),
    ('class-order', 'a -> b: { class: c; target-arrowhead.custom: x }\nclasses.c.custom: x'),
    ('multiple-classes', 'classes: { c.custom: x; d.custom: y }\na -> b: { class: [d; c] }'),
    ('object-arrowhead', 'n: { source-arrowhead: { custom: x } }'),
    ('legacy-label-percentage-test', 'a -> b: automatic\nb -> c: explicit {label-percentage: 25}\nc -> d: explicit zero {label-percentage: 0}\n'),
]

INTEGER_KEYS = ['width', 'height', 'top', 'left', 'grid-rows', 'grid-columns', 'grid-gap', 'vertical-gap', 'horizontal-gap']
INTEGER_VALUES = ['', '+', '-', '+0', '-0', '-1', '0', '01', '+12', '1.0', '1e3', '0x10', '1_000', ' 10', '10 ', '١', '１２',
                  '9223372036854775807', '9223372036854775808', '-9223372036854775808', '-9223372036854775809',
                  '99999999999999999999999999x', 'x99999999999999999999999999']

LEGACY_POSITIONS = [
    ('legacy-edge-icon-position-test', 'a -> b: {\n  icon: https://example.com/route.svg\n  icon-position: inside_middle_center\n}\n'),
    ('legacy-cli-edge-position-percentage-test', 'a -> b: {label: link; label-position: outside-top-right; label-percentage: 0.25}\n'),
    ('legacy-svg-edge-position-test', 'a -> b: {label: link; label-position: outside-top-right}\n'),
    ('valid-svg-edge-position-input', 'a -> b: {label: link { near: outside-top-right }}\n'),
]


def cases():
    rows = []
    for scope, template in SCOPES:
        for key, value in KEYWORDS:
            for quoted in (False, True):
                spelled = f'"{key}"' if quoted else key
                rows.append({'name': f'{scope}/{key}/{quoted}', 'source': template.format(f'{spelled}: {value}')})
    rows += [{'name': name, 'source': source, 'utf16': False} for name, source in REFERENCE_ORDER]
    rows.append({'name': 'utf16', 'source': REFERENCE_ORDER[-1][1], 'utf16': True})
    for scope, template in SCOPES[:3]:
        for key in BOUNDARY_KEYS:
            for form in BOUNDARY_FORMS:
                entry = form.format(key)
                rows.append({'name': f'{scope}/boundary/{entry}', 'source': template.format(entry)})
    rows += [{'name': name, 'source': source} for name, source in CLASSES]
    for scope, template in (SCOPES[0], SCOPES[2]):
        for key in INTEGER_KEYS:
            for value in INTEGER_VALUES:
                rows.append({'name': f'{scope}/integer/{key}/{value}', 'source': template.format(f'{key}: {json.dumps(value, ensure_ascii=False)}')})
    rows += [{'name': name, 'source': source} for name, source in LEGACY_POSITIONS]
    return rows


if __name__ == '__main__':
    target = os.environ.get('D2RS_RECIPE_OUTPUT') or sys.exit('run through scripts/regen-fixtures.py')
    with open(target, 'w') as stream:
        json.dump(cases(), stream, ensure_ascii=False, indent=2)
