#!/usr/bin/env python3
"""Complete source-call inventories and exact geometry/error comparison gates."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "contact", Path(__file__).with_name("isometric-contact-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ContactGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.cases = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/contact-api.json")
            ).read_text()
        )["cases"]

    def operations(self, name):
        return next(c["operations"] for c in self.cases if c["name"] == name)

    def test_complete_inventory(self):
        self.assertEqual(len(self.cases), 20)
        self.assertEqual(sum(len(c["operations"]) for c in self.cases), 223)
        self.score.validate_cases(self.cases)
        for invalid in [[], self.cases[:-1], self.cases + self.cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(invalid)
        for mutation in ["missing", "extra", "unknown"]:
            invalid = copy.deepcopy(self.cases)
            ops = invalid[0]["operations"]
            if mutation == "missing":
                ops.pop()
            elif mutation == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "ignored"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(invalid)

    def test_complete_metadata_and_geometry(self):
        for case in self.cases:
            for operation in case["operations"]:
                expected = operation["got"]
                self.assertEqual(
                    self.score.outcome(expected, copy.deepcopy(expected)), "match"
                )
                self.assertEqual(self.score.outcome(expected, None), "rust-failed")
        operation = self.operations(
            "TestRouteCasingClearsTerracesWithoutMovingInkOrBridge"
        )[0]
        expected = operation["got"]
        for mutation in ["point", "normal", "material", "inventory", "error"]:
            invalid = copy.deepcopy(expected)
            triangles = invalid["builder"]["triangles"]
            if mutation == "point":
                triangles[0]["vertices"][0]["Position"]["x"] += 2**-30
            elif mutation == "normal":
                triangles[0]["vertices"][0]["Normal"]["x"] += 2**-30
            elif mutation == "material":
                triangles[0]["material"]["unlit"] = not triangles[0]["material"][
                    "unlit"
                ]
            elif mutation == "inventory":
                triangles.pop()
            else:
                invalid["builder"]["error"] = "unexpected cancellation"
            self.assertEqual(self.score.outcome(expected, invalid), "mismatch")

    def test_cancellation_and_nonfinite(self):
        ops = self.operations("TestQueueContactRoutesPreserveSourceVerticesAndBounds")
        canceled = next(
            o for o in ops if o["request"].get("context") == "context canceled"
        )
        self.assertEqual(canceled["got"]["error"], "context canceled")
        self.assertIsNone(canceled["got"]["paths"])
        wrong = copy.deepcopy(canceled["got"])
        wrong["error"] = ""
        self.assertEqual(self.score.outcome(canceled["got"], wrong), "mismatch")
        nonfinite = self.operations("TestUprightContactsDoNotInventExteriorRoutes")[-1]
        self.assertEqual(nonfinite["request"]["point"][1], "NaN")
        self.assertEqual(nonfinite["got"]["point"][1], "NaN")
        self.assertFalse(nonfinite["got"]["moved"])

    def test_mesh_sharing_and_support_inputs(self):
        ops = self.operations("TestProjectedCaptionMeshIncludesFinalOutsideSurfaces")
        mesh = next(
            o["request"]["mesh"] for o in ops if o["request"]["op"] == "avoidMesh"
        )
        self.assertTrue(any(m["texture"] for m in mesh["materials"]))
        self.assertLess(len(mesh["materials"]), len(mesh["triangles"]))
        result = next(o["got"] for o in ops if o["request"]["op"] == "avoidMesh")
        self.assertEqual(len(result["rects"]), 2)
        wrong = copy.deepcopy(result)
        wrong["rects"][0]["rect"]["hull"].pop()
        self.assertEqual(self.score.outcome(result, wrong), "mismatch")
        supported = self.operations(
            "TestSolidContactsFollowLowerSupportsAtOriginalRouteHeight"
        )
        requests = [o["request"] for o in supported if o["request"]["op"] == "contacts"]
        self.assertEqual(len(requests), 12)
        self.assertEqual(sum(bool(r["support"]) for r in requests), 6)


if __name__ == "__main__":
    unittest.main()
