#!/usr/bin/env python3
"""Original Dagre/ELK and shared graph admission, with unchanged Go assertions."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
PACKAGES = {
    'd2layouts/d2dagrelayout': ('layout_test.go', 'dagre', {
        'TestLayoutRejectsDenseGraphBeforeDagre': 1,
        'TestLayoutChecksCancellationBeforeDagre': 1,
    }),
    'd2layouts/d2elklayout': ('security_test.go', 'ELK', {
        'TestLayoutRejectsDenseGraphBeforeELK': 1,
        'TestLayoutChecksCancellationBeforeELK': 1,
    }),
    'd2layouts/internal/layoutguard': ('graph_test.go', 'test', {
        'TestCheckGraphPreservesSparseGraphs': 1,
        'TestCheckGraphRawObjectBoundary': 2,
        'TestCheckGraphRawEdgeBoundary': 2,
        'TestCheckGraphRejectsDenseSubgraphs': 1,
        'TestCheckGraphDenseBoundary': 2,
        'TestCheckGraphPreservesSmallParallelGraphs': 1,
        'TestCheckGraphRejectsMalformedSmallGraph/nil_edge': 1,
        'TestCheckGraphRejectsMalformedSmallGraph/nil_source': 1,
        'TestCheckGraphRejectsMalformedSmallGraph/nil_target': 1,
        'TestCheckGraphRejectsMalformedSmallGraph/nil_endpoints': 1,
        'TestCheckGraphObservesCancellation': 1,
    }),
}

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def compare(calls, results):
    expected = {(p, t): n for p, (_, _, tests) in PACKAGES.items() for t, n in tests.items()}
    actual = {}
    for call in calls:
        assert set(call) == {'package', 'test', 'call', 'input', 'result'}
        assert set(call['input']) == {'engine', 'objects', 'edges', 'canceled'}, 'expected data in request'
        assert set(call['result']) == {'error'}
        key = call['package'], call['test']
        actual.setdefault(key, []).append(call['call'])
    assert set(actual) == set(expected), 'incomplete/extra original function inventory'
    assert all(actual[k] == list(range(n)) for k, n in expected.items()), 'changed original call inventory'
    rows = []
    for package, (_, _, tests) in PACKAGES.items():
        for test in dict.fromkeys(t.split('/')[0] for t in tests):
            indexes = [i for i, c in enumerate(calls) if c['package'] == package and c['test'].split('/')[0] == test]
            matched = len(results) == len(calls) and all(results[i] == calls[i]['result'] for i in indexes)
            rows.append(dict(package=package, test=test, calls=len(indexes), outcome='match' if matched else 'mismatch'))
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
    hook = ROOT/'tools/layout-admission-scoreboard/hook_test.go'
    originals = {str(Path(p)/name): (source/p/name).read_bytes() for p, (name, _, _) in PACKAGES.items()}
    bindings = dict(source_commit=references.SOURCE_COMMIT, hook_sha256=sha(hook), sources={p: hashlib.sha256(data).hexdigest() for p, data in originals.items()})
    env = dict(os.environ, GOMODCACHE=str(ROOT/'.reference/go-mod/pkg/mod'), GOCACHE=str(ROOT/'.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off', TMPDIR=str(capture/'tmp'), D2RS_LAYOUT_ADMISSION_CAPTURE=str(capture))
    if not args.reuse:
        capture.mkdir(exist_ok=False); (capture/'tmp').mkdir()
        assertions = {}; runs = {}
        for package, (name, engine, tests) in PACKAGES.items():
            path = Path(package)/name; data = originals[str(path)]
            target = references.TRACE_CHECKOUT/path
            helper = target.parent/'d2rs_layout_admission_test.go'
            assert not helper.exists() and target.read_bytes() == data, 'concurrent source mutation'
            callee = 'CheckGraph' if engine == 'test' else 'DefaultLayout'
            package_name = Path(package).name
            wrapper = f'\nfunc d2rsOriginalAdmission(t *testing.T, ctx context.Context, '+('engine string, ' if engine == 'test' else '')+'g *d2graph.Graph) error {\n'
            wrapper += f' return d2rsAdmission(t, ctx, '+('engine' if engine == 'test' else json.dumps(engine))+f', g, func() error {{ return {callee}(ctx, '+('engine, ' if engine == 'test' else '')+'g) })\n}\n'
            try:
                target.write_text(data.decode().replace(callee+'(', 'd2rsOriginalAdmission(t, '))
                helper.write_text(hook.read_text().replace('D2RS_PACKAGE', package_name)+wrapper)
                names = list(dict.fromkeys(t.split('/')[0] for t in tests))
                result = cli.bounded_run(['go', 'test', './'+package, '-run', '^('+'|'.join(names)+')$', '-json', '-count=1', '-parallel=1'], env, capture, capture/(package_name+'-go.jsonl'), 600)
            finally:
                target.write_bytes(data); helper.unlink(missing_ok=True)
            assert result == {'exit':0, 'failure':None}, (package, result)
            events = cli.events(capture/(package_name+'-go.jsonl'))
            assert all(events.get(t) == 'pass' for t in set(tests)|set(names)), events
            assertions[package] = events; runs[package] = result
        captured = {str(p.relative_to(capture)): sha(p) for p in sorted(capture.rglob('*.json'))}
        captured.update({str(p.relative_to(capture)): sha(p) for p in capture.glob('*-go.jsonl')})
        (capture/'manifest.json').write_text(json.dumps(dict(bindings, go=runs, original_assertions=assertions, captured_sha256=captured), indent=2)+'\n')
    manifest = json.loads((capture/'manifest.json').read_text())
    assert all(manifest.get(k) == v for k, v in bindings.items()), 'changed source/hook'
    for package, (_, _, tests) in PACKAGES.items():
        assert manifest['go'][package] == {'exit':0, 'failure':None}
        assert all(manifest['original_assertions'][package].get(t) == 'pass' for t in set(tests)|{t.split('/')[0] for t in tests}), 'original assertion failed/missing'
    capture_paths = {str(p.relative_to(capture)) for package in PACKAGES for p in (capture/Path(package).name).rglob('*.json')}
    capture_paths |= {str(p.relative_to(capture)) for p in capture.glob('*-go.jsonl')}
    assert capture_paths == set(manifest['captured_sha256']), 'changed original file inventory'
    assert all(sha(capture/p) == digest for p, digest in manifest['captured_sha256'].items()), 'changed capture bytes'
    calls = []
    for package, (_, _, tests) in PACKAGES.items():
        for test, count in tests.items():
            directory = capture/Path(package).name/test.replace('/', '_')
            files = sorted(directory.glob('*.json'))
            assert len(files) == count, (package, test, len(files), count)
            for i, path in enumerate(files):
                call = json.loads(path.read_text()); assert call['test'] == test
                calls.append(dict(package=package, call=i, **call))
    compare(calls, [])
    replay = capture/('replay-'+args.output.stem); replay.mkdir(exist_ok=False)
    # A slow rejected graph cannot hide another call or leave a partial response.
    results = []; native_runs = []
    for i, call in enumerate(calls):
        request = replay/f'{i:02}-input.json'; response = replay/f'{i:02}-output.json'
        request.write_text(json.dumps([call['input']])+'\n')
        env.update(D2RS_LAYOUT_ADMISSION_REQUEST=str(request), D2RS_LAYOUT_ADMISSION_RESPONSE=str(response))
        native = cli.bounded_run([str(runner), '--exact', 'admission_api_tests::replay', '--ignored', '--nocapture'], env, replay, replay/f'{i:02}-native.log', 30)
        result = json.loads(response.read_text()) if response.exists() else []
        results.append(result[0] if native == {'exit':0, 'failure':None} and len(result) == 1 else {'failure':native})
        native_runs.append(native)
    assert runner_hash == sha(runner), 'native executable changed'
    rows = compare(calls, results)
    report = dict(bindings, capture=str(capture), replay=str(replay), runner=str(runner), runner_sha256=runner_hash, original_assertions=manifest['original_assertions'], calls=calls, results=results, rows=rows, native=native_runs, controls=dict(total_bytes=1<<30, file_bytes=128<<20, minimum_free_bytes=20<<30, go_deadline_seconds=600, native_deadline_seconds=30), projection='Complete errors and original Go assertions. Admission reads only object/edge inventories, endpoint identity/presence and context; raw guard preserves missing edge/endpoints. Engine replay uses the measured Rust graph boundary; no geometry/result equivalence claimed.')
    args.output.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(rows, indent=2)); return int(any(r['outcome'] != 'match' for r in rows))

if __name__ == '__main__': raise SystemExit(main())
