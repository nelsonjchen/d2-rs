#!/usr/bin/env python3
"""Failure gates for the original SVG-coverage and receiver observations."""
import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "coverage", Path(__file__).with_name("isometric-coverage-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CoverageGates(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.score = module.scoreboard()
        cls.cases = json.loads((cached_fixture("crates/d2-rs-isometric/tests/fixtures/coverage-api.json")).read_text())["cases"]

    def observation(self, name, op=None):
        case = next(c for c in self.cases if c["name"] == name)
        return next(o for o in case["operations"] if op is None or o["request"]["op"] == op)

    def changed(self, expected, paths):
        self.assertEqual(self.score.outcome(expected, expected), "match")
        self.assertEqual(self.score.outcome(expected, None), "rust-failed")
        for path in paths:
            got = copy.deepcopy(expected)
            value = got
            for k in path[:-1]:
                value = value[k]
            before = value[path[-1]]
            value[path[-1]] = (not before if isinstance(before, bool) else
                before + 2**-30 if isinstance(before, (int, float)) else "changed")
            self.assertEqual(self.score.outcome(expected, got), "mismatch", path)

    def test_complete_inventory(self):
        self.assertEqual(len(self.cases), 20)
        self.assertEqual(sum(len(c["operations"]) for c in self.cases), 1053)
        self.score.validate_cases(self.cases)
        for cases in [[], self.cases[:-1], self.cases + self.cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)
        for change in ["missing", "extra", "unknown"]:
            cases = copy.deepcopy(self.cases)
            ops = cases[0]["operations"]
            if change == "missing":
                ops.pop()
            elif change == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "unknown"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)

    def test_receiver_geometry_metadata_and_source(self):
        got = self.observation("TestGroundReceiversPreserveMeshAndShadowProjection")["got"]
        self.changed(got, [["mesh", "grounds", 0],
            ["mesh", "mesh", "triangles", 0, "v", 0, "Position", "x"],
            ["mesh", "mesh", "triangles", 0, "material"],
            ["mesh", "mesh", "materials", 0, "texture", "sha256"],
            ["source", "mesh", "triangles", 0, "bias"]])

    def test_nonfinite_receiver_admission(self):
        case = next(c for c in self.cases if c["name"] == "TestGroundReceiverValidationRejectsInvalidPlanes")
        grounds = [o["request"]["mesh"]["grounds"][0] for o in case["operations"]]
        self.assertIn("NaN", grounds)
        self.assertIn("+Inf", grounds)
        self.assertIn("-Inf", grounds)
        failures = [o["got"] for o in case["operations"] if o["got"]["error"]]
        self.assertEqual(len(failures), 10)
        self.changed(failures[0], [["error"]])

    def test_cover_identity_opacity_affine_and_geometry(self):
        self.changed(self.observation("TestSVGPaintCoversKeepDifferentMaterialsAndDecalsSeparate")["got"], [["covers", 0]])
        self.changed(self.observation("TestSVGTexturedCoverPreservesOpacityAndTransparentExterior", "cover")["got"],
            [["svg"], ["written"], ["source", "batches", 0, "affine", 0],
             ["source", "mesh", "mesh", "materials", 0, "color", "A"]])

    def test_projection_and_visibility_fragments(self):
        self.changed(self.observation("TestRasterCameraRoundsScaledCoordinates")["got"], [[0]])
        got = self.observation("TestSVGVisibilityIgnoresProjectionFMAResiduals")["got"]
        self.changed(got, [["visible", 0, 0, 0, 0], ["error"]])

    def test_shadow_and_resource_bytes(self):
        got = self.observation("TestSVGShadowRetainsTransparentTextureHole")["got"]
        self.changed(got, [["svg"], ["error"]])
        self.assertEqual(self.score.outcome(got, {"unsupported": "shadow"}), "unsupported")
        case = next(c for c in self.cases if c["name"] == "TestSVGSurfaceResourcesAreSharedAcrossPaintPasses")
        first = case["operations"][0]
        self.assertIsNotNone(first["request"]["document"])
        self.assertTrue(all(o["request"]["document"] is None for o in case["operations"][1:]))
        self.assertTrue(all(o["got"] == first["got"] for o in case["operations"]))
        self.changed(first["got"], [["id"], ["svg"], ["error"]])


if __name__ == "__main__":
    unittest.main()
