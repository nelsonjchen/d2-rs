import importlib.util
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
spec = importlib.util.spec_from_file_location(
    "themes", Path(__file__).with_name("themes-api-scoreboard.py")
)
themes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(themes)


class ThemeGates(unittest.TestCase):
    def cases(self):
        return [
            {"name": "A#0", "test": "A", "got": {"value": '<rect x="1" />'}},
            {"name": "B#0", "test": "B", "got": {"panic": True}},
        ]

    def test_complete_bytes_and_admission(self):
        cases = self.cases()
        self.assertEqual(
            [
                row["outcome"]
                for row in themes.compare(cases, [c["got"] for c in cases])
            ],
            ["match", "match"],
        )

    def test_missing_extra_or_nonarray_results_fail(self):
        for actual in [[], [None], [None] * 3, {"value": ""}]:
            with self.assertRaises(AssertionError):
                themes.compare(self.cases(), actual)

    def test_panic_admission_is_boolean(self):
        rows = themes.compare(self.cases(), [{"value": '<rect x="1" />'}, {"panic": 1}])
        self.assertEqual(rows[1]["outcome"], "mismatch")

    def test_no_svg_or_error_normalization(self):
        rows = themes.compare(self.cases(), [{"value": '<rect x="1"/>'}, {"value": ""}])
        self.assertEqual([r["outcome"] for r in rows], ["mismatch", "mismatch"])

    def test_unsupported_is_nonpassing(self):
        rows = themes.compare(
            self.cases(), [{"unsupported": "missing"}, {"panic": True}]
        )
        self.assertEqual(rows[0]["outcome"], "unsupported")

    def test_every_operation_must_pass_before_function_passes(self):
        rows = [
            {"test": "A", "outcome": "match"},
            {"test": "A", "outcome": "mismatch"},
            {"test": "B", "outcome": "unsupported"},
        ]
        self.assertEqual(
            themes.functions(rows),
            [
                {"name": "A", "outcome": "mismatch"},
                {"name": "B", "outcome": "unsupported"},
            ],
        )

    def test_instrumentation_keeps_original_assertions(self):
        source = (
            themes.references.ROOT
            / themes.references.MANIFEST["d2"]["checkout"]
            / themes.TEST_FILE
        ).read_text()
        actual = themes.instrument(source)
        for assertion in ["t.Fatalf(", "t.Errorf(", "t.Error(", "recover()"]:
            self.assertEqual(source.count(assertion), actual.count(assertion))
        self.assertNotIn(".Render()", actual)
        self.assertEqual(sum(themes.TESTS.values()), 15)


if __name__ == "__main__":
    unittest.main()
