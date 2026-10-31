import importlib.util
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
spec = importlib.util.spec_from_file_location(
    "library", Path(__file__).with_name("library-api-scoreboard.py")
)
library = importlib.util.module_from_spec(spec)
spec.loader.exec_module(library)


class LibraryGates(unittest.TestCase):
    def cases(self):
        return [
            {
                "name": "TestA/x#0",
                "test": "TestA/x",
                "got": {"error": "complete error", "options": {"Mode": False}},
            },
            {"name": "TestB#0", "test": "TestB", "got": {"svg": "<svg />"}},
        ]

    def test_exact_json_and_svg(self):
        cases = self.cases()
        self.assertEqual(
            [r["outcome"] for r in library.compare(cases, [c["got"] for c in cases])],
            ["match", "match"],
        )
        self.assertEqual(
            library.compare(
                cases, [{"error": "error", "options": {"Mode": 0}}, {"svg": "<svg/>"}]
            )[0]["outcome"],
            "mismatch",
        )
        self.assertEqual(
            library.compare(cases, [cases[0]["got"], {"svg": "<svg/>"}])[1]["outcome"],
            "mismatch",
        )

    def test_complete_inventory_required(self):
        for actual in [[], [None], [None] * 3, {}]:
            with self.assertRaises(AssertionError):
                library.compare(self.cases(), actual)

    def test_unsupported_does_not_pass(self):
        self.assertEqual(
            library.compare(
                self.cases(), [{"unsupported": "callback"}, {"svg": "<svg />"}]
            )[0]["outcome"],
            "unsupported",
        )

    def test_function_requires_all_subtests(self):
        self.assertEqual(
            library.functions(
                [
                    {"test": "TestA/a", "outcome": "match"},
                    {"test": "TestA/b", "outcome": "unsupported"},
                ]
            ),
            [{"name": "TestA", "outcome": "unsupported"}],
        )

    def test_original_assertions_unchanged_and_benchmark_untouched(self):
        for name in library.TEST_FILES:
            original = (
                library.references.ROOT
                / library.references.MANIFEST["d2"]["checkout"]
                / name
            ).read_text()
            actual = library.instrument(original)
            for assertion in ["t.Fatal(", "t.Fatalf(", "t.Error(", "t.Errorf("]:
                self.assertEqual(original.count(assertion), actual.count(assertion))
            if "func BenchmarkCompileStyleOnlyBoards" in original:
                self.assertEqual(
                    original.split("func BenchmarkCompileStyleOnlyBoards")[1].split(
                        "func compileForLayoutReuseTest"
                    )[0],
                    actual.split("func BenchmarkCompileStyleOnlyBoards")[1].split(
                        "func compileForLayoutReuseTest"
                    )[0],
                )
        self.assertEqual(sum(library.TESTS.values()), 29)

    def test_byte_limits_and_bounded_runner_failure_are_nonpassing(self):
        # The production bounded runner is shared with existing CLI gates.
        cases = self.cases()
        self.assertEqual(
            library.compare(cases, [{"error": "complete error"}, {"svg": ""}])[1][
                "outcome"
            ],
            "mismatch",
        )
        self.assertEqual(
            library.compare(cases, [{"options": {}}, {"svg": "<svg />"}])[0]["outcome"],
            "mismatch",
        )


if __name__ == "__main__":
    unittest.main()
