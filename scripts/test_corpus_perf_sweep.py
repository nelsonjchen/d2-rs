"""The corpus sweep must time every format and give both CLIs the same output name."""
import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
SCRIPT = Path(__file__).with_name("corpus-perf-sweep.py")
spec = importlib.util.spec_from_file_location("corpus_perf_sweep", SCRIPT)
sweep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sweep)


def row(fmt, engine, wall_ratio, rss_ratio, go_exit=0):
    return {"format": fmt, "engine": engine, "wall_ratio": wall_ratio, "rss_ratio": rss_ratio,
            "go": {"exit": go_exit}}


class CorpusPerfSweepTests(unittest.TestCase):
    def test_isometric_jobs_preserve_formats_engines_and_animation_flags(self):
        with tempfile.TemporaryDirectory() as tmp:
            cases = Path(tmp)
            (cases / "case.json").write_text(json.dumps({"name": "case", "script": "a"}))
            with patch.object(sweep.references, "E2E_CASES", cases):
                normal, iso = sweep.corpus_jobs(), sweep.corpus_jobs(isometric=True)
        self.assertEqual(len(normal), 18)
        self.assertEqual(iso, [(*job[:4], (*job[4], "--isometric")) for job in normal])

    def test_summary_counts_rows_over_go_and_skips_go_failures(self):
        lines = sweep.summarize([row("png", "dagre", 0.5, 1.2), row("png", "dagre", 1.5, 0.8),
                                 row("png", "dagre", 9.0, 9.0, go_exit=1)])
        self.assertEqual(lines[1].split(), ["png", "dagre", "2", "1.00", "1", "1.00", "1"])

    def test_both_implementations_write_the_same_output_name(self):
        seen = []

        def fake_run(binary, engine, source, output, extra=()):
            seen.append((binary, output.name, list(extra)))
            output.write_bytes(b"x")
            return 0, 0.01, 0.01, 1000

        original = sweep.perf_bench.run
        sweep.perf_bench.run = fake_run
        try:
            with tempfile.TemporaryDirectory() as tmp:
                result = sweep.run_job(("case", "a -> b", "dagre", "gif", ("--animate-interval=125",)),
                                       (("go", Path("go")), ("rust", Path("rust"))), Path(tmp), 1)
        finally:
            sweep.perf_bench.run = original
        self.assertEqual([name for _, name, _ in seen], ["out.gif", "out.gif"])
        self.assertTrue(all(extra == ["--animate-interval=125"] for _, _, extra in seen))
        self.assertTrue(result["identical"])

    def run_failing_job(self, errors, partial=False):
        def fake_run(binary, engine, source, output, extra=()):
            if partial:
                output.write_bytes(b"partial output")
            return 1, 0.01, 0.01, 1000

        original_run, original_failure = sweep.perf_bench.run, sweep.failure_text
        sweep.perf_bench.run = fake_run
        sweep.failure_text = lambda binary, engine, directory, fmt, extra: errors[directory.name]
        try:
            with tempfile.TemporaryDirectory() as tmp:
                return sweep.run_job(("case", "a -> b", "dagre", "gif", ()),
                                     (("go", Path("go")), ("rust", Path("rust"))), Path(tmp), 1)
        finally:
            sweep.perf_bench.run, sweep.failure_text = original_run, original_failure

    def test_matching_errors_are_identical(self):
        result = self.run_failing_job({"go": "err: GIF frame pixels exceed the limit",
                                       "rust": "err: GIF frame pixels exceed the limit"})
        self.assertTrue(result["identical"])
        self.assertTrue(result["error_match"])
        self.assertEqual(result["outcome"], "error-match")

    def test_different_errors_are_not_identical(self):
        result = self.run_failing_job({"go": "err: GIF frame pixels exceed the limit",
                                       "rust": "err: something else"})
        self.assertFalse(result["identical"])
        self.assertFalse(result["error_match"])

    def test_matching_partial_outputs_still_compare_errors(self):
        result = self.run_failing_job({"go": "err: partial render written",
                                       "rust": "err: partial render written"}, partial=True)
        self.assertTrue(result["identical"])
        result = self.run_failing_job({"go": "err: partial render written",
                                       "rust": "err: other failure"}, partial=True)
        self.assertFalse(result["identical"])

    def test_board_directories_are_hashed(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "out.png"
            boards = Path(tmp) / "out"
            boards.mkdir()
            (boards / "index.png").write_bytes(b"one")
            first = sweep.output_digest(output)
            (boards / "index.png").write_bytes(b"two")
            self.assertIsNotNone(first)
            self.assertNotEqual(first, sweep.output_digest(output))

    def test_output_differences_in_earlier_repeats_are_not_hidden(self):
        calls = 0

        def fake_run(binary, engine, source, output, extra=()):
            nonlocal calls
            calls += 1
            output.write_bytes(b"different" if calls == 2 else b"same")
            return 0, 0.01, 0.01, 1000

        with patch.object(sweep.perf_bench, "run", fake_run), tempfile.TemporaryDirectory() as tmp:
            result = sweep.run_job(("case", "a", "dagre", "pptx", ()),
                                   (("go", Path("go")), ("rust", Path("rust"))), Path(tmp), 2)
        self.assertFalse(result["identical"])
        self.assertEqual(result["outcome"], "mismatch")

    def run_with_second_go_sample_different(self, fmt):
        calls = 0

        def fake_run(binary, engine, source, output, extra=()):
            nonlocal calls
            calls += 1
            output.write_bytes(b"other Go order" if calls == 3 else b"first Go order")
            return 0, 0.01, 0.01, 1000

        with patch.object(sweep.perf_bench, "run", fake_run), tempfile.TemporaryDirectory() as tmp:
            return sweep.run_job(("case", "a", "dagre", fmt, ()),
                                 (("go", Path("go")), ("rust", Path("rust"))), Path(tmp), 2)

    def test_pdf_rust_can_match_any_observed_go_sample(self):
        result = self.run_with_second_go_sample_different("pdf")
        self.assertTrue(result["identical"])
        self.assertEqual((result["outcome"], result["go_variants"]), ("match", 2))

    def test_pdf_rust_variant_unseen_in_go_is_a_mismatch(self):
        calls = 0

        def fake_run(binary, engine, source, output, extra=()):
            nonlocal calls
            calls += 1
            output.write_bytes({2: b"unseen order", 3: b"other Go order"}.get(calls, b"first Go order"))
            return 0, 0.01, 0.01, 1000

        with patch.object(sweep.perf_bench, "run", fake_run), tempfile.TemporaryDirectory() as tmp:
            result = sweep.run_job(("case", "a", "dagre", "pdf", ()),
                                   (("go", Path("go")), ("rust", Path("rust"))), Path(tmp), 2)
        self.assertFalse(result["identical"])
        self.assertEqual((result["outcome"], result["go_variants"]), ("mismatch", 2))

    def test_report_rows_carry_outcomes_and_pdf_go_variants(self):
        calls = 0

        def fake_run(binary, engine, source, output, extra=()):
            nonlocal calls
            calls += 1
            # Pass 1 makes calls 1-4; in pass 2 the second Go repeat of each job differs.
            output.write_bytes(b"other order" if calls in (7, 11) else b"order")
            return 0, 0.01, 0.01, 1000

        jobs = [("case", "a", "dagre", "svg", ()), ("case", "a", "dagre", "pdf", ())]
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            report = tmp / "report.json"
            argv = ["corpus-perf-sweep.py", "--go", str(tmp / "go"), "--rust", str(tmp / "rust"),
                    "--output", str(report), "--flag", "0", "--repeat", "2"]
            with patch.object(sweep.perf_bench, "run", fake_run), patch.object(sweep, "corpus_jobs", return_value=jobs), \
                    patch.object(sys, "argv", argv), patch("builtins.print") as printed:
                sweep.main()
            rows = json.loads(report.read_text())["rows"]
        self.assertEqual([(r["format"], r["repeat"], r["outcome"], r["go_variants"]) for r in rows],
                         [("svg", 2, "mapseed-oracle-missing", 2), ("pdf", 2, "match", 2)])
        lines = [call.args[0] for call in printed.call_args_list if call.args]
        self.assertIn("outcomes {'mapseed-oracle-missing': 1, 'match': 1}; max PDF Go variants 2", lines)
        self.assertIn("FAIL mapseed-oracle-missing: svg dagre case", lines)

    def test_non_pdf_go_disagreement_without_the_seeded_oracle_is_not_matched(self):
        for fmt in ("svg", "png", "pptx", "gif", "txt"):
            with self.subTest(fmt=fmt):
                result = self.run_with_second_go_sample_different(fmt)
                self.assertFalse(result["identical"])
                self.assertEqual((result["outcome"], result["go_variants"]), ("mapseed-oracle-missing", 2))

    def test_non_pdf_go_disagreement_is_resolved_by_a_seeded_output(self):
        calls = 0

        def fake_run(binary, engine, source, output, extra=()):
            nonlocal calls
            calls += 1
            output.write_bytes(b"other Go order" if calls == 3 else b"first Go order")
            return 0, 0.01, 0.01, 1000

        first = hashlib.sha256(b"first Go order").hexdigest()
        for table, expected, seeds in (({2: first}, "mapseed-match", [2]),
                                       ({}, "go-nondeterministic-no-rust-match", [])):
            calls = 0
            seeded = lambda oracle, engine, source, directory, fmt, extra, seed: table.get(seed)  # noqa: E731
            with patch.object(sweep.perf_bench, "run", fake_run), patch.object(sweep, "seeded_digest", seeded), \
                    tempfile.TemporaryDirectory() as tmp, tempfile.NamedTemporaryFile() as binary:
                oracle = sweep.mapseed.Oracle(Path(binary.name))
                result = sweep.run_job(("case", "a", "dagre", "svg", ()), (("go", Path("go")), ("rust", Path("rust"))),
                                       Path(tmp), 2, oracle, (1, 2, 3))
            self.assertEqual((result["outcome"], result["identical"], result["mapseed"]["mapseed_seeds"]),
                             (expected, expected == "mapseed-match", seeds))

    def test_board_directory_inventories_must_be_complete(self):
        calls = 0

        def fake_run(binary, engine, source, output, extra=()):
            nonlocal calls
            calls += 1
            boards = output.with_suffix("")
            boards.mkdir(exist_ok=True)
            for child in boards.iterdir():
                child.unlink()
            (boards / "index.svg").write_bytes(b"root")
            if calls != 4:
                (boards / "child.svg").write_bytes(b"child")
            return 0, 0.01, 0.01, 1000

        for fmt in ("svg", "pdf"):
            calls = 0
            with self.subTest(fmt=fmt), patch.object(sweep.perf_bench, "run", fake_run), \
                    tempfile.TemporaryDirectory() as tmp:
                result = sweep.run_job(("case", "a", "dagre", fmt, ()),
                                       (("go", Path("go")), ("rust", Path("rust"))), Path(tmp), 2)
                self.assertFalse(result["identical"])
                self.assertEqual(result["outcome"], "mismatch")

    def test_single_and_multiboard_pdf_use_the_same_date_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "out.pdf"
            data = b"%PDF-1.4\n/CreationDate (D:20261115000000)\n/ModDate (D:20261115000000)\nimage\n%%EOF"
            output.write_bytes(data)
            first = sweep.output_digest(output)
            output.write_bytes(data.replace(b"20261115000000", b"20261116000000"))
            self.assertEqual(first, sweep.output_digest(output))
            output.write_bytes(data.replace(b"image", b"other"))
            self.assertNotEqual(first, sweep.output_digest(output))
            output.unlink()
            boards = output.with_suffix("")
            boards.mkdir()
            self.assertIsNone(sweep.output_digest(output))
            child = boards / "child.pdf"
            child.write_bytes(data)
            first = sweep.output_digest(output)
            child.write_bytes(data.replace(b"20261115000000", b"20261116000000"))
            self.assertEqual(first, sweep.output_digest(output))
            child.write_bytes(data.replace(b"(D:20261115000000)\n/Mod", b"(D:2026111500000Z)\n/Mod"))
            self.assertNotEqual(first, sweep.output_digest(output))

    def test_empty_single_output_is_not_a_match(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "out.png"
            output.write_bytes(b"")
            with self.assertRaisesRegex(ValueError, "empty output"):
                sweep.output_digest(output)

    def test_every_case_gets_every_engine_and_export(self):
        with tempfile.TemporaryDirectory() as tmp:
            cases = Path(tmp)
            for name in ("one", "two"):
                (cases / f"{name}.json").write_text(json.dumps({"name": name, "script": "a -> b"}))
            with patch.object(sweep.references, "E2E_CASES", cases):
                jobs = sweep.corpus_jobs()
        expected = {(name, engine, fmt)
                    for name in ("one", "two")
                    for engine in ("dagre", "elk", "tala")
                    for fmt in ("svg", "png", "pdf", "pptx", "gif", "txt")}
        self.assertEqual(len(jobs), len(expected))
        self.assertEqual({(name, engine, fmt) for name, _, engine, fmt, _ in jobs}, expected)


if __name__ == "__main__":
    unittest.main()
