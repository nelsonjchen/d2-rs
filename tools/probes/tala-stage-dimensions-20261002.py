#!/usr/bin/env python3
"""Replay public TALA dimension gates against immutable official/source oracles.

Run with uv and a frozen diagnostic Rust binary. Captures are never overwritten.
Entire stage records and raw error lines stay available, including race-order gaps.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
UPSTREAM = ROOT.parent / 'd2'
TRACE = ROOT.parent / 'd2-v0.9.0-trace'
TRACE_FAMILIES = ('D2_TRACE_', 'D2RS_TRACE_', 'D2_RS_', 'TALA_TRACE_', 'WEFTAN_')
OFFICIAL_SHA = 'e8ba501a413bac33a4d93014c475b804de10fb2863a391583e23a8d88160e311'
GO_TRACE_SHA = '9ad1f67fe3b4af4077dd2828c1aa3023336a1efced1ae31f764b2cd487aac7b6'
UPSTREAM_COMMIT = 'd5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda'


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def resolve(path: str) -> Path:
    path = Path(path)
    return path if path.is_absolute() else ROOT / path


def relative(path: Path) -> str:
    return str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path)


def artifact(path: Path) -> dict:
    return {'path': relative(path), 'sha256': digest(path), 'bytes': path.stat().st_size}


def load_comparator():
    spec = importlib.util.spec_from_file_location('stage_exact', ROOT / 'tools/probes/tala-stage-seed-20261002.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(binary: Path, variant: str, source: Path, tokens: list[str], folder: Path, comparator) -> dict:
    svg = folder / f'{variant}.svg'
    tracing = variant in ('go-source', 'native')
    native = variant.startswith('native')
    env = {key: value for key, value in os.environ.items() if not key.startswith(TRACE_FAMILIES)}
    overrides = {'GOMAXPROCS': '1', 'RAYON_NUM_THREADS': '1'}
    if tracing:
        overrides['WEFTAN_TRACE_JSONL' if native else 'TALA_TRACE_JSONL'] = '1'
    argv = [str(binary), f'--layout={"tala-rs" if native else "tala"}',
            f'--tala-seeds={",".join(tokens)}', str(source), str(svg)]
    start = time.monotonic()
    completed = subprocess.run(argv, cwd=ROOT, env=dict(env, **overrides), capture_output=True, timeout=60)
    stdout = folder / f'{variant}.stdout'
    stderr = folder / f'{variant}.stderr'
    stdout.write_bytes(completed.stdout)
    stderr.write_bytes(completed.stderr)
    events, other_lines, raw_other_lines, malformed, audits = [], [], [], [], []
    for line_index, raw_line in enumerate(completed.stderr.decode('utf-8').splitlines(keepends=True), 1):
        line = raw_line.rstrip('\r\n')
        if line.startswith('{'):
            try:
                event = json.loads(line)
            except ValueError:
                malformed.append(line)
            else:
                if isinstance(event, dict) and event.get('event') == 'stage':
                    try:
                        counts = comparator.audit_stage_representation(event)
                    except AssertionError as error:
                        audits.append({'stderr_line': line_index, 'representation_error': str(error)})
                    else:
                        audits.append({'stderr_line': line_index, 'scalar_counts': counts})
                    events.append(event)
                else:
                    malformed.append(line)
        else:
            other_lines.append(line)
            raw_other_lines.append(raw_line)
    # Raw line prefixes and continuation boundaries are deliberately retained.
    error_lines = other_lines if completed.returncode != 0 else []
    return {'argv': argv, 'cwd': str(ROOT), 'environment_overrides': overrides,
            'controlled_trace_families_removed': list(TRACE_FAMILIES),
            'exit_code': completed.returncode, 'elapsed_seconds': time.monotonic() - start,
            'stdout': artifact(stdout), 'stderr': artifact(stderr),
            'svg': artifact(svg) if svg.exists() else None, 'stage_events': events,
            'stage_representation_audits': audits,
            'raw_non_json_stderr_lines': other_lines, 'raw_cli_error_lines': error_lines,
            'raw_cli_error_text': ''.join(raw_other_lines) if completed.returncode != 0 else '',
            'malformed_or_unexpected_json_lines': malformed}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native', required=True, help='Frozen diagnostic d2-rs executable')
    parser.add_argument('--native-release', help='Optional separately frozen ordinary executable')
    parser.add_argument('--build-manifest', help='Tracked build/source binding manifest')
    parser.add_argument('--output', required=True, help='New capture directory; must not exist')
    parser.add_argument('--source', action='append', default=[], help='Additional native source snapshot')
    args = parser.parse_args()
    comparator = load_comparator()
    output = resolve(args.output)
    output.mkdir(parents=True, exist_ok=False)
    binaries = {'official': ROOT / '.reference/bin/d2-v0.9.0-official',
                'go-source': ROOT / '.reference/bin/d2-v0.9.0-stage-seed-trace',
                'native': resolve(args.native), 'native-off': resolve(args.native)}
    if args.native_release:
        binaries['native-release'] = resolve(args.native_release)
    binary_hashes = {side: artifact(binary) for side, binary in binaries.items()}
    assert binary_hashes['official']['sha256'] == OFFICIAL_SHA
    assert binary_hashes['go-source']['sha256'] == GO_TRACE_SHA
    upstream_commit = subprocess.check_output(['git', '-C', str(UPSTREAM), 'rev-parse', 'HEAD'], text=True).strip()
    pristine_status = subprocess.check_output(['git', '-C', str(UPSTREAM), 'status', '--porcelain'], text=True)
    assert upstream_commit == UPSTREAM_COMMIT and pristine_status == ''

    native_sources = ['AGENTS.md', 'crates/d2-rs-core/src/lib.rs',
                      'crates/d2-tala-rs/src/internal/engine/pipeline.rs',
                      'crates/d2-tala-rs/src/internal/engine/snapshot.rs',
                      'crates/d2-tala-rs/src/internal/grouping/clusters.rs',
                      'crates/d2-rs-cli/src/main.rs'] + args.source
    go_sources = ['d2layouts/d2talalayout/layout.go',
                  'd2layouts/d2talalayout/internal/engine/pipeline.go',
                  'd2layouts/d2talalayout/internal/engine/compound_candidate.go',
                  'd2layouts/d2talalayout/internal/layoutgraph/graph.go',
                  'd2layouts/d2talalayout/internal/layoutgraph/node.go',
                  'd2layouts/d2talalayout/internal/layoutgraph/edge.go']
    snapshots = []
    for owner, base, files in [('native', ROOT, native_sources), ('pristine-go', UPSTREAM, go_sources),
                               ('instrumented-go', TRACE, go_sources)]:
        for name in files:
            source = (base / name).resolve()
            source_name = source.relative_to(base.resolve())
            destination = output / 'sources' / owner / source_name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
            snapshots.append({'original': str(source), 'original_sha256': digest(source),
                              'snapshot': artifact(destination)})
    for source in [Path(__file__).resolve(), ROOT / 'tools/probes/tala-stage-seed-20261002.py']:
        destination = output / 'sources' / 'replay-tool' / source.name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        snapshots.append({'original': str(source), 'original_sha256': digest(source),
                          'snapshot': artifact(destination)})
    build_manifest = None
    if args.build_manifest:
        source = resolve(args.build_manifest)
        destination = output / 'sources' / 'build-manifest' / source.name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        build_manifest = {'original': artifact(source), 'snapshot': artifact(destination)}
        snapshots.append({'original': str(source), 'original_sha256': digest(source),
                          'snapshot': artifact(destination)})
    report = {'upstream_commit': upstream_commit, 'pristine_upstream_clean_before': True,
              'argv': ['uv', 'run', '--no-project', 'python', relative(Path(__file__)), *os.sys.argv[1:]],
              'binary_hashes_before_after': binary_hashes, 'source_snapshots': snapshots,
              'build_manifest': build_manifest,
              'comparison_contract': 'Complete original stage event objects and event/list order; only JSON property order and whitespace ignored. '
                                     'Every f64 field must be an exact IEEE bit string. Integer seed fields stay integer. '
                                     'Raw CLI error lines/text include every err: prefix, continuation line and line terminator. No fields or seed metadata removed.',
              'normalization_masks': [], 'cases': []}
    sources = output / 'inputs'
    sources.mkdir()
    for width in [29999, 30000, 30001]:
        (sources / f'width-{width}.d2').write_text(f'wide: {{\n  width: {width}\n  height: 80\n}}\n')
    plans = [(29999, ['1'], 1), (30000, ['1'], 1)]
    plans += [(30001, [token], 1) for token in ['1', '0', '17', '-17', '-0', '+0']]
    plans += [(30001, ['17', '-17', '0'], repeat) for repeat in [1, 2]]
    for width, tokens, repeat in plans:
        folder = output / f'width-{width}-tokens-{",".join(tokens)}-repeat-{repeat}'
        folder.mkdir()
        source = sources / f'width-{width}.d2'
        runs = {side: run(binary, side, source, tokens, folder, comparator) for side, binary in binaries.items()}
        go, native = runs['go-source']['stage_events'], runs['native']['stage_events']
        actual_seeds = [int(token) for token in tokens]
        expected_count = (38 if width <= 30000 else 8) * len(actual_seeds)
        per_seed = {str(seed): comparator.exact_json([event for event in go if event.get('seed') == seed],
                                                    [event for event in native if event.get('seed') == seed]) for seed in actual_seeds}
        expected_events_each_seed = 38 if width <= 30000 else 8
        observations = len(go) == len(native) == expected_count and all(
            sum(event.get('seed') == seed for event in events) == expected_events_each_seed
            for events in [go, native] for seed in actual_seeds)
        expected_exit = 0 if width <= 30000 else 1
        raw_errors_exact = all(comparator.exact_json(runs['official']['raw_cli_error_lines'], result['raw_cli_error_lines'])
                               for result in runs.values())
        raw_error_text_exact = all(runs['official']['raw_cli_error_text'] == result['raw_cli_error_text']
                                   for result in runs.values())
        no_stdout = all(result['stdout']['bytes'] == 0 for result in runs.values())
        no_disabled_trace = all(not result['stage_events'] for side, result in runs.items()
                                if side not in ('go-source', 'native'))
        all_success_svg_present = all(result['svg'] is not None for result in runs.values()) if not expected_exit else None
        go_svg_equal = (all_success_svg_present and runs['official']['svg']['sha256'] == runs['go-source']['svg']['sha256']) if not expected_exit else None
        native_svg_equal = (all_success_svg_present and runs['native']['svg']['sha256'] == runs['native-off']['svg']['sha256']) if not expected_exit else None
        release_svg_equal = (all_success_svg_present and runs['native']['svg']['sha256'] == runs['native-release']['svg']['sha256']) if not expected_exit and 'native-release' in runs else None
        row = {'width': width, 'requested_seed_tokens': tokens, 'actual_seed_values': actual_seeds,
               'repeat': repeat, 'source': artifact(source), 'runs': runs,
               'stage_counts': {'go': len(go), 'native': len(native)},
               'nonempty_expected_observations': observations,
               'entire_stage_event_objects_exact_actual_order': comparator.exact_json(go, native),
               'per_actual_seed_stage_objects_exact': per_seed,
               'expected_exit_codes_exact': all(result['exit_code'] == expected_exit for result in runs.values()),
               'raw_cli_error_lines_exact': raw_errors_exact,
               'raw_cli_error_text_exact_including_line_terminators': raw_error_text_exact,
               'all_stdout_empty': no_stdout,
               'disabled_trace_runs_emit_no_stage_events': no_disabled_trace,
               'all_success_svg_present': all_success_svg_present,
               'failed_runs_have_no_svg': all(result['svg'] is None for result in runs.values()) if expected_exit else None,
               'official_source_success_svg_byte_equal': go_svg_equal,
               'native_tracing_success_svg_byte_equal': native_svg_equal,
               'native_release_success_svg_byte_equal': release_svg_equal,
               'stage_representation_audits_pass': all('representation_error' not in audit
                                                       for result in runs.values() for audit in result['stage_representation_audits']),
               'first_complete_stage_difference': next(({'index': index, 'go': a, 'native': b}
                                                        for index, (a, b) in enumerate((go[index] if index < len(go) else None,
                                                                                      native[index] if index < len(native) else None)
                                                                                     for index in range(max(len(go), len(native))))
                                                        if not comparator.exact_json(a, b)), None)}
        report['cases'].append(row)
        print(json.dumps({key: row[key] for key in ['width', 'requested_seed_tokens', 'repeat', 'stage_counts',
              'nonempty_expected_observations', 'entire_stage_event_objects_exact_actual_order', 'raw_cli_error_lines_exact']}))
    report['public_error_and_stage_boundary_gates_pass'] = all(
        row['nonempty_expected_observations'] and row['expected_exit_codes_exact'] and row['raw_cli_error_lines_exact']
        and row['raw_cli_error_text_exact_including_line_terminators']
        and row['all_stdout_empty'] and row['disabled_trace_runs_emit_no_stage_events'] and row['stage_representation_audits_pass']
        and (row['failed_runs_have_no_svg'] if row['width'] > 30000 else row['all_success_svg_present']
             and row['official_source_success_svg_byte_equal'] and row['native_tracing_success_svg_byte_equal']
             and (row['native_release_success_svg_byte_equal'] if 'native-release' in row['runs'] else True))
        and all(row['per_actual_seed_stage_objects_exact'].values())
        and not any(result['malformed_or_unexpected_json_lines'] for result in row['runs'].values())
        and (len(row['actual_seed_values']) > 1 or row['entire_stage_event_objects_exact_actual_order'])
        for row in report['cases'])
    report['full_global_ordered_trace_pass'] = all(row['nonempty_expected_observations']
                                                 and row['entire_stage_event_objects_exact_actual_order']
                                                 for row in report['cases'])
    races = [row for row in report['cases'] if len(row['actual_seed_values']) > 1]
    report['race_repeat_entire_event_order_exact'] = {
        side: bool(races[0]['runs'][side]['stage_events']) and bool(races[1]['runs'][side]['stage_events'])
              and comparator.exact_json(races[0]['runs'][side]['stage_events'], races[1]['runs'][side]['stage_events'])
        for side in ['go-source', 'native']}
    report['limitations'] = (
        'Any differing global seed-race order remains an explicit failing full ordered-trace comparison; per-seed observations do not erase or qualify that gap. '
        'Successful SVG byte identity is tested only within Go oracle builds and native trace on/off, not between Go and Rust serializers. '
        'These small controls do not establish full operation trace, rendering, architecture or performance parity.')
    report['binary_hashes_after'] = {side: artifact(binary) for side, binary in binaries.items()}
    assert binary_hashes == report['binary_hashes_after']
    assert all(digest(Path(row['original'])) == row['original_sha256'] for row in snapshots)
    report['pristine_upstream_clean_after'] = subprocess.check_output(['git', '-C', str(UPSTREAM), 'status', '--porcelain'], text=True) == ''
    report['upstream_commit_after'] = subprocess.check_output(['git', '-C', str(UPSTREAM), 'rev-parse', 'HEAD'], text=True).strip()
    assert report['pristine_upstream_clean_after'] and report['upstream_commit_after'] == upstream_commit
    report['files'] = [artifact(path) for path in sorted(output.rglob('*')) if path.is_file()]
    destination = output / 'report.json'
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': artifact(destination), 'public_error_and_stage_boundary_gates_pass': report['public_error_and_stage_boundary_gates_pass'],
                      'full_global_ordered_trace_pass': report['full_global_ordered_trace_pass'],
                      'race_repeat_entire_event_order_exact': report['race_repeat_entire_event_order_exact']}))
    if not report['public_error_and_stage_boundary_gates_pass']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
