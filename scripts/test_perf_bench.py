"""Resource measurements must belong to one invocation, with no stale output."""
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("perf-bench.py")
spec = importlib.util.spec_from_file_location("perf_bench", SCRIPT)
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class PerfBenchTests(unittest.TestCase):
    def test_failed_cli_removes_previous_output_and_keeps_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.d2"
            source.write_text("a")
            output = Path(directory) / "old.svg"
            output.write_text("stale passing output")
            exit_code, wall, cpu, rss = bench.run(Path("/usr/bin/false"), "dagre", source, output)
            self.assertEqual(exit_code, 1)
            self.assertFalse(output.exists())
            self.assertGreater(wall, 0)
            self.assertGreaterEqual(cpu, 0)
            self.assertGreater(rss, 0)

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
