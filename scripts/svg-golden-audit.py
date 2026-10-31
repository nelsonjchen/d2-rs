#!/usr/bin/env python3
"""Requalify every non-passing raw SVG row against the manifest-pinned Go source.

No new golden is claimed when Go cannot reproduce it. Complete side inventories
and output validity are required. Captures are fresh, bounded to 1 GiB total /
128 MiB per file / 20 GiB free, and use 300-second process deadlines.
"""
import argparse
import collections

# Reuse the raw tier's existing version/newline allowance exactly.
import importlib.util
import json
import xml.etree.ElementTree as ET
from pathlib import Path

import references


def module(name):
    spec = importlib.util.spec_from_file_location(name, references.ROOT / 'scripts' / (name + '.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def artifact(path):
    error = Path(str(path) + '.err')
    if path.is_file() == error.is_file():
        raise ValueError(f'expected exactly one SVG or error: {path}')
    chosen = error if error.is_file() else path
    if not chosen.stat().st_size:
        raise ValueError(f'empty output: {chosen}')
    if not error.is_file():
        ET.fromstring(path.read_bytes())
    return chosen, error.is_file()


def classify(go, rust, golden):
    g, gerr = artifact(go)
    r, rerr = artifact(rust)
    result = {'go_output_sha256': references.digest(g), 'rust_output_sha256': references.digest(r)}
    if gerr:
        return dict(result, outcome='go-decode-or-render-error', go_error=g.read_text(),
                    rust_error=r.read_text() if rerr else None)
    normalize = module('d2svg-golden').normalize
    if normalize(g.read_text()) == normalize(golden.read_text()):
        outcome = 'rust-error' if rerr else ('golden-match' if normalize(r.read_text()) == normalize(golden.read_text()) else 'rust-mismatch')
    else:
        outcome = 'stale-in-go'
    return dict(result, outcome=outcome,
                rust_go_match=not rerr and normalize(r.read_text()) == normalize(g.read_text()),
                raw_go_golden_equal=g.read_bytes() == golden.read_bytes())


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--baseline', type=Path, required=True)
    p.add_argument('--capture', type=Path, required=True)
    p.add_argument('--go', type=Path, required=True)
    p.add_argument('--rust', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    source = references.verify_source()
    rows = json.loads(a.baseline.read_text())['rows']
    selected = [r for r in rows if r['outcome'] != 'svg-match']
    assert selected and len({(r['name'], r['engine']) for r in selected}) == len(selected)
    capture = a.capture.resolve()
    capture.mkdir(exist_ok=False)
    themes = {c['name']: c['themeID'] for c in (json.loads(p.read_text()) for p in references.E2E_CASES.glob('*.json'))}
    records = []
    for i, row in enumerate(selected):
        board = source / 'e2etests/testdata' / row['name'] / row['engine']
        records.append(dict(row, input=str(board / 'board.exp.json'), golden=str(board / 'sketch.exp.svg'), index=i))
    import os
    runs = {}
    binaries = {}
    for side, runner in [('go', a.go.resolve()), ('rust', a.rust.resolve())]:
        (capture / side).mkdir()
        manifest = [{'input': r['input'], 'output': str(capture / side / f"{r['index']}.svg"), 'theme_id': themes.get(r['name'])} for r in records]
        path = capture / (side + '-manifest.json')
        path.write_text(json.dumps(manifest, indent=2) + '\n')
        binaries[side] = {'path': str(runner), 'sha256': references.digest(runner)}
        run = module('cli-scoreboard').bounded_run([str(runner), str(path)], os.environ.copy(), capture, capture / (side + '.log'), 300)
        runs[side] = run
        assert run == {'exit': 0, 'failure': None}, run
        assert references.digest(runner) == binaries[side]['sha256']
        actual = {x.name for x in (capture / side).iterdir()}
        expected = set()
        for r in records:
            output, _ = artifact(capture / side / f"{r['index']}.svg")
            expected.add(output.name)
        assert actual == expected, (actual, expected)
    scored = []
    for row in records:
        result = classify(capture / 'go' / f"{row['index']}.svg", capture / 'rust' / f"{row['index']}.svg", Path(row['golden']))
        scored.append(dict(row, **result, input_sha256=references.digest(Path(row['input'])), golden_sha256=references.digest(Path(row['golden']))))
    report = {'source_commit': references.SOURCE_COMMIT, 'go_adapter_sha256': references.digest(references.ROOT / 'tools/svg-golden-oracle/main.go'),
              'baseline_sha256': references.digest(a.baseline), 'binaries': binaries, 'runs': runs,
              'capture': str(capture), 'summary': dict(collections.Counter(r['outcome'] for r in scored)), 'rows': scored}
    a.output.write_text(json.dumps(report, indent=2) + '\n')
    (capture / 'hashes.json').write_text(json.dumps({str(p.relative_to(capture)): references.digest(p) for p in capture.rglob('*') if p.is_file()}, indent=2) + '\n')
    print(report['summary'])


if __name__ == '__main__':
    main()
