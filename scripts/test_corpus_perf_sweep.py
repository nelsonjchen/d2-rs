"""The corpus sweep must time every format and give both CLIs the same output name."""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
SCRIPT = Path(__file__).with_name("corpus-perf-sweep.py")
spec = importlib.util.spec_from_file_location("corpus_perf_sweep", SCRIPT)
sweep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sweep)


def row(fmt, engine, wall_ratio, rss_ratio, go_exit=0):
    return {"format": fmt, "engine": engine, "wall_ratio": wall_ratio, "rss_ratio": rss_ratio,
            "go": {"exit": go_exit}}


class CorpusPerfSweepTests(unittest.TestCase):
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

    def test_every_case_gets_every_engine_and_export(self):
        formats = {fmt for fmt, _ in sweep.EXPORTS} | {"svg"}
        self.assertEqual(formats, {"svg", "png", "pdf", "pptx", "gif"})


if __name__ == "__main__":
    unittest.main()
