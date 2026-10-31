#!/usr/bin/env python3
"""Capture original pinned color tests, then compare native public gradient APIs.

All original assertions remain active. Fresh capture/replay paths, exact results,
1 GiB total / 128 MiB per file / 20 GiB free streaming guard, finite deadlines.
"""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
TEST_FILE = 'lib/color/gradient_test.go'
EXPECTED = {
    'TestOneStopGradientToSVG/linear-gradient(red)': 2,
    'TestOneStopGradientToSVG/radial-gradient(red)': 2,
    'TestParseGradientValidatesStopPositions': 1,
    'TestGradientToSVGEscapesConstructedAttributes': 1,
}
INVALID = [
    'linear-gradient(red 0%"/><script>document.documentElement.dataset.pwned=1</script><!--, blue --><stop/>)',
    'radial-gradient(red calc(1%), blue)', 'linear-gradient(red NaN, blue)',
    'linear-gradient(red +Inf%, blue)', 'linear-gradient(red 20px, blue)',
]
EXPECTED.update({'TestParseGradientValidatesStopPositions/' + x.replace(' ', '_'): 2 for x in INVALID})
TESTS = sorted({x.split('/')[0] for x in EXPECTED})


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source():
    pinned = ROOT / references.MANIFEST['d2']['checkout']
    return {'commit': references.SOURCE_COMMIT,
            'test': sha(pinned / TEST_FILE),
            'implementation': {n: sha(pinned / ('lib/color/' + n)) for n in ['gradient.go', 'color.go']},
            'hook': sha(ROOT / 'tools/color-scoreboard/hook_test.go')}


def hashes(capture):
    paths = [capture / n for n in ['source.json', 'inventory.json', 'go.jsonl', 'run.json']]
    paths += sorted((capture / 'go').glob('*.json'))
    return {str(p.relative_to(capture)): sha(p) for p in paths}


def verify(capture):
    assert json.loads((capture / 'source.json').read_text()) == source()
    assert json.loads((capture / 'hashes.json').read_text()) == hashes(capture)
    assert json.loads((capture / 'run.json').read_text()) == {'exit': 0, 'failure': None}
    assert json.loads((capture / 'inventory.json').read_text()) == EXPECTED
    events = module('cli-scoreboard').events(capture / 'go.jsonl')
    assert all(events.get(t) == 'pass' for t in TESTS), events
    cases = [json.loads(p.read_text()) for p in sorted((capture / 'go').glob('*.json'))]
    expected = {f'{t}#{i}' for t, n in EXPECTED.items() for i in range(n)}
    assert len(cases) == len(expected) and {c['name'] for c in cases} == expected
    sizes = [p.stat().st_size for p in capture.rglob('*') if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    path = references.TRACE_CHECKOUT / TEST_FILE
    original = (pinned / TEST_FILE).read_bytes()
    helper = path.parent / 'd2rs_color_test.go'
    assert path.read_bytes() == original and not helper.exists()
    (capture / 'go').mkdir(parents=True, exist_ok=False)
    (capture / 'source.json').write_text(json.dumps(source(), indent=2) + '\n')
    (capture / 'inventory.json').write_text(json.dumps(EXPECTED, indent=2) + '\n')
    text = original.decode()
    for old, new in [('ParseGradient(', 'd2rsParseGradient(t, '), ('GradientToSVG(', 'd2rsGradientToSVG(t, '), ('ValidColor(', 'd2rsValidColor(t, ')]:
        # Never rewrite function declarations or assertion messages.
        text = text.replace(':= ' + old, ':= ' + new).replace('if ' + old, 'if ' + new)
    env = dict(os.environ, D2RS_COLOR_DUMP=str(capture / 'go'),
               GOMODCACHE=str(ROOT / '.reference/go-mod/pkg/mod'),
               GOCACHE=str(ROOT / '.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off')
    env.pop('TA', None)
    env.pop('TESTDATA_ACCEPT', None)
    try:
        path.write_text(text)
        helper.write_bytes((ROOT / 'tools/color-scoreboard/hook_test.go').read_bytes())
        result = module('cli-scoreboard').bounded_run(
            ['go', 'test', './lib/color', '-run', '^(' + '|'.join(TESTS) + ')$', '-json', '-count=1', '-parallel=1'],
            env, capture, capture / 'go.jsonl', 300)
    finally:
        path.write_bytes(original)
        helper.unlink(missing_ok=True)
    (capture / 'run.json').write_text(json.dumps(result) + '\n')
    assert result == {'exit': 0, 'failure': None}, result
    (capture / 'hashes.json').write_text(json.dumps(hashes(capture), indent=2) + '\n')
    verify(capture)


def score(capture, runner, output):
    cases = verify(capture)
    work = output.parent / (output.stem + '-native')
    work.mkdir(exist_ok=False)
    runner_hash = sha(runner)
    rows = []
    for case in cases:
        key = hashlib.sha256(case['name'].encode()).hexdigest()
        request, log = work / (key + '.request.json'), work / (key + '.json')
        request.write_text(json.dumps(case['request']))
        run = module('cli-scoreboard').bounded_run([str(runner.resolve()), str(request.resolve())], os.environ.copy(), work, log, 30)
        row = {'name': case['name'], 'test': case['test']}
        if run['failure'] or run['exit']:
            row['outcome'] = run['failure'] or 'rust-failed'
        else:
            actual = json.loads(log.read_text())
            row['outcome'] = 'match' if actual == case['got'] else 'mismatch'
            if row['outcome'] != 'match':
                row['expected'], row['actual'] = case['got'], actual
        rows.append(row)
    assert sha(runner) == runner_hash
    report = {'source_commit': references.SOURCE_COMMIT, 'runner': str(runner), 'runner_sha256': runner_hash,
              'capture': str(capture), 'capture_hashes': hashes(capture),
              'summary': dict(collections.Counter(r['outcome'] for r in rows)),
              'master_added_functions': 2, 'existing_regression_functions': 1, 'rows': rows}
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(report['summary'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--rust', type=Path)
    parser.add_argument('--reuse', action='store_true')
    args = parser.parse_args()
    if not args.reuse:
        capture_original(args.capture.resolve())
    if args.output:
        assert args.rust
        score(args.capture.resolve(), args.rust.resolve(), args.output.resolve())


if __name__ == '__main__':
    main()
