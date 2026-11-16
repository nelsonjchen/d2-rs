"""Map-seed classification: only a seeded Go output equal to d2-rs resolves a Go difference."""
import argparse
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import mapseed  # noqa: E402


class FakeOracle(mapseed.Oracle):
    def __init__(self, problem=None):
        super().__init__(Path("/fake/mapseed"))
        self._checked, self._problem = True, problem


def seeded_outputs(table, default="go-a"):
    calls = []

    def run(seed):
        calls.append(seed)
        return table.get(seed, default)
    return run, calls


class ResolveTests(unittest.TestCase):
    def test_agreeing_outputs_match_without_running_seeds(self):
        run, calls = seeded_outputs({})
        resolution = mapseed.resolve(["x"], ["x"], run, FakeOracle())
        self.assertEqual(resolution.classification, mapseed.MATCH)
        self.assertEqual(calls, [])
        self.assertTrue(resolution.passed)

    def test_rust_equal_to_a_seeded_output_is_mapseed_match_with_every_seed(self):
        run, calls = seeded_outputs({3: "go-b", 9: "go-b"})
        resolution = mapseed.resolve(["go-a"], ["go-b"], run, FakeOracle(), range(1, 17))
        self.assertEqual(calls, list(range(1, 17)))  # no early exit
        self.assertEqual((resolution.classification, resolution.seeds, resolution.go_variants),
                         (mapseed.MAPSEED_MATCH, [3, 9], 2))
        self.assertTrue(resolution.passed)
        self.assertEqual(resolution.record()["mapseed_seeds"], [3, 9])

    def test_known_input_is_labelled_known_mapseed_and_still_resampled(self):
        run, calls = seeded_outputs({5: "go-b"})
        resolution = mapseed.resolve(["go-a"], ["go-b"], run, FakeOracle(), (4, 5), known=True)
        self.assertEqual((resolution.classification, resolution.seeds), (mapseed.KNOWN_MAPSEED, [5]))
        # A known input whose stock run matched still runs the seeds; matching
        # one gives the stable label, matching none is still a byte match.
        run, calls = seeded_outputs({})
        self.assertEqual(mapseed.resolve(["x"], ["x"], run, FakeOracle(), (1, 2), known=True).classification,
                         mapseed.MATCH)
        self.assertEqual(calls, [1, 2])

    def test_registry_never_passes_a_case_by_itself(self):
        run, _ = seeded_outputs({2: "go-b"})
        resolution = mapseed.resolve(["go-a"], ["rust"], run, FakeOracle(), (1, 2, 3), known=True)
        self.assertEqual(resolution.classification, mapseed.NO_RUST_MATCH)
        with redirect_stderr(io.StringIO()):
            missing = mapseed.resolve(["go-a"], ["rust"], run, FakeOracle("absent"), (1,), known=True)
        self.assertEqual(missing.classification, mapseed.ORACLE_MISSING)

    def test_go_varying_without_a_rust_match_is_a_d2rs_failure(self):
        run, _ = seeded_outputs({2: "go-b", 7: "go-c"})
        resolution = mapseed.resolve(["go-a"], ["rust"], run, FakeOracle(), range(1, 9))
        self.assertEqual((resolution.classification, resolution.seeds, resolution.go_variants),
                         (mapseed.NO_RUST_MATCH, [], 3))
        self.assertFalse(resolution.passed)

    def test_stable_go_and_different_rust_is_a_mismatch(self):
        run, _ = seeded_outputs({})
        resolution = mapseed.resolve(["go-a"], ["rust"], run, FakeOracle(), range(1, 5))
        self.assertEqual((resolution.classification, resolution.go_variants), (mapseed.MISMATCH, 1))

    def test_stock_repeat_alone_never_resolves_a_go_disagreement(self):
        # d2-rs equals one stock Go repeat, but no seeded output.
        run, _ = seeded_outputs({})
        resolution = mapseed.resolve(["go-a", "go-b"], ["go-b"], run, FakeOracle(), (1, 2))
        self.assertEqual(resolution.classification, mapseed.NO_RUST_MATCH)

    def test_rust_disagreeing_with_itself_is_a_mismatch(self):
        run, calls = seeded_outputs({})
        resolution = mapseed.resolve(["go-a", "go-b"], ["go-a", "go-b"], run, FakeOracle())
        self.assertEqual(resolution.classification, mapseed.MISMATCH)
        self.assertEqual(calls, [])

    def test_missing_oracle_fails_loudly_and_never_passes(self):
        oracle = mapseed.Oracle(Path("/nonexistent/d2-mapseed"))
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            first = mapseed.resolve(["go-a"], ["rust"], lambda seed: self.fail("ran"), oracle)
            mapseed.resolve(["go-a", "go-b"], ["go-a"], lambda seed: self.fail("ran"), oracle)
        self.assertEqual(first.classification, mapseed.ORACLE_MISSING)
        self.assertFalse(first.passed)
        self.assertEqual(stderr.getvalue().count("build-mapseed-oracle.py"), 1)
        self.assertEqual(mapseed.resolve(["go-a"], ["rust"], lambda s: None, None).classification,
                         mapseed.ORACLE_MISSING)

    def test_unpaired_oracle_is_treated_as_missing(self):
        with tempfile.NamedTemporaryFile() as seeded:
            oracle = mapseed.Oracle(seeded.name, "go", probe=lambda binary: "stock" if binary == "go" else "other")
            with redirect_stderr(io.StringIO()):
                resolution = mapseed.resolve(["go-a"], ["rust"], lambda seed: "rust", oracle)
            self.assertEqual(resolution.classification, mapseed.ORACLE_MISSING)
            self.assertIn("does not reproduce", oracle.problem())
            paired = mapseed.Oracle(seeded.name, "go", probe=lambda binary: "same")
            self.assertIsNone(paired.problem())

    def test_failed_seeded_runs_are_neither_variants_nor_matches(self):
        run, _ = seeded_outputs({1: None, 2: None})
        resolution = mapseed.resolve(["go-a"], ["rust"], run, FakeOracle(), (1, 2, 3), usable=lambda key: key)
        self.assertEqual((resolution.classification, resolution.seed_failures, resolution.go_variants),
                         (mapseed.MISMATCH, [1, 2], 1))


class HelperTests(unittest.TestCase):
    def test_seed_lists(self):
        self.assertEqual(mapseed.parse_seeds("1-4,9,3"), (1, 2, 3, 4, 9))
        self.assertEqual(mapseed.DEFAULT_SEEDS, tuple(range(1, 17)))
        for bad in ("0", "5-2", ""):
            with self.assertRaises(ValueError):
                mapseed.parse_seeds(bad)

    def test_summary_counts_each_classification_and_names_seeds(self):
        records = [{"classification": "match"}] * 299 + [
            {"classification": mapseed.MAPSEED_MATCH, "mapseed_seeds": [3, 9]}]
        self.assertEqual(mapseed.summary("dagre", 300, records), "dagre: 299/300 match, 1 mapseed-match (seeds 3,9)")
        records.append({"classification": mapseed.NO_RUST_MATCH, "mapseed_seeds": []})
        self.assertEqual(mapseed.summary("elk", 301, records),
                         "elk: 299/301 match, 1 mapseed-match (seeds 3,9), 1 go-nondeterministic-no-rust-match")

    def test_identity_covers_source_and_arguments(self):
        base = mapseed.identity("a -> b\n", ["--layout=dagre", "svg"])
        self.assertEqual(base, mapseed.identity(b"a -> b\n", ["--layout=dagre", "svg"]))
        self.assertNotEqual(base, mapseed.identity("a -> c\n", ["--layout=dagre", "svg"]))
        self.assertNotEqual(base, mapseed.identity("a -> b\n", ["--layout=elk", "svg"]))

    def test_default_oracle_pairs_with_go(self):
        self.assertEqual(mapseed.default_oracle("/x/d2-go1.27"), Path("/x/d2-go1.27-mapseed"))

    def test_registry_is_written_only_on_request_and_holds_no_outputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "mapseed-cases.json"
            registry = mapseed.Registry(path)
            self.assertFalse(registry.known("x"))
            varied = mapseed.Resolution(mapseed.MAPSEED_MATCH, [3], 2)
            stable = mapseed.Resolution(mapseed.MISMATCH, [], 1)
            cases = [(mapseed.identity("a", ["svg"]), "a", ["svg"], "case a", varied),
                     (mapseed.identity("b", ["svg"]), "b", ["svg"], "case b", stable)]
            self.assertFalse(path.exists())
            self.assertEqual(mapseed.record_cases(registry, cases, "test"), 1)
            data = json.loads(path.read_text())
            self.assertEqual([entry["input"] for entry in data["cases"]], ["case a"])
            self.assertEqual(set(data["cases"][0]), {"identity", "source_sha256", "args", "harness", "input",
                                                      "reason", "recorded"})
            self.assertTrue(mapseed.Registry(path).known(mapseed.identity("a", ["svg"])))
            self.assertEqual(mapseed.record_cases(mapseed.Registry(path), cases, "test"), 0)

    def test_checked_in_registry_parses(self):
        if mapseed.REGISTRY.exists():
            for entry in json.loads(mapseed.REGISTRY.read_text())["cases"]:
                self.assertEqual(len(entry["identity"]), 64)
                self.assertTrue(entry["reason"])

    def test_arguments(self):
        parser = argparse.ArgumentParser()
        mapseed.add_arguments(parser)
        args = parser.parse_args(["--mapseed-seeds", "2-3", "--record-mapseed"])
        self.assertEqual((args.mapseed_seeds, args.record_mapseed, args.go_mapseed), ((2, 3), True, None))
        self.assertEqual(parser.parse_args([]).mapseed_seeds, mapseed.DEFAULT_SEEDS)


if __name__ == "__main__":
    unittest.main()
