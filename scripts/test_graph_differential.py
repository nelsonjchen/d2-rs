"""The wide graph sweep must generate deterministic, balanced, feature-rich D2."""
import importlib.util
import random
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
SCRIPT = Path(__file__).with_name("graph-differential.py")
spec = importlib.util.spec_from_file_location("graph_differential", SCRIPT)
sweep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sweep)


class GraphDifferentialTests(unittest.TestCase):
    def cases(self, seed, count=300):
        rng = random.Random(seed)
        return [(sweep.generate_wide(rng), sweep.flags_for(rng)) for _ in range(count)]

    def test_same_seed_same_cases(self):
        self.assertEqual(self.cases(5), self.cases(5))
        self.assertNotEqual(self.cases(5), self.cases(6))

    def test_braces_and_block_strings_balance(self):
        for source, _ in self.cases(1):
            self.assertEqual(source.count("{"), source.count("}"), source)
            openers = source.count("|md\n") + source.count("|go\n")
            closers = sum(line.strip() == "|" for line in source.splitlines())
            self.assertEqual(openers, closers, source)

    def test_sweep_covers_the_feature_families(self):
        text = "".join(source for source, _ in self.cases(1, 600))
        for feature in ("shape: sequence_diagram", "shape: sql_table", "shape: class", "grid-",
                        "label.near:", "near:", "arrowhead.shape", "style.", "direction:", "|md",
                        ".c0"):
            self.assertIn(feature, text)
        flags = {flag.split("=")[0] for _, case_flags in self.cases(1, 600) for flag in case_flags}
        self.assertEqual(flags, {"--sketch", "--theme"})

    def test_minimize_keeps_structure_lines(self):
        source = "a: {\n  b\n}\nx -> y\n"
        calls = []

        def fake(text, layout, flags, rust, go):
            calls.append(text)
            return "output-diff" if "x -> y" in text else None

        original = sweep.diverges
        sweep.diverges = fake
        try:
            small = sweep.minimize(source, "dagre", (), "rust", "go", "output-diff")
        finally:
            sweep.diverges = original
        self.assertEqual(small, "a: {\n}\nx -> y\n")

    def diverges_with(self, go_digests, rust_digest, seeded=None, calls=None):
        """Runs diverges() against a fake Go that returns go_digests in turn."""
        go_runs = iter(go_digests)

        def fake(binary, source, layout, flags, root, tag, extra_env=None):
            if calls is not None:
                calls.append((str(binary), extra_env))
            return (0, None, rust_digest) if binary == "rust" else (0, None, next(go_runs))

        original, original_seeded = sweep.outcome, sweep.MAPSEED_ORACLE
        sweep.outcome = fake
        sweep.MAPSEED_ORACLE = Path(seeded) if seeded else Path("/nonexistent/mapseed")
        try:
            return sweep.diverges("a -> b\n", "dagre", (), "rust", "go")
        finally:
            sweep.outcome, sweep.MAPSEED_ORACLE = original, original_seeded

    def test_go_resamples_before_an_output_difference_counts(self):
        stable_other = ["go-a"] * (1 + sweep.GO_RESAMPLES)
        self.assertEqual(self.diverges_with(stable_other, "rust-a"), "output-diff")
        self.assertIsNone(self.diverges_with(["same"], "same"))
        flips = ["go-b", "go-b", "go-a"] + ["go-b"] * sweep.GO_RESAMPLES
        self.assertEqual(self.diverges_with(flips, "go-a"), sweep.NONDETERMINISTIC)
        # Go varying among outputs none of which is d2-rs's is still a mismatch.
        varied = ["go-a", "go-b"] * sweep.GO_RESAMPLES
        self.assertEqual(self.diverges_with(varied, "rust-a"), "output-diff")

    def test_seedable_oracle_resamples_with_map_seeds(self):
        with tempfile.NamedTemporaryFile() as seeded:
            calls = []
            stable_other = ["go-a"] * (1 + sweep.GO_RESAMPLES)
            kind = self.diverges_with(stable_other, "rust-a", seeded.name, calls)
        self.assertEqual(kind, "output-diff")
        resamples = [env for binary, env in calls if binary == seeded.name]
        self.assertEqual(resamples, [{"D2RS_GO_MAPSEED": str(seed)}
                                     for seed in range(1, sweep.GO_RESAMPLES + 1)])


if __name__ == "__main__":
    unittest.main()
