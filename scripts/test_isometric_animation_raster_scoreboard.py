#!/usr/bin/env python3
"""Failure gates for exact animation raster inventories and raw texture inputs."""

import copy
import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "animation", Path(__file__).with_name("isometric-animation-raster-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AnimationGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.cases = [
            {
                "name": name,
                "operations": [
                    {
                        "request": {"op": "render", "ground_camera": None},
                        "got": {"error": ""},
                    }
                    for _ in range(count)
                ],
            }
            for name, count in module.EXPECTED_OPERATIONS.items()
        ]

    def test_full_inventory(self):
        self.score.validate_cases(self.cases)

    def test_missing_function(self):
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases[:-1])

    def test_duplicate_function(self):
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases + [self.cases[0]])

    def test_missing_gif_pass(self):
        self.cases[0]["operations"].pop()
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_extra_frame(self):
        self.cases[0]["operations"].append(
            copy.deepcopy(self.cases[0]["operations"][0])
        )
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_empty_observations(self):
        self.cases[0]["operations"] = []
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_ground_camera_is_not_dropped(self):
        del self.cases[0]["operations"][0]["request"]["ground_camera"]
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_complete_errors(self):
        self.assertEqual(
            self.score.outcome({"error": "a\nb"}, {"error": "a b"}), "mismatch"
        )


class TextureGates(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "textures").mkdir()
        self.data = bytes([20, 30, 40, 255])
        self.path = (
            self.root / "textures" / (hashlib.sha256(self.data).hexdigest() + ".pixels")
        )
        self.path.write_bytes(self.data)
        self.cases = [
            {
                "request": {
                    "kind": "rgba",
                    "rect": [4, 5, 5, 6],
                    "stride": 4,
                    "pixels_file": str(self.path),
                }
            }
        ]

    def verify(self):
        module.validate_textures(
            self.root, self.cases, lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
        )

    def test_complete_texture(self):
        self.verify()

    def test_missing_texture(self):
        self.path.unlink()
        with self.assertRaises(AssertionError):
            self.verify()

    def test_corrupt_texture(self):
        self.path.write_bytes(b"bad!")
        with self.assertRaises(AssertionError):
            self.verify()

    def test_extra_texture(self):
        (self.root / "textures" / "extra.pixels").write_bytes(b"x")
        with self.assertRaises(AssertionError):
            self.verify()

    def test_short_texture(self):
        self.cases[0]["request"]["rect"][3] += 1
        with self.assertRaises(AssertionError):
            self.verify()

    def test_bad_stride(self):
        self.cases[0]["request"]["stride"] = 3
        with self.assertRaises(AssertionError):
            self.verify()

    def test_unknown_storage(self):
        self.cases[0]["request"]["kind"] = "unknown"
        with self.assertRaises(AssertionError):
            self.verify()

    def test_symlink_texture(self):
        target = self.root / "original"
        self.path.rename(target)
        self.path.symlink_to(target)
        with self.assertRaises(AssertionError):
            self.verify()

    def test_path_escape(self):
        self.cases[0]["request"]["pixels_file"] = str(
            self.root / ".." / "escape.pixels"
        )
        with self.assertRaises(AssertionError):
            self.verify()

    def test_empty_inventory(self):
        self.cases = []
        with self.assertRaises(AssertionError):
            self.verify()


if __name__ == "__main__":
    unittest.main()
