#!/usr/bin/env python3
"""Guard complete original inventories and exact artwork observations."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "artwork", Path(__file__).with_name("isometric-artwork-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ArtworkGates(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.score = module.scoreboard()
        cls.cases = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/artwork-api.json")
            ).read_text()
        )["cases"]

    def test_complete_inventory(self):
        self.score.validate_cases(self.cases)
        for cases in [[], self.cases[:-1], self.cases + self.cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)
        for mutation in ["missing", "extra", "unknown"]:
            cases = copy.deepcopy(self.cases)
            ops = cases[0]["operations"]
            if mutation == "missing":
                ops.pop()
            elif mutation == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "skip"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)

    def test_icon_policy_and_pixels(self):
        case = next(c for c in self.cases if "ExplicitLocalResolver" in c["name"])
        ops = case["operations"]
        local = [o for o in ops if "local_file" in o["request"]]
        self.assertEqual(len(local), 3)
        self.assertTrue(
            all(
                o["request"]["local_file"] == local[0]["request"]["local_file"]
                for o in local
            )
        )
        self.assertIn("explicit asset resolver", local[0]["got"]["error"])
        self.assertIn("local-file access is disabled", local[1]["got"]["error"])
        self.assertEqual(local[2]["got"]["error"], "")
        texture = local[2]["got"]
        for key in ["sha256", "width", "height"]:
            actual = copy.deepcopy(texture)
            actual["texture"].pop(key)
            self.assertEqual(self.score.outcome(texture, actual), "mismatch")
        for actual in [None, {"unsupported": "icon"}]:
            self.assertNotEqual(self.score.outcome(texture, actual), "match")

    def test_complete_vector_capture(self):
        case = next(c for c in self.cases if "VectorCapture" in c["name"])
        self.assertEqual(len(case["operations"]), 4)
        for op in case["operations"]:
            got = op["got"]
            self.assertGreaterEqual(len(got["surfaces"]), 3)
            self.assertEqual(len(got["surface_ids"]), len(got["mesh"]["triangles"]))
            self.assertEqual(
                set(got["surface_ids"]) - {0}, set(range(1, len(got["surfaces"]) + 1))
            )
            self.assertTrue(
                all(
                    s["svg"] and not s["error"] and "<image" not in s["svg"]
                    for s in got["surfaces"]
                )
            )
            for key in ["scene", "target", "mesh", "surface_ids", "surfaces", "error"]:
                actual = copy.deepcopy(got)
                actual.pop(key)
                self.assertEqual(self.score.outcome(got, actual), "mismatch")
            actual = copy.deepcopy(got)
            actual["surfaces"][0]["svg"] += " "
            self.assertEqual(self.score.outcome(got, actual), "mismatch")


if __name__ == "__main__":
    unittest.main()
