#!/usr/bin/env python3
"""Pinned watch authorization assertions plus explicitly separate parser witnesses."""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import references

ROOT = references.ROOT
TESTS = ['TestNewWatchAccessToken', 'TestWatcherAuthorizeRequest', 'TestD2rsWatchParsingWitnesses']
WITNESSES = ['duplicate_wrong_first', 'duplicate_right_first', 'quoted_cookie',
             'invalid_cookie_then_valid', 'empty_token_with_cookie', 'malformed_token_with_cookie',
             'malformed_neighbor', 'semicolon_token_with_cookie', 'semicolon_neighbor',
             'encoded_path', 'query_bytes', 'duplicate_query']
NAMES = ([TESTS[0]]*2 + [TESTS[1]+'/'+name for name in [
    'rejects_unauthenticated_request','rejects_invalid_cookie',
    'exchanges_token_for_strict_HTTP-only_cookie','exchanges_token_for_strict_HTTP-only_cookie',
    'strips_repeated_token_URL_despite_valid_cookie']]
    + [TESTS[2]+'/'+name for name in WITNESSES])
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()

def compare(calls, results):
    assert len(calls) == len(NAMES), 'missing/extra original calls'
    assert collections.Counter(c['test'] for c in calls) == collections.Counter(NAMES), 'changed original inventory'
    assert [c['call'] for c in calls] == list(range(len(calls))), 'duplicate/reordered call ids'
    for c in calls:
        assert set(c) == {'test','call','input','result'}
        if c['input']['operation'] == 'token':
            assert set(c['input']) == {'operation'}
            assert set(c['result']) == {'error','length','url_safe','distinct'}
        else:
            assert set(c['input']) == {'operation','token','cookie_name','target','cookie'}, 'expected output in input'
            assert set(c['result']) == {'allowed','code','headers','body'}
    return [dict(test=test,calls=sum(c['test'].split('/')[0]==test for c in calls),
                 matched_calls=sum(len(results)==len(calls) and results[i]==c['result'] for i,c in enumerate(calls) if c['test'].split('/')[0]==test),
                 outcome='match' if len(results)==len(calls) and all(results[i]==c['result'] for i,c in enumerate(calls) if c['test'].split('/')[0]==test) else 'mismatch') for test in TESTS]

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--native-test',required=True,type=Path);p.add_argument('--capture',required=True,type=Path)
    p.add_argument('--output',required=True,type=Path);p.add_argument('--reuse',action='store_true')
    args=p.parse_args();source=references.verify_source();capture=args.capture.resolve();runner=args.native_test.resolve();digest=sha(runner)
    spec=importlib.util.spec_from_file_location('cli',ROOT/'scripts/cli-scoreboard.py');cli=importlib.util.module_from_spec(spec);spec.loader.exec_module(cli)
    hook=ROOT/'tools/cli-watch-scoreboard/hook_test.go';original=source/'d2cli/watch_security_test.go'
    data=original.read_bytes()
    bindings=dict(source_commit=references.SOURCE_COMMIT,package='d2cli',hook_sha256=sha(hook),sources={'d2cli/watch_security_test.go':sha(original)})
    env=dict(os.environ,GOMODCACHE=str(ROOT/'.reference/go-mod/pkg/mod'),GOCACHE=str(ROOT/'.reference/go-build'),GOFLAGS='-mod=mod',GOPROXY='off',TMPDIR=str(capture/'tmp'),D2RS_WATCH_CAPTURE=str(capture))
    if not args.reuse:
        capture.mkdir(exist_ok=False);(capture/'tmp').mkdir()
        target=references.TRACE_CHECKOUT/'d2cli/watch_security_test.go';helper=target.with_name('d2rs_watch_security_test.go')
        assert target.read_bytes()==data and not helper.exists(), 'concurrent trace mutation'
        try:
            text=data.decode();assert text.count('t.Parallel()')==2 and text.count('newWatchAccessToken()')==4 and text.count('w.authorizeRequest(')==5
            # Disable scheduling only; every original assertion remains intact.
            text=text.replace('t.Parallel()', '')
            text=text.replace(':= newWatchAccessToken()', ':= d2rsWatchToken(t)')
            text=text.replace('w.authorizeRequest(', 'd2rsWatchAuthorize(t, w, ')
            target.write_text(text);helper.write_bytes(hook.read_bytes())
            go=cli.bounded_run(['go','test','./d2cli','-run','^('+'|'.join(TESTS)+')$','-json','-count=1','-parallel=1'],env,capture,capture/'go.jsonl',600)
        finally:
            target.write_bytes(data);helper.unlink(missing_ok=True)
        assert go=={'exit':0,'failure':None},go
        events=cli.events(capture/'go.jsonl');assert all(events.get(t)=='pass' for t in TESTS+NAMES),events
        (capture/'manifest.json').write_text(json.dumps(dict(bindings,go=go,original_assertions=events,captured_sha256={name:sha(capture/name) for name in ['calls.json','go.jsonl']}),indent=2)+'\n')
    manifest=json.loads((capture/'manifest.json').read_text());assert all(manifest[k]==v for k,v in bindings.items()),'changed hook/source'
    assert all(sha(capture/name)==value for name,value in manifest['captured_sha256'].items()),'changed capture'
    assert manifest['go']=={'exit':0,'failure':None} and all(manifest['original_assertions'].get(t)=='pass' for t in TESTS+NAMES)
    calls=json.loads((capture/'calls.json').read_text());compare(calls,[])
    replay=capture/('replay-'+args.output.stem);replay.mkdir(exist_ok=False)
    request,response=replay/'input.json',replay/'output.json';request.write_text(json.dumps([c['input'] for c in calls])+'\n')
    env.update(D2RS_WATCH_REQUEST=str(request),D2RS_WATCH_RESPONSE=str(response))
    native=cli.bounded_run([str(runner),'--exact','watch::security_api_tests::replay','--ignored','--nocapture'],env,replay,replay/'native.log',30)
    results=json.loads(response.read_text()) if native=={'exit':0,'failure':None} and response.exists() else []
    assert sha(runner)==digest,'changed runner';rows=compare(calls,results)
    report=dict(bindings,runner=str(runner),runner_sha256=digest,capture=str(capture),replay=str(replay),rows=rows,calls=calls,results=results,native=native,original_assertions=manifest['original_assertions'],controls=dict(total_bytes=1<<30,file_bytes=128<<20,minimum_free_bytes=20<<30,go_deadline_seconds=600,native_deadline_seconds=30),projection='Original assertions retained. Complete application status/headers/body and authorization boolean. Native socket framing length/close validated separately; Go ResponseRecorder does not synthesize transport headers. Random tokens compare length, URL safety, error and independence, never random bytes. Separate source-derived parsing witnesses do not credit original tests.')
    args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(rows,indent=2))
    return int(any(r['outcome']!='match' for r in rows))

if __name__=='__main__':raise SystemExit(main())
