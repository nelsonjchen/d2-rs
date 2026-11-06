"""Contracts for the raster parity report's changed-pixel evidence."""
import importlib.util
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "check_render_parity", Path(__file__).with_name("check-render-parity.py")
)
assert SPEC is not None and SPEC.loader is not None
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class RenderPixelDiffTests(unittest.TestCase):
    def test_summarizes_signed_deltas_and_half_open_bounds(self):
        expected = bytes([10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120])
        actual = bytes([11, 20, 28, 40, 50, 60, 70, 80, 90, 99, 111, 120])

        self.assertEqual(
            CHECKER.summarize_pixel_differences(2, 2, expected, actual),
            {
                "bounds": [0, 0, 2, 2],
                "pixel_delta_histogram": {"1,0,-2": 1, "-1,1,0": 1},
                "rows": [
                    {
                        "y": 0,
                        "changed_pixels": 1,
                        "x_min": 0,
                        "x_max": 0,
                        "delta_histogram": {"1,0,-2": 1},
                    },
                    {
                        "y": 1,
                        "changed_pixels": 1,
                        "x_min": 1,
                        "x_max": 1,
                        "delta_histogram": {"-1,1,0": 1},
                    },
                ],
            },
        )

    def test_exact_images_have_no_pixel_diff_record(self):
        pixels = bytes([1, 2, 3, 4, 5, 6])
        self.assertIsNone(CHECKER.summarize_pixel_differences(2, 1, pixels, pixels))


if __name__ == "__main__":
    unittest.main()
