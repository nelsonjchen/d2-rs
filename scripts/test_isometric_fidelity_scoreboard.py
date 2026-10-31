#!/usr/bin/env python3
"""Check full shadow buffers, exact numeric/state output and finite inventories."""

import base64
import copy
import hashlib
import importlib.util
import json
import struct
import unittest

from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "fidelity", Path(__file__).with_name("isometric-fidelity-scoreboard.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class FidelityGates(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.score = module.scoreboard()
        cls.cases = json.loads(
            cached_fixture("crates/d2-rs-isometric/tests/fixtures/fidelity-api.json").read_text()
        )["cases"]
        cls.shadow = next(c for c in cls.cases if c["name"].startswith("TestShadow"))[
            "operations"
        ][0]
        # The transparent authored face leaves every float32 depth at -Inf
        # and every opacity at zero. Check every byte against the Go digest.
        for key, raw in {
            "depth": struct.pack("<I", 0xFF800000) * (2048 * 2048),
            "opacity": bytes(2048 * 2048),
        }.items():
            expected = cls.shadow["got"]["shadow"][key]
            assert expected == {
                "length": len(raw),
                "sha256": hashlib.sha256(raw).hexdigest(),
            }
            cls.shadow["got"]["shadow"][key] = base64.b64encode(raw).decode()

    def test_complete_inventory(self):
        self.score.validate_cases(self.cases)
        self.assertEqual(sum(len(c["operations"]) for c in self.cases), 8)
        for cases in [[], self.cases[:-1], self.cases + self.cases]:
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)
        for mutation in ["missing", "extra", "unknown"]:
            cases = copy.deepcopy(self.cases)
            ops = cases[-1]["operations"]
            if mutation == "missing":
                ops.pop()
            elif mutation == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "ignored"
            with self.assertRaises(AssertionError):
                self.score.validate_cases(cases)

    def test_every_shadow_byte_compares(self):
        got = self.shadow["got"]
        for key in ["depth", "opacity"]:
            invalid = copy.deepcopy(got)
            raw = bytearray(base64.b64decode(invalid["shadow"][key]))
            raw[-1] ^= 1
            invalid["shadow"][key] = base64.b64encode(raw).decode()
            self.assertEqual(self.score.outcome(got, invalid), "mismatch")
        self.assertEqual(self.score.outcome(got, None), "rust-failed")

    def test_short_or_corrupt_buffers_fail(self):
        for key in ["depth", "opacity"]:
            for value in ["", "!", base64.b64encode(b"short").decode()]:
                cases = copy.deepcopy(self.cases)
                shadow = next(c for c in cases if c["name"].startswith("TestShadow"))[
                    "operations"
                ][0]["got"]["shadow"]
                shadow[key] = value
                with self.assertRaises((AssertionError, ValueError)):
                    self.score.validate_cases(cases)

    def test_errors_masks_and_gradient_state(self):
        shadow = next(c for c in self.cases if c["name"].startswith("TestShadow"))
        self.assertEqual(
            [
                o["request"]["triangles"][0]["shadow_opacity"]
                for o in shadow["operations"][1:]
            ],
            [-1, 1.1, "NaN"],
        )
        self.assertEqual(
            [o["got"]["error"] for o in shadow["operations"][1:]],
            ["triangle 0 has invalid shadow opacity"] * 3,
        )
        for case in self.cases:
            for op in case["operations"]:
                expected = op["got"]
                self.assertEqual(self.score.outcome(expected, expected), "match")
                invalid = copy.deepcopy(expected)
                invalid.pop(next(iter(invalid)))
                self.assertEqual(self.score.outcome(expected, invalid), "mismatch")
        paint = next(c for c in self.cases if "Gradient" in c["name"])["operations"][0][
            "got"
        ]
        invalid = copy.deepcopy(paint)
        invalid["writer"]["defs"] = invalid["writer"]["defs"].replace(
            'color-interpolation="linearRGB"', ""
        )
        self.assertEqual(self.score.outcome(paint, invalid), "mismatch")
        invalid = copy.deepcopy(paint)
        invalid["writer"]["transform"]["A"] = 1.000000000000001
        self.assertEqual(self.score.outcome(paint, invalid), "mismatch")

    def test_helper_provenance(self):
        provenance = self.score.source()
        self.assertEqual(
            set(provenance["helpers"]),
            {"tools/isometric-content-scoreboard/scenejson.go"},
        )
        self.assertEqual(
            provenance["helpers"]["tools/isometric-content-scoreboard/scenejson.go"],
            self.score.sha(
                module.ROOT / "tools/isometric-content-scoreboard/scenejson.go"
            ),
        )


if __name__ == "__main__":
    unittest.main()
