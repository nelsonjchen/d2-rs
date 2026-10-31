#!/usr/bin/env python3
"""Failure gates for the original refinement call/publication inventory."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("refinement", Path(__file__).with_name("tala-refinement-scoreboard.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Gates(unittest.TestCase):
    def setUp(self):
        self.calls = []
        payload = dict(graph={"nodes": [], "edges": []}, penalty=10, area=100,
                       sequence_edges={"7": {}}, bindings=[None, None, None])
        for test, count in module.TESTS.items():
            for i in range(count):
                candidate = dict(copy.deepcopy(payload), penalty=11)
                request = dict(ordinary=i == 1, incumbent=copy.deepcopy(payload), candidate=candidate,
                               build_error="", panic=False, context_before="<nil>", context_after="<nil>")
                result = dict(selected=copy.deepcopy(candidate), error="", incumbent_after=copy.deepcopy(payload))
                self.calls.append(dict(test=test, call=i, input=request, result=result))
        self.results = [copy.deepcopy(c["result"]) for c in self.calls]

    def test_full_inventory(self):
        self.assertEqual([r["outcome"] for r in module.compare(self.calls, self.results)], ["match"] * 3)

    def test_missing_and_extra_call(self):
        for calls in [self.calls[:-1], self.calls + [self.calls[-1]]]:
            with self.assertRaises(AssertionError):
                module.compare(calls, self.results)

    def test_missing_extra_or_empty_response_never_passes(self):
        for results in [[], self.results[:-1], self.results + [self.results[-1]]]:
            self.assertTrue(all(r["outcome"] == "mismatch" for r in module.compare(self.calls, results)))

    def test_full_payload_and_complete_error_compare(self):
        for key, value in [("selected", None), ("error", "unexpected trailing diagnostic"),
                           ("incumbent_after", {})]:
            with self.subTest(key=key):
                changed = copy.deepcopy(self.results)
                changed[0][key] = value
                self.assertEqual(module.compare(self.calls, changed)[0]["outcome"], "mismatch")
        changed = copy.deepcopy(self.results)
        changed[0]["selected"]["sequence_edges"] = {}
        self.assertEqual(module.compare(self.calls, changed)[0]["outcome"], "mismatch")

    def test_expected_data_in_request_rejected(self):
        self.calls[0]["input"]["expected"] = self.calls[0]["result"]
        with self.assertRaises(AssertionError):
            module.compare(self.calls, self.results)

    def test_boolean_is_not_number(self):
        self.results[0]["selected"]["graph"]["nodes"] = [True]
        self.calls[0]["result"]["selected"]["graph"]["nodes"] = [1]
        self.assertEqual(module.compare(self.calls, self.results)[0]["outcome"], "mismatch")

    def test_missing_original_subtest_rejected(self):
        self.calls[-1]["test"] = "TestUnexpected"
        with self.assertRaises(AssertionError):
            module.compare(self.calls, self.results)


if __name__ == "__main__":
    unittest.main()
