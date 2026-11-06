"""Exercise matrix failure reporting without rebuilding layout oracles."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('layout_gate', Path(__file__).with_name('check-layout-parity.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class LayoutGateTests(unittest.TestCase):
    def test_route_search_capture_omits_unrelated_large_trace_families(self):
        args = SimpleNamespace(
            search_flavor='ShortestToLongest', gap=False, search_heap=True,
            search_costs=True, sized_calls=False,
        )
        self.assertEqual(
            gate.capture_options(args),
            ['--routing', '--search-flavor', 'ShortestToLongest', '--search-heap', '--search-costs'],
        )

        args.gap = True
        self.assertEqual(
            gate.capture_options(args),
            ['--routing', '--gap', '--search-flavor', 'ShortestToLongest', '--search-heap', '--search-costs'],
        )

    def test_full_trace_capture_keeps_placement_packing_routing_and_rng(self):
        args = SimpleNamespace(search_flavor=None, gap=True, search_heap=False,
                               search_costs=False, sized_calls=False)
        self.assertEqual(
            gate.capture_options(args),
            ['--placement', '--packing', '--routing', '--rng', '--gap'],
        )

    def test_sized_call_capture_is_focused(self):
        args = SimpleNamespace(
            search_flavor=None, gap=False, search_heap=False,
            search_costs=False, sized_calls=True,
        )
        self.assertEqual(gate.capture_options(args), ['--sized-calls'])

        args.sized_calls = False
        args.sized_results = True
        self.assertEqual(gate.capture_options(args), ['--sized-results'])

    def test_port_swap_capture_is_focused(self):
        args = SimpleNamespace(
            swap_ports=True, search_flavor=None, gap=False, search_heap=False,
            search_costs=False, sized_calls=False, sized_results=False,
        )
        self.assertEqual(gate.capture_options(args), ['--swap-ports'])

    def test_matching_trace_cannot_hide_public_graph_or_instrumentation_difference(self):
        for graph, ordinary_identical in [('{"x":-0}', True), ('{"x":0}', False)]:
            with self.subTest(graph=graph, ordinary_identical=ordinary_identical):
                with tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    witness = root / 'witness'
                    witness.mkdir()
                    (witness / 'go.stdout.json').write_text('{"x":0}')
                    (witness / 'rust.stdout.json').write_text(graph)
                    (witness / 'manifest.json').write_text(json.dumps({'oracles': {
                        'go': {'public_bytes_identical': True},
                        'rust': {'public_bytes_identical': ordinary_identical},
                    }}))
                    report = root / 'report.json'
                    args = ['check-layout-parity.py', '--seed', '1', '--fixture', 'test.d2', '--report', str(report)]
                    with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                         patch.object(gate, 'run', return_value=str(witness)), \
                         patch.object(gate, 'digest', return_value='test-only'), \
                         patch.object(gate.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'MATCH 1 events', '')), \
                         patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                        with self.assertRaises(SystemExit):
                            gate.main()
                    result = json.loads(report.read_text())
                    self.assertEqual(result['matched'], 0)
                    self.assertEqual(result['results'][0]['status'], 'divergent')
                    self.assertEqual(result['results'][0]['event_count'], 1)


    def test_empty_search_requires_verified_edgeless_input_and_equal_outputs(self):
        for edge_count, rust_graph, expected in [(0, '{"x":0}', 'inapplicable'),
                                                   (1, '{"x":0}', 'insufficient_evidence'),
                                                   (None, '{"x":0}', 'insufficient_evidence'),
                                                   (0, '{"x":1}', 'divergent')]:
            with self.subTest(edge_count=edge_count, rust_graph=rust_graph), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                witness = root / 'witness'
                witness.mkdir()
                (witness / 'go.stdout.json').write_text('{"x":0}')
                (witness / 'rust.stdout.json').write_text(rust_graph)
                (witness / 'manifest.json').write_text(json.dumps({
                    'measured_edge_count': edge_count,
                    'oracles': {name: {'public_bytes_identical': True, 'search_events': 0}
                                for name in ('go', 'rust')},
                }))
                report = root / 'report.json'
                args = ['check-layout-parity.py', '--seed', '1', '--fixture', 'test.d2',
                        '--search-flavor', 'ShortestToLongest', '--report', str(report)]
                with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                     patch.object(gate, 'run', return_value=str(witness)), \
                     patch.object(gate, 'digest', return_value='test-only'), \
                     patch.object(gate.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'INSUFFICIENT EVIDENCE')), \
                     patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                    if expected == 'inapplicable':
                        gate.main()
                    else:
                        with self.assertRaises(SystemExit):
                            gate.main()
                result = json.loads(report.read_text())
                self.assertEqual(result['matched'], 0)
                self.assertEqual(result['results'][0]['status'], expected)


    def test_empty_worker_search_is_inapplicable_only_when_both_route_inventories_are_empty(self):
        cases = [
            ({'go': 0, 'rust': 0}, 'inapplicable'),
            ({'go': 1, 'rust': 1}, 'insufficient_evidence'),
            ({'go': 0, 'rust': 1}, 'divergent'),
            (None, 'insufficient_evidence'),
        ]
        for route_counts, expected in cases:
            with self.subTest(route_counts=route_counts), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                witness = root / 'witness'
                witness.mkdir()
                (witness / 'go.stdout.json').write_text('{"x":0}')
                (witness / 'rust.stdout.json').write_text('{"x":0}')
                (witness / 'manifest.json').write_text(json.dumps({
                    'measured_edge_count': 1,
                    'oracles': {name: {'public_bytes_identical': True, 'search_events': 0}
                                for name in ('go', 'rust')},
                }))
                if route_counts is not None:
                    for name in ('go', 'rust'):
                        count = route_counts[name]
                        events = [] if count == 0 else [{
                            'event': 'route-flavor', 'flavor': 'ShortestToLongest',
                            'routes': [{'edge': index} for index in range(count)],
                        }]
                        (witness / f'{name}.routing.json').write_text(json.dumps(events))
                report = root / 'report.json'
                args = ['check-layout-parity.py', '--seed', '1', '--fixture', 'test.d2',
                        '--search-flavor', 'ShortestToLongest', '--report', str(report)]
                with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                     patch.object(gate, 'run', return_value=str(witness)), \
                     patch.object(gate, 'digest', return_value='test-only'), \
                     patch.object(gate.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'INSUFFICIENT EVIDENCE')), \
                     patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                    if expected == 'inapplicable':
                        gate.main()
                    else:
                        with self.assertRaises(SystemExit):
                            gate.main()
                result = json.loads(report.read_text())
                self.assertEqual(result['results'][0]['status'], expected)
                if route_counts is not None:
                    self.assertEqual(result['results'][0]['selected_flavor_route_counts'], route_counts)


    def test_heap_mutation_trace_is_a_required_gate_when_requested(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            witness = root / 'witness'
            witness.mkdir()
            for name in ('go', 'rust'):
                (witness / f'{name}.stdout.json').write_text('{"x":0}')
                (witness / f'{name}.routing.json').write_text(json.dumps([{
                    'event': 'route-flavor', 'flavor': 'ShortestToLongest', 'routes': [{'edge': 1}],
                }]))
            (witness / 'manifest.json').write_text(json.dumps({
                'measured_edge_count': 1,
                'oracles': {name: {'public_bytes_identical': True, 'search_events': 1,
                                   'heap_events': 2} for name in ('go', 'rust')},
            }))
            report = root / 'report.json'
            args = ['check-layout-parity.py', '--seed', '1', '--fixture', 'test.d2',
                    '--search-flavor', 'ShortestToLongest', '--search-heap', '--report', str(report)]
            comparisons = [
                subprocess.CompletedProcess([], 0, 'MATCH 1 events', ''),
                subprocess.CompletedProcess([], 1, 'DIVERGENCE at $.queue_len', ''),
            ]
            with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                 patch.object(gate, 'run', return_value=str(witness)), \
                 patch.object(gate, 'digest', return_value='test-only'), \
                 patch.object(gate.subprocess, 'run', side_effect=comparisons), \
                 patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit):
                    gate.main()
            result = json.loads(report.read_text())
            row = result['results'][0]
            self.assertEqual(row['status'], 'divergent')
            self.assertEqual(result['divergent'], 1)
            self.assertEqual(row['event_count'], 1)
            self.assertEqual(row['heap_comparison_exit'], 1)
            self.assertIn('heap trace differs', row['reason'])

    def test_requested_route_cost_breakdown_requires_events_from_both_oracles(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            witness = root / 'witness'
            witness.mkdir()
            for name in ('go', 'rust'):
                (witness / f'{name}.stdout.json').write_text('{"x":0}')
                (witness / f'{name}.routing.json').write_text(json.dumps([{
                    'event': 'route-flavor', 'flavor': 'ShortestToLongest', 'routes': [{'edge': 1}],
                }]))
            (witness / 'manifest.json').write_text(json.dumps({
                'measured_edge_count': 1,
                'oracles': {name: {
                    'public_bytes_identical': True,
                    'search_events': 1,
                    'heap_events': 1,
                    'cost_breakdown_events': 0,
                } for name in ('go', 'rust')},
            }))
            report = root / 'report.json'
            args = ['check-layout-parity.py', '--seed', '1', '--fixture', 'test.d2',
                    '--search-flavor', 'ShortestToLongest', '--search-heap',
                    '--search-costs', '--report', str(report)]
            comparisons = [
                subprocess.CompletedProcess([], 0, 'MATCH 1 events', ''),
                subprocess.CompletedProcess([], 0, 'MATCH 1 events', ''),
            ]
            with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                 patch.object(gate, 'run', return_value=str(witness)), \
                 patch.object(gate, 'digest', return_value='test-only'), \
                 patch.object(gate.subprocess, 'run', side_effect=comparisons), \
                 patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit):
                    gate.main()
            result = json.loads(report.read_text())
            row = result['results'][0]
            self.assertEqual(row['status'], 'insufficient_evidence')
            self.assertEqual(row['cost_breakdown_events'], {'go': 0, 'rust': 0})
            self.assertIn('route-cost breakdown', row['reason'])


    def test_capture_failure_is_retained_in_failed_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / 'report.json'
            args = ['check-layout-parity.py', '--seed', '1', '--fixture', 'panic.d2', '--report', str(report)]
            with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                 patch.object(gate, 'run', side_effect=RuntimeError('rust panic during layout')), \
                 patch.object(gate, 'digest', return_value='test-only'), \
                 patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit):
                    gate.main()
            result = json.loads(report.read_text())
            self.assertEqual(result['matched'], 0)
            self.assertEqual(result['capture_failed'], 1)
            self.assertEqual(result['results'][0]['status'], 'capture_failed')
            self.assertIn('rust panic', result['results'][0]['reason'])

    def test_requested_gap_events_cannot_be_satisfied_by_matching_stages(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            witness = root / 'witness'
            witness.mkdir()
            for implementation in ('go', 'rust'):
                (witness / f'{implementation}.stdout.json').write_text('{"x":0}')
            (witness / 'manifest.json').write_text(json.dumps({'oracles': {
                'go': {'public_bytes_identical': True, 'gap_events': 0},
                'rust': {'public_bytes_identical': True, 'gap_events': 0},
            }}))
            report = root / 'report.json'
            args = ['check-layout-parity.py', '--gap', '--seed', '1', '--fixture', 'test.d2', '--report', str(report)]
            with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                 patch.object(gate, 'run', return_value=str(witness)), \
                 patch.object(gate, 'digest', return_value='test-only'), \
                 patch.object(gate.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'MATCH 1 events', '')), \
                 patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit):
                    gate.main()
            result = json.loads(report.read_text())
            self.assertEqual(result['matched'], 0)
            self.assertEqual(result['results'][0]['status'], 'insufficient_evidence')

    def test_requested_sized_call_trace_requires_events_from_both_oracles(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            witness = root / 'witness'
            witness.mkdir()
            for implementation in ('go', 'rust'):
                (witness / f'{implementation}.stdout.json').write_text('{"x":0}')
            (witness / 'manifest.json').write_text(json.dumps({'oracles': {
                'go': {'public_bytes_identical': True, 'sized_call_events': 0},
                'rust': {'public_bytes_identical': True, 'sized_call_events': 0},
            }}))
            report = root / 'report.json'
            args = ['check-layout-parity.py', '--sized-calls', '--seed', '1',
                    '--fixture', 'test.d2', '--report', str(report)]
            with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                 patch.object(gate, 'run', return_value=str(witness)), \
                 patch.object(gate, 'digest', return_value='test-only'), \
                 patch.object(gate.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'MATCH 1 events', '')), \
                 patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit):
                    gate.main()
            result = json.loads(report.read_text())
            row = result['results'][0]
            self.assertEqual(row['status'], 'insufficient_evidence')
            self.assertEqual(row['sized_call_events'], {'go': 0, 'rust': 0})
            self.assertIn('sized-optimizer trace', row['reason'])

    def test_requested_sized_results_require_events_from_both_oracles(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            witness = root / 'witness'
            witness.mkdir()
            for implementation in ('go', 'rust'):
                (witness / f'{implementation}.stdout.json').write_text('{"x":0}')
            (witness / 'manifest.json').write_text(json.dumps({'oracles': {
                'go': {'public_bytes_identical': True, 'sized_result_events': 0},
                'rust': {'public_bytes_identical': True, 'sized_result_events': 0},
            }}))
            report = root / 'report.json'
            args = ['check-layout-parity.py', '--sized-results', '--seed', '1',
                    '--fixture', 'test.d2', '--report', str(report)]
            with patch.object(gate, 'ROOT', root), patch.object(gate, 'verify_source'), \
                 patch.object(gate, 'run', return_value=str(witness)), \
                 patch.object(gate, 'digest', return_value='test-only'), \
                 patch.object(gate.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'MATCH 1 events', '')), \
                 patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit):
                    gate.main()
            result = json.loads(report.read_text())
            row = result['results'][0]
            self.assertEqual(row['status'], 'insufficient_evidence')
            self.assertEqual(row['sized_result_events'], {'go': 0, 'rust': 0})
            self.assertIn('sized-optimizer move trace', row['reason'])


if __name__ == '__main__':
    unittest.main()
