import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
spec = importlib.util.spec_from_file_location(
    "source", Path(__file__).with_name("library-source-scoreboard.py")
)
source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source)


class OriginalSourceCalls(unittest.TestCase):
    def test_inventory_preserves_explicit_package_for_ambiguous_names(self):
        inventory = source.library.module("master-added-test-inventory")
        with tempfile.TemporaryDirectory(
            dir=source.library.ROOT / ".reference"
        ) as temporary:
            path = Path(temporary) / "library-source.json"
            path.write_text(
                json.dumps(
                    {
                        "source_commit": source.library.references.SOURCE_COMMIT,
                        "package": "d2lib",
                        "rows": [
                            {"test": "TestNilFSDeniesImports", "outcome": "match"},
                            {
                                "test": "TestCompilePropagatesGlobExpansionLimit",
                                "outcome": "match",
                            },
                        ],
                    }
                )
            )
            observed = inventory.observations([path])
            self.assertEqual(
                {row["package"] for rows in observed.values() for row in rows},
                {"d2lib"},
            )

    def test_instrumentation_preserves_every_assertion(self):
        pinned = source.library.references.verify_source()
        original = (pinned / "d2lib/d2_test.go").read_text()
        actual = source.instrument(original)
        for line in original.splitlines():
            if any(
                assertion in line
                for assertion in ["t.Fatal(", "t.Fatalf(", "t.Error(", "t.Errorf("]
            ):
                self.assertIn(line, actual)
        self.assertEqual(actual.count("d2rsSourceParse(t, "), 1)
        self.assertEqual(actual.count("d2rsSourceCompile(t, "), 6)
        self.assertEqual(sum(source.library.TESTS.values()), 7)
        self.assertEqual(len(source.library.TESTS), 6)
        self.assertEqual(
            original.split("func TestGetLayoutDoesNotUseEnvironmentFallback")[1],
            actual.split("func TestGetLayoutDoesNotUseEnvironmentFallback")[1],
        )

    def test_cancellation_category_and_partial_syntax_compare(self):
        cases = [
            {
                "name": "TestA#0",
                "test": "TestA",
                "got": {"error": "context canceled", "canceled": True, "ast": {}},
            }
        ]
        for actual in [
            {"error": "context canceled", "canceled": False, "ast": {}},
            {"error": "context canceled", "canceled": True, "ast": None},
        ]:
            self.assertEqual(
                source.library.compare(cases, [actual])[0]["outcome"], "mismatch"
            )


if __name__ == "__main__":
    unittest.main()
