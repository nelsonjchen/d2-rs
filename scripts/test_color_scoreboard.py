#!/usr/bin/env python3
"""The capture gate rejects incomplete, failed or changed original-test evidence."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('color_scoreboard', Path(__file__).with_name('color-scoreboard.py'))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class CaptureGate(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.capture = Path(self.temp.name)
        (self.capture / 'go').mkdir()
        for i, name in enumerate(f'{t}#{i}' for t, n in scoreboard.EXPECTED.items() for i in range(n)):
            (self.capture / 'go' / f'{i}.json').write_text(json.dumps({'name': name}))
        for name, value in [('source.json', scoreboard.source()), ('inventory.json', scoreboard.EXPECTED),
                            ('run.json', {'exit': 0, 'failure': None})]:
            (self.capture / name).write_text(json.dumps(value))
        (self.capture / 'go.jsonl').write_text('\n'.join(json.dumps({'Test': t, 'Action': 'pass'}) for t in scoreboard.TESTS))
        self.seal()

    def seal(self):
        (self.capture / 'hashes.json').write_text(json.dumps(scoreboard.hashes(self.capture)))

    def test_complete_capture(self):
        self.assertEqual(len(scoreboard.verify(self.capture)), 16)

    def test_missing_even_with_updated_hashes(self):
        next((self.capture / 'go').glob('*.json')).unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_failed_upstream(self):
        (self.capture / 'run.json').write_text(json.dumps({'exit': 1, 'failure': None}))
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_changed_bytes(self):
        (self.capture / 'go.jsonl').write_text('')
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_missing_original_assertions(self):
        (self.capture / 'go.jsonl').write_text('')
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)


if __name__ == '__main__':
    unittest.main()
