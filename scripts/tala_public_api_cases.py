"""Request generators for the d2-rs-lib public TALA API oracle fixtures.

Recipes in tools/fixtures/langlibelk-lib.toml feed these requests to
tools/tala-public-api-oracle (d2graph.DeserializeGraph, then
d2talalayout.Layout or RouteEdges, then d2graph.SerializeGraph). Each family
returns the request list for one crates/d2-rs-lib/tests/fixtures/*.json
artifact.

The matrices are the 2026-10-09 expanders (formerly in the ignored
.reference/tala-*-native-20261009 checkpoints) kept verbatim, so case names
and order match the readers' expectations. Families chain the way those
checkpoints did: a family's base graph is the first request of an earlier
family, and several families keep an earlier family's requests as a prefix.
Every hand-off goes through a JSON round trip, exactly like the original
inputs.json files.

Three inputs are not matrices: graphs copied from hand-authored d2-rs-lib
unit tests (tools/fixtures/seeds/lib-tala-public-api/legacy-graphs.json)
and the d2format key spellings of the near-keys family, which the recipe
asks the oracle for (`key_values`) instead of storing them.
"""
from __future__ import annotations

import base64
import copy
import functools
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEEDS = ROOT / "tools/fixtures/seeds/lib-tala-public-api/legacy-graphs.json"


def _rt(value):
    """JSON round trip: the original families exchanged inputs.json files."""
    return json.loads(json.dumps(value))


def _cached(fn):
    cached = functools.lru_cache(maxsize=None)(lambda *args: json.dumps(fn(*args)))
    return functools.wraps(fn)(lambda *args: json.loads(cached(*args)))


def legacy_graphs() -> dict:
    return json.loads(SEEDS.read_text())


# --- wire geometry ---------------------------------------------------------

@_cached
def wire_geometry():
    base = {'root': {'AbsID': '', 'id': '', 'ChildrenArray': ['a', 'b']}, 'objects': [
        {'AbsID': name, 'id': name, 'box': {'Width': 100, 'Height': 60, 'TopLeft': {'x': 10 + i * 240, 'y': 20}}}
        for i, name in enumerate(['a', 'b'])], 'edges': [
        {'Src': 'a', 'Dst': 'b', 'index': 0, 'dst_arrow': True, 'route': [{'x': 110, 'y': 50}, {'x': 250, 'y': 50}]}], 'rootLevel': 0}
    variants = []

    def variant(name, edit):
        g = copy.deepcopy(base); edit(g); variants.append((name, g))
    variant('valid', lambda g: None)
    for dim in ['Width', 'Height']:
        for value in [-1, 0, 1000000000, 1000000000.5]:
            variant(f'{dim}-{value}', lambda g, d=dim, v=value: g['objects'][0]['box'].__setitem__(d, v))
    for axis in ['x', 'y']:
        for value in [-1000000000.5, -1000000000, 999999900, 999999940, 1000000000, 1000000000.5]:
            variant(f'position-{axis}-{value}', lambda g, a=axis, v=value: g['objects'][0]['box']['TopLeft'].__setitem__(a, v))
    variant('no-positions', lambda g: [o['box'].__setitem__('TopLeft', None) for o in g['objects']])
    variant('no-routes', lambda g: g['edges'][0].__setitem__('route', None))
    variant('single-route', lambda g: g['edges'][0].__setitem__('route', [{'x': 12, 'y': 34}]))
    variant('degenerate-route', lambda g: g['edges'][0].__setitem__('route', [{'x': 12, 'y': 34}, {'x': 12, 'y': 34}]))
    for point in [0, 1]:
        for axis in ['x', 'y']:
            for value in [-1000000000.5, -1000000000, 1000000000, 1000000000.5]:
                variant(f'route-{point}-{axis}-{value}', lambda g, p=point, a=axis, v=value: g['edges'][0]['route'][p].__setitem__(a, v))
    # Source-only declarations exercise validation before route reversal.
    variants += [(name + '-source-arrow', dict(g, edges=[dict(g['edges'][0], src_arrow=True, dst_arrow=False)]))
                 for name, g in list(variants) if name.startswith(('route-', 'single-', 'degenerate-', 'valid', 'no-routes'))]
    variant('dimensions-before-position', lambda g: g['objects'][0].__setitem__('box', {'Width': 0, 'Height': -1, 'TopLeft': {'x': 1000000001, 'y': 0}}))
    variant('position-before-authored', lambda g: (g['objects'][0]['box']['TopLeft'].__setitem__('x', 1000000001), g['objects'][0].__setitem__('attributes', {'top': {'value': 'bad'}})))
    variant('object-before-route', lambda g: (g['objects'][0]['box'].__setitem__('Width', 0), g['edges'][0].__setitem__('route', [{'x': 0, 'y': 0}])))
    variant('source-point-order', lambda g: g['edges'][0].update(**{'src_arrow': True, 'dst_arrow': False}, route=[{'x': 1000000001, 'y': 1000000002}, {'x': -1000000003, 'y': 0}]))
    variant('source-degenerate-index', lambda g: g['edges'][0].update(**{'src_arrow': True, 'dst_arrow': False}, route=[{'x': 1, 'y': 2}, {'x': 1, 'y': 2}, {'x': 3, 'y': 4}]))
    variant('child-order', lambda g: (g['root'].__setitem__('ChildrenArray', ['b', 'a']), g['objects'][0]['box'].__setitem__('Width', 0), g['objects'][1]['box'].__setitem__('Height', 0)))
    variant('quoted-local-id', lambda g: (g['objects'][0].__setitem__('id', 'a"\n'), g['objects'][0]['box'].__setitem__('Width', 0)))
    rows = []
    for name, g in variants:
        for mode in ['layout', 'empty']:
            rows.append(dict(name=f'{name}-{mode}', graph=g, mode=mode, cancel=False, seeds=[1], requested=[]))
    for name, g in variants:
        if name in ['single-route', 'source-point-order', 'position-before-authored', 'dimensions-before-position']:
            for mode in ['layout', 'empty', 'duplicate']:
                for cancel in [False, True]:
                    for seeds in [[1], []]:
                        rows.append(dict(name=f'{name}-{mode}-cancel{cancel}-seeds{seeds}', graph=g, mode=mode, cancel=cancel, seeds=seeds, requested=[0, 0] if mode == 'duplicate' else []))
    # Ordinary complete rerouting confirms retained canonical routes still work.
    for name, g in variants:
        if name in ['valid', 'valid-source-arrow', 'no-routes', 'no-routes-source-arrow']:
            rows.append(dict(name=f'{name}-selected', graph=g, mode='selected', cancel=False, seeds=[1], requested=[0]))
    return _rt(rows)


# --- input attributes ------------------------------------------------------

@_cached
def input_attributes():
    base = wire_geometry()[0]['graph']
    variants = []

    def attrs(g, owner):
        if owner == 'object': return g['objects'][0].setdefault('attributes', {})
        if owner == 'edge': return g['edges'][0].setdefault('attributes', {})
        return g['edges'][0].setdefault(owner, {})

    def add(name, edit):
        g = copy.deepcopy(base); edit(g); variants.append((name, g))
    owners = ['object', 'edge', 'srcArrowhead', 'dstArrowhead']
    style_values = {
        'opacity': ['0', '-0', '1', '0.5', '0x1p-1', '1_0e-1', 'NaN', '+Inf', '-Inf', '-0.1', '1.1', '', ' 0.5', '1e999', 'junk'],
        'strokeWidth': ['-1', '0', '1', '16', '1000000000', '1000000001', '2147483648', '9223372036854775807', '9223372036854775808', '1.0', '1e1', '+1', '01', '1_0', '', ' ', '+', '--1', '٠', '1\n'],
        'fontSize': ['-1', '0', '1', '16', '1000000000', '1000000001', '2147483648', '9223372036854775807', '9223372036854775808', '1.0', '1e1', '+1', '01', '1_0', '', ' ', '+', '--1', '٠', '1\n']}
    bool_values = ['true', 'false', '1', '0', 'True', 'False', 'TRUE', 'FALSE', 't', 'f', 'T', 'F', '', 'yes', 'true ', ' true', 'TrUe', '2', '☃']
    for name in ['animated', 'bold', 'doubleBorder', 'filled', 'italic', 'multiple', 'shadow', '3d', 'underline']:
        style_values[name] = bool_values
    for owner in owners:
        for field, values in style_values.items():
            for i, value in enumerate(values):
                add(f'style-{owner}-{field}-{i}', lambda g, o=owner, f=field, v=value: attrs(g, o).setdefault('style', {}).__setitem__(f, {'value': v}))
        for i, pair in enumerate([[-1, 0], [0, -1], [-1, -1], [0, 0], [1, 0], [0, 1], [20, 10], [1000000000, 1000000000], [1000000001, 0], [0, 1000000001], [-1, 1000000001], [9223372036854775807, 0]]):
            add(f'dimensions-{owner}-{i}', lambda g, o=owner, p=pair: attrs(g, o).__setitem__('labelDimensions', dict(zip(['width', 'height'], p))))
    positions = [''] + [f'{side}_{h}_{v}' for side, h, vs in [
        ('OUTSIDE', 'TOP', ['LEFT', 'CENTER', 'RIGHT']), ('OUTSIDE', 'BOTTOM', ['LEFT', 'CENTER', 'RIGHT']),
        ('OUTSIDE', 'LEFT', ['TOP', 'MIDDLE', 'BOTTOM']), ('OUTSIDE', 'RIGHT', ['TOP', 'MIDDLE', 'BOTTOM']),
        ('INSIDE', 'TOP', ['LEFT', 'CENTER', 'RIGHT']), ('INSIDE', 'MIDDLE', ['LEFT', 'CENTER', 'RIGHT']), ('INSIDE', 'BOTTOM', ['LEFT', 'CENTER', 'RIGHT']),
        ('BORDER', 'TOP', ['LEFT', 'CENTER', 'RIGHT']), ('BORDER', 'BOTTOM', ['LEFT', 'CENTER', 'RIGHT']),
        ('BORDER', 'LEFT', ['TOP', 'MIDDLE', 'BOTTOM']), ('BORDER', 'RIGHT', ['TOP', 'MIDDLE', 'BOTTOM'])] for v in vs] + ['UNLOCKED_TOP', 'UNLOCKED_MIDDLE', 'UNLOCKED_BOTTOM', 'UNSET', 'inside_middle_center', ' INSIDE_MIDDLE_CENTER', 'unknown']
    for owner, field in [('object', 'labelPosition'), ('object', 'iconPosition'), ('edge', 'labelPosition')]:
        for i, value in enumerate(positions):
            add(f'position-{owner}-{field}-{i}', lambda g, o=owner, f=field, v=value: (g['objects'][0] if o == 'object' else g['edges'][0]).__setitem__(f, v))
    for arrow in [False, True]:
        for i, value in enumerate([-1, -0.000001, 0, 0.5, 1, 1.0000001, 1000000000]):
            add(f'percentage-{arrow}-{i}', lambda g, v=value, a=arrow: g['edges'][0].update(labelPercentage=v, src_arrow=a, dst_arrow=not a))
    integers = ['-9223372036854775809', '-9223372036854775808', '-2147483649', '-2147483648', '-1000000001', '-1000000000', '-1', '-0', '0', '+0', '1', '+1', '01', '0000000000000000000001', '1000000000', '1000000001', '2147483647', '2147483648', '9223372036854775807', '9223372036854775808', '1.0', '1e0', '1_0', ' 1', '1 ', '', '+', '--1', '0x1', '١', '1\n']
    for field in ['top', 'left', 'width', 'height']:
        for i, value in enumerate(integers):
            def edit(g, f=field, v=value):
                a = attrs(g, 'object')
                if f in ['top', 'left']: a.update(top={'value': '1'}, left={'value': '1'})
                a[f] = {'value': v}
            add(f'authored-{field}-{i}', edit)
    # Multiple-invalid controls preserve source validation order, including
    # original arrowhead slots before source-arrow canonicalization.
    add('precedence-object-dimensions-style', lambda g: attrs(g, 'object').update(labelDimensions={'width': -1, 'height': 0}, style={'opacity': {'value': 'bad'}}))
    add('precedence-edge-style-position', lambda g: (attrs(g, 'edge').update(style={'opacity': {'value': 'bad'}}), g['edges'][0].__setitem__('labelPosition', 'bad')))
    add('precedence-object-position-authored', lambda g: (g['objects'][0].__setitem__('labelPosition', 'bad'), attrs(g, 'object').__setitem__('top', {'value': 'bad'})))
    add('precedence-object-icon-authored', lambda g: (g['objects'][0].__setitem__('iconPosition', 'bad'), attrs(g, 'object').__setitem__('top', {'value': 'bad'})))
    add('precedence-edge-percentage-arrow', lambda g: (g['edges'][0].__setitem__('labelPercentage', -1), attrs(g, 'srcArrowhead').__setitem__('shape', {'value': 'mystery'})))
    add('precedence-source-shape-style', lambda g: attrs(g, 'srcArrowhead').update(shape={'value': 'mystery'}, style={'opacity': {'value': 'bad'}}))
    add('precedence-source-style-dimensions', lambda g: attrs(g, 'srcArrowhead').update(style={'opacity': {'value': 'bad'}}, labelDimensions={'width': -1, 'height': 0}))
    add('precedence-source-dimensions-destination', lambda g: (attrs(g, 'srcArrowhead').__setitem__('labelDimensions', {'width': -1, 'height': 0}), attrs(g, 'dstArrowhead').__setitem__('shape', {'value': 'mystery'})))
    add('precedence-arrow-dimensions-route', lambda g: (attrs(g, 'dstArrowhead').__setitem__('labelDimensions', {'width': -1, 'height': 0}), g['edges'][0].__setitem__('route', [{'x': 0, 'y': 0}])))
    style_order = list(style_values)
    for i, field in enumerate(style_order[:-1]):
        add('precedence-style-' + field, lambda g, i=i: attrs(g, 'object').__setitem__('style', {f: {'value': 'bad'} for f in style_order[i:]}))
    # Translation consumes measured main-label dimensions even with empty text.
    # Arrowhead dimensions contribute only for nonempty arrowhead labels.

    def measured(g, owner, width, height, text=''):
        attrs(g, owner).update(labelDimensions={'width': width, 'height': height}, label={'value': text})
    add('minimum-empty-main', lambda g: measured(g, 'edge', 500, 700))
    add('minimum-visible-main', lambda g: measured(g, 'edge', 500, 700, 'edge'))
    add('minimum-empty-source-arrow', lambda g: measured(g, 'srcArrowhead', 500, 700))
    add('minimum-visible-source-arrow', lambda g: measured(g, 'srcArrowhead', 500, 700, 'source'))
    add('minimum-visible-target-arrow', lambda g: measured(g, 'dstArrowhead', 500, 700, 'target'))
    add('minimum-visible-both-arrows', lambda g: (measured(g, 'srcArrowhead', 300, 400, 'source'), measured(g, 'dstArrowhead', 350, 250, 'target')))
    add('minimum-empty-huge-arrow', lambda g: measured(g, 'srcArrowhead', 1000000000, 0))
    add('minimum-visible-huge-arrow', lambda g: measured(g, 'srcArrowhead', 1000000000, 0, 'source'))
    add('minimum-combined-overflow', lambda g: (measured(g, 'edge', 500000000, 0), measured(g, 'srcArrowhead', 600000000, 0, 'source')))
    add('minimum-transparent-overflow', lambda g: (measured(g, 'edge', 1000000000, 0), attrs(g, 'edge').update(style={'opacity': {'value': '0'}})))

    def later_edge_error(g):
        measured(g, 'edge', 1000000000, 0)
        e = copy.deepcopy(g['edges'][0]); e['index'] = 1
        e['attributes'] = {'style': {'opacity': {'value': 'bad'}}}
        g['edges'].append(e)
    add('minimum-later-validation-first', later_edge_error)
    add('minimum-source-reversed', lambda g: (measured(g, 'edge', 500, 700), measured(g, 'srcArrowhead', 300, 400, 'source'), measured(g, 'dstArrowhead', 350, 250, 'target'), g['edges'][0].update(src_arrow=True, dst_arrow=False)))
    rows = []
    for name, g in variants:
        for mode in ['layout', 'empty']:
            rows.append(dict(name=f'{name}-{mode}', graph=g, mode=mode, cancel=False, seeds=[1], requested=[]))
        if name.startswith('precedence-'):
            for mode, cancel, requested in [('layout', True, []), ('empty', True, []), ('duplicate', False, [0, 0]), ('duplicate', True, [0, 0])]:
                rows.append(dict(name=f'{name}-{mode}-cancel{cancel}', graph=g, mode=mode, cancel=cancel, seeds=[1], requested=requested))
    # Copy controls to reversed source-only arrows.
    for name, g in variants:
        if name.startswith('precedence-source-'):
            g = copy.deepcopy(g); g['edges'][0].update(src_arrow=True, dst_arrow=False)
            for mode in ['layout', 'empty']:
                rows.append(dict(name=f'{name}-reversed-{mode}', graph=g, mode=mode, cancel=False, seeds=[1], requested=[]))
    return _rt(rows)


@_cached
def _table_audit():
    base = input_attributes()[0]['graph']
    # Reset the first style mutation from the attribute matrix.
    base['objects'][0]['attributes'].pop('style', None)
    variants = []

    def add(name, edit):
        g = copy.deepcopy(base); edit(g); variants.append((name, g))

    def table(g, columns=2, shape='sql_table'):
        o = g['objects'][0]; o.setdefault('attributes', {})['shape'] = {'value': shape}
        o['sql_table'] = {'columns': [{'name': {'label': f'column{i}'}, 'type': {'label': 'int'}} for i in range(columns)]}
    add('non-table-with-sql-columns', lambda g: table(g, 2, 'rectangle'))
    add('non-table-empty-sql-pointer', lambda g: g['objects'][0].update(sql_table={}))
    add('table-negative-column', lambda g: (table(g), g['edges'][0].update(srcTableColumnIndex=-1)))
    add('table-past-last-column', lambda g: (table(g), g['edges'][0].update(srcTableColumnIndex=2)))
    add('table-empty-pointer-column', lambda g: (table(g, 0), g['edges'][0].update(srcTableColumnIndex=0)))
    add('non-table-destination-column', lambda g: g['edges'][0].update(dstTableColumnIndex=0))
    add('table-negative-source-reversed', lambda g: (table(g), g['edges'][0].update(srcTableColumnIndex=-1, src_arrow=True, dst_arrow=False)))
    add('table-valid-column', lambda g: (table(g), g['edges'][0].update(srcTableColumnIndex=1)))
    return _rt([dict(name=f'{name}-{mode}', graph=g, mode=mode, cancel=False, seeds=[1], requested=[]) for name, g in variants for mode in ['layout', 'empty']])


# --- input tables ----------------------------------------------------------

@_cached
def input_tables():
    base = _table_audit()[0]['graph']
    for o in base['objects']:
        o.pop('sql_table', None); o.setdefault('attributes', {})['shape'] = {'value': 'rectangle'}
    variants = []

    def add(name, edit, modes=('layout', 'empty', 'selected')):
        g = copy.deepcopy(base); edit(g); variants.append((name, g, modes))

    def table(g, owner, count=2, shape='sql_table'):
        o = g['objects'][owner]; o['attributes']['shape'] = {'value': shape}
        o['sql_table'] = {'columns': [{'name': {'label': f'c{i}'}, 'type': {'label': 'int'}} for i in range(count)]}
    for owner in [0, 1]:
        for shape in ['sql_table', 'rectangle', 'class', 'SQL_TABLE', '']:
            for kind in ['absent', 'null', 'object', 'columns-null', 'empty', 'two']:
                def edit(g, o=owner, s=shape, k=kind):
                    obj = g['objects'][o]; obj['attributes']['shape'] = {'value': s}
                    if k == 'null': obj['sql_table'] = None
                    elif k == 'object': obj['sql_table'] = {}
                    elif k == 'columns-null': obj['sql_table'] = {'columns': None}
                    elif k == 'empty': obj['sql_table'] = {'columns': []}
                    elif k == 'two': table(g, o, 2, s)
                add(f'payload-{owner}-{shape}-{kind}', edit)
    indices = [None, -1, 0, 1, 2, 2147483647, 2147483648, 9223372036854775807, -9223372036854775808, 9223372036854775808, 1.0, 0.5, True, '1', {}, []]
    for endpoint in ['src', 'dst']:
        for config in ['none', 'source', 'destination', 'both']:
            for reverse in [False, True]:
                for i, index in enumerate(indices):
                    def edit(g, e=endpoint, c=config, v=index, a=reverse):
                        if c in ['source', 'both']: table(g, 0)
                        if c in ['destination', 'both']: table(g, 1, 3)
                        g['edges'][0][e + 'TableColumnIndex'] = v
                        if a: g['edges'][0].update(src_arrow=True, dst_arrow=False)
                    add(f'index-{endpoint}-{config}-{reverse}-{i}', edit)
    # Font defaults depend on actual class/table payload pointers, not shape names.
    for shape in ['rectangle', 'class', 'sql_table', 'text']:
        for present in [None, {}, {'fields': []}]:
            for size in [None, '24']:
                def edit(g, s=shape, p=present, size=size):
                    o = g['objects'][0]; o['attributes']['shape'] = {'value': s}; o['class'] = p
                    if size is not None: o['attributes']['style'] = {'fontSize': {'value': size}}
                add(f'font-{shape}-{present}-{size}', edit)
    # Aggregate counts at the source boundary without a 10k-column layout.
    for counts in [(9999, 0), (10000, 0), (10001, 0), (5000, 5000), (5000, 5001), (1, 10000)]:
        def edit(g, counts=counts):
            for owner, count in enumerate(counts):
                table(g, owner, 0); g['objects'][owner]['sql_table']['columns'] = [{} for _ in range(count)]
            g['edges'][0].setdefault('attributes', {})['style'] = {'opacity': {'value': 'bad'}}
        add(f'limit-{counts}', edit, ('layout', 'empty'))
    # Multiple-invalid controls: deserialization, validation and selection precedence.
    add('precedence-source-before-destination-index', lambda g: (table(g, 0), table(g, 1), g['edges'][0].update(srcTableColumnIndex=-1, dstTableColumnIndex=99)))
    add('precedence-arrow-before-column', lambda g: g['edges'][0].update(srcTableColumnIndex=-1, srcArrowhead={'shape': {'value': 'mystery'}}))
    add('precedence-column-before-route', lambda g: g['edges'][0].update(srcTableColumnIndex=-1, route=[{'x': 0, 'y': 0}]))
    add('precedence-style-before-table-shape', lambda g: (table(g, 0, 2, 'rectangle'), g['objects'][0]['attributes'].update(style={'opacity': {'value': 'bad'}})))
    add('precedence-authored-before-table-shape', lambda g: (table(g, 0, 2, 'rectangle'), g['objects'][0]['attributes'].update(width={'value': '0'})))
    add('precedence-table-before-later-object', lambda g: (table(g, 0, 2, 'rectangle'), g['objects'][1]['attributes'].update(style={'opacity': {'value': 'bad'}})))
    add('precedence-destination-json-before-source-json', lambda g: g['edges'][0].update(srcTableColumnIndex='1', dstTableColumnIndex=True))
    add('precedence-main-json-before-column-json', lambda g: g['edges'][0].update(srcTableColumnIndex='1', attributes={'labelDimensions': {'width': 0.5, 'height': 0}}))
    add('precedence-destination-arrow-json-before-column-json', lambda g: g['edges'][0].update(srcTableColumnIndex='1', dstArrowhead={'labelDimensions': {'width': 0.5, 'height': 0}}))
    # D2 shape names are recognized case-insensitively for engine shape mapping.
    for shape in ['rectangle', 'square', 'page', 'parallelogram', 'document', 'cylinder', 'queue', 'package', 'step', 'callout', 'stored_data', 'person', 'c4-person', 'c4_person', 'diamond', 'oval', 'circle', 'hexagon', 'cloud', 'text', 'code', 'class', 'sql_table', 'image', 'sequence_diagram', 'hierarchy', 'real_square', 'pac\u212aage', '\u017fql_table', 'unknown']:
        for spelling in [shape, shape.upper()]:
            add(f'shape-{shape}-{spelling}', lambda g, s=spelling: g['objects'][0]['attributes'].update(shape={'value': s}))
    rows = []
    for name, g, modes in variants:
        for mode in modes:
            rows.append(dict(name=f'{name}-{mode}', graph=g, mode=mode, cancel=False, seeds=[1], requested=[0] if mode == 'selected' else []))
        if name.startswith('precedence-'):
            for mode, cancel, seeds, selection in [('layout', True, [1], []), ('layout', False, [], []), ('empty', True, [1], []), ('duplicate', False, [1], [0, 0]), ('duplicate', True, [1], [0, 0])]:
                rows.append(dict(name=f'{name}-{mode}-cancel{cancel}-seeds{seeds}', graph=g, mode=mode, cancel=cancel, seeds=seeds, requested=selection))
    # Retain all 16 earlier table-audit cases, verbatim.
    rows.extend(_table_audit())
    assert len({v['name'] for v in rows}) == len(rows)
    return _rt(rows)


@_cached
def _missing_geometry_audit():
    base = input_tables()[0]['graph']
    variants = []

    def add(name, edit):
        g = copy.deepcopy(base); edit(g); variants.append((name, g))
    for value in [None, {}, {'Width': 100, 'Height': 60}, {'Width': 100, 'Height': 60, 'TopLeft': None}, {'Width': 100, 'Height': 60, 'TopLeft': {}}, {'Width': 100, 'Height': 60, 'TopLeft': {'x': 10}}, {'Width': 100, 'Height': 60, 'TopLeft': {'y': 20}}, {'Width': None, 'Height': 60, 'TopLeft': {'x': 10, 'y': 20}}, {'Height': 60, 'TopLeft': {'x': 10, 'y': 20}}, {'Width': 100, 'TopLeft': {'x': 10, 'y': 20}}]:
        add('box-' + str(value), lambda g, v=value: g['objects'][0].update(box=v))
    add('box-absent', lambda g: g['objects'][0].pop('box'))
    for route in [[None], [None, {'x': 200, 'y': 30}], [{'x': 100, 'y': 30}, None], [{}, {'x': 200, 'y': 30}], [{'x': 100}, {'x': 200, 'y': 30}], [{'y': 30}, {'x': 200, 'y': 30}], [{'x': None, 'y': 30}, {'x': 200, 'y': 30}], [{'x': 100, 'y': None}, {'x': 200, 'y': 30}], [{'x': 1000000001, 'y': 30}, None]]:
        add('route-' + str(route), lambda g, v=route: g['edges'][0].update(route=v))
    rows = []
    for name, g in variants:
        for mode in ['layout', 'empty', 'selected']:
            for cancel in [False, True]:
                rows.append(dict(name=f'{name}-{mode}-cancel{cancel}', graph=g, mode=mode, cancel=cancel, seeds=[1], requested=[0] if mode == 'selected' else []))
    return _rt(rows)


# --- missing geometry ------------------------------------------------------

@_cached
def missing_geometry():
    base = input_tables()[0]['graph']
    rows = _missing_geometry_audit()
    variants = []

    def add(name, edit):
        g = copy.deepcopy(base); edit(g); variants.append((name, g))
    for owner in [0, 1]:
        for which in ['absent', 'null', 'empty', 'width-null', 'height-null', 'width-absent', 'height-absent', 'point-null', 'point-empty', 'point-x-null', 'point-y-null']:
            def edit(g, o=owner, w=which):
                obj = g['objects'][o]; b = obj['box']
                if w == 'absent': obj.pop('box')
                elif w == 'null': obj['box'] = None
                elif w == 'empty': obj['box'] = {}
                elif w in ['width-null', 'height-null']: b[w.split('-')[0].title()] = None
                elif w in ['width-absent', 'height-absent']: b.pop(w.split('-')[0].title())
                elif w == 'point-null': b['TopLeft'] = None
                elif w == 'point-empty': b['TopLeft'] = {}
                else: b['TopLeft'][w.split('-')[1]] = None
            add(f'owner-{owner}-{which}', edit)
        for order in ['allocation', 'children']:
            def edit(g, o=owner, k=order):
                g['objects'][o]['box'] = None
                if k == 'allocation': g['objects'].reverse()
                else: g['root']['ChildrenArray'].reverse()
            add(f'visit-{owner}-{order}', edit)
    for reverse in [False, True]:
        route_variants = [('absent', 'absent'), ('null', None), ('empty', []), ('nil-single', [None]), ('nil-both', [None, None]), ('empty-single', [{}]), ('empty-both', [{}, {}]), ('zeros-degenerate', [{'x': None}, {'y': None}]), ('point-x-null', [{'x': None, 'y': 30}, {'x': 200, 'y': 30}]), ('point-y-null', [{'x': 100, 'y': None}, {'x': 200, 'y': 30}])]
        for nil_index in [0, 1, 2, 255, 256]:
            route = [{'x': 10 + i, 'y': 30} for i in range(257)]; route[nil_index] = None
            route_variants.append((f'nil-index-{nil_index}', route))
        for name, route in route_variants:
            def edit(g, v=route, a=reverse):
                e = g['edges'][0]
                if v == 'absent': e.pop('route')
                else: e['route'] = copy.deepcopy(v)
                if a: e.update(src_arrow=True, dst_arrow=False)
            add(f'route-{name}-reversed-{reverse}', edit)
        for prior in ['x-bound', 'y-bound', 'degenerate', 'label', 'style', 'missing-box', 'missing-position', 'width', 'table']:
            def edit(g, p=prior, a=reverse):
                e = g['edges'][0]; e['route'] = [{'x': 0, 'y': 0}, {'x': 20, 'y': 0}, None]
                if a: e.update(src_arrow=True, dst_arrow=False)
                if p == 'x-bound': e['route'][0]['x'] = 1000000001
                elif p == 'y-bound': e['route'][0]['y'] = -1000000001
                elif p == 'degenerate': e['route'][1] = copy.deepcopy(e['route'][0])
                elif p == 'label': e['attributes'] = {'labelDimensions': {'width': -1, 'height': 0}}
                elif p == 'style': e['attributes'] = {'style': {'opacity': {'value': 'invalid'}}}
                elif p == 'missing-box': g['objects'][1]['box'] = None
                elif p == 'missing-position': g['objects'][1]['box']['TopLeft'] = None
                elif p == 'width': g['objects'][1]['box']['Width'] = 0
                elif p == 'table': e['srcTableColumnIndex'] = 0
            add(f'prior-{prior}-reversed-{reverse}', edit)
    # Nil route in either of two parallel edges.
    for edge_index in [0, 1]:
        for reverse in [False, True]:
            def edit(g, i=edge_index, a=reverse):
                other = copy.deepcopy(g['edges'][0]); other['index'] = 1; g['edges'].append(other)
                e = g['edges'][i]; e['route'][1] = None
                if a: e.update(src_arrow=True, dst_arrow=False)
            add(f'two-edges-{edge_index}-reversed-{reverse}', edit)
    for name, g in variants:
        for mode, requested in [('layout', []), ('empty', []), ('selected', [0]), ('duplicate', [0, 0])]:
            for cancel in [False, True]:
                rows.append(dict(name=f'{name}-{mode}-cancel{cancel}', graph=g, mode=mode, cancel=cancel, seeds=[1], requested=requested))
    assert len({v['name'] for v in rows}) == len(rows)
    return _rt(rows)


@_cached
def _conversion_audit():
    base = missing_geometry()[0]['graph']
    # The first inherited case has a nil box; restore valid geometry.
    base['objects'][0]['box'] = {'Width': 100, 'Height': 60, 'TopLeft': {'x': 10, 'y': 20}}
    variants = []
    for field in ['box', 'Width', 'Height', 'TopLeft', 'box-x', 'box-y', 'route', 'point', 'route-x', 'route-y']:
        for value in [True, '10', [], {}]:
            g = copy.deepcopy(base); box = g['objects'][0]['box']; e = g['edges'][0]
            if field == 'box': g['objects'][0]['box'] = value
            elif field in ['Width', 'Height', 'TopLeft']: box[field] = value
            elif field.startswith('box-'): box['TopLeft'][field[-1]] = value
            elif field == 'route': e['route'] = value
            elif field == 'point': e['route'][0] = value
            else: e['route'][0][field[-1]] = value
            variants.append((f'{field}-{json.dumps(value)}', g))
    rows = []
    for name, g in variants:
        for mode in ['layout', 'empty', 'selected']:
            for cancel in [False, True]:
                rows.append(dict(name=f'{name}-{mode}-cancel{cancel}', graph=g, mode=mode, cancel=cancel, seeds=[1], requested=[0] if mode == 'selected' else []))
    return _rt(rows)


# --- geometry conversion ---------------------------------------------------

@_cached
def geometry_conversion():
    rows = _conversion_audit()
    base = copy.deepcopy(rows[0]['graph']); base['objects'][0]['box'] = {'Width': 100, 'Height': 60, 'TopLeft': {'x': 10, 'y': 20}}
    variants = []

    def add(name, edit):
        g = copy.deepcopy(base); edit(g); variants.append((name, g))
    for field in ['box', 'Width', 'Height', 'TopLeft', 'box-x', 'box-y', 'route', 'point', 'route-x', 'route-y']:
        for value in [1, 0.5]:
            def edit(g, f=field, v=value):
                box = g['objects'][0]['box']; e = g['edges'][0]
                if f == 'box': g['objects'][0]['box'] = v
                elif f in ['Width', 'Height', 'TopLeft']: box[f] = v
                elif f.startswith('box-'): box['TopLeft'][f[-1]] = v
                elif f == 'route': e['route'] = v
                elif f == 'point': e['route'][0] = v
                else: e['route'][0][f[-1]] = v
            add(f'numeric-{field}-{value}', edit)
    for owner in [0, 1]:
        for box_key in ['BOX', 'Box', 'box']:
            for transform in ['lower', 'upper', 'mixed']:
                def edit(g, o=owner, k=box_key, t=transform):
                    obj = g['objects'][o]; b = obj.pop('box')
                    transform = lambda s: s.lower() if t == 'lower' else s.upper() if t == 'upper' else s[0].lower() + s[1:]
                    obj[k] = {transform('Width'): b['Width'], transform('Height'): b['Height'], transform('TopLeft'): {'X': b['TopLeft']['x'], 'Y': b['TopLeft']['y']}}
                add(f'case-{owner}-{box_key}-{transform}', edit)
    for which in ['box', 'point']:
        values = [None, {}, {'width': 140}, {'Width': None}, {'height': 80, 'TopLeft': {'Y': 25}}, {'x': 30}, {'x': None, 'y': None}, {'X': 80, 'x': None, 'Y': 40, 'y': None}, {'unknown': True}, {'X': -0.0, 'x': None, 'y': -0.0}]
        for i, value in enumerate(values):
            def edit(g, w=which, v=value):
                obj = g['objects'][0]; b = obj.pop('box')
                if w == 'box': obj.update(BOX=b, box=v)
                else:
                    p = b.pop('TopLeft'); b.update(TOPLEFT=p, TopLeft=v); obj['box'] = b
            add(f'merge-{which}-{i}', edit)
    for reset in [None, {}, {'Width': 100, 'Height': 60, 'TopLeft': None}]:
        def edit(g, v=reset):
            obj = g['objects'][0]; b = obj.pop('box'); obj.update(BOX=b, Box=v, box={'Width': 100, 'Height': 60, 'TopLeft': {'y': 40}})
        add(f'box-reset-{json.dumps(reset)}', edit)
    for key in ['ROUTE', 'Route', 'rOuTe']:
        for reverse in [False, True]:
            def edit(g, k=key, a=reverse):
                e = g['edges'][0]; route = e.pop('route'); e[k] = [{'X': p['x'], 'Y': p['y'], 'unknown': True} for p in route]
                if a: e.update(src_arrow=True, dst_arrow=False)
            add(f'route-case-{key}-{reverse}', edit)
    route_values = [None, [], [{}, {}], [{'y': 30}, {'Y': 30}], [{'x': None, 'y': None}, {'x': None, 'y': None}], [None, {}], [{'X': -0.0, 'x': None, 'Y': -0.0, 'y': None}, {'x': 200, 'y': 30}]]
    for reverse in [False, True]:
        for i, value in enumerate(route_values):
            def edit(g, a=reverse, v=value):
                e = g['edges'][0]; old = e.pop('route'); e.update(ROUTE=old, route=v)
                if a: e.update(src_arrow=True, dst_arrow=False)
            add(f'route-merge-{reverse}-{i}', edit)
        for reset_name, reset in [('shrink', [{}]), ('empty', []), ('nil', None), ('nil-slot', [None])]:
            def edit(g, a=reverse, v=reset):
                e = g['edges'][0]; e.pop('route'); e.update(ROUTE=[{'x': 100, 'y': 30}, {'x': 150, 'y': 35}, {'x': 200, 'y': 40}], Route=v, route=[{'y': 50}, {'y': 60}, {'y': 70}])
                if a: e.update(src_arrow=True, dst_arrow=False)
            add(f'route-regrow-{reverse}-{reset_name}', edit)
    # Earlier saved type errors win even if a later case variant overwrites the
    # malformed field with a valid value or clears the pointer.
    for key, values in [('box', [True, [], {'Width': True}, {'TopLeft': {'x': True}}]), ('route', [True, {}, [True], [{'x': True}]])]:
        for i, value in enumerate(values):
            def edit(g, k=key, v=value):
                obj = g['objects'][0] if k == 'box' else g['edges'][0]; obj[k.upper()] = v
            add(f'earlier-invalid-{key}-{i}', edit)
    for point in [{'x': True, 'y': 'bad'}, {'X': 'bad', 'x': True}, {'Y': 'bad', 'x': True}]:
        for where in ['box', 'route']:
            def edit(g, p=point, w=where):
                if w == 'box': g['objects'][0]['box']['TopLeft'] = p
                else: g['edges'][0]['route'][0] = p
            add(f'point-errors-{where}-{json.dumps(point)}', edit)
    for b in [{'Width': True, 'Height': 'bad'}, {'Width': True, 'TopLeft': 'bad'}, {'HEIGHT': True, 'Height': 60, 'TopLeft': 'bad'}, {'Height': 60, 'TopLeft': {'Y': True, 'x': 'bad'}, 'Width': True}]:
        add(f'box-errors-{json.dumps(b)}', lambda g, v=b: g['objects'][0].update(box=v))
    for geometry_key in ['box', 'BOX']:
        for bad_attr in [{'width': True}, {'width': True, 'height': 'bad'}]:
            def edit(g, k=geometry_key, a=bad_attr):
                o = g['objects'][0]; o[k] = True; o['attributes']['labelDimensions'] = a
            add(f'object-conversion-order-{geometry_key}-{json.dumps(bad_attr)}', edit)
    for geometry_key in ['route', 'ROUTE']:
        for field in ['attributes', 'dstArrowhead', 'dstTableColumnIndex', 'srcArrowhead', 'srcTableColumnIndex']:
            def edit(g, k=geometry_key, f=field):
                e = g['edges'][0]; e[k] = True; e[f] = 'bad' if f.endswith('Index') else {'labelDimensions': {'width': True, 'height': 'bad'}}
            add(f'edge-conversion-order-{geometry_key}-{field}', edit)
    for name, g in variants:
        for mode, requested in [('layout', []), ('empty', []), ('selected', [0]), ('duplicate', [0, 0])]:
            for cancel in [False, True]:
                rows.append(dict(name=f'{name}-{mode}-cancel{cancel}', graph=g, mode=mode, cancel=cancel, seeds=[1], requested=requested))
    assert len({a['name'] for a in rows}) == len(rows)
    rows = _rt(rows)
    assert len(rows) == 1208
    # Table-port expansion.
    base = copy.deepcopy(rows[0]['graph'])
    for width, height in [(0.5, 60), (100, 60), (215, 180), (30, 10), (100.25, 144.5)]:
        for x, y in [(10, 20), (80, 40)]:
            for kind in ['absent', 'null', 'empty-object', 'zero', 'one', 'three']:
                for reverse in [False, True]:
                    g = copy.deepcopy(base); o = g['objects'][0]
                    o['box'] = {'Width': width, 'Height': height, 'TopLeft': {'x': x, 'y': y}}
                    if kind == 'null': o['sql_table'] = None
                    elif kind == 'empty-object': o['sql_table'] = {}
                    elif kind in ['zero', 'one', 'three']: o['sql_table'] = {'columns': [{} for _ in range({'zero': 0, 'one': 1, 'three': 3}[kind])]}
                    if reverse: g['edges'][0].update(src_arrow=True, dst_arrow=False)
                    for mode, requested in [('layout', []), ('empty', []), ('selected', [0]), ('duplicate', [0, 0])]:
                        for cancel in [False, True]:
                            rows.append(dict(name=f'table-ports-{width}x{height}-at{x},{y}-{kind}-reverse{reverse}-{mode}-cancel{cancel}', graph=g, mode=mode, cancel=cancel, seeds=[1], requested=requested))
    assert len(rows) == 2168 and len({a['name'] for a in rows}) == len(rows)
    return _rt(rows)


# --- public records --------------------------------------------------------

@_cached
def public_records():
    rows = geometry_conversion()
    base = copy.deepcopy(rows[0]['graph']); base['objects'][0]['box'] = {'Width': 100, 'Height': 60, 'TopLeft': {'x': 10, 'y': 20}}; base['objects'][0]['attributes']['shape']['value'] = 'rectangle'
    variants = []

    def add(name, edit):
        g = copy.deepcopy(base); edit(g); variants.append((name, g))

    def put(g, path, value):
        cur = g
        for key in path[:-1]:
            if isinstance(cur, dict) and key not in cur: cur[key] = {}
            cur = cur[key]
        cur[path[-1]] = copy.deepcopy(value)
    paths = [(['objects', 0, 'id_val'], 'authored ID'), (['objects', 0, 'zIndex'], 7), (['objects', 0, 'labelPosition'], 'INSIDE_TOP_CENTER'), (['objects', 0, 'iconPosition'], 'INSIDE_BOTTOM_CENTER'), (['objects', 0, 'contentAspectRatio'], 1.25), (['objects', 0, 'class'], {'fields': [{'name': 'field', 'type': 'int', 'visibility': 'private', 'underline': True}], 'methods': [{'name': 'f', 'return': 'bool', 'visibility': 'public'}]}), (['objects', 0, 'sql_table'], {'columns': [{'name': {'label': 'id', 'fontSize': 14, 'labelWidth': 15, 'labelHeight': 20}, 'type': {'label': 'int'}, 'constraint': ['primary_key'], 'reference': 'b.id'}]}), (['objects', 0, 'references'], [{'key': {'path': [{'unquoted_string': {'value': [{'string': 'a'}]}}]}, 'key_path_index': 1, 'map_key_edge_index': 2}]), (['objects', 0, 'attributes', 'label'], {'value': 'hello'}), (['objects', 0, 'attributes', 'labelDimensions'], {'width': 35, 'height': 15}), (['objects', 0, 'attributes', 'icon'], {'Scheme': 'https', 'Host': 'example.com', 'Path': '/icon.svg', 'User': {}}), (['objects', 0, 'attributes', 'iconStyle'], {'fill': {'value': '#ff0000'}}), (['objects', 0, 'attributes', 'constraint'], ['primary_key']), (['objects', 0, 'attributes', 'classes'], ['first', 'second']), (['objects', 0, 'attributes', 'language'], 'markdown'), (['objects', 0, 'attributes', 'near_key'], {'path': [{'unquoted_string': {'value': [{'string': 'top-left'}]}}]}), (['edges', 0, 'index'], 2), (['edges', 0, 'src_arrow'], True), (['edges', 0, 'isCurve'], True), (['edges', 0, 'zIndex'], -3), (['edges', 0, 'labelPosition'], 'OUTSIDE_TOP_CENTER'), (['edges', 0, 'labelPercentage'], 0.25), (['edges', 0, 'srcArrowhead'], {'label': {'value': 'from'}, 'labelDimensions': {'width': 25, 'height': 15}}), (['edges', 0, 'dstArrowhead'], {'shape': {'value': 'triangle'}, 'style': {'filled': {'value': 'true'}}}), (['edges', 0, 'references'], [{'map_key_edge_index': 2}])]
    for field, value in [('tooltip', 'tip'), ('link', 'https://example.com'), ('width', '100'), ('height', '60'), ('left', '10'), ('top', '20'), ('labelPosition', 'inside-center'), ('iconPosition', 'inside-center'), ('tooltipPosition', 'bottom-center')]:
        paths.append((['objects', 0, 'attributes', field], {'value': value}))
    style = {'opacity': '0.8', 'stroke': '#112233', 'fill': '#ffffff', 'fillPattern': 'lines', 'strokeWidth': '2', 'strokeDash': '3', 'borderRadius': '5', 'shadow': 'false', '3d': 'false', 'multiple': 'false', 'font': 'mono', 'fontSize': '14', 'fontColor': '#000000', 'animated': 'false', 'bold': 'true', 'italic': 'true', 'underline': 'false', 'filled': 'true', 'doubleBorder': 'false', 'textTransform': 'none'}
    for field, value in style.items():
        paths.append((['objects', 0, 'attributes', 'style', field], {'value': value}))
    for path, value in paths:
        name = '.'.join(map(str, path))
        for label, variant in [('value', value), ('null', None), ('empty-object', {}), ('empty-array', []), ('wrong-bool', False), ('wrong-number', 3.5), ('wrong-string', 'wrong')]:
            add(f'record-{name}-{label}', lambda g, p=path, v=variant: put(g, p, v))

        def alias(g, p=path, v=value):
            put(g, p, v); owner = g
            for key in p[:-1]: owner = owner[key]
            owner[p[-1].upper()] = owner.pop(p[-1])
        add(f'alias-{name}', alias)

        def unknown(g, p=path, v=value):
            put(g, p, v); owner = g
            for key in p[:-1]: owner = owner[key]
            if isinstance(owner[p[-1]], dict): owner[p[-1]]['not_a_field'] = {'x': True}
            else: owner['not_a_field'] = True
        add(f'unknown-{name}', unknown)
    # Recursive reference alternatives exercise every reachable AST record.
    for branch, payload in [('unquoted_string', {'value': [{'string': 'a', 'raw_string': 'a'}, {'substitution': {'spread': True, 'path': [{'single_quoted_string': {'raw': 'nested', 'value': 'nested'}}]}}], 'pattern': ['a', '*']}), ('double_quoted_string', {'value': [{'string': 'quoted'}]}), ('single_quoted_string', {'raw': 'raw', 'value': 'decoded'}), ('block_string', {'quote': '|', 'tag': 'md', 'value': 'text'})]:
        for text in [None, ',0:0:0-0:0:0', 'file,+01:002:3-4:05:6', 'missing', 'file,1:2-3:4:5', 'file,a:2:3-4:5:6', 'file,9223372036854775808:2:3-4:5:6']:
            node = copy.deepcopy(payload); node['range'] = text
            add(f'ast-{branch}-{text}', lambda g, n=node, b=branch: put(g, ['objects', 0, 'references'], [{'key': {'path': [{b: n}]}}]))
    for scalar in ['label', 'shape', 'direction']:
        for later in [None, {}, {'value': None}, {'value': 'rectangle' if scalar == 'shape' else ''}]:
            def edit(g, s=scalar, v=later):
                a = g['objects'][0]['attributes']; a.pop(s, None); a[s.upper()] = {'value': 'rectangle' if s == 'shape' else ''}; a[s] = v
            add(f'scalar-merge-{scalar}-{json.dumps(later)}', edit)
    for reset in [None, [], [{}], [None]]:
        def edit(g, v=reset):
            o = g['objects'][0]; o['REFERENCES'] = [{'key_path_index': 7}, {'map_key_edge_index': 8}, {'key_path_index': 9}]; o['References'] = v; o['references'] = [{}, {}, {}]
        add(f'reference-regrow-{json.dumps(reset)}', edit)
    for field in ['style', 'near_key', 'iconStyle']:
        def edit(g, f=field):
            key = f.replace('s', '\u017f').replace('S', '\u017f').replace('k', '\u212a'); g['objects'][0]['attributes'][key] = {}
        add('unicode-fold-' + field, edit)
    for value in [{}, None, {'columns': []}, {'fields': [], 'methods': []}]:
        for key in ['class', 'sql_table', 'icon']:
            path = ['objects', 0, key] if key != 'icon' else ['objects', 0, 'attributes', key]
            add(f'empty-pointers-{key}-{json.dumps(value)}', lambda g, p=path, v=value: put(g, p, v))
    for owner in ['root', 'objects', 'edges']:
        def edit(g, o=owner):
            record = g[o] if o == 'root' else g[o][0]; record['unknown'] = {'nested': []}; g['futureField'] = 'drop me'; g['data'] = {'unknown': {'nested': []}}
        add('unknown-record-' + owner, edit)
    for name, g in variants:
        for mode, requested in [('layout', []), ('empty', []), ('selected', [0]), ('duplicate', [0, 0])]:
            for cancel in [False, True]:
                rows.append(dict(name=f'{name}-{mode}-cancel{cancel}', graph=g, mode=mode, cancel=cancel, seeds=[1], requested=requested))
    # Hand-authored inputs of earlier d2-rs-lib unit tests at their original
    # [1,2,3] seeds.
    rows.extend(legacy_graphs()['public_records'])
    assert len({a['name'] for a in rows}) == len(rows)
    return _rt(rows)


def _near_box(kind, text):
    return {kind: ({'value': [{'string': text}]} if kind in ['unquoted_string', 'double_quoted_string'] else {'value': text})}


@_cached
def _near_audit():
    base = copy.deepcopy(public_records()[0]['graph'])
    base['objects'][0]['box'] = {'Width': 100, 'Height': 60, 'TopLeft': {'x': 10, 'y': 20}}
    base['objects'][0]['attributes']['shape']['value'] = 'rectangle'
    box = _near_box
    variants = [('absent', None), ('nil', None), ('empty', {}), ('nil-path', {'path': None}), ('empty-path', {'path': []}), ('nil-part', {'path': [None]}), ('empty-part', {'path': [{}]}), ('null-branches', {'path': [dict.fromkeys(['unquoted_string', 'double_quoted_string', 'single_quoted_string', 'block_string'])]}), ('invalid-second', {'path': [box('unquoted_string', 'b'), {}]}), ('invalid-first', {'path': [None, box('unquoted_string', 'b')]}), ('invalid-after-missing', {'path': [box('unquoted_string', 'missing'), {}]})]
    for kind in ['unquoted_string', 'double_quoted_string', 'single_quoted_string', 'block_string']:
        for text in ['b', 'B', 'missing', 'top-left', 'TOP-LEFT', '', 'b.c', 'width', ' two words ', 'a"b', 'a\nb']:
            variants.append((kind + '-' + repr(text), {'path': [box(kind, text)]}))
        variants.append((kind + '-empty-record', {'path': [{kind: {}}]}))
    for kind in ['unquoted_string', 'double_quoted_string']:
        for name, parts in [('nil-value', None), ('empty-value', []), ('nil-first', [None]), ('empty-first', [{}]), ('substitution-first', [{'substitution': {'path': [box('unquoted_string', 'b')]}}, {'string': 'b'}]), ('first-only', [{'string': 'b'}, {'string': '.missing'}]), ('string-and-substitution', [{'string': 'b', 'substitution': {}}])]:
            variants.append((kind + '-' + name, {'path': [{kind: {'value': parts}}]}))
    variants += [('all-branches-priority', {'path': [{**box('unquoted_string', 'missing'), **box('double_quoted_string', 'b'), **box('single_quoted_string', 'top-left'), **box('block_string', 'b')}]}), ('null-first-live-second', {'path': [{'unquoted_string': None, **box('double_quoted_string', 'b')}]}), ('legacy-quoted-field', {'path': [{'quoted_string': {'value': [{'string': 'b'}]}}]})]
    for depth in [256, 257]:
        for invalid in [False, True]:
            variants.append((f'depth-{depth}-invalid-{invalid}', {'path': [None if invalid else box('unquoted_string', 'b') for _ in range(depth)]}))
    rows = []
    for name, near in variants:
        for mode, requested in [('layout', []), ('empty', []), ('selected', [0]), ('duplicate', [0, 0])]:
            for cancel in [False, True]:
                graph = copy.deepcopy(base)
                if name != 'absent': graph['objects'][0]['attributes']['near_key'] = copy.deepcopy(near)
                rows.append(dict(name=f'near-audit-{name}-{mode}-cancel{cancel}', graph=graph, mode=mode, cancel=cancel, seeds=[1], requested=requested))
    return _rt(rows)


# Object IDs whose canonical d2format key spelling the near-keys family probes
# (NEAR_KEY_VALUES, defined with the raw-graph helpers below).


def near_key_values() -> list[str]:
    """Requests for the oracle's `key_values` mode (d2format.KeyPath)."""
    return list(NEAR_KEY_VALUES)


@_cached
def near_keys(key_values_json: str):
    """`key_values_json`: the oracle's [{value,key}] answer for near_key_values()."""
    key_values = json.loads(key_values_json)
    assert [item['value'] for item in key_values] == near_key_values()
    rows = _near_audit(); base = copy.deepcopy(rows[0]['graph'])
    box = _near_box

    def add(name, g, seeds=[1]):
        for mode, selected in [('layout', []), ('empty', []), ('selected', [0]), ('duplicate', [0, 0])]:
            for cancel in [False, True]:
                rows.append(dict(name=f'{name}-{mode}-cancel{cancel}', graph=g, mode=mode, cancel=cancel, seeds=seeds, requested=selected))
    for item in key_values:
        for kind in ['unquoted_string', 'double_quoted_string', 'single_quoted_string', 'block_string']:
            for nested in [False, True]:
                g = copy.deepcopy(base); key = item['key']; target = 'group.' + key if nested else key
                b = g['objects'][1]; b['id'] = key; b['AbsID'] = target; g['edges'][0]['Dst'] = target
                if nested:
                    group = {'AbsID': 'group', 'id': 'group', 'box': {'Width': 400, 'Height': 300, 'TopLeft': {'x': 200, 'y': 0}}, 'ChildrenArray': [target], 'attributes': {'shape': {'value': 'rectangle'}}}
                    g['objects'].insert(1, group); g['root']['ChildrenArray'] = ['a', 'group']
                else: g['root']['ChildrenArray'] = ['a', target]
                g['objects'][0]['attributes']['near_key'] = {'path': ([box(kind, 'group')] if nested else []) + [box(kind, item['value'])]}
                add(f'canonical-target-{kind}-{item["value"]!r}-nested{nested}', g)
    for near_name, near in [('invalid', {'path': [None]}), ('depth-before-invalid', {'path': [None] * 257})]:
        for failure in ['missing-box', 'negative-width', 'missing-point', 'negative-label-width', 'invalid-label-position', 'invalid-style', 'invalid-route-point', 'single-point-route', 'empty-seeds']:
            g = copy.deepcopy(base); a = g['objects'][0]; a['attributes']['near_key'] = near
            if failure == 'missing-box': a['box'] = None
            elif failure == 'negative-width': a['box']['Width'] = -1
            elif failure == 'missing-point': a['box']['TopLeft'] = None
            elif failure == 'negative-label-width': a['attributes']['labelDimensions'] = {'width': -1, 'height': 0}
            elif failure == 'invalid-label-position': a['labelPosition'] = 'bad position'
            elif failure == 'invalid-style': a['attributes']['style'] = {'opacity': {'value': 'bad'}}
            elif failure == 'invalid-route-point': g['edges'][0]['route'][0] = None
            elif failure == 'single-point-route': g['edges'][0]['route'] = g['edges'][0]['route'][:1]
            add(f'precedence-{near_name}-{failure}', g, [] if failure == 'empty-seeds' else [1])
    assert len({x['name'] for x in rows}) == len(rows)
    return _rt(rows)


NEAR_KEY_VALUES = ["b.c", "", "b b", "b\"c", "b'c", "b\nc", "b\\c", "Width", "a--b", "-", "&a", "a$z", "ñ", "B", " x ", "a\"'b", "a|b", "a@b"]


# --- raw graph -------------------------------------------------------------

def _canonical_graph():
    return {'root': {'AbsID': '', 'id': '', 'ChildrenArray': ['a', 'b']},
            'objects': [{'AbsID': name, 'id': name, 'box': {'Width': 100, 'Height': 60, 'TopLeft': {'x': x, 'y': 20}},
                         'attributes': {'shape': {'value': 'rectangle'}}} for name, x in [('a', 10), ('b', 250)]],
            'edges': [{'Src': 'a', 'Dst': 'b', 'index': 0, 'dst_arrow': True, 'route': [{'x': 110, 'y': 50}, {'x': 250, 'y': 50}]}],
            'rootLevel': 0}


def _encode(value):
    return json.dumps(value, separators=(',', ':'), ensure_ascii=False)


def _raw_rows(entries):
    return [{'name': f'{name}-cancel{cancel}', 'raw_graph': raw, 'mode': 'layout', 'cancel': cancel, 'seeds': [1], 'requested': []}
            for name, raw in entries for cancel in [False, True]]


@_cached
def _raw_audit():
    """Top-level DeserializeGraph field aliases, types and number spellings."""
    base = _canonical_graph(); canonical = _encode(base); entries = [('canonical', canonical)]

    def moved(field, spelling, value):
        g = {k: v for k, v in base.items() if k != field}; g[spelling] = value; return _encode(g)
    for field, spellings in [('root', ['ROOT', 'Root']), ('objects', ['OBJECTS', 'Objects']), ('edges', ['EDGES', 'Edges']), ('rootLevel', ['ROOTLEVEL', 'RootLevel', 'ROOTlEVEL']), ('data', ['DATA', 'Data'])]:
        for spelling in spellings:
            entries.append((f'alias-{field}-{spelling}', moved(field, spelling, base.get(field, {'retained': 'value'}))))
    for value in [None, 0, -1, 2, 1.0, 1.5, '2', True, [], {}, 9223372036854775807, 9223372036854775808]:
        entries.append((f'root-level-{value!r}', _encode(dict(base, rootLevel=value))))
    for value in [None, {}, [], True, 'text', {'nested': {'number': 9007199254740993}}]:
        entries.append((f'data-{value!r}', _encode(dict(base, data=value))))
    for field in ['root', 'objects', 'edges']:
        for value in [False, 'wrong', 42]:
            entries.append((f'type-{field}-{value!r}', _encode(dict(base, **{field: value}))))
    head = _encode({k: v for k, v in base.items() if k != 'rootLevel'})[:-1]
    for first, second in [('"ROOTLEVEL":7', '"rootLevel":null'), ('"rootLevel":8', '"rootLevel":9'), ('"rootLevel":1.0', '"rootLevel":2')]:
        for a, b in [(first, second), (second, first)]:
            entries.append((f'ordered-{a}-then-{b}', head + ',' + a + ',' + b + '}'))
    for lexical in ['1.0', '1e0', '1e1', '-0', '9223372036854775807', '9223372036854775808']:
        entries.append((f'lexical-root-level-{lexical}', head + ',"rootLevel":' + lexical + '}'))
    return _rt(_raw_rows(entries))


@_cached
def raw_graph():
    rows = _raw_audit(); base = json.loads(rows[0]['raw_graph']); variants = []
    encode=lambda x:json.dumps(x,separators=(',',':'),ensure_ascii=False)
    def add(name,raw,seeds=[1]):
        for cancel in [False,True]:rows.append(dict(name='expanded-'+name+'-cancel'+str(cancel),raw_graph=raw,mode='layout',cancel=cancel,seeds=seeds,requested=[]))
    def fields(pairs):return '{'+','.join(encode(k)+':'+v for k,v in pairs)+'}'
    def with_fields(pairs,remove=()):return fields([(k,encode(v)) for k,v in base.items() if k not in remove]+pairs)
    for raw in ['false','true','1','1.0','"text"','[]','[{}]']:add('top-'+raw,raw)
    for raw in ['{}','{"root":null}','{"root":{}}','{"root":{"id":"authored root"}}','{"objects":[],"edges":[]}','{"root":{"AbsID":"","ChildrenArray":[]},"objects":null,"edges":null}']:add('empty-'+raw,raw)
    for name,kind in [('root','d2graph.SerializedObject'),('objects','[]d2graph.SerializedObject'),('edges','[]d2graph.SerializedEdge'),('data','map[string]interface {}'),('rootLevel','int')]:
        for spelling in [name.upper(),name.capitalize(),name.replace('s','\u017f')]:
            add('typed-alias-'+name+'-'+spelling,with_fields([(spelling,'true')],remove=[name]))
    for field in ['root','data']:
        old=copy.deepcopy(base['root']) if field=='root' else {'keep':{'old':True},'number':1}
        old['zIndex' if field=='root' else 'keep_scalar']=9
        for middle in ['{}','null','false','[]']:
            last=encode(base['root']) if field=='root' else '{"keep":{"new":true}}'
            for duplicate in [False,True]:
                pairs=[(field,encode(old)),(field if duplicate else field.upper(),middle),(field if duplicate else field.capitalize(),last)]
                add(f'map-merge-{field}-{middle}-duplicate{duplicate}',with_fields(pairs,remove=[field]))
    for field in ['objects','edges']:
        original=copy.deepcopy(base[field]);original[0]['zIndex']=17
        if len(original)>1:original[1]['zIndex']=23
        for middle in ['[]','null','[{}]','[null]','[false]','true']:
            # Refilling all identities keeps post-reset controls well-formed.
            for final_kind in ['full','short-regrow']:
                pairs=[(field,encode(original)),(field.upper(),middle)]
                if final_kind=='short-regrow':pairs += [(field.capitalize(),encode(base[field][:1]))]
                pairs += [(field,encode(base[field]))]
                add(f'slice-merge-{field}-{middle}-{final_kind}',with_fields(pairs,remove=[field]))
        for value in ['false','42','"wrong"','[]']:
            add('slice-element-'+field+'-'+value,with_fields([(field,'['+value+']')],remove=[field]))
    # Map member replacement, preservation of earlier error and exact input paths.
    for first,second in [('"rootLevel":1.5','"data":false'),('"data":false','"rootLevel":1.5'),('"ROOTLEVEL":1e0','"rootLevel":2'),('"ROOTLEVEL":null','"rootLevel":2')]:
        add('error-order-'+first+'-'+second,encode(base)[:-1]+','+first+','+second+'}')
    for lexical in ['-0','-0.0','0.0','1e0','1E+0','1e6','9007199254740993','9223372036854775807','-9223372036854775808','1.7976931348623157e308','1.7976931348623159e308','1e1000','-1e1000','1e-400','-1e-400']:
        for field,value in [('rootLevel',lexical),('data','{"number":'+lexical+'}'),('unknown',lexical)]:
            add('number-'+field+'-'+lexical,with_fields([(field,value)],remove=[field]))
    for path in ['value','','0','a.b','a/b','a~b']:
        add('overflow-path-'+repr(path),with_fields([('data',fields([(path,'1e1000')]))],remove=['data']))
    for raw in ['{"number":1e1000,"number":0}','{"number":0,"number":1e1000}','{"nested":[1e1000]}','{"nested":{"0":1e1000}}']:
        add('data-overflow-'+raw,with_fields([('data',raw)],remove=['data']))
    for field in ['root','objects','edges']:
        value=copy.deepcopy(base[field]);record=value if field=='root' else value[0];record['extra']='__NUMBER__';raw=encode(value).replace('"__NUMBER__"','1e1000')
        add('record-overflow-'+field,with_fields([(field,raw)],remove=[field]))
    seed_values=[None,True,{},'bad',[],[7,7,9],[7.0],[-0.0],[999999],[1000000],['1000000'],[0.0001],[0.00001],['7.0'],['+7'],['01'],['9223372036854775807'],[9223372036854775807],[9007199254740993],[None],[True],[[]],[[7,None]], [{'z':[1,None],'a':{'nested':True}}], [1]*64,[None]*65]
    for seed_value in seed_values:
        for bad_geometry in [False,True]:
            g=copy.deepcopy(base);g['data']={'tala-seeds':seed_value}
            if bad_geometry:g['objects'][0]['box']['Width']=-1
            add('seed-data-'+repr(seed_value)+'-bad-geometry'+str(bad_geometry),encode(g))
        for opts in [[],[1]]:
            g=copy.deepcopy(base);g['data']={'tala-seeds':seed_value};add('seed-options-'+repr(seed_value)+'-'+repr(opts),encode(g),opts)
    assert len({x['name'] for x in rows}) == len(rows)
    return _rt(rows)


# --- JSON syntax -----------------------------------------------------------

@_cached
def _syntax_audit():
    raws=['',' ','\n\t','{','[','{"','[-]','{"x":}','{"root":true,}','{"root":false,"oops":[}','{} {}','{} true','tru','fals','nul','True','NaN','Infinity','undefined','null','{"root" {}}','{"root":{} "objects":[]}','{"data":{"x" 1}}']
    for token in ['01','-01','+1','1e','1e+','1.','.1','-.1','-']:raws.append('{"rootLevel":'+token+'}')
    for token in ['\\x','\\u12','\\uZZZZ','\\','a\nb','a\tb','a\x00b']:raws.append('{"data":{"x":"'+token+'"}}')
    for token in ['\\ud800','\\udc00','\\ud800\\udc00','\\ud800\\ud800\\udc00','\\udc00\\ud800','\\ud800x','\\u0000','literal<&>','literal\u2028line']:
        raws.append('{"data":{"x":"'+token+'"}}')
    for token in ['\\ud800','\\udc00','\\ud800\\udc00']:
        raws.append('{"data":{"'+token+'":1}}');raws.append('{"'+token+'":1}')
    for depth in [64,128,129,256]:raws.append('{"data":{"deep":'+'['*depth+'0'+']'*depth+'}}')
    rows=[]
    for i,raw in enumerate(raws):
        for cancel in [False,True]:rows.append({'name':f'syntax-{i:03d}-cancel{cancel}','raw_graph':raw,'mode':'layout','cancel':cancel,'seeds':[1],'requested':[]})
    return _rt(rows)


@_cached
def json_syntax():
    inputs = _syntax_audit()
    def add(name,raw):
        for cancel in [False,True]:
            item={'name':name+f'-cancel{cancel}','raw_graph':raw if isinstance(raw,str) else '', 'mode':'layout','cancel':cancel,'seeds':[1],'requested':[]}
            if isinstance(raw,bytes):item['raw_bytes']=base64.b64encode(raw).decode()
            inputs.append(item)
    for literal in ['null','true','false']:
        for pos in range(len(literal)):
            add(f'literal-{literal}-eof-{pos}', '{"unknown":'+literal[:pos])
            for replacement in ['X','"',"'",'\\','\n','\u754c','\u2028']:
                add(f'literal-{literal}-{pos}-{ord(replacement):x}','{"unknown":'+literal[:pos]+replacement+literal[pos+1:]+'}')
    for byte in range(128):
        c=chr(byte)
        for name,raw in [('top',c),('key','{'+c+'}'),('colon','{"x"'+c+'0}'),('array','[0'+c+']'),('object','{"x":0'+c+'}'),('escape','{"data":{"x":"\\'+c+'"}}')]:add(f'ascii-{name}-{byte:02x}',raw)
    for raw in ['[}', '[0}', '{]', '{"x":]', '{"x":0]', '[0,]', '{"x":0,}', '{}\ufeff', '\ufeff{}', '"\U00010000" \U00010001', '{}\u200b', '{}\u0085', '{}\u00a0', '{}\u2028', '{}\ufffd', '{}\x7f']:
        add('delimiter-unicode-'+raw,raw)
    for token in ['-', '-0', '-00', '00', '0.', '0.e0', '0e', '0e+', '0e-', '1e-0', '1e+0001', '1E100000', '1e-100000', '1_0', '0x10', '.0', '+0', '--1']:
        for envelope in ['{}','{{"rootLevel":{}}}','{{"unknown":{}}}']:add('number-'+envelope+'-'+token,envelope.format(token))
    for text in ['\\u','\\u1','\\u12','\\u123','\\u1234','\\u1g','\\u12g','\\u123g','\\u\n','\\u\u754c','\\u`','\\u  ','\\u"', '\\\u754c','\\`','\\\ufffd','\\\u2028','\\\x7f']:
        add('escape-eof-'+repr(text),'"'+text)
        add('escape-complete-'+repr(text),'{"data":{"s":"'+text+'"}}')
    for first in ['d7ff','d800','dbff','dc00','dfff','e000','0000']:
        for second in ['', 'x', '\\u0041','\\ud800','\\udbff','\\udc00','\\udfff','\\uFFFF','\\u12','\\uZZZZ','\\\\udc00']:
            token='\\u'+first+second
            for where,raw in [('value','{"data":{"s":"'+token+'"}}'),('key','{"data":{"'+token+'":1}}'),('unknown','{"'+token+'":1}'),('duplicate','{"data":{"'+token+'":1,"\ufffd":2}}')]:add('surrogate-'+where+'-'+first+'-'+second,raw)
    for depth in [1,127,128,129,256,9998,9999,10000,10001]:
        add('unknown-depth-'+str(depth),'{"unknown":'+'['*depth+'0'+']'*depth+'}')
        add('truncated-depth-'+str(depth),'{"unknown":'+'['*depth)
    for raw in [bytes([c]) for c in range(128,256)]+[b'\xc0\x80',b'\xe0\x80\x80',b'\xed\xa0\x80',b'\xf0\x80\x80\x80',b'\xf4\x90\x80\x80',b'\xe2\x82',b'\xf0\x90\x80',b'\xef\xbf\xbd',b'\xc2\xa0']:
        for where,text in [('top',raw),('value',b'{"data":{"s":"'+raw+b'"}}'),('key',b'{"data":{"'+raw+b'":1}}'),('escape',b'{"unknown":"\\'+raw+b'"}')]:add('bytes-'+where+'-'+raw.hex(),text)
    assert len({i['name'] for i in inputs}) == len(inputs)
    return _rt(inputs)


# --- graph identity --------------------------------------------------------

@_cached
def _identity_audit():
    """Root, local and lookup identities of the canonical two-object graph."""
    base = _canonical_graph(); entries = [('control', base)]

    def alias(g, names):
        mapping = dict(zip(['a', 'b'], names)); g['root']['ChildrenArray'] = list(names)
        for obj in g['objects']: obj['AbsID'] = mapping[obj['AbsID']]
        for edge in g['edges']:
            for side in ['Src', 'Dst']: edge[side] = mapping[edge[side]]
    for value in ['root', 'ROOT', 'root.branch', '"root name"', '界', '']:
        g = copy.deepcopy(base); g['root']['id'] = value; entries.append((f'root-id-{value}', g))
    for value in ['renamed', 'UPPER', 'a.b', '"a b"', '界', '']:
        g = copy.deepcopy(base); g['objects'][0]['id'] = value; entries.append((f'local-id-{value}', g))
    for names in [('alias_a', 'alias_b'), ('C.a', 'C.b'), ('a.b', 'a.c'), ('"a name"', '界')]:
        g = copy.deepcopy(base); alias(g, names); entries.append((f'lookup-alias-{names!r}', g))
    for children in [[], ['a'], ['b'], ['b', 'a'], ['a', 'a'], ['b', 'b'], ['a', 'b', 'a']]:
        g = copy.deepcopy(base); g['root']['ChildrenArray'] = children; entries.append((f'root-children-{children!r}', g))
    g = copy.deepcopy(base); del g['root']['ChildrenArray']; entries.append(('root-no-children', g))
    g = copy.deepcopy(base); g['objects'].reverse(); entries.append(('object-order-reversed', g))
    for name in ['a', 'b']:
        g = copy.deepcopy(base); alias(g, (name, name)); entries.append((f'duplicate-lookup-{name}', g))
    for ids in [('c', 'c.a', 'c.b'), ('container', 'lookup_a', 'lookup_b')]:
        for local in ['c', 'renamed', '']:
            g = copy.deepcopy(base); alias(g, ids[1:]); g['root']['ChildrenArray'] = [ids[0]]
            g['objects'].insert(0, {'AbsID': ids[0], 'id': local, 'box': {'Width': 400, 'Height': 300, 'TopLeft': {'x': 10, 'y': 20}},
                                    'attributes': {'shape': {'value': 'rectangle'}}, 'ChildrenArray': list(ids[1:])})
            entries.append((f'container-{ids!r}-id{local}', g))
    return _rt(_raw_rows([(name, _encode(g)) for name, g in entries]))


@_cached
def graph_identity():
    rows = _identity_audit(); base = json.loads(rows[0]['raw_graph']); variants = {}
    def add(name,graph,seeds=[1],modes=('layout','route-empty','route-single')):
        for mode in modes:
            for cancel in [False,True]:rows.append({'name':f'expanded-{name}-{mode}-cancel{cancel}','raw_graph':json.dumps(graph,separators=(',',':'),ensure_ascii=False),'mode':'layout' if mode=='layout' else 'route','cancel':cancel,'seeds':seeds,'requested':[] if mode in ['layout','route-empty'] else [0] if graph.get('edges') else []})
    def alias(g,names):
        mapping=dict(zip(['a','b'],names));g['root']['ChildrenArray']=list(names)
        for obj in g['objects']:obj['AbsID']=mapping[obj['AbsID']]
        for edge in g['edges']:
            for side in ['Src','Dst']:edge[side]=mapping[edge[side]]
    for root_id in ['','r','r.a','"root name"','\u754c','\u200b']:
        for names in [('a','b'),('lookup_x','lookup_y'),('not.a','not.b')]:
            for near in [False,True]:
                g=copy.deepcopy(base);g['root']['id']=root_id;alias(g,names)
                if near:g['objects'][0]['attributes']['near_key']={'path':[{'unquoted_string':{'value':[{'string':'b'}]}}]}
                add('root-alias-near-'+repr((root_id,names,near)),g)
    for names in [('a','a'),('a','A'),('K','\u212a'),('S','\u017f'),('\u0130','i'),('x.y','x'),('','b'),('"a.b"','b'),('\u754c','\u754c')]:
        g=copy.deepcopy(base)
        for obj,name in zip(g['objects'],names):obj['id']=name
        add('local-names-'+repr(names),g)
    for root_children,a_children,b_children in [(['a'],['b'],[]),(['a','b'],['b'],[]),(['b','a'],['b'],[]),(['a'],['b'],['a']),(['a'],[''],[]),([''],[],[]),(['a','a','b'],[],[]),([],[],[])]:
        g=copy.deepcopy(base);g['root']['ChildrenArray']=root_children;g['objects'][0]['ChildrenArray']=a_children;g['objects'][1]['ChildrenArray']=b_children
        variants['tree-'+repr((root_children,a_children,b_children))]=g;add('tree-'+repr((root_children,a_children,b_children)),g)
    g=copy.deepcopy(base);g['root']['ChildrenArray']=['b','b'];g['objects'][0]['AbsID']='b';variants['lookup-duplicate']=g;add('lookup-duplicate',g)
    # Distinct root/descendant pointers with one computed absolute identity.
    g=copy.deepcopy(base);c=copy.deepcopy(g['objects'][0]);c.update(AbsID='c',id='',ChildrenArray=['a']);c['box']['Width']=400;c['box']['Height']=300
    g['objects'].insert(0,c);g['objects'][2]['id']='a';g['root']['ChildrenArray']=['c','b'];variants['canonical-duplicate']=g;add('canonical-duplicate',g)
    g=copy.deepcopy(base);g['edges'].append(copy.deepcopy(g['edges'][0]));variants['edge-duplicate']=g;add('edge-duplicate',g)
    g=copy.deepcopy(base);g['objects'][1]['id']='A';variants['children-map-collision']=g
    for name,graph in variants.items():
        for bad in ['negative-width','no-box','bad-style','one-route-point','bad-near','bad-data-seeds','no-caller-seeds']:
            g=copy.deepcopy(graph);seeds=[1]
            if bad=='negative-width':g['objects'][0]['box']['Width']=-1
            elif bad=='no-box':g['objects'][0].pop('box')
            elif bad=='bad-style':g['objects'][0]['attributes']['style']={'strokeWidth':{'value':'bad'}}
            elif bad=='one-route-point':g['edges'][0]['route']=[{'x':0,'y':0}]
            elif bad=='bad-near':g['objects'][0]['attributes']['near_key']={'path':[]}
            elif bad=='bad-data-seeds':g['data']={'tala-seeds':['bad']}
            else:seeds=[]
            add(name+'-versus-'+bad,g,seeds)
    for count in [9999,10000,10001]:
        g=copy.deepcopy(base);g['root']['ChildrenArray']=['a']*count;add('scheduled-'+str(count),g)
    for depth in [255,256,257,258]:
        g={'root':{'AbsID':'','id':'','ChildrenArray':['n0']},'objects':[],'edges':[]}
        for index in range(depth):
            obj=copy.deepcopy(base['objects'][0]);obj.update(AbsID='n'+str(index),id='n'+str(index),ChildrenArray=['n'+str(index+1)] if index+1<depth else [])
            g['objects'].append(obj)
        g['objects'][0].pop('box');add('depth-before-geometry-'+str(depth),g)
    # Exercise the full source results for repaired hand-authored test inputs with
    # their original [1,2,3] defaults; the original incomplete inputs stay separate.
    legacy_tests = legacy_graphs()['graph_identity']
    for capture in legacy_tests:
        name = capture['test']
        mode = 'route-single' if name.startswith('tests::routeedges_') else 'layout'
        add('legacy-control-' + name, capture['graph'], [1, 2, 3], (mode,))
    # Source adapter.go: existing edge labels are restored only with a
    # non-nil position, then mirrored with the source-only route. Fresh layout
    # starts both fields at zero/unset. Include absent and null pointer controls.
    label_base = [c['graph'] for c in legacy_tests if c['test'] == 'tests::routeedges_restores_source_only_wire_orientation_and_label-0'][0]
    for position in ['MISSING',None,'','unknown','OUTSIDE_TOP_LEFT','OUTSIDE_BOTTOM_CENTER','OUTSIDE_RIGHT_MIDDLE','INSIDE_MIDDLE_CENTER','INSIDE_BOTTOM_RIGHT','BORDER_TOP_CENTER','BORDER_RIGHT_BOTTOM','UNLOCKED_TOP','UNLOCKED_MIDDLE','UNLOCKED_BOTTOM']:
        for percentage in ['MISSING',None,0.0,0.125,0.75,1.0]:
            for src_arrow,dst_arrow in [(False,False),(False,True),(True,False),(True,True)]:
                g=copy.deepcopy(label_base);e=g['edges'][0];e['src_arrow']=src_arrow;e['dst_arrow']=dst_arrow
                if position=='MISSING':e.pop('labelPosition')
                else:e['labelPosition']=position
                if percentage=='MISSING':e.pop('labelPercentage')
                else:e['labelPercentage']=percentage
                add('label-position-'+repr((position,percentage,src_arrow,dst_arrow)),g)
    assert len({row['name'] for row in rows}) == len(rows)
    return _rt(rows)


FAMILIES = {
    'wire-geometry': wire_geometry,
    'input-attributes': input_attributes,
    'input-tables': input_tables,
    'missing-geometry': missing_geometry,
    'geometry-conversion': geometry_conversion,
    'public-records': public_records,
    'near-keys': near_keys,
    'raw-graph': raw_graph,
    'json-syntax': json_syntax,
    'graph-identity': graph_identity,
}
