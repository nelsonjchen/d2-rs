#!/usr/bin/env python3
"""Failure gates for the target-selection SVG source corpus."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "target_links", Path(__file__).with_name("target-link-scoreboard.py")
)
SCORE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCORE)


class Gates(unittest.TestCase):
    def test_missing_empty_or_corrupt_svg_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            for data in (None, b"", b"not SVG", b"<html/>"):
                with self.subTest(data=data):
                    if data is not None:
                        (directory / "out.svg").write_bytes(data)
                    with self.assertRaises((ValueError, SCORE.ET.ParseError)):
                        SCORE.validate(directory, {})

    def test_inventory_includes_empty_and_unexpected_files(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "out.svg").write_text(
                '<svg xmlns="http://www.w3.org/2000/svg"/>'
            )
            (directory / "unexpected").touch()
            (directory / "stderr.log").touch()
            result = {}
            with self.assertRaises(SCORE.ET.ParseError):
                SCORE.validate(directory, result)
            self.assertEqual(set(result["outputs"]), {"out.svg", "unexpected"})

    def test_extensionless_svg_and_empty_stdout_are_retained(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "stdout.svg").touch()
            (directory / "detail").write_text(
                '<svg xmlns="http://www.w3.org/2000/svg"/>'
            )
            result = {}
            SCORE.validate(directory, result)
            self.assertEqual(set(result["outputs"]), {"stdout.svg", "detail"})

    def test_manifest_hashes_and_complete_inventory_are_required(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "out.svg").write_text(
                '<svg xmlns="http://www.w3.org/2000/svg"/>'
            )
            manifest = directory / SCORE.CLI.RASTER.BOARD_OUTPUT_MANIFEST
            record = {
                "format": "d2-board-output",
                "version": 1,
                "files": [],
                "directories": [],
            }
            manifest.write_text(SCORE.json.dumps(record))
            with self.assertRaisesRegex(ValueError, "inventory"):
                SCORE.validate(directory, {})
            record["files"] = [{"path": "out.svg", "sha256": "wrong"}]
            manifest.write_text(SCORE.json.dumps(record))
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                SCORE.validate(directory, {})


if __name__ == "__main__":
    unittest.main()
