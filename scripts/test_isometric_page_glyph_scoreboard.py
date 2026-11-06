"""The original Page observation must be present, complete and supported."""
import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "page", Path(__file__).with_name("isometric-page-glyph-scoreboard.py")
)
page = importlib.util.module_from_spec(spec)
spec.loader.exec_module(page)


class Gates(unittest.TestCase):
    def setUp(self):
        self.score = page.scoreboard()
        self.case = {"name": page.TEST, "operations": [{
            "request": {"op": "remove-page-glyph"}, "got": {"root": {}, "outer_preserved": True}
        }]}

    def test_complete_inventory(self):
        self.score.validate_cases([self.case])
        for bad in [[], [self.case, self.case], [{**self.case, "operations": []}]]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(bad)

    def test_wrong_operation_rejected(self):
        bad = copy.deepcopy(self.case)
        bad["operations"][0]["request"]["op"] = "other"
        with self.assertRaises(AssertionError):
            self.score.validate_cases([bad])

    def test_missing_changed_unsupported_are_not_matches(self):
        expected = self.case["operations"][0]["got"]
        self.assertEqual(self.score.outcome(expected, expected), "match")
        self.assertEqual(self.score.outcome(expected, None), "rust-failed")
        self.assertEqual(self.score.outcome(expected, {"unsupported": "input"}), "unsupported")
        self.assertEqual(self.score.outcome(expected, {**expected, "outer_preserved": False}), "mismatch")


if __name__ == "__main__":
    unittest.main()
