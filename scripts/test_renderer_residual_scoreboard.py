#!/usr/bin/env python3
"""Original inventory, assertion and native-result failure gates."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('residual', Path(__file__).with_name('renderer-residual-scoreboard.py'))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class Gates(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / 'source.json').write_text(json.dumps(scoreboard.source()))
        for key, suite in scoreboard.SUITES.items():
            root = self.root / key
            (root / 'calls').mkdir(parents=True)
            (root / 'run.json').write_text(json.dumps({'exit': 0, 'failure': None}))
            (root / 'events.jsonl').write_text('\n'.join(json.dumps({'Test': test, 'Action': 'pass'}) for test in suite['tests']))
            for test, count in suite['tests'].items():
                for i in range(count):
                    row = {'test': test, 'name': f'{test}#{i}', 'package': suite['package'],
                           'request': {'operation': 'fixture'}, 'got': {'output': None, 'error': ''}}
                    (root / 'calls' / f'{test}-{i}.json').write_text(json.dumps(row))
        self.seal()

    def seal(self):
        (self.root / 'hashes.json').write_text(json.dumps(scoreboard.base.hashes(self.root)))

    def test_complete(self):
        self.assertEqual(len(scoreboard.verify(self.root)), 63)

    def test_missing_and_extra_calls(self):
        path = next((self.root / 'features/calls').glob('*.json'))
        data = path.read_text()
        path.unlink()
        self.seal()
        with self.assertRaises(AssertionError): scoreboard.verify(self.root)
        path.write_text(data)
        path.with_name('extra.json').write_text(data)
        self.seal()
        with self.assertRaises(AssertionError): scoreboard.verify(self.root)

    def test_failed_original_assertion(self):
        # Even a Go-only infrastructure helper must really pass.
        path = self.root / 'png_helpers/events.jsonl'
        path.write_text(path.read_text().replace('"pass"', '"fail"'))
        self.seal()
        with self.assertRaises(AssertionError): scoreboard.verify(self.root)

    def test_missing_original_event(self):
        (self.root / 'e2e/events.jsonl').write_text('')
        self.seal()
        with self.assertRaises(AssertionError): scoreboard.verify(self.root)

    def test_corrupt_or_changed_capture(self):
        path = next((self.root / 'icon/calls').glob('*.json'))
        path.write_text('{}')
        with self.assertRaises(AssertionError): scoreboard.verify(self.root)

    def test_missing_native_inventory(self):
        cases = scoreboard.verify(self.root)
        for actual in [None, {}, [], [c['got'] for c in cases][1:]]:
            self.assertTrue(all(r['outcome'] == 'invalid-inventory' for r in scoreboard.classify(cases, actual, {'exit': 0, 'failure': None})))

    def test_timeout_or_budget_cannot_pass(self):
        cases = scoreboard.verify(self.root)
        for failure in ['timeout', 'budget-exceeded']:
            rows = scoreboard.classify(cases, [c['got'] for c in cases], {'exit': -15, 'failure': failure})
            self.assertEqual({r['outcome'] for r in rows}, {failure})

    def test_complete_error_and_bytes(self):
        cases = scoreboard.verify(self.root)
        actual = [dict(c['got']) for c in cases]
        self.assertEqual({r['outcome'] for r in scoreboard.classify(cases, actual, {'exit': 0, 'failure': None})}, {'match'})
        for key, value in [('error', 'different error'), ('output', 'changed bytes')]:
            actual[0] = {**cases[0]['got'], key: value}
            rows = scoreboard.classify(cases, actual, {'exit': 0, 'failure': None})
            self.assertEqual(rows[0]['outcome'], 'mismatch')


if __name__ == '__main__':
    unittest.main()
