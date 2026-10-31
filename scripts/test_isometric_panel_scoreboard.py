#!/usr/bin/env python3
"""Require complete original inventories, panel retention and exact mesh bytes."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "panel_score", Path(__file__).with_name("isometric-panel-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PanelGates(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.score = module.scoreboard()
        cls.cases = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/panel-api.json")
            ).read_text()
        )["cases"]

    def test_complete_inventory(self):
        self.score.validate_cases(self.cases)
        for value in [[], self.cases[:-1], self.cases + self.cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(value)
        for change in ["missing", "extra", "unknown"]:
            value = copy.deepcopy(self.cases)
            ops = value[0]["operations"]
            if change == "missing":
                ops.pop()
            elif change == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "skip"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(value)

    def test_mesh_texture_and_sharing(self):
        for case in self.cases:
            for op in case["operations"]:
                got = op["got"]
                self.assertEqual(self.score.outcome(got, got), "match")
                for value in [None, {"unsupported": "panel"}]:
                    self.assertNotEqual(self.score.outcome(got, value), "match")
                for key in ["triangles", "materials", "groups", "owners"]:
                    value = copy.deepcopy(got)
                    value["state"]["mesh"].pop(key)
                    self.assertEqual(self.score.outcome(got, value), "mismatch")
                value = copy.deepcopy(got)
                value["state"]["mesh"]["materials"][-1]["texture"]["sha256"] = "0" * 64
                self.assertEqual(self.score.outcome(got, value), "mismatch")
                value = copy.deepcopy(got)
                value["state"]["mesh"]["triangles"][0]["v"][0]["Position"]["x"] += 1e-12
                self.assertEqual(self.score.outcome(got, value), "mismatch")

    def test_retained_panels_and_source(self):
        case = next(c for c in self.cases if "FollowingSourcePanel" in c["name"])
        got = case["operations"][3]["got"]
        self.assertEqual(len(got["state"]["panels"]), 2)
        for key in ["document", "width", "height", "first", "last", "animated"]:
            value = copy.deepcopy(got)
            value["state"]["panels"][0].pop(key)
            self.assertEqual(self.score.outcome(got, value), "mismatch")
        for key in ["source", "error"]:
            value = copy.deepcopy(got)
            value.pop(key)
            self.assertEqual(self.score.outcome(got, value), "mismatch")


if __name__ == "__main__":
    unittest.main()
