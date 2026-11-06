#!/usr/bin/env python3
"""Replay original ELK option constructors, preserving every Go assertion."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
PACKAGE = 'd2layouts/d2elklayout'
TESTS = {'TestUnsetEdgeEdgeSpacingKeepsPreviousDefault': 2, 'TestExplicitZeroEdgeEdgeSpacingIsHonored': 2, 'TestUnsetEdgeEdgeSpacingIsOmitted': 3}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compare(calls, results):
    assert len(calls) == sum(TESTS.values()), 'incomplete/extra original call inventory'
    assert {test:sum(c['test'] == test for c in calls) for test in TESTS} == TESTS
    for test,count in TESTS.items():
        kinds = [c['input']['kind'] for c in calls if c['test'] == test]
        assert sorted(kinds) == (['edge-label','leaf','port'] if count == 3 else ['container','root'])
    for call in calls:
        assert set(call) == {'test', 'input', 'result'}
        assert set(call['input']) == {'kind','options'}, 'expected data in native request'
    rows = []
    for test in TESTS:
        indexes = [i for i,c in enumerate(calls) if c['test'].split('/')[0] == test]
        matched = sum(len(results) == len(calls) and calls[i]['result'] == results[i] for i in indexes)
        rows.append(dict(test=test, calls=len(indexes), matched_calls=matched,
            outcome='match' if matched == len(indexes) else 'mismatch'))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-test', required=True, type=Path)
    parser.add_argument('--capture', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--reuse', action='store_true')
    args = parser.parse_args()
    source = references.verify_source()
    spec = importlib.util.spec_from_file_location('cli', ROOT/'scripts/cli-scoreboard.py')
    cli = importlib.util.module_from_spec(spec); spec.loader.exec_module(cli)
    capture = args.capture.resolve()
    runner = args.native_test.resolve(); runner_hash = sha(runner)
    hook = ROOT/'tools/elk-options-scoreboard/hook_test.go'
    originals = {name:(source/PACKAGE/name).read_bytes() for name in ['options_test.go','layout.go']}
    bindings = dict(source_commit=references.SOURCE_COMMIT, package=PACKAGE, hook_sha256=sha(hook),
        sources={str(Path(PACKAGE)/name):hashlib.sha256(data).hexdigest() for name,data in originals.items()})
    env = dict(os.environ, GOMODCACHE=str(ROOT/'.reference/go-mod/pkg/mod'), GOCACHE=str(ROOT/'.reference/go-build'),
        GOFLAGS='-mod=mod', GOPROXY='off', TMPDIR=str(capture/'tmp'), D2RS_ELK_OPTIONS_CAPTURE=str(capture/'calls'))
    if not args.reuse:
        capture.mkdir(exist_ok=False); (capture/'tmp').mkdir(); (capture/'calls').mkdir()
        target = references.TRACE_CHECKOUT/PACKAGE/'options_test.go'
        helper = target.parent/'d2rs_elk_options_test.go'
        data = originals[target.name]
        assert not helper.exists() and target.read_bytes() == data, 'concurrent source mutation'
        text = data.decode()
        # Constructor wrappers return their real result. The original tests'
        # assertions, map iterations, and explicit-zero marshaling stay active.
        text = text.replace('newRootLayoutOptions(&opts)', 'd2rsRootOptions(t, &opts)')
        text = text.replace('newContainerLayoutOptions(&opts)', 'd2rsContainerOptions(t, &opts)')
        start = text.index('func TestUnsetEdgeEdgeSpacingIsOmitted(')
        end = text.index('func TestNativeELKMigrationProfile(', start)
        section = text[start:end]
        assert section.count('json.Marshal(opts)') == 1
        text = text[:start] + section.replace('json.Marshal(opts)', 'd2rsBareOptionsMarshal(t, opts)') + text[end:]
        try:
            target.write_text(text)
            helper.write_bytes(hook.read_bytes())
            result = cli.bounded_run(['go','test','./'+PACKAGE,'-run','^('+'|'.join(TESTS)+')$','-json','-count=1','-parallel=1'],
                env,capture,capture/'go.jsonl',600)
        finally:
            target.write_bytes(data); helper.unlink(missing_ok=True)
        assert result == {'exit':0,'failure':None}, result
        events = cli.events(capture/'go.jsonl')
        assert all(events.get(name) == 'pass' for name in TESTS), events
        paths = sorted((capture/'calls').glob('*.json')) + [capture/'go.jsonl']
        manifest = dict(bindings,go=result,original_assertions=events,
            captured_sha256={str(p.relative_to(capture)):sha(p) for p in paths})
        (capture/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    manifest = json.loads((capture/'manifest.json').read_text())
    assert all(manifest.get(k) == v for k,v in bindings.items()), 'changed source/hook'
    assert manifest['go'] == {'exit':0,'failure':None}
    paths = sorted((capture/'calls').glob('*.json')) + [capture/'go.jsonl']
    assert {str(p.relative_to(capture)):sha(p) for p in paths} == manifest['captured_sha256'], 'changed capture inventory/bytes'
    calls = [json.loads(p.read_text()) for p in paths if p.suffix == '.json']
    compare(calls, [])
    assert all(manifest['original_assertions'].get(c['test']) == 'pass' for c in calls), 'original assertion missing/failed'
    replay = capture/('replay-'+args.output.stem); replay.mkdir(exist_ok=False)
    request, response = replay/'input.json', replay/'output.json'
    request.write_text(json.dumps([c['input'] for c in calls])+'\n')
    env.update(D2RS_ELK_OPTIONS_REQUEST=str(request), D2RS_ELK_OPTIONS_RESPONSE=str(response))
    native = cli.bounded_run([str(runner),'--exact','codec::options_api_tests::replay','--ignored','--nocapture'],
        env,replay,replay/'native.log',600)
    results = json.loads(response.read_text()) if native == {'exit':0,'failure':None} and response.exists() else []
    assert sha(runner) == runner_hash, 'native executable changed'
    rows = compare(calls, results)
    report = dict(bindings,capture=str(capture),replay=str(replay),runner=str(runner),runner_sha256=runner_hash,
        rows=rows,calls=calls,results=results,native=native,original_assertions=manifest['original_assertions'],
        controls=dict(total_bytes=1<<30,file_bytes=128<<20,minimum_free_bytes=20<<30,deadline_seconds=600),
        projection='Complete ELK root/container/leaf/port/edge-label option JSON from seven original constructor calls. Inputs only in native requests; original Go assertions unchanged. No layout geometry or Go pointer identity claim.')
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(rows,indent=2))
    return int(any(row['outcome'] != 'match' for row in rows))


if __name__ == '__main__':
    raise SystemExit(main())
