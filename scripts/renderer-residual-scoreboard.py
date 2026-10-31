#!/usr/bin/env python3
"""Observe remaining original feature, scene, PDF and snapshot contracts.

Complete native results compare, including PDF bytes under the existing shared
UTC clock/catalog-sort profile. Snapshot-maintenance helpers are infrastructure,
never native passes. All original Go assertions remain active. Capture budgets:
1 GiB total / 128 MiB per file / 20 GiB free, 600-second subprocess deadlines.
"""
import argparse
import collections
import json
import os
from pathlib import Path

import references
import importlib.util

ROOT = references.ROOT


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


base = module('renderer-contract-scoreboard')
SUITES = {
    'features': {
        'package': 'internal/d2layoutfeatures', 'file': 'features_test.go',
        'tests': {'TestFeatureSupport': 48, 'TestUnknownLayout': 4},
        'imports': '"github.com/d2lang/d2/d2graph"\n"github.com/d2lang/d2/d2format"',
        'replacements': {'Check(': 'd2rsFeatureCheck(t, '},
        'wrappers': '''func d2rsFeatureCheck(t *testing.T, name string, g *d2graph.Graph) error {
    input := ""
    if g.AST != nil { input = d2format.Format(g.AST) }
    err := Check(name, g)
    d2rsRecord(t, map[string]any{"operation":"features", "layout":name, "input":input}, nil, err)
    return err
}''',
        'sources': ['features.go'],
    },
    'icon': {
        'package': 'd2renderers/d2isometric', 'file': 'scene_test.go',
        'tests': {'TestIconURLIdentityAndOwnership': 1},
        'imports': '"github.com/d2lang/d2/d2target"',
        'replacements': {'BuildScene(': 'd2rsIconScene(t, '},
        'wrappers': '''func d2rsIconScene(t *testing.T, d *d2target.Diagram, opts *RenderOpts) (*Scene, error) {
    users := []any{}
    for _, shape := range d.Shapes {
        if shape.Icon == nil || shape.Icon.User == nil { users = append(users, nil); continue }
        password, present := shape.Icon.User.Password()
        users = append(users, map[string]any{"username":shape.Icon.User.Username(),"password":password,"hasPassword":present})
    }
    scene, err := BuildScene(d, opts)
    var output any
    if err == nil {
        source, owned := d.Shapes[0].Icon, scene.Nodes[0].Metadata.Original.Icon
        output = map[string]any{"scene":scene,"urlOwned":source != owned,"userOwned":source.User != owned.User}
    }
    d2rsRecord(t, map[string]any{"operation":"icon", "diagram":d, "users":users}, output, err)
    return scene, err
}''',
        'sources': ['scene.go', 'hierarchy.go', 'layout.go'],
    },
    'pdf': {
        'package': 'internal/testutil', 'file': 'pdf_test.go',
        'tests': {'TestPDFPageFillDoesNotDependOnNavigation': 8},
        'imports': '"bytes"\n"io"\n"github.com/d2lang/d2/d2target"\n"github.com/d2lang/d2/lib/pdf"',
        'replacements': {'document.AddPDFPage(': 'd2rsPDFPage(t, document, ',
                         'document.ExportTo(': 'd2rsPDFExport(t, document, '},
        'wrappers': '''var d2rsPDFMu sync.Mutex
var d2rsPDFPages = map[*pdf.GoFPDF][]any{}
func d2rsPDFPage(t *testing.T, document *pdf.GoFPDF, png []byte, titles []pdf.BoardTitle, theme int64, fill string, shapes []d2target.Shape, pad int64, x, y float64, pages map[string]int, nav bool) error {
    err := document.AddPDFPage(png, titles, theme, fill, shapes, pad, x, y, pages, nav)
    if err != nil { t.Fatal(err) }
    d2rsPDFMu.Lock()
    defer d2rsPDFMu.Unlock()
    d2rsPDFPages[document] = append(d2rsPDFPages[document], map[string]any{"png":png,"titles":titles,"theme":theme,"fill":fill,"shapes":shapes,"pad":pad,"x":x,"y":y,"pageMap":pages,"nav":nav})
    return err
}
func d2rsPDFExport(t *testing.T, document *pdf.GoFPDF, sink io.Writer) error {
    var data bytes.Buffer
    err := document.ExportTo(&data)
    if err == nil { _, err = sink.Write(data.Bytes()) }
    d2rsPDFMu.Lock()
    pages := d2rsPDFPages[document]
    delete(d2rsPDFPages, document)
    d2rsPDFMu.Unlock()
    d2rsRecord(t, map[string]any{"operation":"pdf","pages":pages}, data.Bytes(), err)
    return err
}''',
        'sources': ['../../lib/pdf/pdf.go'],
    },
    'e2e': {
        'package': 'e2etests', 'file': 'isometric_test.go',
        'tests': {'TestIsometricSVGSnapshot': 2, 'TestIsometricBoards': 0},
        'imports': '"context"\n"github.com/d2lang/d2/d2target"\n"github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg"',
        'replacements': {'d2isometricimg.Render(': 'd2rsSnapshotRender(t, '},
        'wrappers': '''func d2rsSnapshotRender(t *testing.T, ctx context.Context, d *d2target.Diagram, opts *d2isometricimg.Options) ([]byte, error) {
    out, err := d2isometricimg.Render(ctx, d, opts)
    d2rsRecord(t, map[string]any{"operation":"isometric_svg","diagram":d,"options":opts}, out, err)
    return out, err
}''',
        'sources': ['../d2renderers/d2isometric/d2isometricimg/render.go'],
    },
    'png_helpers': {
        'package': 'e2etests/png', 'file': 'snapshot_test.go',
        'tests': {'TestSnapshotPixels': 0, 'TestPixelRoundingBound': 0},
        'imports': '', 'replacements': {}, 'wrappers': '', 'sources': [],
    },
}
INFRASTRUCTURE = {
    'TestIsometricBoards': 'Go e2e board-name enumeration and stale-snapshot acceptance/removal helper',
    'TestSnapshotPixels': 'Go PNG golden accept/reject and got-file housekeeping helper',
    'TestPixelRoundingBound': 'Go snapshot-comparator tolerance helper; native raster gate remains exact',
}


def sha(path):
    return base.sha(path)


def source():
    pinned = references.verify_source()
    paths = [str(Path(s['package']) / n) for s in SUITES.values()
             for n in [s['file'], *s['sources']]]
    return {'commit': references.SOURCE_COMMIT,
            'files': {n: sha(pinned / n) for n in paths},
            'observer': sha(base.TEMPLATE), 'script': sha(Path(__file__)),
            'transform_script': sha(ROOT / 'scripts/renderer-contract-scoreboard.py'),
            'clock_helper': sha(ROOT / 'tools/cli-scoreboard/pdf_clock.go'),
            'profile': {'SOURCE_DATE_EPOCH': '946782245', 'timezone': 'UTC', 'catalog_sort': True}}


def validate_rows(rows, suite):
    assert collections.Counter(r['test'].split('/')[0] for r in rows) == collections.Counter(suite['tests'])
    assert all(r['package'] == suite['package'] and 'request' in r and set(r['got']) == {'output', 'error'} for r in rows)
    counts = collections.Counter(r['test'] for r in rows)
    assert {r['name'] for r in rows} == {f'{t}#{i}' for t, n in counts.items() for i in range(n)}
    assert len({r['name'] for r in rows}) == len(rows)


def verify(capture):
    assert json.loads((capture / 'source.json').read_text()) == source()
    assert json.loads((capture / 'hashes.json').read_text()) == base.hashes(capture)
    cases = []
    for key, suite in SUITES.items():
        root = capture / key
        assert json.loads((root / 'run.json').read_text()) == {'exit': 0, 'failure': None}
        events = module('cli-scoreboard').events(root / 'events.jsonl')
        assert all(events.get(t) == 'pass' for t in suite['tests']) and 'fail' not in events.values(), events
        rows = [json.loads(p.read_text()) for p in sorted((root / 'calls').glob('*.json'))]
        validate_rows(rows, suite)
        cases.extend(rows)
    sizes = [p.stat().st_size for p in capture.rglob('*') if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    capture.mkdir(exist_ok=False)
    pinned = references.verify_source()
    (capture / 'tmp').mkdir()
    (capture / 'source.json').write_text(json.dumps(source(), indent=2) + '\n')
    # Reuse the already prescribed oracle clock controls, validating every byte.
    pdf = references.TRACE_CHECKOUT / 'lib/pdf/pdf.go'
    original_pdf = pdf.read_bytes()
    plain = (pinned / 'lib/pdf/pdf.go').read_text()
    controlled = plain.replace('newGofPDF.SetMargins(0, 0, 0)', 'newGofPDF.SetMargins(0, 0, 0)\n\td2rsPDFClock(newGofPDF)')
    assert original_pdf in [plain.encode(), controlled.encode()]
    helper = references.TRACE_CHECKOUT / 'lib/pdf/d2rs_clock.go'
    assert helper.read_bytes() == (ROOT / 'tools/cli-scoreboard/pdf_clock.go').read_bytes()
    try:
        pdf.write_text(controlled)
        for key, suite in SUITES.items():
            root = capture / key
            (root / 'calls').mkdir(parents=True)
            test = references.TRACE_CHECKOUT / suite['package'] / suite['file']
            original = (pinned / suite['package'] / suite['file']).read_bytes()
            hook = test.parent / 'd2rs_renderer_residual_test.go'
            assert test.read_bytes() == original and not hook.exists()
            text = base.transform(original.decode(), suite['replacements'])
            observer = base.TEMPLATE.read_text().replace('PACKAGE_PATH', suite['package'])
            package = 'png_test' if key == 'png_helpers' else ('e2etests' if key == 'e2e' else Path(suite['package']).name)
            observer = observer.replace('PACKAGE', package).replace('EXTRA_IMPORTS', suite['imports']).replace('WRAPPERS', suite['wrappers'])
            env = dict(os.environ, D2RS_RENDERER_CONTRACT_DUMP=str(root / 'calls'),
                       TMPDIR=str(capture / 'tmp'), SOURCE_DATE_EPOCH='946782245', TZ='UTC',
                       GOMODCACHE=str(ROOT / '.reference/go-mod/pkg/mod'),
                       GOCACHE=str(ROOT / '.reference/go-build'), GOFLAGS='-mod=mod', GOPROXY='off')
            env.pop('TA', None)
            env.pop('TESTDATA_ACCEPT', None)
            try:
                test.write_text(text)
                hook.write_text(observer)
                result = module('cli-scoreboard').bounded_run([
                    'go', 'test', './' + suite['package'], '-json', '-count=1', '-parallel=1',
                    '-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0',
                    '-run', '^(' + '|'.join(suite['tests']) + ')$',
                ], env, capture, root / 'events.jsonl', 600)
            finally:
                test.write_bytes(original)
                hook.unlink(missing_ok=True)
            (root / 'run.json').write_text(json.dumps(result) + '\n')
            assert result == {'exit': 0, 'failure': None}, (key, result)
    finally:
        pdf.write_bytes(original_pdf)
    (capture / 'hashes.json').write_text(json.dumps(base.hashes(capture), indent=2) + '\n')
    verify(capture)


def classify(cases, actual, run):
    rows = []
    for i, case in enumerate(cases):
        row = {k: case[k] for k in ['name', 'test', 'package']}
        if run != {'exit': 0, 'failure': None}:
            row['outcome'] = run['failure'] or 'rust-failed'
        elif not isinstance(actual, list) or len(actual) != len(cases):
            row['outcome'] = 'invalid-inventory'
        else:
            row['outcome'] = 'match' if module('language-scoreboard').same_json(actual[i], case['got']) else 'mismatch'
            if row['outcome'] != 'match':
                row.update(expected=case['got'], actual=actual[i])
        if case['test'] == 'TestIsometricSVGSnapshot':
            row['qualification'] = 'Original Render bytes; Go-only snapshot acceptance, inventory and skip-flag assertions separately pass in the original body.'
        rows.append(row)
    return rows


def score(capture, runner, output):
    assert not output.exists()
    cases = verify(capture)
    work = output.with_suffix('')
    work.mkdir(exist_ok=False)
    request = work / 'requests.json'
    request.write_text(json.dumps([c['request'] for c in cases]))
    digest = sha(runner)
    log = work / 'responses.json'
    env = dict(os.environ, SOURCE_DATE_EPOCH='946782245', TZ='UTC')
    run = module('cli-scoreboard').bounded_run([str(runner), str(request)], env, work, log, 600)
    try:
        actual = json.loads(log.read_text())
    except (ValueError, UnicodeError):
        actual = None
    assert sha(runner) == digest
    rows = classify(cases, actual, run)
    for key, suite in SUITES.items():
        for test, count in suite['tests'].items():
            if count == 0:
                rows.append({'name': test, 'test': test, 'package': suite['package'],
                             'outcome': 'not-applicable', 'reason': INFRASTRUCTURE[test],
                             'original_go_assertion': 'pass'})
    report = {'source_commit': references.SOURCE_COMMIT, 'capture': str(capture),
              'capture_hashes': base.hashes(capture), 'runner': str(runner), 'runner_sha256': digest,
              'source': source(), 'summary': dict(collections.Counter(r['outcome'] for r in rows)), 'rows': rows}
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(report['summary'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', required=True, type=Path)
    parser.add_argument('--rust', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--reuse', action='store_true')
    args = parser.parse_args()
    if not args.reuse:
        capture_original(args.capture.resolve())
    score(args.capture.resolve(), args.rust.resolve(), args.output.resolve())


if __name__ == '__main__':
    main()
