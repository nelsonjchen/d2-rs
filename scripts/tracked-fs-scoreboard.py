#!/usr/bin/env python3
"""Original trackedFS regular-file policy and nonblocking FIFO contracts.

Retains original Go assertions. Both implementations use the same paths;
complete errors/typed denial/opened dependency lists compare exactly.
Capture has streaming 1 GiB total / 128 MiB per file / 20 GiB free guards.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import references

ROOT = references.ROOT
TESTS = ['TestTrackedFSHonorsLocalFilePolicy', 'TestTrackedFSRejectsFIFOWithoutBlocking']
COUNTS = [5, 1]

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def compare(expected, actual):
    assert set(expected) == set(TESTS), 'missing original functions'
    assert not set(actual) - set(expected), 'extra native functions'
    rows = []
    for name, count in zip(TESTS, COUNTS):
        calls = expected[name]
        assert len(calls) == count, 'incomplete original call inventory'
        assert [c['call'] for c in calls] == list(range(count)), 'duplicate/unordered calls'
        assert all(c['test'] == name for c in calls)
        assert all(set(c['input']) == {'policy', 'root', 'path'} for c in calls)
        assert all(set(c['result']) == {'error', 'denied', 'opened'} for c in calls)
        rows.append(dict(test=name, calls=count, outcome='match' if actual.get(name) == calls else 'mismatch'))
    return rows

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-test', required=True, type=Path)
    parser.add_argument('--capture', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    source = references.verify_source()
    capture = args.capture.resolve(); capture.mkdir(exist_ok=False)
    work = capture/'shared-work'; work.mkdir()
    runner = args.native_test.resolve(); runner_hash = sha(runner)
    files = ['watch_test.go', 'watch_unix_test.go']
    originals = {n: (source/'d2cli'/n).read_bytes() for n in files}
    targets = {n: references.TRACE_CHECKOUT/'d2cli'/n for n in files}
    helper = references.TRACE_CHECKOUT/'d2cli/d2rs_tracked_fs_test.go'
    hook = ROOT/'tools/tracked-fs-scoreboard/hook_test.go'
    assert not helper.exists() and all(targets[n].read_bytes() == v for n,v in originals.items())
    spec = importlib.util.spec_from_file_location('cli', ROOT/'scripts/cli-scoreboard.py')
    cli = importlib.util.module_from_spec(spec); spec.loader.exec_module(cli)
    env = dict(os.environ, GOMODCACHE=str(ROOT/'.reference/go-mod/pkg/mod'), GOCACHE=str(ROOT/'.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off', D2RS_TRACKED_WORK=str(work))
    env.pop('D2RS_TRACKED_NATIVE',None)
    runs, responses, events = {}, {}, {}
    try:
        for name, original in originals.items():
            transformed = original.decode().replace('trackedFS', 'd2rsTrackedFS').replace('t.TempDir()', 'd2rsTrackedTempDir(t)').replace('localfile.Rooted(', 'd2rsTrackedRooted(').replace('localfile.Unrestricted()', 'd2rsTrackedUnrestricted()')
            if name == 'watch_unix_test.go':
                transformed = transformed.replace('"github.com/d2lang/d2/lib/localfile"','')
            targets[name].write_text(transformed)
        helper.write_bytes(hook.read_bytes())
        for mode in ['go','rust']:
            output = capture/mode; output.mkdir(); (output/'calls').mkdir(); (output/'tmp').mkdir()
            env.update(TMPDIR=str(output/'tmp'),D2RS_TRACKED_CAPTURE=str(output/'calls'))
            if mode == 'rust': env['D2RS_TRACKED_NATIVE'] = str(runner)
            runs[mode] = cli.bounded_run(['go','test','./d2cli','-run','^('+'|'.join(TESTS)+')$','-json','-count=1','-parallel=1'],env,capture,output/'events.jsonl',600)
            calls = [json.loads(p.read_text()) for p in (output/'calls').glob('*-call.json')]
            responses[mode] = {}
            for call in sorted(calls,key=lambda c:(c['test'],c['call'])):
                responses[mode].setdefault(call['test'],[]).append(call)
            events[mode] = cli.events(output/'events.jsonl')
    finally:
        for name,data in originals.items(): targets[name].write_bytes(data)
        helper.unlink(missing_ok=True)
    assert runs['go'] == {'exit':0,'failure':None}, runs
    assert all(events['go'].get(t) == 'pass' for t in TESTS), events['go']
    rows = compare(responses['go'],responses['rust'])
    for row in rows:
        row.update(go_assertions=events['go'].get(row['test']),native_assertions=events['rust'].get(row['test']))
        if runs['rust']['failure'] is not None or row['native_assertions'] != 'pass':row['outcome']='mismatch'
    assert sha(runner) == runner_hash, 'native runner changed during capture'
    assert not list(work.iterdir()), 'temporary file leakage'
    report = dict(source_commit=references.SOURCE_COMMIT,package='d2cli',capture=str(capture),runner=str(runner),runner_sha256=runner_hash,hook_sha256=sha(hook),sources_sha256={n:hashlib.sha256(v).hexdigest() for n,v in originals.items()},runs=runs,rows=rows,observations=responses,controls=dict(total_bytes=1<<30,file_bytes=128<<20,minimum_free_bytes=20<<30,deadline_seconds=600,native_deadline_seconds=1),projection='complete Open errors, typed policy denial, successfully opened dependency lists; original close assertions; no byte/path normalization')
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(rows,indent=2))
    return int(any(r['outcome'] != 'match' for r in rows))

if __name__ == '__main__': raise SystemExit(main())
