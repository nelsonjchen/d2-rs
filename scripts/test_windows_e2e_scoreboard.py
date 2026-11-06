"""Output-inventory failure contracts for the Windows e2e scoreboard."""
import argparse
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "windows_e2e", Path(__file__).with_name("windows-e2e-scoreboard.py")
)
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class WindowsE2eContracts(unittest.TestCase):
    def test_missing_child_or_empty_output_cannot_match(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            golden = root / "e2etests/testdata/witness/dagre/sketch.exp.svg"
            golden.parent.mkdir(parents=True)
            golden.write_text("<svg/>")
            args = argparse.Namespace(
                wine="wine", go=Path("go.exe"), rust=Path("rust.exe"), timeout=1
            )
            case = dict(name="witness", expErr="", talaLayoutError="", themeID=None,
                        dagreFeatureError="", elkFeatureError="", hasMtexts=False,
                        script="a; layers: {one: {b}}")

            def render(command, **kwargs):
                binary = command[1]
                output = Path(command[-1].removeprefix("Z:"))
                output.write_text("<svg/>")
                if binary.endswith("go.exe"):
                    child = output.parent / "output/one.svg"
                    child.parent.mkdir()
                    child.write_text("<svg/>")
                return subprocess.CompletedProcess(command, 0, b"", b"")

            with patch.object(scoreboard, "UPSTREAM", root), patch.object(
                scoreboard.subprocess, "run", side_effect=render
            ):
                self.assertEqual(scoreboard.run_case(args, case, "dagre")["outcome"], "svg-mismatch")
            with patch.object(scoreboard, "UPSTREAM", root), patch.object(
                scoreboard.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"", b"")
            ):
                self.assertEqual(scoreboard.run_case(args, case, "dagre")["outcome"], "missing-output")
            with patch.object(scoreboard, "UPSTREAM", root), patch.object(
                scoreboard.subprocess, "run", side_effect=subprocess.TimeoutExpired("wine", 1)
            ):
                self.assertEqual(scoreboard.run_case(args, case, "dagre")["outcome"], "process-failed")
            args.budget_lock = threading.Lock()
            args.output_bytes = 0
            args.budget_exhausted = False
            with patch.object(scoreboard, "UPSTREAM", root), patch.object(
                scoreboard, "MAX_TOTAL", 1
            ), patch.object(scoreboard.subprocess, "run", side_effect=render) as process:
                self.assertEqual(scoreboard.run_case(args, case, "dagre")["outcome"], "output-budget-exceeded")
                self.assertEqual(process.call_count, 1)
                self.assertEqual(scoreboard.run_case(args, case, "dagre")["outcome"], "output-budget-exceeded")
                self.assertEqual(process.call_count, 1)

    def test_version_normalization_preserves_geometry_and_other_outputs(self):
        with tempfile.TemporaryDirectory() as scratch:
            left, right = Path(scratch) / "left", Path(scratch) / "right"
            left.mkdir()
            right.mkdir()
            for directory, version in [(left, "0.1.0"), (right, "v0.9.0")]:
                (directory / "output.svg").write_text(f'<svg data-d2-version="{version}" x="1"/>')
                (directory / "manifest.json").write_text('{"boards":["output.svg"]}')
            self.assertEqual(scoreboard.inventory(left)[0], scoreboard.inventory(right)[0])
            (left / "output.svg").write_text('<svg data-d2-version="0.1.0"><text>data-d2-version="label-one"</text></svg>')
            (right / "output.svg").write_text('<svg data-d2-version="v0.9.0"><text>data-d2-version="label-two"</text></svg>')
            self.assertNotEqual(scoreboard.inventory(left)[0], scoreboard.inventory(right)[0])
            (left / "output.svg").write_text('<svg data-d2-version="0.1.0" x="1"/>')
            (right / "output.svg").write_text('<svg data-d2-version="v0.9.0" x="2"/>')
            self.assertNotEqual(scoreboard.inventory(left)[0], scoreboard.inventory(right)[0])
            (right / "manifest.json").unlink()
            self.assertNotEqual(set(scoreboard.inventory(left)[0]), set(scoreboard.inventory(right)[0]))


if __name__ == "__main__":
    unittest.main()
