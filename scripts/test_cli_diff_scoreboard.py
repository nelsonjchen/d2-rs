#!/usr/bin/env python3
"""Clock controls must inject both processes without normalizing their output."""
import importlib.util
import os
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
spec = importlib.util.spec_from_file_location("cli_diff", Path(__file__).with_name("cli-diff-scoreboard.py"))
cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cli)


class ClockTests(unittest.TestCase):
    def test_ordinary_run_removes_inherited_clock(self):
        with patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "946782245"}), patch.object(cli.subprocess, "run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, b"", b"")
            cli.run(Path("/binary"), "fmt --check uf.d2", Path("/work"))
            self.assertNotIn("SOURCE_DATE_EPOCH", run.call_args.kwargs["env"])

    def test_clock_is_explicit_and_complete_logs_survive(self):
        output = b"\x1b[2m2000-01-02T03:04:05Z\x1b[0m \x1b[33mWARN \x1b[0m /work/uf.d2\n"
        with patch.object(cli.subprocess, "run") as run:
            run.return_value = subprocess.CompletedProcess([], 1, output, b"err: found 1 unformatted file. Run d2 fmt to fix.\n")
            result = cli.run(Path("/binary"), "fmt --check uf.d2", Path("/work"), 946782245)
            self.assertEqual(run.call_args.kwargs["env"]["SOURCE_DATE_EPOCH"], "946782245")
            self.assertEqual(result["stdout"].encode(), output)
            self.assertEqual(result["exit"], 1)
            self.assertEqual(result["stderr"], "err: found 1 unformatted file. Run d2 fmt to fix.\n")

    def test_clock_cannot_claim_uninstrumented_oracle(self):
        with patch.object(sys, "argv", ["cli-diff-scoreboard.py", "unused.json", "--epoch", "946782245"]):
            with self.assertRaises(SystemExit) as error:
                cli.main()
            self.assertEqual(error.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
