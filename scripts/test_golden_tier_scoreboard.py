#!/usr/bin/env python3
"""Failure gates for the pinned sketch/exporter golden scoreboards."""
import importlib.util
import os
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SCORE = load("golden_tier", "golden-tier-scoreboard.py")
DELTA = load("golden_tier_delta", "golden-tier-scoreboard-delta.py")
SVG = b'<svg><text>hello</text></svg>'


class GoldenTierTests(unittest.TestCase):
    def test_go_must_reproduce_golden(self):
        other = b'<svg><text>other</text></svg>'
        self.assertEqual(SCORE.score_golden("sketch", SVG, other, SVG), "go-golden-mismatch")
        self.assertEqual(SCORE.score_golden("sketch", SVG, None, SVG), "unbound-golden")

    def test_missing_empty_timeout_and_crash_never_pass(self):
        for native in [None, b""]:
            self.assertEqual(SCORE.score_golden("sketch", SVG, SVG, native), "rust-missing-output")
        for failure in ["timeout", "budget-exceeded", "rust-failed"]:
            self.assertEqual(SCORE.score_golden("sketch", SVG, SVG, SVG, failure), failure)

    def test_corrupt_output_cannot_match_itself(self):
        self.assertEqual(SCORE.score_golden("sketch", b"<svg", b"<svg", b"<svg"), "go-invalid-output")
        self.assertEqual(SCORE.score_golden("exporter", b"{}", b"{}", b"{}"), "go-invalid-output")
        self.assertEqual(SCORE.score_golden("sketch", SVG, SVG, b"<svg"), "rust-invalid-output")

    def test_svg_allowance_preserves_content_and_geometry(self):
        a = b'<svg data-d2-version="old"><path d="M0 0"/></svg>'
        b = b'<svg data-d2-version="new"><path d="M0 0"/></svg>\n'
        self.assertTrue(SCORE.equal("sketch", a, b))
        self.assertFalse(SCORE.equal("sketch", a, b.replace(b"M0 0", b"M0 1")))

    def test_exporter_clears_only_upstream_contract_fields(self):
        source = {"shapes": [{"pos": {"x": 123, "y": 45}, "width": 10, "height": 20,
                              "labelPosition": "INSIDE_TOP_LEFT", "labelWidth": 9, "labelHeight": 8,
                              "label": "keep", "stroke": "red"}],
                  "connections": [{"route": [{"x": 1, "y": 2}], "labelPosition": "OUTSIDE_TOP",
                                   "labelWidth": 3, "labelHeight": 4, "labelPercentage": .4,
                                   "srcArrow": {"labelWidth": 6}}],
                  "root": {"width": 88}, "legend": {"shapes": [{"width": 99}]}}
        result = SCORE.exporter_contract(source)
        self.assertEqual(source["shapes"][0]["width"], 10)
        self.assertEqual(result["shapes"][0]["width"], 0)
        self.assertNotIn("labelPosition", result["shapes"][0])
        self.assertEqual(result["connections"][0]["route"], [])
        self.assertEqual(result["connections"][0]["labelPosition"], "")
        self.assertEqual(result["connections"][0]["srcArrow"], {"labelWidth": 6})
        self.assertEqual(result["connections"][0]["labelPercentage"], .4)
        self.assertEqual(result["root"], source["root"])
        self.assertEqual(result["legend"], source["legend"])

    def test_every_golden_remains_when_capture_is_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "go").mkdir()
            (root / "go.log").touch()
            golden = root / "retained/sketch.exp.svg"
            golden.parent.mkdir()
            golden.write_bytes(SVG)
            binary = root / "binary"
            binary.write_bytes(b"native")
            with patch.object(SCORE, "golden_paths", return_value=(root, [golden])), patch.object(SCORE, "ROOT", root):
                report = SCORE.assess("sketch", root, binary)
            self.assertEqual(report["goldens"], 1)
            self.assertEqual(report["summary"], {"unbound-golden": 1})

    def test_delta_rejects_missing_and_lost_matches(self):
        before = {"tier": "sketch", "rows": [{"name": "a", "outcome": "golden-match"},
                                               {"name": "b", "outcome": "golden-match"}]}
        after = {"tier": "sketch", "rows": [{"name": "a", "outcome": "timeout"}]}
        self.assertEqual(DELTA.delta(before, after), {"improvements": [], "regressions": ["a"], "missing": ["b"]})

    def test_assertion_regression_is_distinct_from_golden_match(self):
        before = {"tier": "exporter", "rows": [{"name": "a", "outcome": "golden-match", "assertion_outcome": "assertion-match"}]}
        after = {"tier": "exporter", "rows": [{"name": "a", "outcome": "golden-match", "assertion_outcome": "assertion-mismatch"}]}
        self.assertEqual(DELTA.delta(before, after)["regressions"], ["a:assertions"])

    def test_exporter_pre_scrub_assertions(self):
        self.assertTrue(SCORE.exporter_assertions("TestExport/shape/image_dimensions", {"shapes": [{"width": 200, "height": 230}]}))
        self.assertFalse(SCORE.exporter_assertions("TestExport/shape/image_dimensions", {"shapes": [{"width": 0, "height": 0}]}))

    def test_json_boolean_is_distinct_from_number(self):
        left = b'{"shapes":[],"connections":[],"root":{},"isFolderOnly":true}'
        right = left.replace(b'true', b'1')
        self.assertFalse(SCORE.equal("exporter", left, right))

    def test_streaming_guard_includes_upstream_temporary_outputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            capture, upstream = root / "capture", root / "upstream"
            capture.mkdir()
            upstream.mkdir()
            # Sparse file checks logical output limits without allocating bulk.
            with (upstream / "large.got.svg").open("wb") as stream:
                stream.truncate((128 << 20) + 1)
            result = SCORE.CLI.bounded_run([sys.executable, "-c", "import time; time.sleep(5)"],
                                           os.environ.copy(), capture, capture / "log", 10,
                                           extra_directories=[upstream])
            self.assertEqual(result["failure"], "budget-exceeded")

    def test_complete_native_output_inventory_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            (output / "sketch.svg").write_bytes(SVG)
            self.assertEqual(SCORE.native_inventory("sketch", output), "rust-missing-output")
            (output / "target.json").write_text('{}')
            self.assertIsNone(SCORE.native_inventory("sketch", output))
            (output / "unexpected.svg").write_bytes(SVG)
            self.assertEqual(SCORE.native_inventory("sketch", output), "rust-inventory-mismatch")


if __name__ == "__main__":
    unittest.main()
