import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "lifecycle", Path(__file__).with_name("imageasset-lifecycle-scoreboard.py")
)
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)


class LifecycleGateTests(unittest.TestCase):
    def cases(self):
        return [
            {
                "name": name,
                "operations": [
                    {"request": {"op": op}, "got": {}}
                    for op in (
                        ["data-uri"] * 5
                        if name == "TestResourceDataURIContext"
                        else ["new"] * 4 + ["close"] * 6
                    )
                ],
            }
            for name in score.TESTS
        ]

    def test_requires_both_functions_and_complete_operation_inventory(self):
        cases = self.cases()
        score.validate_cases(cases)
        for changed in [cases[:-1], cases + cases[:1], []]:
            with self.assertRaises(AssertionError):
                score.validate_cases(changed)
        for kind in ["missing", "extra", "wrong-op"]:
            changed = copy.deepcopy(cases)
            ops = changed[0]["operations"]
            if kind == "missing":
                ops.pop()
            elif kind == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "other"
            with self.assertRaises(AssertionError):
                score.validate_cases(changed)

    def test_bytes_complete_errors_and_ownership_types_are_exact(self):
        expected = [
            {"data": "YWJj", "error": ""},
            {"owned": True, "isolated": False},
            {"error": "context canceled", "data": None},
        ]
        self.assertEqual(score.outcome(expected, expected), "match")
        for changed in [expected[:-1], expected + [{}], [], {}, None]:
            self.assertNotEqual(score.outcome(expected, changed), "match")
        for i, field, value in [
            (0, "data", "YWJk"),
            (1, "owned", 1),
            (1, "isolated", True),
            (2, "error", "canceled"),
            (2, "data", ""),
        ]:
            changed = copy.deepcopy(expected)
            changed[i][field] = value
            self.assertEqual(score.outcome(expected, changed), "mismatch")
        self.assertEqual(
            score.outcome(expected, {"unsupported": "absent"}), "unsupported"
        )


if __name__ == "__main__":
    unittest.main()
