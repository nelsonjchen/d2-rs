#!/usr/bin/env python3
"""Failure gates for original SVG index and precision observations."""

import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "precision", Path(__file__).with_name("isometric-precision-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PrecisionGates(unittest.TestCase):
    def test_missing_extra_and_wrong_operation_fail(self):
        score = module.scoreboard()
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

    def test_live_state_float_bits_and_source_bytes_cannot_be_lost(self):
        score = module.scoreboard()
        expected = [
            {
                "error": "isometric SVG visibility exceeds fragment spatial index limit",
                "references": 2,
                "cells": [[1], [2]],
                "work": 18,
                "fragments": [{"id": 1, "points": [[0, 1, 2]]}],
            },
            "0000000000000000",
            {"path": "M0 0L2 0L0 2Z", "source": [[0, 1, 2]]},
        ]
        self.assertEqual(score.outcome(expected, expected), "match")
        for change in [
            "error",
            "references",
            "cells",
            "work",
            "fragments",
            "bits",
            "path",
            "source",
        ]:
            invalid = copy.deepcopy(expected)
            if change in {"error", "references", "cells", "work", "fragments"}:
                del invalid[0][change]
            elif change == "bits":
                invalid[1] = "8000000000000000"
            elif change == "path":
                invalid[2]["path"] += " "
            else:
                invalid[2]["source"][0][0] += 1e-12
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        for invalid in [None, [], expected[:-1], expected + expected]:
            self.assertNotEqual(score.outcome(expected, invalid), "match")


if __name__ == "__main__":
    unittest.main()
