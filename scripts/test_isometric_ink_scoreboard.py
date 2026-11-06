#!/usr/bin/env python3
"""Admission inventory, exact error and compact zero-slice transport gates."""

import base64
import copy
import importlib.util
import json
import unittest

from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "ink", Path(__file__).with_name("isometric-ink-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class InkGates(unittest.TestCase):
    def test_inventory(self):
        score = module.scoreboard()
        cases = [
            {
                "name": name,
                "operations": [
                    {"request": {"op": "fit" if "Fit" in name else "segments"}}
                    for _ in range(n)
                ],
            }
            for name, n in score.TESTS.items()
        ]
        score.validate_cases(cases)
        for invalid in [[], cases + cases, [{"name": "unknown", "operations": []}]]:
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)
        for change in ["missing", "extra", "wrong"]:
            invalid = copy.deepcopy(cases)
            ops = invalid[0]["operations"]
            if change == "missing":
                ops.pop()
            elif change == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "ignored"
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)

    def test_complete_error_and_result(self):
        score = module.scoreboard()
        expected = {"count": 0, "error": "context canceled"}
        self.assertEqual(score.outcome(expected, expected), "match")
        for invalid in [
            {"count": 0, "error": ""},
            {"count": 9, "error": ""},
            {"count": 0},
            {"count": 0, "error": "context deadline exceeded"},
        ]:
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        self.assertEqual(score.outcome(expected, None), "rust-failed")

    def test_exact_original_admission_witnesses(self):
        p = cached_fixture("crates/d2-rs-isometric/tests/fixtures/ink-api.json")
        cases = json.loads(p.read_text())["cases"]
        module.scoreboard().validate_cases(cases)
        cancelled, oversized = cases[0]["operations"]
        self.assertTrue(cancelled["request"]["canceled"])
        self.assertEqual(len(cancelled["request"]["triangles"]), 12)
        self.assertEqual(cancelled["got"], {"count": 0, "error": "context canceled"})
        self.assertFalse(oversized["request"]["canceled"])
        self.assertEqual(oversized["request"]["zeroCount"], 100001)
        self.assertEqual(oversized["request"]["triangles"], [])
        self.assertEqual(
            oversized["got"]["error"],
            "isometric outline exceeds 100000 input triangles",
        )
        fit = cases[1]["operations"][0]
        self.assertTrue(fit["request"]["canceled"])
        texture = fit["request"]["texture"]
        self.assertEqual(
            texture["rect"], {"Min": {"X": 0, "Y": 0}, "Max": {"X": 32, "Y": 32}}
        )
        self.assertEqual(texture["stride"], 128)
        self.assertEqual(base64.b64decode(texture["pixels"]), bytes(4096))
        self.assertEqual(fit["got"], {"fit": 1, "error": "context canceled"})


if __name__ == "__main__":
    unittest.main()
