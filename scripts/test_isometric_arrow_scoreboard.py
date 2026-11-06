#!/usr/bin/env python3
"""Complete inventories, query geometry and cancellation-result gates."""

import copy
import importlib.util
import json
import unittest

from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "arrow", Path(__file__).with_name("isometric-arrow-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ArrowGates(unittest.TestCase):
    def test_inventory(self):
        score = module.scoreboard()
        self.assertEqual((len(score.TESTS), sum(score.TESTS.values())), (8, 51))
        cases = [
            {
                "name": n,
                "operations": [
                    {"request": {"op": op}}
                    for op, k in score.OPS[n].items()
                    for _ in range(k)
                ],
            }
            for n in score.TESTS
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

    def test_complete_result(self):
        score = module.scoreboard()
        expected = {
            "route": [{"x": -1, "y": 0.08, "z": 0}, {"x": -3, "y": 0.08, "z": 0}],
            "state": {
                "error": "context canceled",
                "work": 629,
                "cachePoints": 15,
                "markers": 2,
                "owners": {
                    "component": {"prepared": True, "caps": 2, "obstacles": [[0.1, 3]]}
                },
            },
            "source": [{"x": -1, "y": 0.08, "z": 0}],
        }
        self.assertEqual(score.outcome(expected, expected), "match")
        for field in expected:
            invalid = copy.deepcopy(expected)
            del invalid[field]
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        for field, value in [
            ("error", ""),
            ("work", 630),
            ("cachePoints", 16),
            ("owners", {}),
        ]:
            invalid = copy.deepcopy(expected)
            invalid["state"][field] = value
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        invalid = copy.deepcopy(expected)
        invalid["route"].reverse()
        self.assertEqual(score.outcome(expected, invalid), "mismatch")
        self.assertEqual(score.outcome(expected, None), "rust-failed")

    def test_unexported_go_points_cannot_disappear(self):
        p = cached_fixture("crates/d2-rs-isometric/tests/fixtures/arrow-api.json")
        cases = json.loads(p.read_text())["cases"]
        module.validate_query_geometry(cases)
        owner = next(
            iter(cases[0]["operations"][1]["request"]["builder"]["owners"].values())
        )
        owner["obstacles"][0]["polygons"][0]["points"][0] = {}
        with self.assertRaises(AssertionError):
            module.validate_query_geometry(cases)

    def test_fixture_contains_exact_cancellation_witness(self):
        p = cached_fixture("crates/d2-rs-isometric/tests/fixtures/arrow-api.json")
        case = json.loads(p.read_text())["cases"][0]
        self.assertEqual(len(case["operations"]), 4)
        op = case["operations"][2]
        self.assertTrue(op["request"]["builder"]["canceled"])
        self.assertTrue(op["request"]["builder"]["markers"])
        self.assertEqual(op["got"]["state"]["error"], "context canceled")
        self.assertEqual(op["got"]["state"]["work"], op["request"]["builder"]["work"])


if __name__ == "__main__":
    unittest.main()
