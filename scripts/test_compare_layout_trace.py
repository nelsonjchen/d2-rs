#!/usr/bin/env python3
"""Evidence requirements for the strict TALA trace comparator."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


class TraceEvidenceTests(unittest.TestCase):
    def compare(self, left, right, *flags):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ('go.jsonl', 'rust.jsonl')]
            for path, data in zip(paths, (left, right)):
                path.write_text(data)
            return subprocess.run(
                [sys.executable, str(Path(__file__).with_name('compare-layout-trace.py')),
                 *map(str, paths), *flags], capture_output=True, text=True,
            )

    def test_empty_or_non_event_logs_do_not_prove_parity(self):
        for log in ('', 'diagnostic text\n', '{"diagnostic": true}\n'):
            with self.subTest(log=log):
                result = self.compare(log, log)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('INSUFFICIENT EVIDENCE', result.stderr)

    def test_absent_requested_family_does_not_prove_parity(self):
        result = self.compare('{"event":"stage"}\n', '{"event":"stage"}\n', '--event', 'rng')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('INSUFFICIENT EVIDENCE', result.stderr)

    def test_additive_stage_seed_is_ignored(self):
        result = self.compare('{"event":"stage","stage":"Normalize"}\n',
                              '{"event":"stage","stage":"Normalize","seed":1}\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('MATCH 1 events', result.stdout)

    def test_snapshot_node_order_is_significant(self):
        left = '{"event":"placement","nodes":[{"id":"a","x_bits":"1"},{"id":"b","x_bits":"2"}]}\n'
        result = self.compare(left, left)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('MATCH 1 events', result.stdout)

        reordered = ('{"event":"placement","nodes":[{"id":"b","x_bits":"2"},'
                     '{"id":"a","x_bits":"1"}]}\n')
        result = self.compare(left, reordered)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('DIVERGENCE', result.stdout)

    def test_snapshot_edges_use_source_ordinal_and_endpoints(self):
        left = ('{"event":"stage","edges":[{"id":"3069564216","index":0,'
                '"from":"a","to":"b","x_bits":"1"}]}\n')
        right = ('{"event":"stage","edges":[{"id":"0","index":0,'
                 '"from":"a","to":"b","x_bits":"1"}]}\n')
        result = self.compare(left, right)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('MATCH 1 events', result.stdout)

        changed_endpoint = right.replace('"to":"b"', '"to":"c"')
        result = self.compare(left, changed_endpoint)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('DIVERGENCE', result.stdout)

        ordered = ('{"event":"stage","edges":[{"id":"go-a","index":0,'
                   '"from":"a","to":"b"},{"id":"go-b","index":1,'
                   '"from":"b","to":"c"}]}\n')
        reordered = ('{"event":"stage","edges":[{"id":"rust-b","index":1,'
                     '"from":"b","to":"c"},{"id":"rust-a","index":0,'
                     '"from":"a","to":"b"}]}\n')
        result = self.compare(ordered, reordered)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('DIVERGENCE', result.stdout)

    def test_scoped_source_ids_require_opt_in_and_preserve_geometry(self):
        left = ('{"event":"placement","root":"r","nodes":[{"id":"r.1",'
                '"x_bits":"1"}],"edges":[{"id":"local-go","index":0,'
                '"from":"r","to":"r.1"}]}\n')
        right = ('{"event":"placement","root":"board.r","nodes":[{"id":"board.r.1",'
                 '"x_bits":"1"}],"edges":[{"id":"0","index":0,'
                 '"from":"board.r","to":"board.r.1"}]}\n')
        strict = self.compare(left, right)
        self.assertNotEqual(strict.returncode, 0)
        self.assertIn('DIVERGENCE', strict.stdout)

        localized = self.compare(left, right, '--normalize-scoped-ids')
        self.assertEqual(localized.returncode, 0, localized.stderr)
        self.assertIn('MATCH 1 events', localized.stdout)

        changed_geometry = right.replace('"x_bits":"1"', '"x_bits":"2"')
        localized = self.compare(left, changed_geometry, '--normalize-scoped-ids')
        self.assertNotEqual(localized.returncode, 0)
        self.assertIn('DIVERGENCE', localized.stdout)

    def test_scoped_id_normalization_refuses_ambiguous_suffixes(self):
        left = ('{"event":"stage","nodes":[{"id":"a","x_bits":"1"},'
                '{"id":"nested.a","x_bits":"2"}]}\n')
        right = ('{"event":"stage","nodes":[{"id":"outer.a","x_bits":"1"},'
                 '{"id":"outer.nested.a","x_bits":"2"}]}\n')
        result = self.compare(left, right, '--normalize-scoped-ids')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('DIVERGENCE', result.stdout)

    def test_owner_snapshot_context_must_match_exactly(self):
        left = ('{"event":"placement","phase":"orthogonal-start",'
                '"root":"r","cell_bits":"0", "nodes":['
                '{"id":"r","x_bits":"1"},'
                '{"id":"r.a","x_bits":"2"},'
                '{"id":"unrelated","x_bits":"99"}]}\n'
                '{"event":"placement","phase":"initialized",'
                '"root":"r","nodes":[{"id":"r","x_bits":"1"},'
                '{"id":"r.a","x_bits":"2"}]}\n')
        right = ('{"event":"placement","phase":"orthogonal-start",'
                 '"root":"board.r","cell_bits":"0", "nodes":['
                 '{"id":"board.r","x_bits":"1"},'
                 '{"id":"board.r.a","x_bits":"2"},'
                 '{"id":"board.other","x_bits":"88"}]}\n'
                 '{"event":"placement","phase":"initialized",'
                 '"root":"board.r","nodes":[{"id":"board.r",'
                 '"x_bits":"1"},{"id":"board.r.a","x_bits":"2"}]}\n')
        strict = self.compare(left, right)
        self.assertNotEqual(strict.returncode, 0)

        context_mismatch = self.compare(
            left, right, '--project-owner-snapshots', '--normalize-scoped-ids'
        )
        self.assertNotEqual(context_mismatch.returncode, 0)
        self.assertIn('CONTEXT DIVERGENCE', context_mismatch.stdout)
        self.assertIn('Go-only=1, Rust-only=1', context_mismatch.stdout)
        self.assertIn("Go-only IDs=['unrelated']", context_mismatch.stdout)
        self.assertIn("Rust-only IDs=['board.other']", context_mismatch.stdout)

        matching_left = left.replace(
            '{"id":"unrelated","x_bits":"99"}',
            '{"id":"r.context","x_bits":"88"}',
        )
        matching_right = right.replace('board.other', 'board.r.context')
        projected = self.compare(
            matching_left, matching_right,
            '--project-owner-snapshots', '--normalize-scoped-ids'
        )
        self.assertEqual(projected.returncode, 0, projected.stderr)
        self.assertIn('MATCH 2 events', projected.stdout)
        self.assertIn('verified owner context across 1 orthogonal snapshots', projected.stdout)

        changed_geometry = matching_right.replace('"x_bits":"2"', '"x_bits":"3"')
        projected = self.compare(
            matching_left, changed_geometry,
            '--project-owner-snapshots', '--normalize-scoped-ids'
        )
        self.assertNotEqual(projected.returncode, 0)
        self.assertIn('DIVERGENCE', projected.stdout)

    def test_owner_snapshot_projection_refuses_unknown_or_ambiguous_nodes(self):
        left = ('{"event":"placement","phase":"orthogonal-start",'
                '"root":"r","nodes":[{"id":"r.a","x_bits":"1"},'
                '{"id":"outer.r.a","x_bits":"1"}]}\n'
                '{"event":"placement","phase":"initialized",'
                '"root":"r","nodes":[{"id":"r.a","x_bits":"1"}]}\n')
        right = ('{"event":"placement","phase":"orthogonal-start",'
                 '"root":"outer.r","nodes":[{"id":"outer.r.a",'
                 '"x_bits":"1"}]}\n'
                 '{"event":"placement","phase":"initialized",'
                 '"root":"outer.r","nodes":[{"id":"outer.r.a",'
                 '"x_bits":"1"}]}\n')
        result = self.compare(
            left, right, '--project-owner-snapshots', '--normalize-scoped-ids'
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('DIVERGENCE', result.stdout)

        missing = ('{"event":"placement","phase":"orthogonal-start",'
                   '"root":"outer.r","nodes":[{"id":"outer.r.a",'
                   '"x_bits":"1"},{"id":"outer.r.missing",'
                   '"x_bits":"1"}]}\n'
                   '{"event":"placement","phase":"initialized",'
                   '"root":"outer.r","nodes":[{"id":"outer.r.a",'
                   '"x_bits":"1"},{"id":"outer.r.missing",'
                   '"x_bits":"1"}]}\n')
        result = self.compare(
            left, missing, '--project-owner-snapshots', '--normalize-scoped-ids'
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('DIVERGENCE', result.stdout)

    def test_rng_scope_localizes_names_but_preserves_node_order(self):
        left = ('{"event":"rng-scope","stream":"node-placement",'
                '"root":"r","nodes":["r.1","r.2"]}\n')
        right = ('{"event":"rng-scope","stream":"node-placement",'
                 '"root":"board.r","nodes":["board.r.1","board.r.2"]}\n')
        result = self.compare(left, right, '--normalize-scoped-ids')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('MATCH 1 events', result.stdout)

        reordered = right.replace('["board.r.1","board.r.2"]',
                                  '["board.r.2","board.r.1"]')
        result = self.compare(left, reordered, '--normalize-scoped-ids')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('$[0].nodes[0]', result.stdout)

    def test_rng_scope_localizes_container_map_ids_but_preserves_map_order(self):
        left = ('{"event":"rng-scope","root":"r","nodes":["r.1"],'
                '"containers":[{"id":"r","children":2},'
                '{"id":"r.inner","children":1}]}\n')
        right = ('{"event":"rng-scope","root":"board.r","nodes":["board.r.1"],'
                 '"containers":[{"id":"board.r","children":2},'
                 '{"id":"board.r.inner","children":1}]}\n')
        result = self.compare(left, right, '--normalize-scoped-ids')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('MATCH 1 events', result.stdout)

        reordered = right.replace(
            '[{"id":"board.r","children":2},{"id":"board.r.inner","children":1}]',
            '[{"id":"board.r.inner","children":1},{"id":"board.r","children":2}]',
        )
        result = self.compare(left, reordered, '--normalize-scoped-ids')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('$[0].containers[0]', result.stdout)

    def test_candidate_order_is_still_significant(self):
        left = '{"event":"placement-candidates","points":[1,2]}\n'
        right = '{"event":"placement-candidates","points":[2,1]}\n'
        result = self.compare(left, right)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('$[0].points[0]', result.stdout)

        scoped = self.compare(left, right, '--normalize-scoped-ids')
        self.assertNotEqual(scoped.returncode, 0)
        self.assertIn('$[0].points[0]', scoped.stdout)

    def test_rng_draw_difference_is_retained(self):
        result = self.compare('{"event":"rng","draw":1}\n', '{"event":"rng","draw":2}\n')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('$[0].draw', result.stdout)

    def test_explicitly_ignored_constructor_events_do_not_hide_draws(self):
        left = '{"event":"rng-init","stream":"helper"}\n{"event":"rng","draw":1}\n'
        right = '{"event":"rng-init","stream":"helper"}\n{"event":"rng-init","stream":"unused"}\n{"event":"rng","draw":1}\n'
        result = self.compare(left, right, '--ignore-event', 'rng-init')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('MATCH 1 events', result.stdout)

        divergent = self.compare(left, right.replace('"draw":1', '"draw":2'),
                                 '--ignore-event', 'rng-init')
        self.assertNotEqual(divergent.returncode, 0)
        self.assertIn('$[0].draw', divergent.stdout)

    def test_related_families_keep_their_interleaving(self):
        pop = '{"event":"route-search-pop"}\n'
        neighbor = '{"event":"route-search-neighbor"}\n'
        result = self.compare(pop + neighbor, neighbor + pop, '--event-prefix', 'route-search-')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('$[0].event', result.stdout)

    def test_absent_event_prefix_is_insufficient(self):
        result = self.compare('{"event":"stage"}\n', '{"event":"stage"}\n',
                              '--event-prefix', 'route-search-')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('INSUFFICIENT EVIDENCE', result.stderr)


if __name__ == '__main__':
    unittest.main()
