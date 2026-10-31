#!/usr/bin/env python3
"""Capture the original pinned lib/svg tests, then compare native typed markup APIs.

All original assertions stay active. Values and error/panic admission compare;
raw Go errors remain. Fresh capture/replay paths,
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
TEST_FILE = 'lib/svg/text_test.go'
TESTS = ['TestEscapeAttribute', 'TestRenderAttributesRoundTrip',
'TestRenderAttributesPreservesOrder', 'TestHrefAttributes', 'TestAttrRejectsUnsafeNames',
'TestRenderAttributesRejectsDuplicates', 'TestParseAttributes',
'TestParseAttributesRejectsMarkupAndEvents', 'TestScopedIDAndLocalIRI',
'TestParseLocalIRI', 'TestXMLValuesNormalizeInvalidEncodingAndCharacters']

EXPECTED = {
  "TestXMLValuesNormalizeInvalidEncodingAndCharacters/\"bad\\x00value\"": 4,
  "TestAttrRejectsUnsafeNames/a:b:c": 1,
  "TestXMLValuesNormalizeInvalidEncodingAndCharacters/\"\\xff\"": 4,
  "TestRenderAttributesRejectsDuplicates": 3,
  "TestRenderAttributesPreservesOrder": 4,
  "TestScopedIDAndLocalIRI": 5,
  "TestAttrRejectsUnsafeNames/x\\\"_onclick": 1,
  "TestParseLocalIRI": 5,
  "TestAttrRejectsUnsafeNames/#00": 1,
  "TestAttrRejectsUnsafeNames/:": 1,
  "TestParseAttributes": 2,
  "TestParseAttributesRejectsMarkupAndEvents": 4,
  "TestAttrRejectsUnsafeNames/ONCLICK": 1,
  "TestRenderAttributesRoundTrip": 2,
  "TestHrefAttributes": 2,
  "TestXMLValuesNormalizeInvalidEncodingAndCharacters/\"bad\\x01value\"": 4,
  "TestAttrRejectsUnsafeNames/bad_name": 1,
  "TestAttrRejectsUnsafeNames/:x": 1,
  "TestEscapeAttribute": 1,
  "TestAttrRejectsUnsafeNames/1st": 1,
  "TestAttrRejectsUnsafeNames/onload": 1,
  "TestAttrRejectsUnsafeNames/x:": 1,
  "TestAttrRejectsUnsafeNames/svg:onfocus": 1
}



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
            'implementation': {n: sha(pinned / ('lib/svg/' + n)) for n in ['text.go', 'markup.go']},
            'hook': sha(ROOT / 'tools/svg-api-scoreboard/hook_test.go')}


def hashes(capture):
    paths = [capture / n for n in ['source.json', 'inventory.json', 'go.jsonl', 'run.json']]
    paths += sorted((capture / 'go').glob('*.json'))
    return {str(p.relative_to(capture)): sha(p) for p in paths}


def verify(capture):
    assert json.loads((capture / 'source.json').read_text()) == source()
    assert json.loads((capture / 'hashes.json').read_text()) == hashes(capture)
    assert json.loads((capture / 'run.json').read_text()) == {'exit': 0, 'failure': None}
    events = module('cli-scoreboard').events(capture / 'go.jsonl')
    assert all(events.get(t) == 'pass' for t in TESTS), events
    cases = [json.loads(p.read_text()) for p in sorted((capture / 'go').glob('*.json'))]
    inventory = json.loads((capture / 'inventory.json').read_text())
    assert inventory == EXPECTED
    assert {x.split('/')[0] for x in inventory} == set(TESTS)
    expected = {f'{t}#{i}' for t, n in inventory.items() for i in range(n)}
    assert len(cases) == len(expected) and {c['name'] for c in cases} == expected
    sizes = [p.stat().st_size for p in capture.rglob('*') if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    path = references.TRACE_CHECKOUT / TEST_FILE
    original = (pinned / TEST_FILE).read_bytes()
    helper = path.parent / 'd2rs_svg-api_test.go'
    assert path.read_bytes() == original and not helper.exists()
    (capture / 'go').mkdir(parents=True, exist_ok=False)
    (capture / 'source.json').write_text(json.dumps(source(), indent=2) + '\n')
    text = original.decode()
    import re
    functions = ['EscapeAttribute', 'RenderAttributes', 'HrefAttributes', 'Attr',
                 'ParseAttributes', 'ScopedID', 'LocalIRI', 'ParseLocalIRI', 'Text', 'TrustedFragment']
    lines = []
    for line in text.splitlines(keepends=True):
        if not any(x in line for x in ['t.Fatalf(', 't.Errorf(', 't.Fatal(']):
            for name in functions:
                line = re.sub(r'\b' + name + r'\(', 'd2rs' + name + '(t, ', line)
        lines.append(line)
    text = ''.join(lines)
    env = dict(os.environ, D2RS_SVG_API_DUMP=str(capture / 'go'),
               GOMODCACHE=str(ROOT / '.reference/go-mod/pkg/mod'),
               GOCACHE=str(ROOT / '.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off')
    env.pop('TA', None)
    env.pop('TESTDATA_ACCEPT', None)
    try:
        path.write_text(text)
        helper.write_bytes((ROOT / 'tools/svg-api-scoreboard/hook_test.go').read_bytes())
        result = module('cli-scoreboard').bounded_run(
            ['go', 'test', './lib/svg', '-run', '^(' + '|'.join(TESTS) + ')$', '-json', '-count=1', '-parallel=1'],
            env, capture, capture / 'go.jsonl', 300)
    finally:
        path.write_bytes(original)
        helper.unlink(missing_ok=True)
    (capture / 'run.json').write_text(json.dumps(result) + '\n')
    assert result == {'exit': 0, 'failure': None}, result
    counts = collections.Counter(json.loads(p.read_text())['test'] for p in (capture / 'go').glob('*.json'))
    (capture / 'inventory.json').write_text(json.dumps(dict(counts), indent=2) + '\n')
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
            try:
                actual = json.loads(log.read_text())
            except (ValueError, UnicodeError):
                row['outcome'] = 'invalid-output'
                rows.append(row)
                continue
            row['outcome'] = ('unsupported' if isinstance(actual, dict) and 'unsupported' in actual
                              else 'match' if actual == case['got'] else 'mismatch')
            if row['outcome'] != 'match':
                row['expected'], row['actual'] = case['got'], actual
        rows.append(row)
    assert sha(runner) == runner_hash
    report = {'source_commit': references.SOURCE_COMMIT, 'runner': str(runner), 'runner_sha256': runner_hash,
              'capture': str(capture), 'capture_hashes': hashes(capture),
              'summary': dict(collections.Counter(r['outcome'] for r in rows)),
              'master_added_functions': 11, 'rows': rows}
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
