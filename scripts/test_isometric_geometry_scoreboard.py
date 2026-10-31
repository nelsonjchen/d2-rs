#!/usr/bin/env python3
"""Complete-call and source-preservation gates for the geometry batch."""

import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class GeometryGates(unittest.TestCase):
    def test_inventory(self):
        score = module.scoreboard()
        self.assertEqual((len(score.TESTS), sum(score.TESTS.values())), (12, 43))
        cases = [
            {
                "name": name,
                "operations": [
                    {"request": {"op": op}}
                    for op, n in score.OPS[name].items()
                    for _ in range(n)
                ],
            }
            for name in score.TESTS
        ]
        score.validate_cases(cases)
        for invalid in [[], cases[:-1], cases + cases[:1]]:
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)
        for kind in ["missing", "extra", "wrong"]:
            invalid = copy.deepcopy(cases)
            ops = invalid[0]["operations"]
            if kind == "missing":
                ops.pop()
            elif kind == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "ignored"
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)

    def test_complete_ordered_values_and_errors(self):
        score = module.scoreboard()
        expected = {
            "paths": [
                [{"x": 1.0, "y": 0.08, "z": 2.0}, {"x": 2.0, "y": 0.3, "z": 2.0}]
            ],
            "error": "context canceled",
            "source": [{"strokeDash": 4, "points": [["NaN", 0.08, 0]]}],
        }
        self.assertEqual(score.outcome(expected, expected), "match")
        for field in expected:
            invalid = copy.deepcopy(expected)
            del invalid[field]
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        invalid = copy.deepcopy(expected)
        invalid["paths"][0].reverse()
        self.assertEqual(score.outcome(expected, invalid), "mismatch")
        invalid = copy.deepcopy(expected)
        invalid["source"][0]["strokeDash"] = 0
        self.assertEqual(score.outcome(expected, invalid), "mismatch")
        invalid = copy.deepcopy(expected)
        invalid["paths"][0][0]["x"] += 1e-12
        self.assertEqual(score.outcome(expected, invalid), "mismatch")
        self.assertEqual(score.outcome(expected, None), "rust-failed")
        self.assertEqual(
            score.outcome(expected, {"unsupported": "context"}), "unsupported"
        )

    def test_header_ownership_and_fit(self):
        score = module.scoreboard()
        expected = {
            "fits": False,
            "surface": {
                "width": 3.5,
                "depth": 0.4,
                "center": {"x": 0, "y": 0.032, "z": 1.75},
                "angle": 0,
                "align": "left",
            },
            "owner": {},
            "board": {},
            "nodes": None,
        }
        for field, value in [
            ("fits", True),
            ("owner", None),
            ("board", None),
            ("nodes", []),
        ]:
            invalid = copy.deepcopy(expected)
            invalid[field] = value
            self.assertEqual(score.outcome(expected, invalid), "mismatch")


if __name__ == "__main__":
    unittest.main()
