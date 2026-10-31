#!/usr/bin/env python3
"""Inventory and complete-observation gates for original route-lane tests."""

import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "lanes", Path(__file__).with_name("isometric-lanes-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class LaneGates(unittest.TestCase):
    def test_inventory_rejects_missing_extra_and_wrong_operations(self):
        score = module.scoreboard()
        self.assertEqual((len(score.TESTS), sum(score.TESTS.values())), (8, 28))
        cases = [
            {
                "name": name,
                "operations": [
                    {"request": {"op": op}}
                    for op, count in score.OPS[name].items()
                    for _ in range(count)
                ],
            }
            for name in score.TESTS
        ]
        score.validate_cases(cases)
        for invalid in [[], cases[:-1], cases + cases[:1]]:
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)
        for mode in ["missing", "extra", "wrong"]:
            invalid = copy.deepcopy(cases)
            ops = invalid[0]["operations"]
            if mode == "missing":
                ops.pop()
            elif mode == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "ignored"
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)

    def test_complete_paths_sources_errors_and_nil_slices(self):
        score = module.scoreboard()
        expected = {
            "paths": [[{"x": 0, "y": 0.08, "z": 0}, {"x": 2, "y": 0.08, "z": 0.1142}]],
            "error": "",
            "source": [{"points": [["NaN", 0.08, 0]]}],
            "nodes": None,
            "boards": [{"nodeIds": None}],
        }
        self.assertEqual(score.outcome(expected, expected), "match")
        for field in expected:
            invalid = copy.deepcopy(expected)
            del invalid[field]
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        for field, value in [("nodes", []), ("error", "context canceled")]:
            invalid = copy.deepcopy(expected)
            invalid[field] = value
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        invalid = copy.deepcopy(expected)
        invalid["paths"][0].reverse()
        self.assertEqual(score.outcome(expected, invalid), "mismatch")
        invalid = copy.deepcopy(expected)
        invalid["paths"][0][1]["z"] += 1e-12
        self.assertEqual(score.outcome(expected, invalid), "mismatch")
        self.assertEqual(score.outcome(expected, None), "rust-failed")


if __name__ == "__main__":
    unittest.main()
