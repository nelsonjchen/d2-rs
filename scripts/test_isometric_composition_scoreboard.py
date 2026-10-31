#!/usr/bin/env python3
"""Inventory, pixel integrity and comparison failure gates for composition calls."""

import collections
import copy
import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path


def module(name):
    spec = importlib.util.spec_from_file_location(
        name, Path(__file__).with_name(name + ".py")
    )
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


composition = module("isometric-composition-scoreboard")
raster = module("isometric-raster-scoreboard")


class CompositionGates(unittest.TestCase):
    def setUp(self):
        self.score = composition.scoreboard()
        self.cases = [
            {
                "name": name,
                "operations": [
                    {"request": {"op": op}, "got": {}}
                    for op, count in ops.items()
                    for _ in range(count)
                ],
            }
            for name, ops in composition.TESTS.items()
        ]

    def test_exact_function_and_operation_inventory(self):
        self.assertEqual(len(self.cases), 20)
        self.assertEqual(sum(len(c["operations"]) for c in self.cases), 146)
        self.score.validate_cases(self.cases)
        for cases in [[], self.cases[:-1], self.cases + self.cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)
        for action in ["missing", "extra", "unknown"]:
            cases = copy.deepcopy(self.cases)
            ops = cases[0]["operations"]
            if action == "missing":
                ops.pop()
            elif action == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "unknown"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)

    def pixels(self, directory):
        data = bytes([128, 64, 0, 255])
        digest = hashlib.sha256(data).hexdigest()
        path = directory / (digest + ".rgba")
        path.write_bytes(data)
        value = {"width": 1, "height": 1, "stride": 4, "sha256": digest}
        return path, value

    def test_complete_pixel_inventory(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            path, value = self.pixels(directory)
            self.assertEqual(
                raster.validate_image_store(directory, [value]), {path.name: 4}
            )
            extra = directory / "extra.rgba"
            extra.write_bytes(b"")
            with self.assertRaises(AssertionError):
                raster.validate_image_store(directory, [value])
            extra.unlink()
            path.unlink()
            with self.assertRaises(AssertionError):
                raster.validate_image_store(directory, [value])

    def test_corrupt_and_truncated_pixels_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            path, value = self.pixels(directory)
            for data in [b"", b"\x80\x40\x00", b"\x81\x40\x00\xff"]:
                path.write_bytes(data)
                with self.assertRaises(AssertionError):
                    raster.validate_image_store(directory, [value])

    def test_empty_pixel_inventory_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(AssertionError):
                raster.validate_image_store(Path(tmp), [])

    def test_failure_and_unsupported_cannot_hide_mismatch(self):
        expected = [{"error": ""}, {"error": "canceled"}]
        self.assertEqual(raster.case_outcome(self.score, expected, None), "rust-failed")
        self.assertEqual(
            raster.case_outcome(self.score, expected, expected[:-1]), "mismatch"
        )
        actual = [{"unsupported": "no model"}, {"error": "wrong"}]
        self.assertEqual(raster.case_outcome(self.score, expected, actual), "mismatch")
        actual[1] = expected[1]
        self.assertEqual(
            raster.case_outcome(self.score, expected, actual), "unsupported"
        )

    def test_mesh_link_and_source_changes_fail(self):
        original = {
            "mesh": {"positions": [0.0, 1.0], "material": 1, "group": 2},
            "links": [{"url": "https://example.test", "points": [1.0]}],
            "source": {"stroke": "#123456"},
            "error": "",
        }
        self.assertEqual(self.score.outcome(original, original), "match")
        for keys in [
            ("mesh", "positions", 0),
            ("mesh", "material"),
            ("mesh", "group"),
            ("links", 0, "url"),
            ("links", 0, "points", 0),
            ("source", "stroke"),
            ("error",),
        ]:
            changed = copy.deepcopy(original)
            target = changed
            for key in keys[:-1]:
                target = target[key]
            value = target[keys[-1]]
            target[keys[-1]] = (
                value + 0.125 if isinstance(value, (float, int)) else "changed"
            )
            self.assertEqual(self.score.outcome(original, changed), "mismatch", keys)


if __name__ == "__main__":
    unittest.main()
