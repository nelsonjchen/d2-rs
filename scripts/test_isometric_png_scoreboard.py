"""The PNG tier rejects incomplete/corrupt/extra evidence and tolerance matches."""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import isometric_png_scoreboard as png_score

spec = importlib.util.spec_from_file_location("png_helpers", Path(__file__).with_name("test_render_artifact_gate.py"))
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)

class PNGScoreboardTests(unittest.TestCase):
    def test_exact_pixels_even_when_encodings_differ(self):
        with tempfile.TemporaryDirectory() as directory:
            a, b = Path(directory) / "a.png", Path(directory) / "b.png"
            a.write_bytes(helpers.png(level=1))
            b.write_bytes(helpers.png(level=9))
            self.assertNotEqual(a.read_bytes(), b.read_bytes())
            self.assertTrue(png_score.strict_pixels(a, b)["matched"])
            b.write_bytes(helpers.png(pixels=b"\x10\x20\x31"))
            self.assertFalse(png_score.strict_pixels(a, b)["matched"])

    def test_missing_truncated_and_empty_are_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            a, b = Path(directory) / "a.png", Path(directory) / "b.png"
            a.write_bytes(helpers.png())
            self.assertFalse(png_score.strict_pixels(a, b)["matched"])
            for data in (b"", helpers.png()[:-1], b"bad PNG"):
                b.write_bytes(data)
                self.assertFalse(png_score.strict_pixels(a, b)["matched"])

    def test_process_failure_cannot_pass_existing_pixels(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = root / "expected" / "one.exp.png"
            expected.parent.mkdir()
            expected.write_bytes(helpers.png())
            actual = root / "go" / "one.png"
            actual.parent.mkdir()
            actual.write_bytes(helpers.png())
            extra = actual.with_name("extra.png")
            extra.write_bytes(helpers.png())
            with patch.object(png_score, "inventory", return_value=(root, expected.parent, [expected])):
                rows = png_score.score(root, root / "attempt", root / "runner", None, 1, None)
                self.assertEqual({r["board"]: r["outcome"] for r in rows}, {"one": "incomplete-run", "extra": "go-extra"})
                rows = png_score.score(root, root / "attempt", root / "runner", None, 0, "budget-exceeded")
                self.assertEqual(rows[1]["outcome"], "incomplete-run")

    def test_golden_inventory_is_nonempty_and_complete(self):
        _, _, goldens = png_score.inventory()
        self.assertEqual(len(goldens), 20)
        self.assertEqual(len(set(goldens)), 20)

if __name__ == "__main__":
    unittest.main()
