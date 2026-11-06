#!/usr/bin/env python3
"""Check complete inventories, byte arrays, ownership and resource-state gates."""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "output_api", Path(__file__).with_name("isometric-output-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class OutputGates(unittest.TestCase):
    def setUp(self):
        self.score = module.scoreboard()
        self.cases = json.loads(
            (
                cached_fixture("crates/d2-rs-isometric/tests/fixtures/output-api.json")
            ).read_text()
        )["cases"]

    def test_inventory(self):
        self.score.validate_cases(self.cases)
        self.assertEqual(sum(len(c["operations"]) for c in self.cases), 114)
        for invalid in [[], self.cases[:-1], self.cases + self.cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(invalid)
        for mutation in ["missing", "extra", "unknown"]:
            invalid = copy.deepcopy(self.cases)
            ops = invalid[-1]["operations"]
            if mutation == "missing":
                ops.pop()
            elif mutation == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "ignored"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(invalid)

    def test_entire_output_compares(self):
        for case in self.cases:
            for op in case["operations"]:
                expected = op["got"]
                self.assertEqual(self.score.outcome(expected, expected), "match")
                self.assertEqual(self.score.outcome(expected, None), "rust-failed")
                if isinstance(expected, dict):
                    invalid = copy.deepcopy(expected)
                    invalid.pop(next(iter(invalid)))
                    self.assertEqual(self.score.outcome(expected, invalid), "mismatch")

    def test_palette_delta_and_delays(self):
        import base64

        case = next(
            c for c in self.cases if c["name"] == "TestSharedPaletteAndOpaqueDelta"
        )
        lookup = case["operations"][0]["got"]
        data = bytearray(base64.b64decode(lookup["lookup"]))
        self.assertEqual(len(data), 1 << 18)
        data[-1] ^= 1
        invalid = dict(lookup, lookup=base64.b64encode(data).decode())
        self.assertEqual(self.score.outcome(lookup, invalid), "mismatch")
        deltas = [o["got"] for o in case["operations"] if o["request"]["op"] == "delta"]
        self.assertEqual(deltas[-1]["rect"], [0, 0, 1, 1])
        invalid = dict(deltas[0], rect=[0, 0, 8, 4])
        self.assertEqual(self.score.outcome(deltas[0], invalid), "mismatch")
        self.assertEqual(
            sum(o["got"] for o in case["operations"] if o["request"]["op"] == "delay"),
            833,
        )

    def test_writer_and_mask_ownership(self):
        writer = next(c for c in self.cases if c["name"] == "TestFrameAndWriterLimits")
        self.assertEqual([o["got"]["count"] for o in writer["operations"]], [0, 0])
        self.assertEqual([o["got"]["remaining"] for o in writer["operations"]], [2, 2])
        self.assertEqual(writer["operations"][1]["got"]["error"], "context canceled")
        loop = next(
            c for c in self.cases if c["name"] == "TestNativeGIFLoopCopiesAnimatedMasks"
        )["operations"][0]
        for field in ["distinct_document", "distinct_root", "distinct_mask"]:
            self.assertTrue(loop["got"][field])
            invalid = dict(loop["got"], **{field: False})
            self.assertEqual(self.score.outcome(loop["got"], invalid), "mismatch")
        source = loop["got"]["source"]["Root"]["Mask"]["Root"]["Animations"][0]
        adapted = loop["got"]["document"]["Root"]["Mask"]["Root"]["Animations"][0]
        self.assertEqual(source["Duration"], 2_300_000_000)
        self.assertEqual(adapted["Duration"], 8_000_000_000 // 3)


if __name__ == "__main__":
    unittest.main()
