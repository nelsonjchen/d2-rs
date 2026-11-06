#!/usr/bin/env python3
"""Failure gates for complete native content observations."""

import base64
import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "surface", Path(__file__).with_name("isometric-content-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SurfaceGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.cases = [
            {
                "name": name,
                "operations": [
                    {
                        "request": {"op": kind},
                        "got": {
                            "error": "",
                            "frame": {
                                "width": 1,
                                "height": 1,
                                "stride": 4,
                                "pixels": base64.b64encode(
                                    bytes([1, 2, 3, 255])
                                ).decode(),
                            },
                        },
                    }
                    for kind, count in module.EXPECTED_OPERATIONS[name].items()
                    for _ in range(count)
                ],
            }
            for name in self.score.TESTS
        ]

    def test_complete_inventory(self):
        self.score.validate_cases(self.cases)

    def test_missing_function(self):
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases[:-1])

    def test_duplicate_function(self):
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases + [self.cases[0]])

    def test_empty_observation(self):
        self.cases[0]["operations"] = []
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_missing_operation(self):
        self.cases[0]["operations"].pop()
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_missing_pixels(self):
        self.cases[0]["operations"][0]["got"]["frame"]["pixels"] = ""
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_corrupt_pixels(self):
        self.cases[0]["operations"][0]["got"]["frame"]["pixels"] = "not base64!"
        with self.assertRaises(ValueError):
            self.score.validate_cases(self.cases)

    def test_nonpremultiplied_pixels(self):
        self.cases[0]["operations"][0]["got"]["frame"]["pixels"] = base64.b64encode(
            bytes([255, 0, 0, 1])
        ).decode()
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_bad_stride(self):
        self.cases[0]["operations"][0]["got"]["frame"]["stride"] = 8
        with self.assertRaises(AssertionError):
            self.score.validate_cases(self.cases)

    def test_error_not_normalized(self):
        self.assertEqual(
            self.score.outcome({"error": "a\nb"}, {"error": "a b"}), "mismatch"
        )

    def test_full_pixel_comparison(self):
        a = self.cases[0]["operations"][0]["got"]
        b = copy.deepcopy(a)
        b["frame"]["pixels"] = base64.b64encode(bytes([1, 2, 4, 255])).decode()
        self.assertEqual(self.score.outcome(a, b), "mismatch")

    def test_missing_result(self):
        self.assertEqual(self.score.outcome({}, None), "rust-failed")


class NativeInventoryGates(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.cases = [{"operations": [{}]}]
        (self.directory / "0.json").write_text(json.dumps([{"error": ""}]))

    def test_complete(self):
        self.assertEqual(
            module.validate_actual(self.directory, self.cases), [[{"error": ""}]]
        )

    def test_empty_corpus(self):
        with self.assertRaises(AssertionError):
            module.validate_actual(self.directory, [])

    def test_missing_case(self):
        (self.directory / "0.json").unlink()
        with self.assertRaises(AssertionError):
            module.validate_actual(self.directory, self.cases)

    def test_extra_case(self):
        (self.directory / "extra.json").write_text("[]")
        with self.assertRaises(AssertionError):
            module.validate_actual(self.directory, self.cases)

    def test_missing_call(self):
        (self.directory / "0.json").write_text("[]")
        with self.assertRaises(AssertionError):
            module.validate_actual(self.directory, self.cases)

    def test_corrupt_result(self):
        (self.directory / "0.json").write_text("{")
        with self.assertRaises(ValueError):
            module.validate_actual(self.directory, self.cases)


class AuthoredNumberGates(unittest.TestCase):
    def test_negative_zero_input_survives_transport(self):
        import math

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "case.json"
            path.write_text('{"matrix":{"E":-0}}')
            value = module.load_original(path)
            self.assertEqual(math.copysign(1, value["matrix"]["E"]), -1)
            self.assertIn("-0.0", json.dumps(value))


if __name__ == "__main__":
    unittest.main()
