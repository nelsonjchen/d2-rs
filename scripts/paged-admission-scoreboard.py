#!/usr/bin/env python3
"""Original paged admission/writer calls, exact errors, budgets and output bytes.

Streaming capture limits: 1 GiB total / 128 MiB per file / 20 GiB free.
Original Go assertions remain active; native requests contain inputs only.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
TEST = 'TestIsometricPagedAdmissionAndBoundedWriter'
NAMES = [TEST] * 3 + [TEST+'/'+name for name in ['pixels','encoded','regions','strings']] + [TEST]*2

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def compare(calls, results):
    assert len(calls) == 9, 'incomplete original call inventory'
    assert [call['call'] for call in calls] == list(range(9)), 'duplicate or reordered calls'
    assert [call['test'] for call in calls] == NAMES, 'changed original subtests'
    assert [call['input']['kind'] for call in calls] == ['page']*7 + ['write']*2
    assert all(set(call) == {'test','call','input','result'} for call in calls)
    for call in calls:
        kind = call['input']['kind']
        assert set(call['input']) == (set(['kind','budget','width','height','canceled','page']) if kind=='page' else set(['kind','remaining','canceled','data']))
        assert set(call['result']) == (set(['error','budget']) if kind=='page' else set(['error','remaining','n','output']))
    rows = [dict(test=TEST, calls=len(calls), outcome='match' if results == [call['result'] for call in calls] else 'mismatch')]
    return rows

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-test',required=True,type=Path)
    parser.add_argument('--capture',required=True,type=Path)
    parser.add_argument('--output',required=True,type=Path)
    args = parser.parse_args()
    source = references.verify_source()
    capture = args.capture.resolve(); capture.mkdir(exist_ok=False)
    (capture/'calls').mkdir(); (capture/'tmp').mkdir()
    spec = importlib.util.spec_from_file_location('cli', ROOT/'scripts/cli-scoreboard.py')
    cli = importlib.util.module_from_spec(spec); spec.loader.exec_module(cli)
    target = references.TRACE_CHECKOUT/'d2cli/isometric_paged_test.go'
    original = (source/'d2cli/isometric_paged_test.go').read_bytes()
    helper = references.TRACE_CHECKOUT/'d2cli/d2rs_paged_admission_test.go'
    hook = ROOT/'tools/paged-admission-scoreboard/hook_test.go'
    assert target.read_bytes() == original and not helper.exists(), 'concurrent source modification'
    runner = args.native_test.resolve(); runner_hash = sha(runner)
    env = dict(os.environ, GOMODCACHE=str(ROOT/'.reference/go-mod/pkg/mod'), GOCACHE=str(ROOT/'.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off', TMPDIR=str(capture/'tmp'), D2RS_PAGED_CAPTURE=str(capture/'calls'))
    try:
        # These sites occur only in the named original function. No assertions,
        # constructor values, contexts or data are changed.
        text = original.decode()
        assert text.count('r.admitPage(page)') == 4 and text.count('w.Write(') == 2
        target.write_text(text.replace('r.admitPage(page)','d2rsPagedAdmit(t, r, page)').replace('w.Write(', 'd2rsPagedWrite(t, w, '))
        helper.write_bytes(hook.read_bytes())
        go = cli.bounded_run(['go','test','./d2cli','-run','^'+TEST+'$','-json','-count=1','-parallel=1'],env,capture,capture/'go.jsonl',600)
    finally:
        target.write_bytes(original)
        helper.unlink(missing_ok=True)
    assert go == {'exit':0,'failure':None}, go
    events = cli.events(capture/'go.jsonl')
    assert all(events.get(name)=='pass' for name in set(NAMES)), events
    calls = [json.loads(path.read_text()) for path in sorted((capture/'calls').glob('*.json'))]
    compare(calls, [])
    request = capture/'native-input.json'; response = capture/'native-output.json'
    request.write_text(json.dumps([call['input'] for call in calls])+'\n')
    env.update(D2RS_PAGED_REQUEST=str(request),D2RS_PAGED_RESPONSE=str(response))
    native = cli.bounded_run([str(runner),'--exact','isometric_cli::paged::admission_api_tests::replay','--ignored','--nocapture'],env,capture,capture/'native.log',30)
    results = json.loads(response.read_text()) if response.exists() else []
    rows = compare(calls, results)
    if native != {'exit':0,'failure':None}:
        rows[0]['outcome']='mismatch'
    assert runner_hash == sha(runner), 'native executable changed'
    report = dict(source_commit=references.SOURCE_COMMIT,package='d2cli',capture=str(capture),runner=str(runner),runner_sha256=runner_hash,hook_sha256=sha(hook),source_sha256=hashlib.sha256(original).hexdigest(),go=go,native=native,rows=rows,calls=calls,results=results,original_assertions=events,controls=dict(total_bytes=1<<30,file_bytes=128<<20,minimum_free_bytes=20<<30,go_deadline_seconds=600,native_deadline_seconds=30),projection='complete page budget transitions/errors; original bytes.Buffer writer admission/cancellation with exact n, remaining and output bytes; no generic downstream writer-failure qualification')
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(rows,indent=2))
    return int(any(row['outcome']!='match' for row in rows))

if __name__=='__main__':raise SystemExit(main())
