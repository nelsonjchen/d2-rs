"""The corpus sweep must time every format and give both CLIs the same output name."""
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
        self.assertEqual(len(normal), 7)
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

    def test_empty_single_output_is_not_a_match(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "out.png"
            output.write_bytes(b"")
            with self.assertRaisesRegex(ValueError, "empty output"):
                sweep.output_digest(output)

    def test_every_case_gets_every_engine_and_export(self):
        formats = {fmt for fmt, _ in sweep.EXPORTS} | {"svg"}
        self.assertEqual(formats, {"svg", "png", "pdf", "pptx", "gif"})


if __name__ == "__main__":
    unittest.main()
