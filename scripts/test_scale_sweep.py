"""The scale scoreboard must expose timeouts, missing outputs and every sample."""
import importlib.util
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
import struct
import zlib

sys.path.insert(0, str(Path(__file__).parent))
SCRIPT = Path(__file__).with_name("scale-sweep.py")
spec = importlib.util.spec_from_file_location("scale_sweep", SCRIPT)
sweep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sweep)


def sample(outcome="ok", digest="same", wall=1, cpu=1, rss=100):
    return {"outcome": outcome, "hash": digest, "wall": wall, "cpu": cpu, "rss": rss}


def row(go, rust, fmt="svg", repeat=3):
    return sweep.finish_row({"family": "chain", "size": 25, "engine": "dagre", "format": fmt,
                             "repeat": repeat, "go": {"samples": go}, "rust": {"samples": rust}})


class ScaleSweepTests(unittest.TestCase):
    def test_every_sample_must_match_even_when_last_matches(self):
        result = row([sample()] * 3, [sample(digest="different"), sample(), sample()])
        self.assertEqual(result["status"], "parity-mismatch")
        self.assertFalse(result["identical"])

    def test_missing_output_and_incomplete_samples_fail(self):
        self.assertEqual(row([sample()] * 3, [sample("missing-output")] * 3)["status"], "rust-failed")
        self.assertEqual(row([sample()], [sample()])["status"], "incomplete")
        self.assertEqual(row([sample("missing-output")], [sample()])["status"], "go-failed")

    def test_go_timeout_is_skipped_but_rust_timeout_is_failure(self):
        self.assertEqual(row([sample("timeout")], [])["status"], "skip-go-timeout")
        self.assertEqual(row([sample()] * 3, [sample("timeout")] * 3)["status"], "rust-failed")

    def test_go_budget_rejection_must_be_checked_against_rust(self):
        error = {**sample("error"), "exit": 1, "stderr": "frame pixels exceed limit"}
        self.assertEqual(row([error], [error])["status"], "skip-go-error")
        self.assertEqual(row([error], [sample()])["status"], "parity-exit-mismatch")
        different = {**error, "stderr": "different error"}
        self.assertEqual(row([error], [different])["status"], "parity-error-mismatch")
        # Equal displayed excerpts cannot hide a difference later in stderr.
        self.assertEqual(row([{**error, "stderr_hash": "first"}],
                             [{**error, "stderr_hash": "second"}])["status"], "parity-error-mismatch")

    def test_pdf_comparison_is_explicitly_incomparable(self):
        result = row([sample()] * 3, [sample(digest="different")] * 3, fmt="pdf")
        self.assertEqual(result["status"], "measured")
        self.assertIsNone(result["identical"])

    def test_medians_and_growth_expose_rising_ratio_below_go(self):
        small = row([sample()] * 3, [sample(wall=.2), sample(wall=.4), sample(wall=.9)])
        large = row([sample()] * 3, [sample(wall=.8)] * 3)
        large["size"] = 100
        self.assertEqual(small["wall_ratio"], .4)
        self.assertFalse(large["over_go"])
        trend, = sweep.trends([large, small])
        self.assertTrue(trend["rising"])
        self.assertEqual(trend["relative_growth"]["wall"], 2)

    def test_generators_are_deterministic_and_dense_has_more_edges(self):
        for family in sweep.FAMILIES + sweep.RASTER_FAMILIES:
            self.assertEqual(sweep.generate(family, 25), sweep.generate(family, 25))
        self.assertGreater(sweep.generate("dense", 100).count(" -> "), sweep.generate("sparse", 100).count(" -> "))

    def test_png_validation_requires_complete_pixels_and_trailer(self):
        def chunk(kind, data):
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)

        data = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(b"\x00\x00\x00\x00")) + chunk(b"IEND", b""))
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "out.png"
            path.write_bytes(data)
            sweep.validate_output(path, "png")
            for invalid in (data[:-12], data + b"trailing", data[:8]):
                path.write_bytes(invalid)
                with self.assertRaises(ValueError):
                    sweep.validate_output(path, "png")

    def worker(self, command, directory, timeout=2, limit=1024 * 1024):
        request = {"command": command, "directory": str(directory), "timeout": timeout,
                   "remaining": limit, "file_limit": limit}
        process = subprocess.run([sys.executable, str(SCRIPT), "--measure-child", json.dumps(request)],
                                 capture_output=True, text=True, check=True, timeout=5)
        return json.loads(process.stdout)

    def test_worker_timeout_reaps_process_and_reports_its_usage(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = self.worker([sys.executable, "-c", "import time; time.sleep(10)"], Path(tmp), timeout=.1)
        self.assertEqual(result["outcome"], "timeout")
        self.assertLess(result["wall"], 2)
        self.assertGreater(result["rss"], 0)

    def test_file_limit_is_a_failure_even_when_child_handles_write_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "out.svg"
            result = self.worker([sys.executable, "-c", "import sys; open(sys.argv[1], 'wb').write(b'x' * 65536)", str(output)],
                                 Path(tmp), limit=16384)
            self.assertLessEqual(output.stat().st_size, 16384)
        self.assertEqual(result["outcome"], "output-budget")

    def test_inventory_tolerates_staged_output_renamed_during_monitoring(self):
        renamed = Mock()
        renamed.stat.side_effect = FileNotFoundError("staged output renamed")
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            output = directory / "out.svg"
            output.write_text("complete")
            with patch.object(Path, "rglob", return_value=iter([renamed, output])):
                self.assertEqual(sweep.inventory(directory), {"out.svg": 8})

    def test_worker_crash_is_a_failed_sample_with_diagnostics_and_no_metrics(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            args = SimpleNamespace(timeout=1, file_mib=1)
            failed = sweep.run_sample(directory / "missing-cli", "dagre", "svg", directory / "input.d2",
                                      directory / "failed", args, sweep.MIB)
            self.assertEqual(failed["outcome"], "worker-failed")
            self.assertEqual(failed["worker_exit"], 1)
            self.assertIn("FileNotFoundError", failed["worker_stderr"])
            result = row([sample()], [failed], repeat=1)
            self.assertEqual(result["status"], "worker-failed")
            self.assertIsNone(result["cpu_ratio"])
            # The next sample still runs and is validated after the failure.
            binary = directory / "cli"
            binary.write_text(f"#!{sys.executable}\nimport pathlib, sys\n"
                              "pathlib.Path(sys.argv[-1]).write_text('<svg xmlns=\"http://www.w3.org/2000/svg\"/>')\n")
            binary.chmod(0o755)
            following = sweep.run_sample(binary, "dagre", "svg", directory / "input.d2",
                                         directory / "following", args, sweep.MIB)
            self.assertEqual(following["outcome"], "ok")

    def test_corrupt_worker_response_fails_instead_of_aborting(self):
        for response in ("not JSON", "{}", "null"):
            with self.subTest(response=response), tempfile.TemporaryDirectory() as tmp:
                directory = Path(tmp)
                worker = subprocess.CompletedProcess([], 0, stdout=response, stderr="worker diagnostic")
                with patch.object(sweep.subprocess, "run", return_value=worker):
                    result = sweep.run_sample(Path("cli"), "dagre", "svg", Path("input.d2"),
                                              directory / "sample", SimpleNamespace(timeout=1, file_mib=1), sweep.MIB)
                self.assertEqual(result["outcome"], "worker-failed")
                self.assertEqual(result["worker_stderr"], "worker diagnostic")
                self.assertEqual(result["worker_stdout"], response)

    def test_sweep_retains_failed_worker_row_and_continues_queue(self):
        failed = {**sample("worker-failed", wall=None, cpu=None, rss=None), "bytes": 0,
                  "worker_exit": 1, "worker_stderr": "original worker traceback"}
        ok = {**sample(), "bytes": 0}
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "sweep.json"
            argv = [str(SCRIPT), "--families=chain", "--sizes=2,3", "--engines=dagre", "--formats=svg",
                    "--repeat=1", "--output", str(output)]
            metadata = (f'{sweep.references.MANIFEST["d2"]["go_toolchain"]} '
                        f'vcs.revision={sweep.references.SOURCE_COMMIT} vcs.modified=false GOARCH=arm64')
            with (patch.object(sys, "argv", argv),
                  patch.object(sweep.platform, "machine", return_value="aarch64"),
                  patch.object(sweep, "binary_record", return_value={"version": sweep.references.MANIFEST["d2"]["base_tag"]}),
                  patch.object(sweep.references, "command", return_value="HEAD"),
                  patch.object(sweep.subprocess, "check_output", return_value=metadata),
                  patch.object(sweep, "run_sample", side_effect=[failed, ok, ok]),
                  contextlib.redirect_stdout(io.StringIO())):
                self.assertEqual(sweep.main(), 1)
            report = json.loads(output.read_text())
            self.assertTrue(report["complete"])
            self.assertEqual([r["status"] for r in report["rows"]], ["worker-failed", "measured"])
            self.assertEqual(report["rows"][0]["go"]["samples"][0]["worker_stderr"], "original worker traceback")


if __name__ == "__main__":
    unittest.main()
