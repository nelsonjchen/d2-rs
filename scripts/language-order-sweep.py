#!/usr/bin/env python3
"""Seeded language ordering/import-board interactions over upstream sources.

Uses differential-scoreboard's inventories, provenance and capture budgets.
The older language-combination queue remains unchanged and reproducible.
"""
import importlib.util
from pathlib import Path
import random
import sys

spec = importlib.util.spec_from_file_location(
    'combinations', Path(__file__).with_name('language-combination-sweep.py'))
combinations = importlib.util.module_from_spec(spec)
spec.loader.exec_module(combinations)
diff = combinations.diff


def order_jobs(seed, cap):
    rng = random.Random(seed)
    corpus = combinations.upstream_cases()
    rng.shuffle(corpus)
    families = ('import-board-target', 'filter-override-order', 'class-board-order',
                'variable-null-order', 'spread-alias-order')
    seen = set()
    for index in range(cap * 16):
        family = families[index % len(families)]
        name, original = corpus[index // len(families) % len(corpus)]
        color = rng.choice(('red', 'blue', 'orange', '#123456'))
        shape = rng.choice(('rectangle', 'circle', 'oval', 'person'))
        collection = rng.choice(('layers', 'scenarios', 'steps'))
        fixtures = {}
        options = ['--layout=' + rng.choice(('dagre', 'elk', 'tala'))]
        if family == 'import-board-target':
            fixtures = {'lib/model.d2': original,
                        'lib/relay.d2': '...@model\n'}
            imported = rng.choice(('@lib/model', '@lib/relay'))
            source = f'{collection}: {{review: {imported}; final: {{probe -> next}}}}\n'
            source += rng.choice((f'{collection}.review.probe.style.fill: {color}\n',
                                  f'{collection}.review: {{probe -> next}}\n',
                                  f'{collection}.final.probe: null\n'))
            if rng.randrange(2):
                options += ['--target=' + collection + '.' + rng.choice(('review', 'final'))]
        elif family == 'filter-override-order':
            selector = rng.choice(('&leaf: true', '!&leaf: true', '&level: 1',
                                   '&connected: true', '&shape: circle', '&label: *'))
            statements = [f'**: {{{selector}; style.fill: {color}}}\n',
                          f'probe: {{a -> b; c: {{shape: {shape}}}}}\n',
                          rng.choice(('probe.a: null\nprobe.a\n',
                                      'probe.b: suspend\nprobe.b: unsuspend\n',
                                      'probe.c.shape: circle\n'))]
            rng.shuffle(statements)
            source = original + '\n' + ''.join(statements)
        elif family == 'class-board-order':
            declarations = (f'classes: {{hunt: {{style.fill: {color}; shape: {shape}}}; '
                            'other: {style.bold: true}}\n')
            board = (f'{collection}: {{review: {{classes.hunt.style.fill: null; '
                     'probe.class: [hunt; other]}; final: {probe.class: hunt}}}\n')
            source = declarations + original + '\n' + board
            source += rng.choice(('probe.class: hunt\n',
                                  'classes.hunt: null\nclasses.hunt.style.stroke: blue\n',
                                  '**.class: [other; hunt]\n'))
        elif family == 'variable-null-order':
            source = (f'vars: {{hunt: {{shape: {shape}; style.fill: {color}}}; word: hello}}\n'
                      + original + '\n')
            source += rng.choice((
                'probe: {...${hunt}; style.fill: null}\nvars.hunt.style.fill: blue\n',
                f'{collection}: {{review: {{vars.word: null; probe: ${{word}}}}}}\n',
                'probe: ${hunt}\nprobe: null\nprobe: {...${hunt}}\n',
                'box: {vars.hunt: null; probe: ${hunt}}\n'))
        else:
            fixtures = {'lib/model.d2': original,
                        'lib/relay.d2': 'copy: @model\n...@model\n'}
            source = rng.choice(('a: @lib/model\nb: @lib/model\n',
                                 '...@lib/relay\n',
                                 'a: @lib/model\na: null\na: @lib/model\n'))
            source += rng.choice((f'**.style.fill: {color}\n',
                                  'a: {probe -> next}\n',
                                  f'{collection}: {{review: {{a: null; b: @lib/model}}}}\n'))
        identity = (source, tuple(options), tuple(sorted(fixtures.items())))
        if identity in seen:
            continue
        seen.add(identity)
        yield name, source, 'order-' + family, options, fixtures
        if len(seen) == cap:
            return
    raise RuntimeError('could not generate requested distinct jobs')


if __name__ == '__main__':
    diff.language_jobs = order_jobs
    if '--language-matrix' not in sys.argv:
        sys.argv.append('--language-matrix')
    sys.exit(diff.main())
