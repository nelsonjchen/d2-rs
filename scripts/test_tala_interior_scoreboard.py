#!/usr/bin/env python3
"""Failure gates for the original compound-refinement call inventory."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("interior", Path(__file__).with_name("tala-interior-scoreboard.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Gates(unittest.TestCase):
    def setUp(self):
        graph = dict(nodes=[{"point": {"x": 1, "y": 2}}] * 9,
                     edges=[{"points": [{"x": 3, "y": 4}], "curve": False}] * 6)
        request = dict(before=dict(wire={}, view=graph), selected=dict(wire={}, view=graph))
        result = dict(graph=graph, error="", before_after=graph, selected_after=graph)
        self.calls = [dict(test=test, call=i, input=copy.deepcopy(request), result=copy.deepcopy(result))
                      for test, count in module.TESTS.items() for i in range(count)]
        self.results = [copy.deepcopy(c["result"]) for c in self.calls]

    def test_complete_inventory(self):
        self.assertEqual([r["outcome"] for r in module.compare(self.calls, self.results)], ["match"] * 5)

    def test_missing_extra_or_empty_calls_rejected(self):
        for calls in [[], self.calls[:-1], self.calls + [self.calls[-1]]]:
            with self.assertRaises(AssertionError):
                module.compare(calls, self.results)

    def test_missing_extra_or_empty_response_fails(self):
        for results in [[], self.results[:-1], self.results + [self.results[-1]]]:
            self.assertTrue(all(r["outcome"] == "mismatch" for r in module.compare(self.calls, results)))

    def test_full_geometry_curve_and_input_mutation_compared(self):
        for key in ["graph", "before_after", "selected_after"]:
            for path, value in [("nodes", []), ("edges", [])]:
                changed = copy.deepcopy(self.results)
                changed[0][key][path] = value
                self.assertEqual(module.compare(self.calls, changed)[0]["outcome"], "mismatch")
        changed = copy.deepcopy(self.results)
        changed[0]["graph"]["edges"][0]["curve"] = True
        self.assertEqual(module.compare(self.calls, changed)[0]["outcome"], "mismatch")

    def test_complete_error_compared(self):
        self.results[0]["error"] = "unexpected trailing diagnostic"
        self.assertEqual(module.compare(self.calls, self.results)[0]["outcome"], "mismatch")

    def test_expected_data_in_request_rejected(self):
        self.calls[0]["input"]["expected"] = self.calls[0]["result"]
        with self.assertRaises(AssertionError):
            module.compare(self.calls, self.results)

    def test_boolean_not_number(self):
        self.results[0]["graph"]["nodes"][0]["point"]["x"] = True
        self.assertEqual(module.compare(self.calls, self.results)[0]["outcome"], "mismatch")

    def test_numeric_transport_is_lossless_and_preserves_original(self):
        original = copy.deepcopy(self.calls[0]["input"])
        converted = module.native_input(original)
        self.assertTrue(module.same_json(original, converted))
        self.assertIs(type(original["before"]["view"]["nodes"][0]["point"]["x"]), int)
        self.assertIs(type(converted["before"]["view"]["nodes"][0]["point"]["x"]), float)
        for bad in [True, 2**53 + 1]:
            changed = copy.deepcopy(original)
            changed["before"]["view"]["nodes"][0]["point"]["x"] = bad
            with self.assertRaises(AssertionError):
                module.native_input(changed)


if __name__ == "__main__":
    unittest.main()
