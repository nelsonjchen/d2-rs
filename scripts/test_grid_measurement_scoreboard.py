#!/usr/bin/env python3
"""Fail-closed measurement inventories and complete float-bit comparison."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("grid", Path(__file__).with_name("grid-measurement-scoreboard.py"))
grid = importlib.util.module_from_spec(spec)
spec.loader.exec_module(grid)


class GateTests(unittest.TestCase):
    def setUp(self):
        self.calls = []
        for test, count in grid.TESTS.items():
            for i in range(count):
                size_count = 100000 if "Singleton" in test else 2
                ranges = [[0, 0], [0, 1], [0, 2]] if "Singleton" in test else [[a, b] for a in range(2) for b in range(a, 3)]
                if "AdditionOrder" in test:
                    ranges = [[0, 0]] * 100 + ranges
                name = test + ("/" + sorted(grid.BOUNDARIES)[i] if "Bounds" in test else "")
                self.calls.append(dict(test=name, input=dict(sizes=["0000000000000000"] * size_count,
                    gap="0000000000000000", ranges=ranges),
                    result=[r + ["0000000000000000", "0000000000000000"] for r in ranges]))
        self.results = [call["result"] for call in self.calls]

    def test_complete_results_match(self):
        self.assertTrue(all(r["outcome"] == "match" for r in grid.compare(self.calls, self.results)))

    def test_missing_extra_or_wrong_boundary_sessions_fail(self):
        for calls in [self.calls[:-1], self.calls + [self.calls[0]]]:
            with self.assertRaises(AssertionError):
                grid.compare(calls, self.results)
        calls = copy.deepcopy(self.calls)
        calls[100]["test"] += "unknown"
        with self.assertRaises(AssertionError):
            grid.compare(calls, self.results)

    def test_missing_original_measurement_fails(self):
        calls = copy.deepcopy(self.calls)
        calls[0]["input"]["ranges"].pop()
        calls[0]["result"].pop()
        with self.assertRaises(AssertionError):
            grid.compare(calls, self.results)

    def test_missing_extra_native_results_fail(self):
        for results in [[], self.results[:-1], self.results + [self.results[0]]]:
            self.assertTrue(all(r["outcome"] == "mismatch" for r in grid.compare(self.calls, results)))

    def test_signed_zero_nan_payload_and_interval_are_exact(self):
        for index, value in [(0, 1), (0, False), (0, 0.0), (2, "8000000000000000"), (3, "7ff8000000000001")]:
            results = copy.deepcopy(self.results)
            results[0][0][index] = value
            self.assertEqual(grid.compare(self.calls, results)[0]["outcome"], "mismatch")

    def test_expectations_do_not_enter_native_request(self):
        calls = copy.deepcopy(self.calls)
        calls[0]["input"]["expected"] = calls[0]["result"]
        with self.assertRaises(AssertionError):
            grid.compare(calls, self.results)


if __name__ == "__main__":
    unittest.main()
