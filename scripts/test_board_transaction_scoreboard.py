#!/usr/bin/env python3
"""The transaction gate must reject incomplete and changed observations."""

import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "transaction", Path(__file__).with_name("board-transaction-scoreboard.py")
)
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)


class Gate(unittest.TestCase):
    def setUp(self):
        self.rows = {
            name: {
                "name": name,
                "touched": name == score.TESTS[2],
                "error": "failure" if name != score.TESTS[2] else "",
                "callbacks": ["out/current.svg"],
                "after": {
                    "out": {"type": "directory", "mode": 0o755},
                    "out/current.svg": {"type": "file", "mode": 0o640, "bytes": "b2xk"},
                },
            }
            for name in score.TESTS
        }

    def test_complete_observations_match(self):
        self.assertTrue(
            all(
                r["outcome"] == "match"
                for r in score.compare(self.rows, copy.deepcopy(self.rows))
            )
        )

    def test_missing_native_result_fails(self):
        actual = copy.deepcopy(self.rows)
        del actual[score.TESTS[0]]
        self.assertEqual(score.compare(self.rows, actual)[0]["outcome"], "mismatch")

    def test_incomplete_go_inventory_is_rejected(self):
        del self.rows[score.TESTS[0]]
        with self.assertRaises(AssertionError):
            score.compare(self.rows, self.rows)

    def test_every_contract_field_is_compared(self):
        for field, value in [
            ("error", "different"),
            ("touched", True),
            ("callbacks", ["out/wrong.svg"]),
            ("after", {}),
        ]:
            with self.subTest(field=field):
                actual = copy.deepcopy(self.rows)
                actual[score.TESTS[0]][field] = value
                self.assertEqual(
                    score.compare(self.rows, actual)[0]["outcome"], "mismatch"
                )
        for field, value in [("mode", 0o644), ("bytes", "bmV3")]:
            actual = copy.deepcopy(self.rows)
            actual[score.TESTS[0]]["after"]["out/current.svg"][field] = value
            self.assertEqual(score.compare(self.rows, actual)[0]["outcome"], "mismatch")

    def test_extra_native_result_is_rejected(self):
        actual = copy.deepcopy(self.rows)
        actual["unexpected"] = {}
        with self.assertRaises(AssertionError):
            score.compare(self.rows, actual)

    def test_unretired_staging_is_rejected(self):
        self.rows[score.TESTS[0]]["after"][".d2-board-output-leftover"] = {
            "type": "directory"
        }
        with self.assertRaises(AssertionError):
            score.compare(self.rows, self.rows)


if __name__ == "__main__":
    unittest.main()
