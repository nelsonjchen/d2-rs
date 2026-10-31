#!/usr/bin/env python3
"""The empty-board audit must bind all three original target inputs."""
import importlib.util
import json
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('wasm_empty', Path(__file__).with_name('wasm-empty-scoreboard.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class EmptyBoardGate(unittest.TestCase):
    def setUp(self):
        self.inputs = [{'Name': e, 'Diagram': {'name': e}} for e in ['dagre', 'elk', 'tala']]
        self.cases = [{'board': i['Name'], 'request': json.dumps({'diagram': i['Diagram'], 'options': {'pad': 0, 'themeID': 0, 'sketch': False}})} for i in self.inputs]

    def test_complete_distinct_inventory(self):
        audit.validate_cases(self.cases, self.inputs)

    def test_missing_duplicate_or_changed_inputs_fail(self):
        for cases in [self.cases[:2], [self.cases[0]] * 3, [dict(self.cases[0], request='{}'), *self.cases[1:]]]:
            with self.subTest(cases=cases), self.assertRaises((AssertionError, KeyError)):
                audit.validate_cases(cases, self.inputs)

    def test_stale_output_cannot_count_as_golden(self):
        self.assertEqual(audit.golden_outcome(b'<svg class="hash"/>', '<svg/>'), 'stale-in-go-wasm')
        self.assertEqual(audit.golden_outcome(b'<svg data-d2-version="v0.9.0"/>', '<svg data-d2-version="v0.8.1-HEAD"/>\n'), 'go-reproduces-golden')


if __name__ == '__main__':
    unittest.main()
