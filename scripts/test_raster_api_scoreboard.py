"""Fail-closed checks for the transparent raster API capture gate."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

from PIL import Image

spec = importlib.util.spec_from_file_location(
    "raster_api", Path(__file__).with_name("raster-api-scoreboard.py")
)
api = importlib.util.module_from_spec(spec)
spec.loader.exec_module(api)


class RasterAPIGate(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)

    def png(self, name, pixel=(10, 20, 30, 40), size=(2, 3)):
        path = self.root / name
        Image.new("RGBA", size, pixel).save(path)
        return path

    def test_exact_rgba(self):
        self.assertTrue(
            api.compare_frames(self.png("go.png"), self.png("rust.png"))["matched"]
        )

    def test_alpha_difference_fails(self):
        self.assertFalse(
            api.compare_frames(
                self.png("go.png"), self.png("rust.png", (10, 20, 30, 41))
            )["matched"]
        )

    def test_zero_alpha_rgb_difference_fails(self):
        self.assertFalse(
            api.compare_frames(
                self.png("go.png", (1, 2, 3, 0)), self.png("rust.png", (4, 5, 6, 0))
            )["matched"]
        )

    def test_dimensions_fail(self):
        self.assertFalse(
            api.compare_frames(self.png("go.png"), self.png("rust.png", size=(3, 2)))[
                "matched"
            ]
        )

    def test_missing_fails(self):
        self.assertFalse(
            api.compare_frames(self.png("go.png"), self.root / "absent.png")["matched"]
        )

    def test_empty_fails(self):
        actual = self.root / "empty.png"
        actual.touch()
        self.assertFalse(api.compare_frames(self.png("go.png"), actual)["matched"])

    def test_truncation_fails(self):
        expected, actual = self.png("go.png"), self.png("rust.png")
        actual.write_bytes(actual.read_bytes()[:-4])
        self.assertFalse(api.compare_frames(expected, actual)["matched"])

    def test_crc_fails(self):
        expected, actual = self.png("go.png"), self.png("rust.png")
        data = bytearray(actual.read_bytes())
        data[29] ^= 1
        actual.write_bytes(data)
        self.assertFalse(api.compare_frames(expected, actual)["matched"])

    def test_trailing_data_fails(self):
        expected, actual = self.png("go.png"), self.png("rust.png")
        actual.write_bytes(actual.read_bytes() + b"extra")
        self.assertFalse(api.compare_frames(expected, actual)["matched"])

    def capture(self):
        directory = self.root / "go/Test/000"
        directory.mkdir(parents=True)
        (directory / "case.json").write_text('{"test":"Test"}')
        (directory / "board.scene.json").write_text("{}")
        (directory / "board.options.json").write_text("{}")
        self.png("go/Test/000/board.png")
        return directory

    def test_complete_inventory(self):
        self.capture()
        self.assertEqual(len(api.capture_cases(self.root, {"Test": 1})), 1)

    def test_missing_input_fails(self):
        directory = self.capture()
        (directory / "board.scene.json").unlink()
        with self.assertRaises(AssertionError):
            api.capture_cases(self.root, {"Test": 1})

    def test_missing_outcome_fails(self):
        directory = self.capture()
        (directory / "board.png").unlink()
        with self.assertRaises(AssertionError):
            api.capture_cases(self.root, {"Test": 1})

    def test_ambiguous_outcome_fails(self):
        directory = self.capture()
        (directory / "board.err").write_text("error")
        with self.assertRaises(AssertionError):
            api.capture_cases(self.root, {"Test": 1})

    def test_extra_file_fails(self):
        directory = self.capture()
        (directory / "extra.png").write_bytes((directory / "board.png").read_bytes())
        with self.assertRaises(AssertionError):
            api.capture_cases(self.root, {"Test": 1})

    def test_missing_case_fails(self):
        self.capture()
        with self.assertRaises(AssertionError):
            api.capture_cases(self.root, {"Test": 2})

    def test_empty_inventory_fails(self):
        with self.assertRaises(AssertionError):
            api.capture_cases(self.root, {})


if __name__ == "__main__":
    unittest.main()
