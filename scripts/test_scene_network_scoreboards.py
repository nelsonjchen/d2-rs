#!/usr/bin/env python3
"""Contract tests for the scene and real-network observation gates."""

import copy
import importlib.util
import unittest
from pathlib import Path


def module(name):
    spec = importlib.util.spec_from_file_location(
        name, Path(__file__).with_name(name + ".py")
    )
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


class ObservationGateTests(unittest.TestCase):
    def test_paint_order_inventory_and_atomic_payload_are_complete(self):
        score = module("isometric-order-scoreboard").scoreboard()
        self.assertEqual(len(score.TESTS), 6)
        self.assertEqual(sum(score.TESTS.values()), 12)
        cases = [
            {
                "name": name,
                "operations": [{"request": {"op": "order"}} for _ in range(count)],
            }
            for name, count in score.TESTS.items()
        ]
        score.validate_cases(cases)
        for invalid in [[], cases[:-1], cases + cases[:1]]:
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)
        for change in ["missing", "extra", "wrong"]:
            invalid = copy.deepcopy(cases)
            ops = invalid[0]["operations"]
            if change == "missing":
                ops.pop()
            elif change == "extra":
                ops.append(ops[0])
            else:
                ops[0]["request"]["op"] = "ignored"
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)
        expected = [
            {
                "units": [
                    {
                        "id": 1,
                        "opacity": 0.35,
                        "batches": [
                            {"id": 2, "polygons": [[[0.0, 1.0, 2.0], [3.0, 4.0, 5.0]]]}
                        ],
                    },
                    {"id": 3, "opacity": 1.0, "batches": []},
                ],
                "stats": {"relations": 2, "cyclic_groups": 0, "conflicting_pairs": 0},
                "error": "",
            }
        ]
        self.assertEqual(score.outcome(expected, expected), "match")
        for change in ["order", "opacity", "batch", "point", "stats", "error"]:
            invalid = copy.deepcopy(expected)
            if change == "order":
                invalid[0]["units"].reverse()
            elif change == "opacity":
                invalid[0]["units"][0]["opacity"] = 1.0
            elif change == "batch":
                invalid[0]["units"][0]["batches"][0]["id"] = 4
            elif change == "point":
                invalid[0]["units"][0]["batches"][0]["polygons"][0].reverse()
            elif change == "stats":
                invalid[0]["stats"]["cyclic_groups"] = 1
            else:
                invalid[0]["error"] = "context canceled"
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        for invalid in [[], expected + expected, None]:
            self.assertNotEqual(score.outcome(expected, invalid), "match")

    def test_visibility_inventory_preserves_every_call(self):
        score = module("isometric-visibility-scoreboard").scoreboard()
        self.assertEqual(len(score.TESTS), 8)
        self.assertEqual(sum(score.TESTS.values()), 27)
        cases = [
            {
                "name": name,
                "operations": [
                    {"request": {"op": op}}
                    for op, count in score.OPS[name].items()
                    for _ in range(count)
                ],
            }
            for name in score.TESTS
        ]
        score.validate_cases(cases)
        for invalid in [[], cases[:-1], cases + cases[:1]]:
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)
        invalid = copy.deepcopy(cases)
        invalid[-1]["operations"].pop()
        with self.assertRaises(AssertionError):
            score.validate_cases(invalid)
        expected = [{"visible": [None, [[[0.0, 1.0, 2.0]]]], "error": ""}]
        self.assertEqual(score.outcome(expected, expected), "match")
        for change in ["nil", "point", "error", "order"]:
            invalid = copy.deepcopy(expected)
            if change == "nil":
                invalid[0]["visible"][0] = []
            elif change == "point":
                invalid[0]["visible"][1][0][0][2] += 0.0000000001
            elif change == "error":
                invalid[0]["error"] = "context canceled"
            else:
                invalid[0]["visible"].reverse()
            self.assertEqual(score.outcome(expected, invalid), "mismatch")

    def test_admission_inventory_and_complete_errors(self):
        score = module("isometric-admission-scoreboard").scoreboard()
        self.assertEqual(len(score.TESTS), 13)
        cases = [
            {
                "name": name,
                "operations": [
                    {
                        "request": {
                            "op": "validate",
                            "mutation": name.partition("/")[2].replace("_", " ")
                            or "nil diagram",
                        }
                    }
                ],
            }
            for name in score.TESTS
        ]
        score.validate_cases(cases)
        for invalid in [[], cases[:-1], cases + cases[:1]]:
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)
        invalid = copy.deepcopy(cases)
        invalid[0]["operations"][0]["request"]["mutation"] = "nonfinite route"
        with self.assertRaises(AssertionError):
            score.validate_cases(invalid)
        expected = [
            {
                "scene_nil": True,
                "error": "isometric: nonfinite or excessive numeric value",
            }
        ]
        self.assertEqual(score.outcome(expected, expected), "match")
        self.assertEqual(
            score.outcome(expected, [{"scene_nil": False, "error": ""}]), "mismatch"
        )
        self.assertEqual(
            score.outcome(expected, {"unsupported": "unrepresentable input"}),
            "unsupported",
        )

    def test_layout_inventory_uses_all_original_calls(self):
        score = module("isometric-layout-scoreboard").scoreboard()
        self.assertEqual(len(score.TESTS), 7)
        self.assertEqual(sum(score.TESTS.values()), 19)
        cases = [
            {
                "name": name,
                "operations": [{"request": {"op": "build"}} for _ in range(count)],
            }
            for name, count in score.TESTS.items()
        ]
        score.validate_cases(cases)
        for invalid in [[], cases[:-1], cases + cases[:1]]:
            with self.assertRaises(AssertionError):
                score.validate_cases(invalid)
        invalid = copy.deepcopy(cases)
        invalid[-1]["operations"].pop()
        with self.assertRaises(AssertionError):
            score.validate_cases(invalid)
        invalid = copy.deepcopy(cases)
        invalid[-1]["operations"][0]["request"]["op"] = "ignored"
        with self.assertRaises(AssertionError):
            score.validate_cases(invalid)

    def test_complete_function_and_operation_inventory(self):
        for name in ["imageasset-network-scoreboard", "isometric-scene-scoreboard"]:
            score = module(name)
            cases = [
                {
                    "name": test,
                    "operations": [
                        {"request": {"op": op}}
                        for op, count in score.OPS[test].items()
                        for _ in range(count)
                    ],
                }
                for test in score.TESTS
            ]
            score.validate_cases(cases)
            for invalid in [[], cases[:-1], cases + cases[:1]]:
                with self.assertRaises(AssertionError):
                    score.validate_cases(invalid)
            for change in ["missing", "extra", "wrong"]:
                invalid = copy.deepcopy(cases)
                ops = invalid[0]["operations"]
                if change == "missing":
                    ops.pop()
                elif change == "extra":
                    ops.append(ops[0])
                else:
                    ops[0]["request"]["op"] = "unknown"
                with self.assertRaises(AssertionError):
                    score.validate_cases(invalid)

    def test_no_resource_hit_cache_or_error_loss(self):
        score = module("imageasset-network-scoreboard")
        expected = [
            {
                "error": "imageasset: HTTP request failed",
                "hits": 0,
                "resource": None,
                "unavailable": True,
                "cache_entries": 1,
                "cache_bytes": 91,
            },
            {
                "error": "",
                "hits": 2,
                "resource": {"data": "YWJj", "width": 2, "height": 3},
            },
        ]
        self.assertEqual(score.outcome(expected, expected), "match")
        for index, field, value in [
            (0, "hits", 1),
            (0, "unavailable", 1),
            (0, "error", "HTTP request failed"),
            (0, "cache_entries", 0),
            (0, "cache_bytes", 90),
            (1, "resource", {"data": "YWJk", "width": 2, "height": 3}),
        ]:
            invalid = copy.deepcopy(expected)
            invalid[index][field] = value
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        for invalid in [[], expected[:-1], expected + [{}], {}, None]:
            self.assertNotEqual(score.outcome(expected, invalid), "match")

    def test_scene_ownership_order_and_text_cannot_be_ignored(self):
        score = module("isometric-scene-scoreboard")
        expected = [
            {
                "scene": {
                    "nodes": [{"id": "b", "label": "<script>\u2028"}, {"id": "a"}]
                },
                "source": {"nodes": [{"id": "b"}]},
                "error": "",
            }
        ]
        self.assertEqual(score.outcome(expected, expected), "match")
        for change in ["alias", "order", "text", "error", "extra"]:
            invalid = copy.deepcopy(expected)
            if change == "alias":
                invalid[0]["source"]["nodes"][0]["id"] = "mutated"
            elif change == "order":
                invalid[0]["scene"]["nodes"].reverse()
            elif change == "text":
                invalid[0]["scene"]["nodes"][0]["label"] = "<script>"
            elif change == "error":
                invalid[0]["error"] = "failed"
            else:
                invalid[0]["scene"]["extra"] = True
            self.assertEqual(score.outcome(expected, invalid), "mismatch")
        self.assertEqual(
            score.outcome(expected, {"unsupported": "missing API"}), "unsupported"
        )
        self.assertEqual(score.outcome(expected, None), "rust-failed")


if __name__ == "__main__":
    unittest.main()
