#!/usr/bin/env python3
"""Full ownership-admission responses must match; absent output cannot pass."""

import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "manifest", Path(__file__).with_name("board-manifest-scoreboard.py")
)
manifest = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manifest)


class Gates(unittest.TestCase):
    def setUp(self):
        self.capture = {
            "originalCount": 3,
            "calls": [{"error": f'error "é-{i}"'} for i in range(16)],
        }
        self.actual = [{"error": c["error"]} for c in self.capture["calls"]]

    def test_full_errors(self):
        self.assertTrue(
            all(
                r["outcome"] == "match"
                for r in manifest.compare(self.capture, self.actual)
            )
        )
        bad = copy.deepcopy(self.actual)
        bad[0]["error"] = 'error "e\u0301-0"'
        self.assertEqual(manifest.compare(self.capture, bad)[0]["outcome"], "mismatch")

    def test_missing_or_extra_response(self):
        for bad in [self.actual[:-1], self.actual + self.actual[:1]]:
            with self.assertRaises(AssertionError):
                manifest.compare(self.capture, bad)

    def test_missing_original_calls(self):
        bad = copy.deepcopy(self.capture)
        bad["originalCount"] = 2
        with self.assertRaises(AssertionError):
            manifest.compare(bad, self.actual)

    def test_extra_or_absent_response_fields(self):
        for result in [{}, {"error": self.actual[0]["error"], "extra": 1}]:
            bad = copy.deepcopy(self.actual)
            bad[0] = result
            self.assertEqual(
                manifest.compare(self.capture, bad)[0]["outcome"], "mismatch"
            )


if __name__ == "__main__":
    unittest.main()
