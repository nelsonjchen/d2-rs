#!/usr/bin/env python3
"""Original Grid constructor/storage admission, with unchanged Go assertions."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
_spec = importlib.util.spec_from_file_location('grid_language_gate', ROOT/'scripts/language-scoreboard.py')
_language = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(_language)
PACKAGE = 'd2layouts/d2grid'
TESTS = {
    'TestNewGridDiagramRejectsUnsafeCapacity/max_int_panic_regression': 1,
    'TestNewGridDiagramRejectsUnsafeCapacity/twenty_million_allocator_regression': 1,
    'TestGridCapacityBoundaryAndOccupiedStorage': 3,
}

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def compare(calls, results):
    actual = {}
    for call in calls:
        assert set(call) == {'test', 'input', 'result'}
        assert set(call['input']) == {'operation', 'rows', 'columns', 'objects'}, 'expected data in request'
        assert call['input']['operation'] in {'construct', 'layout', 'capacity'}
        actual[call['test']] = actual.get(call['test'], 0) + 1
    assert actual == TESTS, 'incomplete/extra original function or call inventory'
    rows = []
    for name in dict.fromkeys(t.split('/')[0] for t in TESTS):
        indexes = [i for i,c in enumerate(calls) if c['test'].split('/')[0] == name]
        match = len(results) == len(calls) and all(_language.same_json(calls[i]['result'], results[i]) for i in indexes)
        rows.append(dict(package=PACKAGE, test=name, calls=len(indexes), outcome='match' if match else 'mismatch'))
    return rows

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-test', required=True, type=Path)
    parser.add_argument('--capture', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--reuse', action='store_true')
    args = parser.parse_args()
    source = references.verify_source()
    capture = args.capture.resolve()
    spec = importlib.util.spec_from_file_location('cli', ROOT/'scripts/cli-scoreboard.py')
    cli = importlib.util.module_from_spec(spec); spec.loader.exec_module(cli)
    runner = args.native_test.resolve(); runner_hash = sha(runner)
    hook = ROOT/'tools/grid-capacity-scoreboard/hook_test.go'
    original = source/PACKAGE/'resource_limits_test.go'
    data = original.read_bytes()
    bindings = dict(source_commit=references.SOURCE_COMMIT, package=PACKAGE,
        hook_sha256=sha(hook), sources={str(original.relative_to(source)):sha(original)})
    env = dict(os.environ, GOMODCACHE=str(ROOT/'.reference/go-mod/pkg/mod'),
        GOCACHE=str(ROOT/'.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off',
        TMPDIR=str(capture/'tmp'), D2RS_GRID_CAPACITY_CAPTURE=str(capture/'calls'))
    names = list(dict.fromkeys(t.split('/')[0] for t in TESTS))
    if not args.reuse:
        capture.mkdir(exist_ok=False); (capture/'tmp').mkdir()
        target = references.TRACE_CHECKOUT/PACKAGE/original.name
        helper = target.parent/'d2rs_grid_capacity_test.go'
        assert not helper.exists() and target.read_bytes() == data, 'concurrent source mutation'
        try:
            target.write_text(data.decode().replace('newGridDiagram(', 'd2rsGridConstruct(t, ')
                .replace('d2graph.GridCapacity(', 'd2rsGridCapacity(t, ')
                .replace('gd.layoutEvenly(', 'd2rsGridLayout(t, gd, '))
            helper.write_bytes(hook.read_bytes())
            result = cli.bounded_run(['go', 'test', './'+PACKAGE, '-run', '^('+'|'.join(names)+')$',
                '-json', '-count=1', '-parallel=1'], env, capture, capture/'go.jsonl', 600)
        finally:
            target.write_bytes(data); helper.unlink(missing_ok=True)
        assert result == {'exit':0, 'failure':None}, result
        events = cli.events(capture/'go.jsonl')
        assert all(events.get(t) == 'pass' for t in set(TESTS)|set(names)), events
        paths = sorted((capture/'calls').rglob('*.json')) + [capture/'go.jsonl']
        manifest = dict(bindings, go=result, original_assertions=events,
            captured_sha256={str(p.relative_to(capture)):sha(p) for p in paths})
        (capture/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    manifest = json.loads((capture/'manifest.json').read_text())
    assert all(manifest.get(k) == v for k,v in bindings.items()), 'changed source/hook'
    assert manifest['go'] == {'exit':0, 'failure':None}
    assert all(manifest['original_assertions'].get(t) == 'pass' for t in set(TESTS)|set(names))
    paths = sorted((capture/'calls').rglob('*.json')) + [capture/'go.jsonl']
    assert {str(p.relative_to(capture)) for p in paths} == set(manifest['captured_sha256']), 'changed original file inventory'
    assert all(sha(capture/p) == v for p,v in manifest['captured_sha256'].items()), 'changed original bytes'
    calls = []
    for name, count in TESTS.items():
        files = sorted((capture/'calls'/name.replace('/','_')).glob('*.json'))
        assert len(files) == count, (name,len(files),count)
        calls.extend(json.loads(p.read_text()) for p in files)
    compare(calls, [])
    replay = capture/('replay-'+args.output.stem); replay.mkdir(exist_ok=False)
    results = []; runs = []
    for i,call in enumerate(calls):
        request = replay/f'{i:02}-input.json'; response = replay/f'{i:02}-output.json'
        request.write_text(json.dumps([call['input']])+'\n')
        env.update(D2RS_GRID_CAPACITY_REQUEST=str(request), D2RS_GRID_CAPACITY_RESPONSE=str(response))
        run = cli.bounded_run([str(runner), '--exact', 'grid::capacity_api_tests::replay', '--ignored', '--nocapture'],
            env, replay, replay/f'{i:02}-native.log', 30)
        values = json.loads(response.read_text()) if response.exists() else []
        results.append(values[0] if run == {'exit':0, 'failure':None} and len(values) == 1 else {'failure':run})
        runs.append(run)
    assert runner_hash == sha(runner), 'native executable changed'
    rows = compare(calls,results)
    report = dict(bindings,capture=str(capture),replay=str(replay),runner=str(runner),runner_sha256=runner_hash,
        original_assertions=manifest['original_assertions'],calls=calls,results=results,rows=rows,native=runs,
        controls=dict(total_bytes=1<<30,file_bytes=128<<20,minimum_free_bytes=20<<30,go_deadline_seconds=600,native_deadline_seconds=30),
        projection='Original constructor dimensions, occupied storage extents, complete errors, and even-layout content size. Typed positive dimensions; no Go string parsing or error identity claim. Original Go assertions remain active.')
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(rows,indent=2)); return int(any(r['outcome'] != 'match' for r in rows))

if __name__ == '__main__': raise SystemExit(main())
