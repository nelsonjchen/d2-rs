#!/usr/bin/env python3
"""Failure gates for original hierarchy and lighting observations."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "hierarchy", Path(__file__).with_name("isometric-hierarchy-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class HierarchyGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.cases = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/hierarchy-api.json")
            ).read_text()
        )["cases"]

    def test_complete_inventory(self):
        self.assertEqual(len(self.cases), 21)
        self.assertEqual(sum(len(c["operations"]) for c in self.cases), 358)
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

    def observation(self, name, op=None):
        case = next(c for c in self.cases if c["name"] == name)
        return next(
            o["got"]
            for o in case["operations"]
            if op is None or o["request"]["op"] == op
        )

    def check_changes(self, expected, paths):
        self.assertEqual(self.score.outcome(expected, expected), "match")
        self.assertEqual(self.score.outcome(expected, None), "rust-failed")
        for keys in paths:
            actual = copy.deepcopy(expected)
            v = actual
            for k in keys[:-1]:
                v = v[k]
            old = v[keys[-1]]
            v[keys[-1]] = (
                not old
                if isinstance(old, bool)
                else old + 2**-30
                if isinstance(old, (float, int))
                else "changed"
            )
            self.assertEqual(self.score.outcome(expected, actual), "mismatch", keys)

    def test_support_geometry_budgets_and_identity(self):
        e = self.observation("TestHierarchySupportKeepsEveryCapAndPrintSurface")
        self.check_changes(
            e,
            [
                ["builder", "state", "mesh", "triangles", 0, "v", 0, "Position", "x"],
                ["builder", "state", "mesh", "triangles", 0, "material"],
                ["builder", "state", "pixels"],
                ["builder", "supports", "lower"],
                ["builder", "drop"],
                ["builder", "state", "error"],
            ],
        )

    def test_terrace_source_ownership_and_plan(self):
        e = self.observation("TestHierarchyTerracesBoundDeepNestingAndPreserveRegions")
        self.check_changes(
            e,
            [
                ["boards", 1, "position", "y"],
                ["boards", 1, "size", "y"],
                ["boards", 1, "nodeIds"],
                ["source", 1, "position", "y"],
                ["nodes", "1", "fill"],
            ],
        )

    def test_lighting_coefficients_and_patches(self):
        e = self.observation("TestSVGTextureLightingMatchesNativeLinearShader")
        self.check_changes(e, [["slope", 0], ["intercept", 1]])
        e = self.observation("TestSVGCurvedGradientSamplesTheNativeHighlight")
        self.check_changes(
            e,
            [
                ["gradient", "stops", 0, "color", "x"],
                ["gradient", "start", 0],
                ["gradient", "valid"],
                ["error"],
            ],
        )
        e = self.observation(
            "TestSVGCurvedGradientSubdividesOnlyWhenASecondAxisIsNeeded"
        )
        self.check_changes(
            e, [[0, "vertices", 0, "point", 0], [0, "gradient", "stops", 0, "offset"]]
        )

    def test_shadow_svg_and_failure(self):
        e = self.observation("TestSVGShadowUnionsEqualOpacityAndKeepsNearestCaster")
        self.assertIn('opacity="0.055216"', e["svg"])
        self.check_changes(e, [["svg"], ["error"]])
        self.assertEqual(
            self.score.outcome(e, {"unsupported": "shadow"}), "unsupported"
        )


if __name__ == "__main__":
    unittest.main()
