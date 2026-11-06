#!/usr/bin/env python3
"""Complete bounded inventories and exact solid/caption comparison gates."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "solid", Path(__file__).with_name("isometric-solid-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SolidGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.fixture = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/solid-api.json")
            ).read_text()
        )["cases"]

    def test_complete_inventory(self):
        cases = [
            {
                "name": name,
                "operations": [
                    {"request": {"op": op}}
                    for op, count in operations.items()
                    for _ in range(count)
                ],
            }
            for name, operations in module.TESTS.items()
        ]
        self.assertEqual(len(cases), 20)
        self.assertEqual(sum(len(c["operations"]) for c in cases), 260)
        self.score.validate_cases(cases)
        for invalid in [[], cases[:-1], cases + cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(invalid)
        for change in ["missing", "extra", "unknown"]:
            invalid = copy.deepcopy(cases)
            ops = invalid[0]["operations"]
            if change == "missing":
                ops.pop()
            elif change == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "ignored"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(invalid)

    def test_full_geometry_and_metadata(self):
        expected = next(
            c
            for c in self.fixture
            if c["name"] == "TestRouteCaptionKnockoutPreservesTriangleMetadata"
        )["operations"][0]["got"]
        for change in [
            "point",
            "normal",
            "uv",
            "bias",
            "group",
            "owner",
            "alpha",
            "inventory",
            "error",
        ]:
            actual = copy.deepcopy(expected)
            t = actual["mesh"]["triangles"][0]
            if change in {"point", "normal"}:
                t["v"][0]["Position" if change == "point" else "Normal"]["x"] += 2**-30
            elif change == "uv":
                t["v"][0]["U"] += 2**-30
            elif change in {"bias", "group", "owner", "alpha"}:
                t[change] += 1
            elif change == "inventory":
                actual["mesh"]["triangles"].pop()
            else:
                actual["error"] = "unexpected"
            self.assertEqual(self.score.outcome(expected, actual), "mismatch")
        self.assertEqual(self.score.outcome(expected, expected), "match")
        self.assertEqual(self.score.outcome(expected, None), "rust-failed")

    def test_complete_texture_hashes_and_sharing(self):
        expected = next(
            c
            for c in self.fixture
            if c["name"] == "TestNativeSolidsKeepGeometryAcrossPaintStyles"
        )["operations"][0]["got"]
        for change in ["pixels", "width", "material", "sharing"]:
            actual = copy.deepcopy(expected)
            materials = actual["state"]["mesh"]["materials"]
            textured = next(m for m in materials if m["texture"])
            if change == "pixels":
                textured["texture"]["sha256"] = "0" * 64
            elif change == "width":
                textured["texture"]["width"] += 1
            elif change == "material":
                textured["roughness"] += 2**-30
            else:
                actual["state"]["mesh"]["triangles"][0]["material"] = 0
            self.assertEqual(self.score.outcome(expected, actual), "mismatch")


if __name__ == "__main__":
    unittest.main()
