"""Failure gates for the original resolver capture/replay boundary."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("resolver", Path(__file__).with_name("layout-resolver-scoreboard.py"))
resolver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resolver)


class ResolverGate(unittest.TestCase):
    def setUp(self):
        self.calls = []
        for test, counts in zip(resolver.TESTS, [(3, 4, 5), (3, 4, 5), (2, 2, 3), (2, 2, 3), (3, 24, 24)]):
            for op, count in zip(["create", "resolve", "layout"], counts):
                for i in range(count):
                    call = dict(op=op, test=test, resolver=i, flags={})
                    if test == "TestConcurrentResolvedLayouts":
                        call["test"] += "/" + ["dagre", "elk", "tala"][i if op == "create" else i // 8]
                    if op == "resolve": call["error"] = ""
                    if op == "layout": call.update(input={"objects": [], "edges": None}, result={"error": "", "graph": {"objects": [], "edges": None}})
                    self.calls.append(call)
        self.results = [c.get("result", {"error": c.get("error", "")}) for c in self.calls]
        self.execution = dict(exit=0, failure=None)
        self.events = {t: "pass" for t in resolver.TESTS}

    def rows(self):
        return resolver.compare(self.calls, self.results, self.execution, self.events)

    def test_complete(self):
        self.assertTrue(all(r["outcome"] == "match" for r in self.rows()))

    def test_missing_extra_and_empty_results(self):
        for results in [[], self.results[:-1], self.results + [self.results[0]]]:
            with self.subTest(size=len(results)), self.assertRaises(AssertionError):
                resolver.compare(self.calls, results, self.execution, self.events)

    def test_changed_call_inventory(self):
        self.calls[0]["op"] = "resolve"
        with self.assertRaises(AssertionError): self.rows()

    def test_missing_original_function(self):
        self.calls[0]["test"] = "Other"
        with self.assertRaises(AssertionError): self.rows()

    def test_original_assertions_required(self):
        for action in ["fail", "skip", None]:
            self.events[resolver.TESTS[0]] = action
            with self.subTest(action=action), self.assertRaises(AssertionError): self.rows()

    def test_timeout_budget_and_failed_process(self):
        for execution in [dict(exit=1, failure=None), dict(exit=-15, failure="timeout"), dict(exit=-15, failure="budget-exceeded"), dict(exit=-15, failure="free-space-exhausted")]:
            self.execution = execution
            self.assertTrue(all(r["outcome"] == "mismatch" for r in self.rows()))

    def test_full_graph_and_error(self):
        for replacement in [{"error": "wrong", "graph": {"objects": [], "edges": None}}, {"error": "", "graph": {"objects": []}}, {"error": "", "graph": {"objects": [], "edges": []}}]:
            self.results = copy.deepcopy([c.get("result", {"error": c.get("error", "")}) for c in self.calls])
            index = next(i for i, c in enumerate(self.calls) if c["op"] == "layout")
            self.results[index] = replacement
            self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_input_only_requests(self):
        for request in resolver.requests(self.calls):
            self.assertNotIn("result", request)
            self.assertNotIn("error", request)

    def test_all_numeric_bits_and_types(self):
        self.assertTrue(resolver.equal({"x": 1}, {"x": 1.0}))
        self.assertFalse(resolver.equal({"x": 0.0}, {"x": -0.0}))
        self.assertFalse(resolver.equal({"x": 1.0}, {"x": 1.0000000000000002}))
        self.assertFalse(resolver.equal({"x": True}, {"x": 1}))


if __name__ == "__main__": unittest.main()
