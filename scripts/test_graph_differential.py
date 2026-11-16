"""The wide graph sweep must generate deterministic, balanced, feature-rich D2."""
import importlib.util
import io
import json
import random
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
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

        def fake(text, layout, flags, rust, go, resampler):
            calls.append(text)
            return ("output-diff" if "x -> y" in text else None), {}

        original = sweep.diverges
        sweep.diverges = fake
        try:
            small = sweep.minimize(source, "dagre", (), "rust", "go", "output-diff", None)
        finally:
            sweep.diverges = original
        self.assertEqual(small, "a: {\n}\nx -> y\n")


FAKE_CLI = """#!{python}
import os, sys
seed = os.environ.get("D2RS_GO_MAPSEED")
variants = {variants!r}
text = variants.get(seed, variants[None])
open(sys.argv[-1], "w").write('<svg xmlns="http://www.w3.org/2000/svg"><text>' + text + '</text></svg>')
"""


class MapSeedClassificationTests(unittest.TestCase):
    """End to end through fuzz_common.run with fake CLIs whose SVG depends on D2RS_GO_MAPSEED."""

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)

    def cli(self, name, variants):
        path = self.root / name
        path.write_text(FAKE_CLI.format(python=sys.executable, variants=variants))
        path.chmod(0o755)
        return path

    def classify(self, go, rust, seeded=None, known=False, seeds=(1, 2, 3, 4)):
        go_path = self.cli("go", go)
        seeded_path = self.cli("go-mapseed", seeded) if seeded is not None else self.root / "absent-mapseed"
        registry = sweep.mapseed.Registry(self.root / "registry.json")
        if known:
            registry.add(sweep.Resampler.identity("a -> b\n", "dagre", ()), "a -> b\n",
                         sweep.Resampler.args("dagre", ()), "test", "test", "test")
        resampler = sweep.Resampler(go_path, seeded_path, seeds, registry)
        with redirect_stderr(io.StringIO()):
            return sweep.diverges("a -> b\n", "dagre", (), self.cli("rust", rust), go_path, resampler)

    # Seeds 2 and 4 take the other map order.
    SEEDED = {None: "A", "2": "B", "4": "B"}

    def test_match(self):
        self.assertEqual(self.classify({None: "A"}, {None: "A"}), (None, {"classification": "match"}))

    def test_mapseed_match_lists_the_matching_seeds(self):
        kind, record = self.classify({None: "A"}, {None: "B"}, self.SEEDED)
        self.assertIsNone(kind)
        self.assertEqual((record["classification"], record["mapseed_seeds"], record["go_variants"]),
                         ("mapseed-match", [2, 4], 2))

    def test_known_mapseed(self):
        kind, record = self.classify({None: "A"}, {None: "B"}, self.SEEDED, known=True)
        self.assertEqual((kind, record["classification"]), (None, "known-mapseed"))

    def test_go_nondeterministic_no_rust_match_is_a_failure(self):
        kind, record = self.classify({None: "A"}, {None: "C"}, self.SEEDED)
        self.assertEqual((kind, record["go_variants"]), ("go-nondeterministic-no-rust-match", 2))

    def test_mismatch(self):
        kind, record = self.classify({None: "A"}, {None: "C"}, {None: "A"})
        self.assertEqual((kind, record["classification"]), ("output-diff", "mismatch"))

    def test_missing_oracle_never_passes(self):
        kind, record = self.classify({None: "A"}, {None: "B"})
        self.assertEqual(kind, "mapseed-oracle-missing")

    def test_unpaired_oracle_never_passes(self):
        # A seeded build that does not reproduce --go with the variable unset.
        kind, _ = self.classify({None: "A"}, {None: "B"}, {None: "Z", "2": "B"})
        self.assertEqual(kind, "mapseed-oracle-missing")

    def test_seeded_runs_receive_the_seed(self):
        _, record = self.classify({None: "A"}, {None: "B"}, self.SEEDED, seeds=(1, 2, 3, 4, 5, 6))
        self.assertEqual(record["seeds_tried"], [1, 2, 3, 4, 5, 6])
        self.assertEqual(record["mapseed_seeds"], [2, 4])

    ERR_CLI = """#!{python}
import os, sys
table = {table!r}
code, text, write = table.get(os.environ.get("D2RS_GO_MAPSEED"), table[None])
if write:
    open(sys.argv[-1], "w").write('<svg xmlns="http://www.w3.org/2000/svg"/>')
if code == "kill":
    os.kill(os.getpid(), 9)
sys.stderr.write(text)
sys.exit(code)
"""

    def err_cli(self, name, table):
        path = self.root / name
        path.write_text(self.ERR_CLI.format(python=sys.executable, table=table))
        path.chmod(0o755)
        return path

    def classify_errors(self, go, rust, seeded=None):
        go_path = self.err_cli("go", go)
        seeded_path = self.err_cli("go-mapseed", seeded) if seeded else self.root / "absent-mapseed"
        resampler = sweep.Resampler(go_path, seeded_path, (1, 2, 3), sweep.mapseed.Registry(self.root / "r.json"))
        with redirect_stderr(io.StringIO()):
            return sweep.diverges("a -> b\n", "dagre", (), self.err_cli("rust", rust), go_path, resampler)

    ERROR = (1, "err: failed to compile input.d2: bad key\n", False)

    def test_go_errors_are_scored_against_rust(self):
        self.assertEqual(self.classify_errors({None: self.ERROR}, {None: self.ERROR}),
                         (None, {"classification": "error-match"}))
        # Different text, exit code, or any file left behind: error-mismatch (Go stable under the seeds).
        for rust in ((1, "err: other\n", False), (2, self.ERROR[1], False), (1, self.ERROR[1], True), (0, "", True)):
            kind, record = self.classify_errors({None: self.ERROR}, {None: rust}, {None: self.ERROR})
            self.assertEqual((kind, record["classification"]), ("error-mismatch", "error-mismatch"), rust)
        # The full stderr counts, not only the first error line.
        two = (1, self.ERROR[1] + "err: second\n", False)
        self.assertEqual(self.classify_errors({None: two}, {None: self.ERROR}, {None: two})[0], "error-mismatch")

    def test_go_error_text_varying_with_map_order_is_resampled(self):
        other = (1, "err: failed to compile input.d2: other key\n", False)
        kind, record = self.classify_errors({None: self.ERROR}, {None: other}, {None: self.ERROR, "2": other})
        self.assertEqual((kind, record["classification"], record["mapseed_seeds"]), (None, "mapseed-match", [2]))
        kind, _ = self.classify_errors({None: self.ERROR}, {None: (1, "err: x\n", False)},
                                       {None: self.ERROR, "2": other})
        self.assertEqual(kind, "go-nondeterministic-no-rust-match")
        self.assertEqual(self.classify_errors({None: self.ERROR}, {None: other})[0], "mapseed-oracle-missing")

    def test_only_go_crashes_are_unscored(self):
        for crash in (("kill", "", False), (2, "panic: runtime error\n", False)):
            kind, record = self.classify_errors({None: crash}, {None: self.ERROR})
            self.assertEqual((kind, record["classification"]), (None, "go-crash"), crash)
        self.assertFalse(sweep.failing({"classification": "go-crash"}))
        self.assertFalse(sweep.failing({"classification": "error-match"}))
        self.assertTrue(sweep.failing({"classification": "error-mismatch"}))

    def test_summary_findings_and_exit_status(self):
        go, rust, seeded = self.cli("go", {None: "A"}), self.cli("rust", {None: "B"}), self.cli("go-mapseed", self.SEEDED)
        out = self.root / "out"
        argv = [str(rust), "--go", str(go), "--go-mapseed", str(seeded), "--mapseed-seeds", "1-4",
                "--cases", "2", "--layouts", "dagre", "--jobs", "1", "--out", str(out),
                "--mapseed-registry", str(self.root / "registry.json")]
        printed = io.StringIO()
        with redirect_stdout(printed):
            self.assertEqual(sweep.main(argv), 0)
        self.assertIn("dagre: 0/2 match, 2 mapseed-match (seeds 2,4)", printed.getvalue())
        findings = [json.loads(line) for line in (out / "findings.jsonl").read_text().splitlines()]
        self.assertEqual([(f["classification"], f["mapseed_seeds"]) for f in findings],
                         [("mapseed-match", [2, 4])] * 2)
        self.assertFalse((self.root / "registry.json").exists())
        with redirect_stdout(io.StringIO()):
            self.assertEqual(sweep.main(argv + ["--record-mapseed"]), 0)
        self.assertEqual(len(json.loads((self.root / "registry.json").read_text())["cases"]), 2)
        printed = io.StringIO()
        with redirect_stdout(printed):
            self.assertEqual(sweep.main(argv), 0)
        self.assertIn("dagre: 0/2 match, 2 known-mapseed (seeds 2,4)", printed.getvalue())
        bad = self.cli("rust", {None: "C"})
        printed = io.StringIO()
        with redirect_stdout(printed):
            self.assertEqual(sweep.main([str(bad), *argv[1:]]), 1)
        self.assertIn("FAIL go-nondeterministic-no-rust-match: " + str(out / "case-00000-dagre.d2"), printed.getvalue())


if __name__ == "__main__":
    unittest.main()
