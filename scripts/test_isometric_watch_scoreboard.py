#!/usr/bin/env python3
"""Failure gates for real isometric watch captures."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "watch_score", Path(__file__).with_name("isometric-watch-scoreboard.py")
)
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)


class Gates(unittest.TestCase):
    def test_inventory_includes_every_named_phase(self):
        cases = score.cases()
        names = [
            case["name"] + "/" + step["name"]
            for case in cases
            for step in case["steps"]
        ]
        self.assertEqual(len(names), 20)
        self.assertEqual(len(set(names)), 20)
        self.assertIn("source-reload/restored", names)
        self.assertIn("png-folder-navigation/second", names)

    def test_missing_preview_or_output_never_passes(self):
        for row in (
            {"message": {}, "outputs": {"out.svg": "a"}},
            {
                "message": {"svg": '<svg xmlns="http://www.w3.org/2000/svg"/>'},
                "outputs": {},
            },
            {"message": {"err": "failed", "svg": "x"}, "outputs": {"out.svg": "a"}},
        ):
            self.assertEqual(
                score.compare_step(
                    {"isometric": True}, row, row, Path("."), Path("."), "svg"
                ),
                "render-failed",
            )

    def test_equal_corrupt_preview_never_passes(self):
        row = {"message": {"svg": "broken"}, "outputs": {"out.svg": "a"}}
        with self.assertRaises(score.ET.ParseError):
            score.compare_step(
                {"isometric": True}, row, row, Path("."), Path("."), "svg"
            )

    def test_unexpected_mode_never_passes(self):
        row = {
            "message": {"svg": '<svg xmlns="http://www.w3.org/2000/svg"/>'},
            "outputs": {"out.svg": "a"},
        }
        self.assertEqual(
            score.compare_step(
                {"isometric": True}, row, row, Path("."), Path("."), "svg"
            ),
            "mode-mismatch",
        )

    def test_extra_or_changed_outputs_never_pass(self):
        row = {
            "message": {
                "svg": '<svg xmlns="http://www.w3.org/2000/svg"><title>D2 isometric diagram</title></svg>'
            },
            "outputs": {"out.svg": "a"},
        }
        for outputs in ({"out.svg": "b"}, {"out.svg": "a", "extra.svg": "b"}):
            other = dict(row, outputs=outputs)
            self.assertEqual(
                score.compare_step(
                    {"isometric": True}, row, other, Path("."), Path("."), "svg"
                ),
                "artifact-mismatch",
            )

    def test_equal_corrupt_png_calls_shared_gate(self):
        row = {
            "message": {
                "svg": '<svg xmlns="http://www.w3.org/2000/svg"><title>D2 isometric diagram</title></svg>'
            },
            "outputs": {"out.png": "a"},
        }
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "files").mkdir()
            (root / "files/out.png").write_bytes(b"corrupt")
            with self.assertRaises(ValueError):
                score.compare_step({"isometric": True}, row, row, root, root, "png")

    def test_snapshot_keeps_unexpected_staging_files(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            work = root / "work"
            work.mkdir()
            (work / ".incomplete").write_bytes(b"temporary")
            self.assertEqual(
                set(score.snapshot(work, root / "capture")), {".incomplete"}
            )

    def test_watch_single_file_and_prior_tree_both_use_png_gates(self):
        from PIL import Image

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            files = root / "files"
            (files / "out").mkdir(parents=True)
            Image.new("RGB", (2, 2), "white").save(files / "out.png")
            Image.new("RGB", (2, 2), "red").save(files / "out/child.png")
            manifest = files / "out" / score.RASTER.BOARD_OUTPUT_MANIFEST
            manifest.write_text(
                json.dumps(
                    {
                        "format": "d2-board-output",
                        "version": 1,
                        "files": [
                            {
                                "path": "child.png",
                                "sha256": score.digest(files / "out/child.png"),
                            }
                        ],
                        "directories": [],
                    }
                )
            )
            row = {
                "message": {
                    "svg": '<svg xmlns="http://www.w3.org/2000/svg"><title>D2 isometric diagram</title></svg>'
                },
                "outputs": {
                    str(p.relative_to(files)): score.digest(p)
                    for p in files.rglob("*")
                    if p.is_file()
                },
            }
            self.assertEqual(
                score.compare_step({"isometric": True}, row, row, root, root, "png"),
                "source-match",
            )
            (files / "out/child.png").unlink()
            with self.assertRaises(ValueError):
                score.compare_step({"isometric": True}, row, row, root, root, "png")

    def test_budget_exhaustion_and_early_exit_fail(self):
        with (
            patch.object(score.ISO, "direct_guard", return_value="budget-exceeded"),
            self.assertRaisesRegex(RuntimeError, "budget-exceeded"),
        ):
            score.check_guard(Path("."))

        class Exited:
            returncode = 1

            def poll(self):
                return 1

        with (
            patch.object(score, "check_guard"),
            self.assertRaisesRegex(RuntimeError, "watch exited early"),
        ):
            score.wait_for(lambda: None, Exited(), Path("."))


if __name__ == "__main__":
    unittest.main()
