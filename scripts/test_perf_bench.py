"""Resource measurements must belong to one invocation, with no stale output."""
import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("perf-bench.py")
spec = importlib.util.spec_from_file_location("perf_bench", SCRIPT)
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class PerfBenchTests(unittest.TestCase):
    @staticmethod
    def samples(*outputs, exits=None):
        return {"exit": 0, "sample_exits": exits or [0] * len(outputs), "output_hashes": list(outputs)}

    def outcome(self, fmt, go, rust):
        return bench.compare_samples(fmt, go, rust)["outcome"]

    def test_non_pdf_go_disagreement_without_the_seeded_oracle_never_matches(self):
        first, second = {"out.svg": "a"}, {"out.svg": "b"}
        for fmt in ("svg", "png", "gif", "pptx", "txt"):
            with self.subTest(fmt=fmt):
                go = self.samples(first, second, first)
                with redirect_stderr(io.StringIO()):
                    result = bench.compare_samples(fmt, go, self.samples(first, first, first))
                    self.assertEqual((result["outcome"], result["go_variants"]), ("mapseed-oracle-missing", 2))
                    missing = bench.mapseed.Oracle(Path("/nonexistent/go-mapseed"))
                    result = bench.compare_samples(fmt, go, self.samples(first, first, first), lambda seed: first, missing)
                    self.assertEqual(result["outcome"], "mapseed-oracle-missing")
                # d2-rs disagreeing with itself cannot be resolved by any seed.
                self.assertEqual(self.outcome(fmt, go, self.samples(first, second, first)), "mismatch")

    def seeded(self, fmt, go, rust, table, known=False):
        with tempfile.NamedTemporaryFile() as binary:
            oracle = bench.mapseed.Oracle(Path(binary.name))
            return bench.compare_samples(fmt, go, rust, lambda seed: table.get(seed, go["output_hashes"][0]),
                                         oracle, (1, 2, 3, 4), known)

    def test_non_pdf_go_disagreement_is_resolved_only_by_a_seeded_output(self):
        first, second, other = {"out.svg": "a"}, {"out.svg": "b"}, {"out.svg": "c"}
        go = self.samples(first, second, first)
        result = self.seeded("svg", go, self.samples(second, second, second), {3: second})
        self.assertEqual((result["outcome"], result["mapseed"]["mapseed_seeds"], result["go_variants"]),
                         ("mapseed-match", [3], 2))
        self.assertEqual(self.seeded("svg", go, self.samples(second, second), {3: second}, known=True)["outcome"],
                         "known-mapseed")
        # Equal to a stock Go repeat but to no seeded output: a d2-rs failure.
        self.assertEqual(self.seeded("svg", go, self.samples(second, second), {})["outcome"],
                         "go-nondeterministic-no-rust-match")
        self.assertEqual(self.seeded("png", go, self.samples(other, other), {2: second})["outcome"],
                         "go-nondeterministic-no-rust-match")
        # A seeded run that failed is not a variant.
        self.assertEqual(self.seeded("svg", go, self.samples(second), {1: None, 3: second})["mapseed"]["seed_failures"],
                         [1])

    def test_non_pdf_requires_every_rust_repeat_to_equal_the_single_go_output(self):
        same, other = {"out.png": "a"}, {"out.png": "b"}
        go = self.samples(same, same, same)
        self.assertEqual(bench.compare_samples("png", go, self.samples(same, same, same)),
                         {"outcome": "match", "go_variants": 1})
        self.assertEqual(self.outcome("png", go, self.samples(same, other, same)), "mismatch")
        self.assertEqual(self.outcome("png", go, self.samples(other, other, other)), "mismatch")

    def test_pdf_accepts_any_observed_go_variant_and_counts_variants(self):
        first, second = {"out.pdf": "map-order-1"}, {"out.pdf": "map-order-2"}
        go = self.samples(first, second, first)
        self.assertEqual(bench.compare_samples("pdf", go, self.samples(first, first, second)),
                         {"outcome": "match", "go_variants": 2})
        self.assertEqual(self.outcome("pdf", go, self.samples(first, {"out.pdf": "unseen"}, first)), "mismatch")

    def test_multi_file_outputs_compare_complete_inventories(self):
        full = {"out/index.svg": "root", "out/one.svg": "child", "out/manifest.json": "manifest"}
        partial = {"out/index.svg": "root", "out/one.svg": "child"}
        extra = {**full, "out/two.svg": "stray"}
        for fmt in ("svg", "pdf"):
            with self.subTest(fmt=fmt):
                go = self.samples(full, full)
                self.assertEqual(self.outcome(fmt, go, self.samples(full, full)), "match")
                self.assertEqual(self.outcome(fmt, go, self.samples(full, partial)), "mismatch")
                self.assertEqual(self.outcome(fmt, go, self.samples(extra, full)), "mismatch")
        with redirect_stderr(io.StringIO()):
            self.assertEqual(self.outcome("svg", self.samples(full, partial), self.samples(full, full)),
                             "mapseed-oracle-missing")

    def test_failed_empty_or_incomplete_samples_never_match(self):
        good = {"out.svg": "hash"}
        cases = [
            ("go-failed", self.samples(), self.samples(good)),
            ("go-failed", self.samples({}), self.samples(good)),
            ("go-failed", self.samples(good, exits=[0, 0]), self.samples(good)),
            ("go-failed", self.samples(good, good, exits=[0, 1]), self.samples(good, good)),
            ("rust-failed", self.samples(good), self.samples()),
            ("rust-failed", self.samples(good, good), self.samples(good, None)),
            ("rust-failed", self.samples(good, good), self.samples(good, good, exits=[1, 0])),
        ]
        for fmt in ("svg", "pdf"):
            for expected, go, rust in cases:
                with self.subTest(fmt=fmt, expected=expected, go=go, rust=rust):
                    self.assertEqual(self.outcome(fmt, go, rust), expected)

    def test_report_rows_and_summary_carry_outcomes_and_pdf_go_variants(self):
        calls = 0

        def fake_run(binary, engine, source, out, extra=()):
            nonlocal calls
            calls += 1
            # Each job makes 6 calls (warm-up, then 2 repeats; Go before Rust): call 11 is the second Go PDF.
            out.write_bytes(b"order-2" if out.suffix == ".pdf" and calls == 11 else b"order-1")
            return 0, 0.01, 0.01, 1000

        jobs = [(1, "case", "a", "dagre", "svg", ()), (1, "case", "a", "dagre", "pdf", ())]
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            for name in ("go", "rust"):
                (directory / name).write_bytes(name.encode())
            report = directory / "report.json"
            argv = ["perf-bench.py", "--go", str(directory / "go"), "--rust", str(directory / "rust"),
                    "--output", str(report), "--repeat", "2", "--warmup", "1"]
            with patch.object(bench, "run", fake_run), patch.object(bench, "benchmark_jobs", return_value=jobs), \
                    patch.object(sys, "argv", argv), patch("builtins.print"):
                bench.main()
            data = json.loads(report.read_text())
        self.assertEqual([(r["format"], r["outcome"], r["go_variants"], r["identical"]) for r in data["rows"]],
                         [("svg", "match", 1, True), ("pdf", "match", 2, True)])
        self.assertEqual(data["summary"]["dagre"]["outcomes"], {"match": 2})
        self.assertEqual(data["summary"]["dagre"]["max_pdf_go_variants"], 2)

    def test_isometric_gate_covers_every_format_and_engine(self):
        with tempfile.TemporaryDirectory() as directory:
            cases = Path(directory)
            for name, _, _ in bench.ISOMETRIC_CASES:
                (cases / (name.replace("/", "__") + ".json")).write_text(
                    json.dumps({"name": name, "script": "a -> b"}))
            with patch.object(bench.references, "E2E_CASES", cases), patch.object(bench, "pick_cases", return_value=[]):
                jobs = list(bench.benchmark_jobs(12))
        self.assertEqual({job[3] for job in jobs}, set(bench.ENGINES))
        self.assertEqual({job[4] for job in jobs if job[1] == "real_world/fulcro_rad"}, {"svg", "png", "pdf"})
        self.assertTrue(any(job[1:5] == ("patterns/paper", "a -> b", "dagre", "svg") for job in jobs))
        self.assertTrue(any(job[1:5] == ("stable/us_map", "a -> b", "dagre", "png") for job in jobs))
        self.assertEqual({job[3] for job in jobs if job[1] == "stable/grid_animated"}, set(bench.ENGINES))
        self.assertEqual({job[4] for job in jobs}, {"svg", "png", "gif", "pdf", "pptx"})
        self.assertEqual({job[4] for job in jobs if job[1] == "unicode/emojis"}, {"svg", "gif"})
        for fmt in ("gif", "pdf", "pptx"):
            self.assertGreaterEqual(len({job[1] for job in jobs if job[4] == fmt}), 2)
        self.assertTrue(any(job[1:5] == ("regression/grid_oom", "a -> b", "dagre", "svg") for job in jobs))
        self.assertTrue(all("--isometric" in job[5] for job in jobs))
        self.assertTrue(all("--animate-interval=125" in job[5] for job in jobs if job[4] == "gif"))

    def test_pdf_comparison_ignores_only_dates(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "out.pdf"
            data = b"%PDF-1.4\n/CreationDate (D:20261115000000+00'00')\n/ModDate (D:20261115000000+00'00')\nstream\nimage\nendstream\n%%EOF"
            path.write_bytes(data)
            expected = bench.output_inventory(Path(directory))
            path.write_bytes(data.replace(b"20261115000000", b"20261116010203"))
            self.assertEqual(bench.output_inventory(Path(directory)), expected)
            self.assertEqual(len(bench.output_bytes(path)), len(data))
            for changed in (data.replace(b"image", b"other"), data.replace(b"ModDate", b"OtherXX"),
                            data.replace(b"+00'00')\n/Mod", b"-00'00')\n/Mod")):
                path.write_bytes(changed)
                self.assertNotEqual(bench.output_inventory(Path(directory)), expected)
            path = path.with_suffix(".svg")
            path.write_bytes(data)
            self.assertEqual(bench.output_bytes(path), data)

    def test_failed_cli_removes_previous_output_and_keeps_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.d2"
            source.write_text("a")
            output = Path(directory) / "old.svg"
            output.write_text("stale passing output")
            boards = output.with_suffix("")
            boards.mkdir()
            (boards / "child.svg").write_text("stale board")
            exit_code, wall, cpu, rss = bench.run(Path("/usr/bin/false"), "dagre", source, output)
            self.assertEqual(exit_code, 1)
            self.assertFalse(output.exists())
            self.assertFalse(boards.exists())
            self.assertGreater(wall, 0)
            self.assertGreaterEqual(cpu, 0)
            self.assertGreater(rss, 0)

    def test_inventory_compares_every_board_and_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            self.assertEqual(bench.output_inventory(output), {})
            (output / "out").mkdir()
            child = output / "out" / "child.svg"
            child.write_text("child")
            manifest = output / "out" / ".d2-board-output-manifest-v1.json"
            manifest.write_text('{"files":["child.svg"]}')
            expected = bench.output_inventory(output)
            self.assertEqual(set(expected), {"out/child.svg", "out/.d2-board-output-manifest-v1.json"})
            child.write_text("changed")
            self.assertNotEqual(bench.output_inventory(output), expected)
            child.write_text("child")
            manifest.unlink()
            self.assertNotEqual(bench.output_inventory(output), expected)
            child.write_text("")
            with self.assertRaisesRegex(ValueError, "empty output"):
                bench.output_inventory(output)

    def test_resource_high_water_mark_is_isolated_per_worker(self):
        def measure(code):
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--measure-child", sys.executable, "-c", code],
                capture_output=True, text=True, check=True,
            )
            return json.loads(result.stdout)

        big = measure("data = bytearray(64 * 1024 * 1024)")
        small = measure("pass")
        self.assertEqual(big[0], 0)
        self.assertEqual(small[0], 0)
        self.assertGreater(big[3], small[3] + 32 * 1024 * 1024)


if __name__ == "__main__":
    unittest.main()
