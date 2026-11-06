#!/usr/bin/env python3
"""Complete original font call inventories, buffers and resolver requests."""

import base64
import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "font_score", Path(__file__).with_name("isometric-font-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class FontGates(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.score = module.scoreboard()
        fixture = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/font-api.json")
            ).read_text()
        )
        cls.cases = fixture["cases"]
        # Tiny complete premultiplied images suffice for inventory/byte gates.
        for c in cls.cases:
            for op in c["operations"]:
                if op["request"]["op"] == "text":
                    op["got"]["frame"] = {
                        "width": 2,
                        "height": 1,
                        "stride": 8,
                        "pixels": base64.b64encode(
                            bytes([1, 2, 3, 255, 4, 5, 6, 255])
                        ).decode(),
                    }
        cls.fallback = cls.cases[1]["operations"][0]

    def test_complete_inventory(self):
        self.score.validate_cases(self.cases)
        for value in [[], self.cases[:-1], self.cases + self.cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(value)
        for change in ["missing", "extra", "unknown"]:
            cases = copy.deepcopy(self.cases)
            ops = cases[0]["operations"]
            if change == "missing":
                ops.pop()
            elif change == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "skip"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)

    def test_complete_texture_bytes(self):
        got = self.fallback["got"]
        for value in [None, {"unsupported": "font"}]:
            self.assertNotEqual(self.score.outcome(got, value), "match")
        invalid = copy.deepcopy(got)
        raw = bytearray(base64.b64decode(invalid["frame"]["pixels"]))
        raw[-2] ^= 1
        invalid["frame"]["pixels"] = base64.b64encode(raw).decode()
        self.assertEqual(self.score.outcome(got, invalid), "mismatch")
        for value in ["", "!", base64.b64encode(b"short").decode()]:
            cases = copy.deepcopy(self.cases)
            cases[1]["operations"][0]["got"]["frame"]["pixels"] = value
            with self.assertRaises((AssertionError, ValueError)):
                self.score.validate_cases(cases)

    def test_family_and_resolver_state(self):
        got = self.fallback["got"]
        self.assertTrue(got["requested"])
        self.assertTrue(got["requests"])
        for key in ["requested", "requests", "faces", "layout", "error"]:
            invalid = copy.deepcopy(got)
            invalid.pop(key)
            self.assertEqual(self.score.outcome(got, invalid), "mismatch")
        invalid = copy.deepcopy(got)
        invalid["requests"][0]["Family"] = "other"
        self.assertEqual(self.score.outcome(got, invalid), "mismatch")
        invalid = copy.deepcopy(got)
        invalid["layout"]["FontSize"] += 1e-15
        self.assertEqual(self.score.outcome(got, invalid), "mismatch")

    def test_complete_rich_documents(self):
        for op in self.cases[0]["operations"]:
            got = op["got"]
            self.assertEqual(self.score.outcome(got, got), "match")
            if op["request"]["op"] == "rich":
                invalid = copy.deepcopy(got)
                invalid["document"]["Assets"].pop(
                    next(iter(invalid["document"]["Assets"]))
                )
                self.assertEqual(self.score.outcome(got, invalid), "mismatch")


if __name__ == "__main__":
    unittest.main()
