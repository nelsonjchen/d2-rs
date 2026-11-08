"""Resource measurements must belong to one invocation, with no stale output."""
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("perf-bench.py")
spec = importlib.util.spec_from_file_location("perf_bench", SCRIPT)
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class PerfBenchTests(unittest.TestCase):
    def test_isometric_gate_covers_both_formats_and_every_engine(self):
        with tempfile.TemporaryDirectory() as directory:
            cases = Path(directory)
            for name, _, _ in bench.ISOMETRIC_CASES:
                (cases / (name.replace("/", "__") + ".json")).write_text(
                    json.dumps({"name": name, "script": "a -> b"}))
            with patch.object(bench.references, "E2E_CASES", cases), patch.object(bench, "pick_cases", return_value=[]):
                jobs = list(bench.benchmark_jobs(12))
        self.assertEqual({job[3] for job in jobs}, set(bench.ENGINES))
        self.assertEqual({job[4] for job in jobs if job[1] == "real_world/fulcro_rad"}, {"svg", "png"})
        self.assertTrue(any(job[1:5] == ("patterns/paper", "a -> b", "dagre", "svg") for job in jobs))
        self.assertTrue(any(job[1:5] == ("stable/us_map", "a -> b", "dagre", "png") for job in jobs))
        self.assertEqual({job[3] for job in jobs if job[1] == "stable/grid_animated"}, set(bench.ENGINES))
        self.assertTrue(any(job[1:5] == ("regression/grid_oom", "a -> b", "dagre", "svg") for job in jobs))
        self.assertTrue(all(job[5] == ("--isometric",) for job in jobs))

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
