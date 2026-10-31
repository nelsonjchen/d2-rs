#!/usr/bin/env python3
"""Qualify diagnostic stage seed provenance without rewriting trace records."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]

def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def exact_json(left: object, right: object) -> bool:
    """Compare complete objects, retaining scalar types and float signed zero."""
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(exact_json(value, right[key]) for key, value in left.items())
    if isinstance(left, list):
        return len(left) == len(right) and all(exact_json(a, b) for a, b in zip(left, right))
    if isinstance(left, float):
        return struct.pack('>d', left) == struct.pack('>d', right)
    return left == right

def audit_stage_representation(event: dict) -> dict:
    """The source stage schema emits all f64 data as exact IEEE bit strings."""
    counts = {'integer_seed_values': 0, 'integer_edge_indexes': 0,
              'floating_point_bit_strings': 0, 'numeric_float_values': 0}
    def visit(value: object, path: tuple = ()) -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                visit(child, path + (key,))
        elif isinstance(value, list):
            for child in value:
                visit(child, path + ('[]',))
        elif type(value) is float:
            raise AssertionError(f'Unexpected numeric float at stage field {path}; stage f64 fields must use bits')
        elif type(value) is int:
            if path == ('seed',):
                assert -(1 << 63) <= value < (1 << 63)
                counts['integer_seed_values'] += 1
            else:
                assert path == ('edges', '[]', 'index'), path
                assert value >= 0
                counts['integer_edge_indexes'] += 1
        elif path and str(path[-1]).endswith('_bits'):
            assert type(value) is str and re.fullmatch(r'0x[0-9a-f]{16}', value), (path, value)
            counts['floating_point_bit_strings'] += 1
    assert event.get('event') == 'stage'
    assert type(event.get('seed')) is int or event.get('seed') is None
    visit(event)
    return counts

def audit_capture(report_path: Path) -> None:
    """Recheck preserved observations without rerunning any layout engines."""
    report = json.loads(report_path.read_text())
    counters = {side: {'integer_seed_values': 0, 'integer_edge_indexes': 0,
                      'floating_point_bit_strings': 0, 'numeric_float_values': 0}
                for side in ['go', 'rust']}
    cases = []
    for case in report['cases']:
        streams = {}
        for side, variant in [('go', 'new-go-enabled'), ('rust', 'rust-enabled')]:
            run = case['runs'][variant]
            stderr = Path(run['argv'][-1]).with_suffix('.stderr')
            assert digest(stderr) == run['stderr_sha256']
            captured = []
            for line_index, line in enumerate(stderr.read_text().splitlines(), 1):
                if line.startswith('{'):
                    event = json.loads(line)
                    if event.get('event') == 'stage':
                        captured.append({'stderr_line': line_index, 'event': event})
            assert exact_json(captured, run['stage_events'])
            streams[side] = [entry['event'] for entry in captured]
            for event in streams[side]:
                for key, value in audit_stage_representation(event).items():
                    counters[side][key] += value
        equality = exact_json(streams['go'], streams['rust'])
        assert equality == case['entire_stage_event_objects_exact_in_actual_order']
        contexts = {}
        for seed in sorted({event['seed'] for events in streams.values() for event in events}):
            contexts[str(seed)] = exact_json([event for event in streams['go'] if event['seed'] == seed],
                                             [event for event in streams['rust'] if event['seed'] == seed])
        cases.append({'name': case['name'], 'kind': case['kind'], 'requested_seeds': case['requested_seeds'],
                      'repeat': case['repeat'], 'stage_counts': {side: len(events) for side, events in streams.items()},
                      'entire_event_objects_exact_in_actual_order_with_scalar_types': equality,
                      'explicit_seed_context_subsequences_exact_with_scalar_types': contexts})
    for artifact in report['files']:
        assert digest(ROOT / artifact['path']) == artifact['sha256']
    assert not exact_json({'x_bits': '0x0000000000000000'}, {'x_bits': '0x8000000000000000'})
    assert not exact_json(0, False) and not exact_json(0, 0.0) and not exact_json(-0.0, 0.0)
    audit = {
        'scope': 'Complete original event dictionaries, actual event/list order and scalar types; no dropped fields. '
                 'Stage f64 fields are hex IEEE-754 bit strings, which preserve signed zero and every bit. '
                 'Actual captured numeric floats are forbidden by the audited Go/Rust source schema.',
        'report': {'path': str(report_path.relative_to(ROOT)), 'sha256': digest(report_path)},
        'scalar_counts': counters, 'cases': cases,
        'original_raw_stderr_stage_events_reparsed_and_exact': True,
        'all_original_capture_file_hashes_verified': len(report['files']),
        'signed_zero_bitstring_and_float_negative_controls_pass': True,
        'integer_boolean_and_float_type_negative_controls_pass': True,
    }
    destination = report_path.with_name('typed-stage-audit.json')
    destination.write_text(json.dumps(audit, indent=2) + '\n')
    print(json.dumps({'audit': str(destination), 'sha256': digest(destination), 'scalar_counts': counters}))

def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', default='.reference/tala-stage-seed-runtime-20261002/qualified-1')
    parser.add_argument('--audit-report', help='Audit an existing capture without engine runs')
    args = parser.parse_args()
    if args.audit_report:
        audit_capture(ROOT / args.audit_report)
        return
    capture = ROOT / args.output
    capture.mkdir(parents=True)
    binaries = {
        'official': ROOT / '.reference/bin/d2-v0.9.0-official',
        'old-go': ROOT / '.reference/bin/d2-v0.9.0-rng-operation-trace',
        'new-go': ROOT / '.reference/bin/d2-v0.9.0-stage-seed-trace',
        'rust': ROOT / '.reference/bin/d2-rs-cluster-alignment-diagnostic-20261002',
    }
    hashes = {side: digest(binary) for side, binary in binaries.items()}
    assert hashes['new-go'] == '9ad1f67fe3b4af4077dd2828c1aa3023336a1efced1ae31f764b2cd487aac7b6'
    assert hashes['rust'] == '7432d68ceac912f9271439a5d61681a262353dc5a5256fb841b01c0da93d0141'
    plans = []
    for name in ['root-wide-right', 'root-wide-down']:
        source = ROOT / f'tests/d2/tala-cluster-alignment-fixed-{name}.d2'
        for seed in [-17, 0, 1, 17]:
            plans.append((name, source, [seed], 1, 'single-seed'))
        for repeat in [1, 2]:
            plans.append((name, source, [1, 17], repeat, 'seed-race'))
    compound_source = ROOT / 'tools/probes/tala-stage-seed-go-tests-20261002/compound-probe.d2'
    for seed in [-17, 17]:
        plans.append(('compound-probe', compound_source, [seed], 1, 'compound'))
    cases = []
    for name, source, seeds, repeat, kind in plans:
        case_dir = capture / f'{name}-seeds-{",".join(map(str, seeds))}-repeat-{repeat}'
        case_dir.mkdir()
        runs = {}
        for variant, side, tracing in [
            ('official-disabled', 'official', False),
            ('old-go-disabled', 'old-go', False),
            ('new-go-disabled', 'new-go', False),
            ('new-go-enabled', 'new-go', True),
            ('rust-disabled', 'rust', False),
            ('rust-enabled', 'rust', True),
        ]:
            svg = case_dir / f'{variant}.svg'
            environment = {key: value for key, value in os.environ.items()
                           if not key.startswith(('D2_TRACE_', 'D2RS_TRACE_', 'TALA_TRACE_', 'WEFTAN_'))}
            flags = {'GOMAXPROCS': '1', 'RAYON_NUM_THREADS': '1'}
            if tracing:
                flags.update({'D2_TRACE_GRID_STATE': '1', 'D2_TRACE_LAYOUT_FINAL': '1'})
                flags['WEFTAN_TRACE_JSONL' if side == 'rust' else 'TALA_TRACE_JSONL'] = '1'
            environment.update(flags)
            argv = [str(binaries[side]), f'--layout={"tala-rs" if side == "rust" else "tala"}',
                    f'--tala-seeds={",".join(map(str, seeds))}', str(source), str(svg)]
            start = time.monotonic()
            completed = subprocess.run(argv, cwd=ROOT, env=environment, capture_output=True, timeout=90)
            stdout = case_dir / f'{variant}.stdout'; stderr = case_dir / f'{variant}.stderr'
            stdout.write_bytes(completed.stdout); stderr.write_bytes(completed.stderr)
            events = []; malformed = []; final_records = []
            for line_index, line in enumerate(completed.stderr.decode('utf-8').splitlines(), 1):
                if line.startswith('{'):
                    try:
                        event = json.loads(line)
                    except ValueError:
                        malformed.append({'stderr_line': line_index, 'text': line})
                    else:
                        if event.get('event') == 'stage':
                            audit_stage_representation(event)
                            events.append({'stderr_line': line_index, 'event': event})
                if line.startswith(('GRID_STATE ', 'GRID_ROUTE ', 'GRID_LABEL ')) and ' stage=layout-final ' in line:
                    final_records.append(line)
            runs[variant] = {
                'argv': argv, 'cwd': str(ROOT), 'environment_overrides': flags,
                'exit_code': completed.returncode, 'elapsed_seconds': time.monotonic() - start,
                'svg_sha256': digest(svg) if svg.exists() else None,
                'stdout_sha256': digest(stdout), 'stderr_sha256': digest(stderr),
                'stage_events': events, 'malformed_json_lines': malformed, 'final_records': final_records,
            }
        go_events = [entry['event'] for entry in runs['new-go-enabled']['stage_events']]
        rust_events = [entry['event'] for entry in runs['rust-enabled']['stage_events']]
        first_difference = None
        for index in range(max(len(go_events), len(rust_events))):
            left = go_events[index] if index < len(go_events) else None
            right = rust_events[index] if index < len(rust_events) else None
            if not exact_json(left, right):
                first_difference = {'event_index': index, 'go_event': left, 'rust_event': right}
                break
        go_final = runs['new-go-enabled']['final_records']; rust_final = runs['rust-enabled']['final_records']
        selected = {side: [event['seed'] for event in events if event['stage'] == 'SelectedResult']
                    for side, events in [('go', go_events), ('rust', rust_events)]}
        compound = {side: [event for event in events if event['stage'].startswith('Compound/')]
                    for side, events in [('go', go_events), ('rust', rust_events)]}
        # Context subsequences are an explicit analysis. Full actual event order
        # remains separately compared and preserved; this is not normalization.
        seed_context = {}
        for seed in sorted(set(seeds) | {event['seed'] for event in go_events + rust_events}):
            go_context = [event for event in go_events if event['seed'] == seed]
            rust_context = [event for event in rust_events if event['seed'] == seed]
            seed_context[str(seed)] = {'go_count': len(go_context), 'rust_count': len(rust_context),
                                      'ordered_entire_event_objects_exact': exact_json(go_context, rust_context)}
        row = {
            'name': name, 'kind': kind, 'source': str(source.relative_to(ROOT)),
            'source_sha256': digest(source), 'requested_seeds': seeds, 'repeat': repeat,
            'runs': runs, 'stage_event_counts': {'go': len(go_events), 'rust': len(rust_events)},
            'entire_stage_event_objects_exact_in_actual_order': bool(go_events) and exact_json(go_events, rust_events),
            'first_entire_event_difference': first_difference,
            'explicit_seed_context_subsequence_analysis': seed_context,
            'selected_seed_metadata': selected,
            'selected_seed_metadata_exact': bool(selected['go']) and selected['go'] == selected['rust'],
            'compound_event_counts': {side: len(events) for side, events in compound.items()},
            'compound_seeds': {side: sorted(set(event['seed'] for event in events)) for side, events in compound.items()},
            'exact_final_observations': bool(go_final) and bool(rust_final) and sorted(go_final) == sorted(rust_final),
            'go_official_old_new_disabled_enabled_svg_exact': len({runs[key]['svg_sha256'] for key in
                    ['official-disabled', 'old-go-disabled', 'new-go-disabled', 'new-go-enabled']}) == 1,
            'native_disabled_enabled_svg_exact': runs['rust-disabled']['svg_sha256'] == runs['rust-enabled']['svg_sha256'],
            'native_svg_bytes_equal_official': runs['rust-enabled']['svg_sha256'] == runs['official-disabled']['svg_sha256'],
            'all_exit_codes_zero': all(run['exit_code'] == 0 for run in runs.values()),
            'malformed_trace_json_count': sum(len(run['malformed_json_lines']) for run in runs.values()),
            'unknown_seed_context_count': sum(event.get('seed') is None or 'scope' in event for event in go_events + rust_events),
        }
        cases.append(row)
        (capture / 'partial-report.json').write_text(json.dumps({'binary_hashes': hashes, 'cases': cases}, indent=2) + '\n')
        print(json.dumps({key: row[key] for key in ['name', 'kind', 'requested_seeds', 'repeat', 'stage_event_counts',
            'entire_stage_event_objects_exact_in_actual_order', 'exact_final_observations',
            'selected_seed_metadata', 'compound_event_counts', 'compound_seeds',
            'go_official_old_new_disabled_enabled_svg_exact', 'native_disabled_enabled_svg_exact']}), flush=True)
    repeated = []
    for name in ['root-wide-right', 'root-wide-down']:
        pair = [case for case in cases if case['name'] == name and case['kind'] == 'seed-race']
        row = {'name': name}
        for side, variant in [('go', 'new-go-enabled'), ('rust', 'rust-enabled')]:
            streams = [[entry['event'] for entry in case['runs'][variant]['stage_events']] for case in pair]
            row[side + '_actual_order_repeated_exact'] = exact_json(streams[0], streams[1])
        repeated.append(row)
    assert hashes == {side: digest(binary) for side, binary in binaries.items()}
    files = []
    for path in sorted(capture.rglob('*')):
        if path.is_file() and path.name not in ['report.json', 'partial-report.json']:
            files.append({'path': str(path.relative_to(ROOT)), 'bytes': path.stat().st_size, 'sha256': digest(path)})
    report = {
        'schema_version': 1, 'scope': 'Seed metadata plus strict entire parsed stage objects; actual order preserved. '
            'Separate seed-context subsequences do not establish identical race interleaving.',
        'binary_hashes_before_after': hashes, 'binary_paths': {side: str(path) for side, path in binaries.items()},
        'cases': cases, 'race_repeat_order': repeated, 'files': files,
    }
    (capture / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(capture / 'report.json'), 'sha256': digest(capture / 'report.json')}), flush=True)

if __name__ == '__main__':
    main()
