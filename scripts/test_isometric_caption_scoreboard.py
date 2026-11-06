#!/usr/bin/env python3
"""Complete inventory, ordered geometry and resource-budget replay gates."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "caption", Path(__file__).with_name("isometric-caption-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CaptionGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.cases = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/caption-api.json")
            ).read_text()
        )["cases"]

    def test_complete_inventory(self):
        self.assertEqual(len(self.cases), 20)
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

    def test_complete_geometry_and_state(self):
        for case in self.cases:
            for operation in case["operations"]:
                expected = operation["got"]
                self.assertEqual(
                    self.score.outcome(expected, copy.deepcopy(expected)), "match"
                )
                self.assertEqual(self.score.outcome(expected, None), "rust-failed")
        place = next(
            c
            for c in self.cases
            if c["name"] == "TestRouteCaptionsSeparateSharedCorridor"
        )["operations"][0]["got"]
        for field in ["work", "refs"]:
            invalid = copy.deepcopy(place)
            invalid["state"][field] += 1
            self.assertEqual(self.score.outcome(place, invalid), "mismatch")
        ink = next(
            c
            for c in self.cases
            if c["name"] == "TestClassicInkSelectsPhysicalCubeEdges"
        )["operations"][0]["got"]
        for mutation in ["point", "support", "order", "error"]:
            invalid = copy.deepcopy(ink)
            if mutation == "point":
                invalid["segments"][0]["a"]["x"] += 2**-30
            elif mutation == "support":
                invalid["segments"][0]["support"].pop()
            elif mutation == "order":
                invalid["segments"].reverse()
            else:
                invalid["error"] = "unexpected cancellation"
            self.assertEqual(self.score.outcome(ink, invalid), "mismatch")

    def test_budget_and_nonfinite_witnesses(self):
        ops = next(
            c
            for c in self.cases
            if c["name"] == "TestRouteCaptionBudgetsAndDegenerateRoutes"
        )["operations"]
        self.assertEqual(ops[1]["request"]["state"]["work"], 2_000_000)
        self.assertEqual(ops[1]["got"]["state"]["work"], 2_000_000)
        runs = ops[2]["request"]["state"]["rects"]
        self.assertEqual(len(runs), 1)
        self.assertEqual(runs[0]["count"], 19_000)
        self.assertEqual(ops[3]["request"]["width"], "+Inf")
        self.assertEqual(ops[3]["got"]["surface"]["width"], 0)
        self.assertEqual(ops[4]["got"]["surface"]["width"], 0)

    def test_precision_and_cyclic_face_inventory(self):
        depth = next(
            c
            for c in self.cases
            if c["name"] == "TestClassicInkDepthRoundsCancellationProducts"
        )["operations"][0]
        self.assertEqual(depth["got"], {"depth": -(2**-27), "inside": True})
        faces = next(
            c for c in self.cases if c["name"] == "TestClassicInkSupportClipCyclicInput"
        )["operations"]
        self.assertEqual(len(faces), 3)
        self.assertTrue(all(len(o["got"]["triangles"]) >= 4 for o in faces))
        expected = faces[0]["got"]
        invalid = copy.deepcopy(expected)
        invalid["triangles"][1]["vertices"][0]["Position"]["x"] += 2**-30
        self.assertEqual(self.score.outcome(expected, invalid), "mismatch")


if __name__ == "__main__":
    unittest.main()
