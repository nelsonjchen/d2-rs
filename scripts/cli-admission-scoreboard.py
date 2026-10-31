#!/usr/bin/env python3
"""Original CLI cancellation and empty-diagram admission, with unchanged Go assertions."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
TESTS = ['TestFmtHonorsCanceledContextBeforeWriting', 'TestEmptyDiagramRejectsUnavailableLayouts']
NAMES = [TESTS[0], TESTS[1]+'/external/flag', TESTS[1]+'/external/source']

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def compare(calls, results):
    assert len(calls) == len(NAMES), 'incomplete original call inventory'
    assert [c['test'] for c in calls] == NAMES, 'changed original test inventory/order'
    assert [c['call'] for c in calls] == list(range(len(NAMES))), 'duplicate/reordered calls'
    for call in calls:
        assert set(call) == {'test', 'call', 'input', 'result'}
        assert set(call['input']) == {'directory', 'args', 'canceled', 'files'}
        assert set(call['result']) == {'error', 'stdout', 'files'}
        for files in [call['input']['files'], call['result']['files']]:
            assert set(files) == {'input.d2', 'output.svg'}
            assert all(f is None or set(f) == {'data', 'mode'} for f in files.values())
    return [dict(test=test, calls=sum(c['test'].split('/')[0] == test for c in calls), outcome='match' if len(results) == len(calls) and all(results[i] == c['result'] for i, c in enumerate(calls) if c['test'].split('/')[0] == test) else 'mismatch') for test in TESTS]

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
    hook = ROOT/'tools/cli-admission-scoreboard/hook_test.go'
    names = ['fmt_test.go', 'layout_availability_test.go', 'layout_test.go']
    originals = {name: (source/'d2cli'/name).read_bytes() for name in names}
    env = dict(os.environ, GOMODCACHE=str(ROOT/'.reference/go-mod/pkg/mod'), GOCACHE=str(ROOT/'.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off', TMPDIR=str(capture/'tmp'), D2RS_ADMISSION_CAPTURE=str(capture))
    bindings = dict(source_commit=references.SOURCE_COMMIT, hook_sha256=sha(hook), sources={name: hashlib.sha256(data).hexdigest() for name, data in originals.items()})
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture/'calls').mkdir(); (capture/'tmp').mkdir()
        helper = references.TRACE_CHECKOUT/'d2cli/d2rs_cli_admission_test.go'
        assert not helper.exists()
        assert all((references.TRACE_CHECKOUT/'d2cli'/name).read_bytes() == data for name, data in originals.items()), 'concurrent source mutation'
        try:
            for name, data in originals.items():
                text = data.decode()
                if name in ['fmt_test.go', 'layout_availability_test.go']:
                    assert text.count('t.TempDir()') == 1
                    text = text.replace('t.TempDir()', 'd2rsAdmissionDir(t)')
                if name == 'fmt_test.go':
                    assert text.count('Run:  Run,') == 1
                    text = text.replace('Run:  Run,', 'Run: func(ctx context.Context, ms *xmain.State) error { return d2rsAdmissionRun(t, ctx, ms, directory, []string{"fmt", inputPath}) },')
                if name == 'layout_test.go':
                    assert text.count('Run: Run,') == 1
                    text = text.replace('Run: Run,', 'Run: func(ctx context.Context, ms *xmain.State) error { return d2rsAdmissionRun(t, ctx, ms, directory, args) },')
                (references.TRACE_CHECKOUT/'d2cli'/name).write_text(text)
            helper.write_bytes(hook.read_bytes())
            go = cli.bounded_run(['go', 'test', './d2cli', '-run', '^('+'|'.join(TESTS)+')$', '-json', '-count=1', '-parallel=1'], env, capture, capture/'go.jsonl', 600)
        finally:
            for name, data in originals.items():
                (references.TRACE_CHECKOUT/'d2cli'/name).write_bytes(data)
            helper.unlink(missing_ok=True)
        assert go == {'exit':0, 'failure':None}, go
        events = cli.events(capture/'go.jsonl')
        assert all(events.get(name) == 'pass' for name in NAMES+TESTS), events
        captured = {str(p.relative_to(capture)): sha(p) for p in sorted((capture/'calls').glob('*.json'))}
        captured['go.jsonl'] = sha(capture/'go.jsonl')
        (capture/'manifest.json').write_text(json.dumps(dict(bindings, go=go, original_assertions=events, captured_sha256=captured), indent=2)+'\n')
    manifest = json.loads((capture/'manifest.json').read_text())
    assert all(manifest.get(key) == value for key, value in bindings.items()), 'changed captured source/hook'
    assert manifest['go'] == {'exit':0, 'failure':None}
    assert all(manifest['original_assertions'].get(name) == 'pass' for name in NAMES+TESTS)
    assert set(manifest['captured_sha256']) == {'go.jsonl'} | {str(p.relative_to(capture)) for p in (capture/'calls').glob('*.json')}
    assert all(sha(capture/name) == digest for name, digest in manifest['captured_sha256'].items()), 'changed original capture bytes'
    calls = [json.loads(p.read_text()) for p in sorted((capture/'calls').glob('*.json'))]
    compare(calls, [])
    # Fresh replay paths; never overwrite an immutable prior output binding.
    replay = capture/('replay-'+args.output.stem); replay.mkdir(exist_ok=False)
    request = replay/'native-input.json'; response = replay/'native-output.json'
    request.write_text(json.dumps([c['input'] for c in calls])+'\n')
    env.update(D2RS_ADMISSION_REQUEST=str(request), D2RS_ADMISSION_RESPONSE=str(response))
    native = cli.bounded_run([str(runner), '--exact', 'cli_admission_api_tests::replay', '--ignored', '--nocapture'], env, replay, replay/'native.log', 30)
    results = json.loads(response.read_text()) if response.exists() else []
    rows = compare(calls, results)
    if native != {'exit':0, 'failure':None}:
        for row in rows: row['outcome'] = 'mismatch'
    assert runner_hash == sha(runner), 'native executable changed'
    report = dict(bindings, package='d2cli', capture=str(capture), replay=str(replay), runner=str(runner), runner_sha256=runner_hash, go=manifest['go'], native=native, original_assertions=manifest['original_assertions'], rows=rows, calls=calls, results=results, controls=dict(total_bytes=1<<30, file_bytes=128<<20, minimum_free_bytes=20<<30, go_deadline_seconds=600, native_deadline_seconds=30), projection='complete terminal error through xmain.mainFatal/cmdlog and native write_cli_error, stdout and input/output bytes and modes; built-in enabled layout profile; Go errors.Is identity remains an original Go assertion')
    args.output.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(rows, indent=2))
    return int(any(r['outcome'] != 'match' for r in rows))

if __name__ == '__main__': raise SystemExit(main())
