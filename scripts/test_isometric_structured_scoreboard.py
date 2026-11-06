#!/usr/bin/env python3
"""Gates for complete original structured and sequence calls."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "structured", Path(__file__).with_name("isometric-structured-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class StructuredGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.cases = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/structured-api.json")
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
        self.assertEqual(len(cases), 13)
        self.assertEqual(sum(len(c["operations"]) for c in cases), 50)
        self.score.validate_cases(cases)
        for bad in [[], cases[:-1], cases + cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(bad)
        for key in ["extra", "missing", "unknown"]:
            bad = copy.deepcopy(cases)
            if key == "extra":
                bad[0]["operations"].append(bad[0]["operations"][0])
            elif key == "missing":
                bad[0]["operations"].pop()
            else:
                bad[0]["operations"][0]["request"]["op"] = "unknown"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(bad)

    def observations(self, name):
        return next(c["operations"] for c in self.cases if c["name"] == name)

    def check_changes(self, e, paths):
        self.assertEqual(self.score.outcome(e, e), "match")
        self.assertEqual(self.score.outcome(e, None), "rust-failed")
        for keys in paths:
            a = copy.deepcopy(e)
            v = a
            for k in keys[:-1]:
                v = v[k]
            value = v[keys[-1]]
            v[keys[-1]] = (
                value + 2**-30 if isinstance(value, (int, float)) else "changed"
            )
            self.assertEqual(self.score.outcome(e, a), "mismatch", keys)

    def test_complete_mesh_budgets_and_identity(self):
        e = self.observations("TestStructuredRailsUseRealRowsAndCompiledFootprints")[0][
            "got"
        ]
        self.check_changes(
            e,
            [
                ["state", "mesh", "triangles", 0, "v", 0, "Position", "x"],
                ["state", "mesh", "triangles", 0, "material"],
                ["state", "mesh", "materials", 0, "roughness"],
                ["state", "error"],
                ["rich", "rows"],
                ["rich", "pixels"],
            ],
        )

    def test_text_cells_and_viewport(self):
        e = self.observations(
            "TestStructuredCellsRetainNativeFontsBaselinesAndSemantics"
        )[0]["got"]
        self.check_changes(
            e,
            [
                ["cells", 0, "row"],
                ["cells", 0, "run", "Origin", "Y"],
                ["cells", 0, "run", "Text"],
                ["cells", 0, "run", "Font", "Family"],
                ["viewport", "width"],
                ["error"],
            ],
        )

    def test_complete_routes_and_errors(self):
        ops = self.observations(
            "TestSequenceRoutesPreserveCompiledVerticesInMixedScenes"
        )
        e = ops[0]["got"]
        self.check_changes(e, [["paths", 0, 0, "x"], ["lanes", 0, 0, "z"]])
        canceled = next(o for o in ops if o["request"]["canceled"])
        self.assertEqual(canceled["got"]["error"], "context canceled")
        self.check_changes(canceled["got"], [["error"]])

    def test_texture_captions_and_packets(self):
        e = self.observations(
            "TestSequenceInkHasNativeDashSpacingFlatMarkersAndAuthoredAnimation"
        )[-1]["got"]
        self.check_changes(e, [["packets", 0, "lengths", 1], ["packets", 0, "forward"]])
        op = next(
            o
            for o in self.observations(
                "TestSequenceRichCaptionIgnoresAutomaticClearance"
            )
            if o["request"]["op"] == "edges"
        )
        e = op["got"]
        idx = next(
            i for i, m in enumerate(e["state"]["mesh"]["materials"]) if m["texture"]
        )
        self.check_changes(
            e,
            [
                ["state", "mesh", "materials", idx, "texture", "sha256"],
                ["placer", "rects", 0, "rect", "x"],
            ],
        )


if __name__ == "__main__":
    unittest.main()
