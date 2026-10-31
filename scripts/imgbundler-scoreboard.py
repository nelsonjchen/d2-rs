#!/usr/bin/env python3
"""Capture pinned imgbundler test calls, retaining original Go assertions.

This first batch covers scanner/replacement/budget contracts; resolver policy,
HTTP ownership and scheduling tests remain uncovered. 1 GiB total / 128 MiB
per file / 20 GiB minimum free; fresh capture and native output directories.
"""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TEST_FILE = 'lib/imgbundler/resource_limits_test.go'
TESTS = {
    'TestBundleCapsImageReferences',
    'TestApplyReplacementsCapsOutputAndHandlesDuplicates',
    'TestBundleBudgetIsCumulativeAndSticky',
    'TestBundleSkipsLargeDataURI',
    'TestBundleIgnoresOversizedReferencesForOtherPass',
    'TestBundleCapsEligibleReferenceBytes',
}
# Explicit names/counts are checked independently of the produced inventory.
EXPECTED_CALLS = {
    'TestBundleCapsImageReferences': 2,
    'TestApplyReplacementsCapsOutputAndHandlesDuplicates': 4,
    'TestBundleBudgetIsCumulativeAndSticky': 3,
    'TestBundleSkipsLargeDataURI': 1,
    'TestBundleIgnoresOversizedReferencesForOtherPass': 1,
    'TestBundleCapsEligibleReferenceBytes': 1,
}


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source():
    pinned = ROOT / references.MANIFEST['d2']['checkout']
    return {'commit': references.SOURCE_COMMIT,
            'test': sha(pinned / TEST_FILE),
            'implementation': sha(pinned / 'lib/imgbundler/imgbundler.go'),
            'hook': sha(ROOT / 'tools/imgbundler-scoreboard/hook_test.go')}


def hashes(capture):
    paths = [capture / n for n in ['source.json', 'inventory.json', 'go.jsonl', 'run.json']]
    paths += sorted((capture / 'go').glob('*.json'))
    return {str(p.relative_to(capture)): sha(p) for p in paths}


def verify(capture):
    assert json.loads((capture / 'source.json').read_text()) == source()
    assert json.loads((capture / 'hashes.json').read_text()) == hashes(capture)
    assert json.loads((capture / 'run.json').read_text()) == {'exit': 0, 'failure': None}
    events = module('cli-scoreboard').events(capture / 'go.jsonl')
    cases = [json.loads(p.read_text()) for p in sorted((capture / 'go').glob('*.json'))]
    expected = {f'{t}#{i}' for t, count in EXPECTED_CALLS.items() for i in range(count)}
    assert len(cases) == len(expected) and {c['name'] for c in cases} == expected
    assert all(events.get(t) == 'pass' for t in TESTS), events
    assert json.loads((capture / 'inventory.json').read_text()) == sorted(TESTS)
    sizes = [p.stat().st_size for p in capture.rglob('*') if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    path = references.TRACE_CHECKOUT / TEST_FILE
    original = (pinned / TEST_FILE).read_bytes()
    helper = path.parent / 'd2rs_imgbundler_test.go'
    assert path.read_bytes() == original and not helper.exists()
    (capture / 'go').mkdir(parents=True, exist_ok=False)
    (capture / 'source.json').write_text(json.dumps(source(), indent=2) + '\n')
    (capture / 'inventory.json').write_text(json.dumps(sorted(TESTS), indent=2) + '\n')
    text = original.decode()
    # Rewrite only selected original function bodies, preserving every assertion.
    parts = re.split(r'(?=^func Test)', text, flags=re.MULTILINE)
    for i, part in enumerate(parts):
        if any(part.startswith('func ' + t + '(') for t in TESTS):
            for old, new in [('findImageElements(', 'd2rsScan(t, '), ('applyReplacements(', 'd2rsApply(t, '),
                             ('budget.reserveBundled(', 'd2rsReserve(t, budget, '),
                             ('BundleLocalWithPolicy(', 'd2rsLocal(t, '), ('BundleRemote(', 'd2rsRemote(t, ')]:
                part = part.replace(old, new)
            parts[i] = part
    env = dict(os.environ, D2RS_IMGBUNDLER_DUMP=str(capture / 'go'),
               GOMODCACHE=str(ROOT / '.reference/go-mod/pkg/mod'),
               GOCACHE=str(ROOT / '.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off')
    env.pop('TA', None)
    env.pop('TESTDATA_ACCEPT', None)
    try:
        path.write_text(''.join(parts))
        helper.write_bytes((ROOT / 'tools/imgbundler-scoreboard/hook_test.go').read_bytes())
        result = module('cli-scoreboard').bounded_run(
            ['go', 'test', './lib/imgbundler', '-run', '^(' + '|'.join(sorted(TESTS)) + ')$',
             '-json', '-count=1', '-parallel=1'], env, capture, capture / 'go.jsonl', 600)
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
    cli = module('cli-scoreboard')
    runner_hash = sha(runner)
    rows = []
    for case in cases:
        key = hashlib.sha256(case['name'].encode()).hexdigest()
        request = work / (key + '.request.json')
        log = work / (key + '.log')
        request.write_text(json.dumps(case['request']))
        run = cli.bounded_run([str(runner.resolve()), str(request.resolve())], os.environ.copy(), work, log, 30)
        row = {'name': case['name'], 'test': case['test']}
        if run['failure'] or run['exit']:
            row['outcome'] = run['failure'] or 'rust-failed'
        else:
            try:
                actual = json.loads(log.read_text())
            except ValueError:
                actual = {'invalid_output': True}
            row['outcome'] = 'unsupported' if 'unsupported' in actual else 'match' if actual == case['got'] else 'mismatch'
            if row['outcome'] == 'mismatch':
                row['differences'] = module('language-scoreboard').differences(case['got'], actual)
        rows.append(row)
    assert sha(runner) == runner_hash
    report = {'source_commit': references.SOURCE_COMMIT, 'runner_sha256': runner_hash,
              'capture_hashes': hashes(capture), 'summary': dict(collections.Counter(r['outcome'] for r in rows)),
              'captured_functions': len(TESTS), 'rows': rows}
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
