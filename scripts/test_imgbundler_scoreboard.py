"""Integrity gates for the bounded imgbundler capture corpus."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('imgbundler', Path(__file__).with_name('imgbundler-scoreboard.py'))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class CaptureIntegrity(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.capture = Path(self.tmp.name)
        (self.capture / 'go').mkdir()
        (self.capture / 'source.json').write_text(json.dumps(scoreboard.source()))
        (self.capture / 'inventory.json').write_text(json.dumps(sorted(scoreboard.TESTS)))
        (self.capture / 'run.json').write_text(json.dumps({'exit': 0, 'failure': None}))
        self.events = {t: 'pass' for t in scoreboard.TESTS}
        (self.capture / 'go.jsonl').write_text('\n'.join(json.dumps({'Test':t,'Action':a}) for t,a in self.events.items()))
        for t, count in scoreboard.EXPECTED_CALLS.items():
            for i in range(count):
                (self.capture / 'go' / f'{t}-{i}.json').write_text(json.dumps({'name': f'{t}#{i}'}))
        self.seal()

    def seal(self):
        (self.capture / 'hashes.json').write_text(json.dumps(scoreboard.hashes(self.capture)))

    def test_complete_corpus_passes(self):
        self.assertEqual(len(scoreboard.verify(self.capture)), 12)

    def test_missing_call_is_failure_even_with_resealed_hashes(self):
        next((self.capture / 'go').glob('*.json')).unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_changed_capture_fails_hash_check(self):
        next((self.capture / 'go').glob('*.json')).write_text('{}')
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_timeout_and_failed_go_assertion_cannot_pass(self):
        for result in [{'exit': 0, 'failure': 'timeout'}, {'exit': 1, 'failure': None}]:
            (self.capture / 'run.json').write_text(json.dumps(result))
            self.seal()
            with self.assertRaises(AssertionError):
                scoreboard.verify(self.capture)
        (self.capture / 'run.json').write_text(json.dumps({'exit': 0, 'failure': None}))
        self.events[next(iter(self.events))] = 'fail'
        (self.capture / 'go.jsonl').write_text('\n'.join(json.dumps({'Test':t,'Action':a}) for t,a in self.events.items()))
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_empty_capture_is_failure(self):
        for p in (self.capture / 'go').glob('*.json'):
            p.unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)


if __name__ == '__main__':
    unittest.main()
