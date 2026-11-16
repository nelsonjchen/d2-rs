"""The reducer replays widened-format findings with their exact flags."""
import importlib.util
import io
import json
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("fuzz-minimize.py")
SPEC = importlib.util.spec_from_file_location("fuzz_minimize", MODULE_PATH)
MINIMIZER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MINIMIZER)


class FuzzMinimizeTests(unittest.TestCase):
    def test_kind_forwards_output_format_and_render_flags(self):
        calls = []

        def run(binary, directory, layout, name, output_format, render_flags):
            calls.append((binary, layout, name, output_format, tuple(render_flags)))
            return 0, "", binary

        with mock.patch.object(MINIMIZER, "run", run):
            kind = MINIMIZER.kind(
                "rust",
                "go",
                "a",
                "elk",
                "png",
                ["--sketch", "--scale=0.5"],
            )

        self.assertEqual(kind, "output-diff")
        self.assertEqual(
            calls,
            [
                ("go", "elk", "render", "png", ("--sketch", "--scale=0.5")),
                ("rust", "elk", "render", "png", ("--sketch", "--scale=0.5")),
            ],
        )


FAKE = """#!{python}
import os, sys
keep = "keep" in open("input.d2").read()
text = {body}
open(sys.argv[-1], "w").write('<svg xmlns="http://www.w3.org/2000/svg"><text>' + text + '</text></svg>')
"""


class MapSeedReductionTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)

    def cli(self, name, body):
        path = self.root / name
        path.write_text(FAKE.format(python=sys.executable, body=body))
        path.chmod(0o755)
        return path

    def run_main(self, findings, seeded=True):
        go = self.cli("go", '"A"')
        if seeded:
            # Map seed 2 takes the other order, but only while "keep" is in the input.
            self.cli("go-mapseed", '"B" if keep and os.environ.get("D2RS_GO_MAPSEED") == "2" else "A"')
        rust = self.cli("rust", '"C" if keep else "A"')
        witnesses = self.root / "witnesses"
        witnesses.mkdir(exist_ok=True)
        (witnesses / "w.d2").write_text("a\nkeep\nb -> c\n")
        index = self.root / "findings.jsonl"
        index.write_text("".join(json.dumps({"kind": kind, "file": "w.d2"}) + "\n" for kind in findings))
        stdout, stderr = io.StringIO(), io.StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            MINIMIZER.main([str(rust), str(go), str(index), str(witnesses), "dagre", "--mapseed-seeds", "1-3"])
        return stdout.getvalue(), stderr.getvalue()

    def test_no_rust_match_is_reduced_with_the_seeded_predicate(self):
        out, _ = self.run_main(["go-nondeterministic-no-rust-match"])
        self.assertIn("--- go-nondeterministic-no-rust-match from w.d2\nkeep\n", out)
        self.assertIn("UNIQUE 1", out)

    def test_predicate_needs_go_variation_and_no_rust_match(self):
        go_mapseed = self.cli("go-mapseed", '"B" if os.environ.get("D2RS_GO_MAPSEED") == "2" else "A"')
        stable = self.cli("go-stable", '"A"')
        rust_b, rust_c = self.cli("rust-b", '"B"'), self.cli("rust-c", '"C"')
        self.assertEqual(MINIMIZER.no_rust_match(rust_c, go_mapseed, "x", "dagre", seeds=(1, 2)),
                         "go-nondeterministic-no-rust-match")
        self.assertIsNone(MINIMIZER.no_rust_match(rust_b, go_mapseed, "x", "dagre", seeds=(1, 2)))
        self.assertIsNone(MINIMIZER.no_rust_match(rust_c, stable, "x", "dagre", seeds=(1, 2)))

    def test_missing_oracle_findings_are_skipped_with_a_message(self):
        out, err = self.run_main(["mapseed-oracle-missing"])
        self.assertIn("skipping 1 mapseed-oracle-missing findings", err)
        self.assertIn("build-mapseed-oracle.py", err)
        self.assertIn("UNIQUE 0", out)
        (self.root / "go-mapseed").unlink()
        out, err = self.run_main(["go-nondeterministic-no-rust-match"], seeded=False)
        self.assertIn("seedable-map Go oracle not found", err)
        self.assertIn("UNIQUE 0", out)


if __name__ == "__main__":
    unittest.main()
