"""Fuzz outcomes must cover every board and reject incomplete captures."""
import base64
import json
from pathlib import Path
import sys
import tempfile
import unittest

from fuzz_common import EMPTY_INVENTORY, first_error, run


class FuzzCaptureTests(unittest.TestCase):
    def test_error_comparison_masks_random_board_output_nonce(self):
        directory = Path("/tmp/fuzz-case")
        go = "err: open /tmp/fuzz-case/.d2-board-output-651375696/layers/x.txt: no such file"
        rust = "err: open /tmp/fuzz-case/.d2-board-output-34282235957100/layers/x.txt: no such file"
        self.assertEqual(first_error(go, directory), first_error(rust, directory))

    def capture(self, directory, name, files):
        binary = directory / f"{name}.py"
        binary.write_text(
            f"#!{sys.executable}\nfrom pathlib import Path\n"
            f"files = {files!r}\n"
            "for name, data in files.items():\n"
            " p = Path(name); p.parent.mkdir(parents=True, exist_ok=True); p.write_text(data)\n"
        )
        binary.chmod(0o755)
        return run(binary, directory, "dagre", name)

    def capture_bytes(self, directory, name, files, output_format, render_flags=()):
        binary = directory / f"{name}.py"
        encoded = {path: base64.b64encode(data).decode() for path, data in files.items()}
        binary.write_text(
            f"#!{sys.executable}\nimport base64, sys\nfrom pathlib import Path\n"
            f"files = {encoded!r}\n"
            "for name, data in files.items():\n"
            " p = Path(name); p.parent.mkdir(parents=True, exist_ok=True); "
            "p.write_bytes(base64.b64decode(data))\n"
        )
        binary.chmod(0o755)
        return run(binary, directory, "dagre", name, output_format, render_flags)

    def test_complete_board_inventory_and_manifest_bytes_are_compared(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "input.d2").write_text("layers: {child: {a}}")
            files = {"output/index.svg": '<svg xmlns="http://www.w3.org/2000/svg"/>',
                     "output/child.svg": '<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>',
                     "output/.d2-board-output-manifest-v1.json": json.dumps({"version": 1})}
            original = self.capture(directory, "original", files)
            same = self.capture(directory, "same", files)
            changed = self.capture(directory, "changed", {**files, "output/child.svg": "<svg><path/></svg>"})
            missing = self.capture(directory, "missing", {k: v for k, v in files.items() if k != "output/child.svg"})
            manifest = self.capture(directory, "manifest", {**files, "output/.d2-board-output-manifest-v1.json": "{}"})
            self.assertEqual(original[0], 0)
            self.assertEqual(original[2], same[2])
            for outcome in (changed, missing, manifest):
                self.assertEqual(outcome[0], 0)
                self.assertNotEqual(original[2], outcome[2])

    def test_success_without_valid_svg_is_a_failed_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "input.d2").write_text("a")
            for index, files in enumerate(({"output.svg": ""}, {"output.svg": "<svg>"},
                                           {"output.svg": "<html/>"}, {"output.json": "{}"})):
                code, _, digest = self.capture(directory, f"bad{index}", files)
                self.assertEqual(code, "invalid-output")
                self.assertIsNone(digest)

    def test_empty_cli_output_is_distinct_from_missing_board(self):
        # Go intentionally emits no file for a folder-only diagram. Its empty
        # inventory must differ from a Rust render, and from a corrupt file.
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "input.d2").write_text("")
            empty = self.capture(directory, "empty", {})
            extra = self.capture(directory, "extra", {"output.svg": "<svg/>"})
            self.assertEqual(empty[0], 0)
            self.assertEqual(empty[2], EMPTY_INVENTORY)
            self.assertNotEqual(empty[2], extra[2])

    def test_selected_raster_format_is_validated_and_hashed(self):
        png = base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
        )
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "input.d2").write_text("a")
            valid = self.capture_bytes(directory, "png", {"output.png": png}, "png", ("--sketch",))
            invalid = self.capture_bytes(directory, "badpng", {"output.png": png[:-1]}, "png")
        self.assertEqual(valid[0], 0)
        self.assertIsNotNone(valid[2])
        self.assertEqual(invalid[0], "invalid-output")
        self.assertIsNone(invalid[2])


if __name__ == "__main__":
    unittest.main()
