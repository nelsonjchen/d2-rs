"""Low-level raw raster captures require every complete nonempty frame."""

import base64
import copy
import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "raster_api", Path(__file__).with_name("isometric-raster-api-scoreboard.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
SCORE = MODULE.scoreboard()


def cases():
    frame = {"width": 1, "height": 1, "stride": 4, "pixels": "AAAA/w=="}
    result = []
    for name, counts in SCORE.OPS.items():
        operations = []
        for index in range(counts["new"]):
            operations.append(
                {
                    "request": {"op": "new", "id": index + 1},
                    "got": {"frame": copy.deepcopy(frame), "error": ""},
                }
            )
        for _ in range(counts.get("frame", 0)):
            operations.append(
                {
                    "request": {"op": "frame", "id": 1},
                    "got": {"frame": copy.deepcopy(frame), "error": ""},
                }
            )
        result.append({"name": name, "operations": operations})
    return result


class RasterCaptureTests(unittest.TestCase):
    def test_complete_inventory(self):
        SCORE.validate_cases(cases())

    def test_missing_or_extra_function_fails(self):
        for changed in [cases()[:-1], cases() + [cases()[0]]]:
            with self.assertRaises(AssertionError):
                SCORE.validate_cases(changed)

    def test_missing_or_extra_frame_fails(self):
        for extra in [False, True]:
            changed = cases()
            if extra:
                changed[0]["operations"].append(
                    copy.deepcopy(changed[0]["operations"][0])
                )
            else:
                changed[0]["operations"].pop()
            with self.assertRaises(AssertionError):
                SCORE.validate_cases(changed)

    def test_empty_corrupt_short_or_extra_pixels_fail(self):
        for pixels in [
            "",
            "bad!",
            base64.b64encode(bytes(3)).decode(),
            base64.b64encode(bytes(5)).decode(),
        ]:
            changed = cases()
            changed[0]["operations"][0]["got"]["frame"]["pixels"] = pixels
            with self.assertRaises((AssertionError, ValueError)):
                SCORE.validate_cases(changed)

    def test_missing_or_invalid_dimensions_fail(self):
        for frame in [
            None,
            {"width": 0, "height": 1, "stride": 0, "pixels": ""},
            {"width": 1, "height": 1, "stride": 8, "pixels": "AAAA/w=="},
        ]:
            changed = cases()
            changed[0]["operations"][0]["got"]["frame"] = frame
            with self.assertRaises(AssertionError):
                SCORE.validate_cases(changed)

    def test_unknown_raster_id_fails(self):
        changed = cases()
        changed[0]["operations"][-1]["request"]["id"] = 999
        with self.assertRaises(AssertionError):
            SCORE.validate_cases(changed)

    def test_complete_bytes_and_errors_compare(self):
        expected = [operation["got"] for operation in cases()[0]["operations"]]
        self.assertEqual(SCORE.outcome(expected, expected), "match")
        for mutation in ["pixels", "error", "missing", "extra"]:
            actual = copy.deepcopy(expected)
            if mutation == "pixels":
                actual[0]["frame"]["pixels"] = "AQAA/w=="
            elif mutation == "error":
                actual[0]["error"] = "context canceled"
            elif mutation == "missing":
                actual.pop()
            else:
                actual.append(actual[0])
            self.assertEqual(SCORE.outcome(expected, actual), "mismatch")
        self.assertEqual(SCORE.outcome(expected, None), "rust-failed")
        self.assertEqual(
            SCORE.outcome(expected, {"unsupported": "missing API"}), "unsupported"
        )


if __name__ == "__main__":
    unittest.main()
