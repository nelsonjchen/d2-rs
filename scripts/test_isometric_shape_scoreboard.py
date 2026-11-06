#!/usr/bin/env python3
"""Reject incomplete original-call inventories and changed geometry/paint/errors."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "shapes", Path(__file__).with_name("isometric-shape-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ShapeGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.cases = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/shape-api.json")
            ).read_text()
        )["cases"]

    def test_complete_inventory(self):
        cases = [
            {
                "name": name,
                "operations": [
                    {"request": {"op": op}} for op, n in ops.items() for _ in range(n)
                ],
            }
            for name, ops in module.TESTS.items()
        ]
        self.assertEqual(len(cases), 20)
        self.assertEqual(sum(len(c["operations"]) for c in cases), 187)
        self.score.validate_cases(cases)
        for bad in [[], cases[:-1], cases + cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(bad)
        for change in ["missing", "extra", "unknown"]:
            bad = copy.deepcopy(cases)
            if change == "missing":
                bad[0]["operations"].pop()
            elif change == "extra":
                bad[0]["operations"].append(bad[0]["operations"][0])
            else:
                bad[0]["operations"][0]["request"]["op"] = "unsupported"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(bad)

    def test_complete_geometry_and_identity(self):
        e = next(
            c
            for c in self.cases
            if c["name"] == "TestNativeCanonicalInkIsPrintedOverContinuousMatteSides"
        )["operations"][0]["got"]
        for change in [
            "point",
            "normal",
            "uv",
            "material",
            "sharing",
            "inventory",
            "budget",
            "error",
        ]:
            a = copy.deepcopy(e)
            t = a["state"]["mesh"]["triangles"][0]
            if change in ["point", "normal"]:
                t["v"][0]["Position" if change == "point" else "Normal"]["x"] += 2**-30
            elif change == "uv":
                t["v"][0]["U"] += 2**-30
            elif change == "material":
                a["state"]["mesh"]["materials"][0]["roughness"] += 2**-30
            elif change == "sharing":
                t["material"] += 1
            elif change == "inventory":
                a["state"]["mesh"]["triangles"].pop()
            elif change == "budget":
                a["state"]["pixels"] += 1
            else:
                a["state"]["error"] = "changed"
            self.assertEqual(self.score.outcome(e, a), "mismatch")
        self.assertEqual(self.score.outcome(e, e), "match")
        self.assertEqual(self.score.outcome(e, None), "rust-failed")

    def test_texture_and_cancellation(self):
        ops = next(
            c
            for c in self.cases
            if c["name"] == "TestNativeFaceTextureBudgetAndCancellation"
        )["operations"]
        canceled = next(o for o in ops if o["request"].get("canceled"))
        self.assertIn("context canceled", canceled["got"]["state"]["error"])
        e = next(
            o["got"]
            for o in ops
            if o["request"]["op"] == "face" and o["got"]["texture"]
        )
        for key in ["width", "height", "sha256"]:
            a = copy.deepcopy(e)
            a["texture"][key] = "changed"
            self.assertEqual(self.score.outcome(e, a), "mismatch")


if __name__ == "__main__":
    unittest.main()
