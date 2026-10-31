#!/usr/bin/env python3
"""Failure gates for complete rejection snapshots."""

import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "preflight", Path(__file__).with_name("board-preflight-scoreboard.py")
)
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


def fixture():
    nodes = {
        ".": {"type": "directory", "mode": 448},
        "output/child.svg": {"type": "symlink", "mode": 493, "link": "/shared/victim"},
        "victim": {"type": "file", "mode": 420, "bytes": "a2VlcCBtZQ=="},
    }
    return {
        test: {
            "name": test,
            "error": 'preflight: symlink "/shared/child.svg"',
            "before": copy.deepcopy(nodes),
            "after": copy.deepcopy(nodes),
        }
        for test in preflight.CALLS
    }


class Gates(unittest.TestCase):
    def test_complete_snapshot_matches(self):
        expected = fixture()
        self.assertTrue(
            all(r["outcome"] == "match" for r in preflight.compare(expected, expected))
        )

    def test_changed_error_bytes_modes_links_and_inventory_fail(self):
        expected = fixture()
        name = preflight.TESTS[0]
        for change in ["error", "bytes", "mode", "link", "extra", "missing"]:
            actual = copy.deepcopy(expected)
            row = actual[name]
            if change == "error":
                row["error"] += " suffix"
            elif change in {"bytes", "mode"}:
                row["after"]["victim"][change] = 0
            elif change == "link":
                row["after"]["output/child.svg"]["link"] = "/elsewhere"
            elif change == "extra":
                row["after"][".d2-board-output-leftover"] = {
                    "type": "directory",
                    "mode": 448,
                }
            else:
                del actual[name]
            self.assertEqual(
                preflight.compare(expected, actual)[0]["outcome"], "mismatch", change
            )

    def test_changed_or_missing_original_observations_fail(self):
        for change in ["missing", "changed", "empty"]:
            expected = fixture()
            if change == "missing":
                expected.pop(preflight.TESTS[0])
            elif change == "changed":
                expected[preflight.TESTS[0]]["after"].pop("victim")
            else:
                expected[preflight.TESTS[0]]["before"] = {}
            with self.assertRaises(AssertionError):
                preflight.compare(expected, fixture())

    def test_extra_native_call_fails(self):
        actual = fixture()
        actual["extra"] = {}
        with self.assertRaises(AssertionError):
            preflight.compare(fixture(), actual)


if __name__ == "__main__":
    unittest.main()
