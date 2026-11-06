import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "policy", Path(__file__).with_name("imageasset-policy-scoreboard.py")
)
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)


class PolicyGateTests(unittest.TestCase):
    def cases(self):
        return [
            {
                "name": name,
                "operations": [
                    {"request": {"op": op}, "got": {}}
                    for op, count in score.OPS[name].items()
                    for _ in range(count)
                ],
            }
            for name in score.TESTS
        ]

    def test_requires_all_functions_and_operations(self):
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

    def test_full_resource_error_and_cache_state(self):
        expected = [
            {"error": "", "resource": {"data": "YWJj", "width": 2, "height": 3}},
            {
                "error": "localfile: access denied",
                "unavailable": False,
                "cache_entries": 1,
            },
        ]
        self.assertEqual(score.outcome(expected, expected), "match")
        for changed in [expected[:-1], expected + [{}], [], {}, None]:
            self.assertNotEqual(score.outcome(expected, changed), "match")
        for index, field, value in [
            (0, "resource", None),
            (0, "resource", {"data": "YWJk", "width": 2, "height": 3}),
            (1, "error", "access denied"),
            (1, "unavailable", 0),
            (1, "cache_entries", 0),
        ]:
            changed = copy.deepcopy(expected)
            changed[index][field] = value
            self.assertEqual(score.outcome(expected, changed), "mismatch")
        self.assertEqual(
            score.outcome(expected, {"unsupported": "absent"}), "unsupported"
        )


if __name__ == "__main__":
    unittest.main()
