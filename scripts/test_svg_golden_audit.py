#!/usr/bin/env python3
"""Missing, corrupt and ambiguous source artifacts must never qualify a golden."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('svg_golden_audit', Path(__file__).with_name('svg-golden-audit.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class AuditGate(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.go, self.rust, self.golden = [self.root / x for x in ['go.svg', 'rust.svg', 'golden.svg']]
        for p in [self.go, self.rust, self.golden]:
            p.write_text('<svg/>')

    def test_exact_golden(self):
        self.assertEqual(audit.classify(self.go, self.rust, self.golden)['outcome'], 'golden-match')

    def test_stale_is_not_golden_match(self):
        self.golden.write_text('<svg><g/></svg>')
        self.assertEqual(audit.classify(self.go, self.rust, self.golden)['outcome'], 'stale-in-go')

    def test_missing_output(self):
        self.go.unlink()
        with self.assertRaises(ValueError):
            audit.classify(self.go, self.rust, self.golden)

    def test_both_output_and_error(self):
        Path(str(self.go) + '.err').write_text('decode error')
        with self.assertRaises(ValueError):
            audit.artifact(self.go)

    def test_empty_and_corrupt(self):
        for payload in ['', '<svg>']:
            self.go.write_text(payload)
            with self.assertRaises((ValueError, audit.ET.ParseError)):
                audit.artifact(self.go)

    def test_go_error_cannot_be_a_golden_match(self):
        self.go.unlink()
        Path(str(self.go) + '.err').write_text('decode error')
        self.assertEqual(audit.classify(self.go, self.rust, self.golden)['outcome'], 'go-decode-or-render-error')


if __name__ == '__main__':
    unittest.main()
