#!/usr/bin/env python3
"""Reject incomplete/corrupt raw raster captures and missing original operations."""

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "raster_score", Path(__file__).with_name("isometric-raster-scoreboard.py")
)
raster = importlib.util.module_from_spec(spec)
spec.loader.exec_module(raster)


class RasterGate(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.pixels = bytes([1, 2, 3, 255])
        self.digest = hashlib.sha256(self.pixels).hexdigest()
        self.image = {"width": 1, "height": 1, "stride": 4, "sha256": self.digest}
        self.file = self.directory / (self.digest + ".rgba")
        self.file.write_bytes(self.pixels)

    def verify(self):
        return raster.validate_image_store(self.directory, [{"frame": self.image}])

    def test_complete(self):
        self.verify()

    def test_empty_inventory(self):
        with self.assertRaises(AssertionError):
            raster.validate_image_store(self.directory, [])

    def test_missing_file(self):
        self.file.unlink()
        with self.assertRaises(AssertionError):
            self.verify()

    def test_extra_file(self):
        (self.directory / "extra.rgba").write_bytes(self.pixels)
        with self.assertRaises(AssertionError):
            self.verify()

    def test_truncated_file(self):
        self.file.write_bytes(self.pixels[:-1])
        with self.assertRaises(AssertionError):
            self.verify()

    def test_corrupt_same_size(self):
        self.file.write_bytes(bytes([1, 2, 4, 255]))
        with self.assertRaises(AssertionError):
            self.verify()

    def test_zero_width(self):
        self.image["width"] = 0
        with self.assertRaises(AssertionError):
            self.verify()

    def test_zero_height(self):
        self.image["height"] = 0
        with self.assertRaises(AssertionError):
            self.verify()

    def test_wrong_stride(self):
        self.image["stride"] = 8
        with self.assertRaises(AssertionError):
            self.verify()

    def test_wrong_extent(self):
        self.image["height"] = 2
        with self.assertRaises(AssertionError):
            self.verify()

    def test_missing_hash(self):
        self.image["sha256"] = ""
        with self.assertRaises(AssertionError):
            self.verify()

    def test_inconsistent_shared_binding(self):
        other = dict(self.image, height=2)
        with self.assertRaises(AssertionError):
            raster.validate_image_store(self.directory, [self.image, other])


class FunctionGate(unittest.TestCase):
    def setUp(self):
        self.score = raster.scoreboard()
        self.cases = [
            {
                "name": name,
                "operations": [
                    {"request": {"op": kind}, "got": {"error": ""}}
                    for kind, n in kinds.items()
                    for _ in range(n)
                ],
            }
            for name, kinds in raster.EXPECTED_OPERATIONS.items()
        ]

    def test_complete_inventory(self):
        self.score.validate_cases(self.cases)

    def test_missing_function(self):
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases[:-1])

    def test_duplicate_function(self):
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases + [self.cases[0]])

    def test_missing_operation(self):
        self.cases[0]["operations"].pop()
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_wrong_operation(self):
        self.cases[0]["operations"][0]["request"]["op"] = "unknown"
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_error_not_normalized(self):
        self.assertEqual(
            self.score.outcome({"error": "a\nb"}, {"error": "a b"}), "mismatch"
        )

    def test_unsupported_is_not_match(self):
        self.assertEqual(
            self.score.outcome({}, {"unsupported": "typed nil"}), "unsupported"
        )

    def test_partial_unsupported_cannot_hide_mismatch(self):
        self.assertEqual(
            raster.case_outcome(
                self.score,
                [{"error": ""}, {"error": "a"}],
                [{"unsupported": "typed nil"}, {"error": "b"}],
            ),
            "mismatch",
        )

    def test_partial_unsupported_stays_unsupported(self):
        self.assertEqual(
            raster.case_outcome(
                self.score,
                [{"error": ""}, {"error": "a"}],
                [{"unsupported": "typed nil"}, {"error": "a"}],
            ),
            "unsupported",
        )

    def test_missing_operation_result_cannot_pass(self):
        self.assertEqual(raster.case_outcome(self.score, [{}], []), "mismatch")

    def test_stride_integer_preserved(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "test.json"
            p.write_text('{"stride":9223372036854775807}')
            self.assertEqual(raster.load_original(p)["stride"], 2**63 - 1)


class ResultGate(RasterGate):
    def setUp(self):
        super().setUp()
        images = self.directory / "images"
        images.mkdir()
        self.file.rename(images / self.file.name)
        (self.directory / "0.json").write_text(
            json.dumps([{"error": "", "frame": self.image}])
        )
        self.cases = [{"operations": [{}]}]

    def verify(self):
        return raster.validate_actual(self.directory, self.cases)

    def test_missing_case(self):
        (self.directory / "0.json").unlink()
        with self.assertRaises(AssertionError):
            self.verify()

    def test_extra_case(self):
        (self.directory / "1.json").write_text("[]")
        with self.assertRaises(AssertionError):
            self.verify()

    def test_empty_operation_results(self):
        (self.directory / "0.json").write_text("[]")
        with self.assertRaises(AssertionError):
            self.verify()

    # The image-store-specific tests exercise its original direct fixture.
    test_empty_inventory = RasterGate.test_empty_inventory

    def test_missing_file(self):
        next((self.directory / "images").iterdir()).unlink()
        with self.assertRaises(AssertionError):
            self.verify()

    def test_extra_file(self):
        (self.directory / "images/extra.rgba").write_bytes(self.pixels)
        with self.assertRaises(AssertionError):
            self.verify()

    def test_truncated_file(self):
        next((self.directory / "images").iterdir()).write_bytes(self.pixels[:-1])
        with self.assertRaises(AssertionError):
            self.verify()

    def test_corrupt_same_size(self):
        next((self.directory / "images").iterdir()).write_bytes(bytes([1, 2, 4, 255]))
        with self.assertRaises(AssertionError):
            self.verify()

    def verify_changed_image(self):
        (self.directory / "0.json").write_text(
            json.dumps([{"error": "", "frame": self.image}])
        )
        return self.verify()

    def test_zero_width(self):
        self.image["width"] = 0
        with self.assertRaises(AssertionError):
            self.verify_changed_image()

    def test_zero_height(self):
        self.image["height"] = 0
        with self.assertRaises(AssertionError):
            self.verify_changed_image()

    def test_wrong_stride(self):
        self.image["stride"] = 8
        with self.assertRaises(AssertionError):
            self.verify_changed_image()

    def test_wrong_extent(self):
        self.image["height"] = 2
        with self.assertRaises(AssertionError):
            self.verify_changed_image()

    def test_missing_hash(self):
        self.image["sha256"] = ""
        with self.assertRaises(AssertionError):
            self.verify_changed_image()

    def test_inconsistent_shared_binding(self):
        values = [self.image, dict(self.image, height=2)]
        with self.assertRaises(AssertionError):
            raster.validate_image_store(self.directory / "images", values)


if __name__ == "__main__":
    unittest.main()
