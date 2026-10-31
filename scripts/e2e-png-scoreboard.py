#!/usr/bin/env python3
"""Observe original TestPNG calls; compare complete native PNG pixels strictly.

Only the trace checkout is instrumented, and all original assertions remain
active against capture-local copies. No upstream one-quantum allowance is used
for native qualification. A fresh output tree is required for every replay.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import references

ROOT = references.ROOT
PACKAGE = 'e2etests/png'


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def expected_outputs(source):
    paths = sorted((source / PACKAGE / 'testdata').rglob('isometric*.exp.png'))
    assert paths, 'empty golden inventory'
    return {p.relative_to(source / PACKAGE / 'testdata').as_posix().replace('.exp.png', '.png'): p for p in paths}


def score(source, capture, replay, events, run):
    gate = module('isometric_png_scoreboard')
    expected = expected_outputs(source)
    originals = {p.relative_to(capture / 'go').as_posix() for p in (capture / 'go').rglob('*') if p.is_file()}
    native = {p.relative_to(replay / 'native').as_posix() for p in (replay / 'native').rglob('*') if p.is_file()}
    options = {p.relative_to(capture / 'options').with_suffix('.png').as_posix() for p in (capture / 'options').rglob('*.json')}
    passed = {e['Test'] for e in events if e.get('Action') == 'pass' and 'Test' in e}
    rows = []
    complete = bool(expected) and originals == native == options == set(expected) and run == {'exit': 0, 'failure': None}
    complete = complete and 'TestPNG' in passed and all('TestPNG/' + n.removesuffix('.png') in passed for n in expected)
    for name, golden in expected.items():
        qualification = gate.strict_pixels(golden, capture / 'go' / name)
        comparison = gate.strict_pixels(capture / 'go' / name, replay / 'native' / name)
        rows.append(dict(board=name, qualification=qualification, comparison=comparison,
                         outcome='match' if complete and qualification['matched'] and comparison['matched'] else 'mismatch'))
    return rows, dict(expected=sorted(expected), go=sorted(originals), native=sorted(native), options=sorted(options), complete=complete)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--rust', required=True, type=Path)
    parser.add_argument('--reuse', action='store_true')
    args = parser.parse_args()
    source = references.verify_source()
    assert subprocess.check_output(['git', '-C', str(references.TRACE_CHECKOUT), 'rev-parse', 'HEAD'], text=True).strip() == references.SOURCE_COMMIT
    capture, output, runner = args.capture.resolve(), args.output.resolve(), args.rust.resolve()
    replay = output.with_suffix('')
    assert not output.exists() and not replay.exists(), 'fresh replay/output paths required'
    cli = module('cli-scoreboard')
    test = source / PACKAGE / 'render_test.go'
    hook = ROOT / 'tools/e2e-png-scoreboard/hook_test.go'
    bindings = dict(source_commit=references.SOURCE_COMMIT, source_sha256=references.digest(test), hook_sha256=references.digest(hook),
                    goldens={name: references.digest(path) for name, path in expected_outputs(source).items()})
    if not args.reuse:
        capture.mkdir(parents=True, exist_ok=False)
        (capture / 'tmp').mkdir()
        target = references.TRACE_CHECKOUT / PACKAGE / test.name
        helper = target.with_name('d2rs_png_observer_test.go')
        original = test.read_bytes()
        assert target.read_bytes() == original and not helper.exists(), 'concurrent trace mutation'
        text = original.decode()
        for old, new in [('dir := filepath.Join("testdata", fixture)', 'dir := d2rsPNGSnapshotDir(t, fixture)'),
                         ('d2isometricimg.Render(context.Background(), board,', 'd2rsPNGRender(t, context.Background(), board,')]:
            assert text.count(old) == 1
            text = text.replace(old, new)
        env = dict(os.environ, D2RS_PNG_CAPTURE=str(capture), TESTDATA_ACCEPT='', TA='',
                   GOMODCACHE=str(ROOT / '.reference/go-mod/pkg/mod'), GOCACHE=str(ROOT / '.reference/go-build'),
                   GOTOOLCHAIN=references.MANIFEST['d2']['go_toolchain'], GOFLAGS='-mod=mod', GOPROXY='off', TMPDIR=str(capture / 'tmp'))
        try:
            target.write_text(text)
            helper.write_bytes(hook.read_bytes())
            command = ['go', 'test', './' + PACKAGE, '-json', '-run', '^TestPNG$', '-count=1', '-timeout=15m']
            run = cli.bounded_run(command, env, capture, capture / 'go.log', 1000)
        finally:
            assert target.read_text() == text and helper.read_bytes() == hook.read_bytes(), 'trace changed during capture'
            target.write_bytes(original)
            helper.unlink()
        manifest = dict(bindings, run=run, files={str(p.relative_to(capture)): references.digest(p)
                                                 for p in capture.rglob('*') if p.is_file()})
        (capture / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    else:
        manifest = json.loads((capture / 'manifest.json').read_text())
        assert all(manifest[k] == v for k, v in bindings.items()), 'source/golden/hook changed'
        for name, digest in manifest['files'].items():
            assert references.digest(capture / name) == digest, 'capture changed: ' + name
    assert manifest['run'] == {'exit': 0, 'failure': None}, manifest['run']
    replay.mkdir(parents=True)
    (replay / 'tmp').mkdir()
    requests = [dict(board=str(path), out=str(replay / 'native' / path.parent.relative_to(capture / 'inputs')), mode='png')
                for path in sorted((capture / 'inputs').rglob('board.json'))]
    assert requests and len(requests) == len({str(p.parent) for p in expected_outputs(source).values()}), 'missing/extra original inputs'
    request = replay / 'requests.jsonl'
    request.write_text(''.join(json.dumps(row) + '\n' for row in requests))
    # bounded_run owns the process group and guards all output. A small worker
    # supplies stdin without an unguarded pipe or shell command construction.
    runner_hash = references.digest(runner)
    run = cli.bounded_run([sys.executable, __file__, '--worker', str(runner), str(request)],
                          dict(os.environ, TMPDIR=str(replay / 'tmp')), replay, replay / 'native.log', 1000,
                          extra_directories=(capture,))
    assert references.digest(runner) == runner_hash, 'native executable changed'
    events = [json.loads(line) for line in (capture / 'go.log').read_text().splitlines() if line.startswith('{')]
    boards, inventory = score(source, capture, replay, events, run)
    matched = bool(boards) and inventory['complete'] and all(row['outcome'] == 'match' for row in boards)
    report = dict(bindings, package=PACKAGE, capture=str(capture), replay=str(replay), rust=str(runner), rust_sha256=runner_hash,
                  run=run, budgets=dict(total=1 << 30, file=128 << 20, free=20 << 30), inventory=inventory,
                  rows=[dict(test='TestPNG', calls=len(boards), matched_calls=sum(r['outcome'] == 'match' for r in boards),
                             outcome='match' if matched else 'mismatch')], boards=boards)
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(report['rows'])
    return 0 if matched else 1


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--worker':
        with Path(sys.argv[3]).open('rb') as stream:
            subprocess.run([sys.argv[2]], stdin=stream, check=True, timeout=950)
    else:
        raise SystemExit(main())
