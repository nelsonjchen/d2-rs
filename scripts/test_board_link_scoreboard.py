#!/usr/bin/env python3
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("links", Path(__file__).with_name("board-link-scoreboard.py"))
links = importlib.util.module_from_spec(spec)
spec.loader.exec_module(links)


def calls():
    return [
        dict(test="TestRelinkIncludesConnections", input=dict(op="relink", diagram={"shapes": [], "connections": []}),
             result=dict(diagram={"shapes": [], "connections": []}, error="")),
        dict(test="TestRelinkIncludesConnections", input=dict(op="link", path="../child #%.svg"),
             result=dict(value="../child%20%23%25.svg")),
    ]


class Gates(unittest.TestCase):
    def test_complete_results(self):
        original = calls()
        self.assertEqual(links.compare(original, [c["result"] for c in original])[0]["outcome"], "match")

    def test_incomplete_inventory(self):
        original = calls()
        for results in [[], [original[0]["result"]], [c["result"] for c in original] + [{}]]:
            with self.assertRaises(AssertionError):
                links.compare(original, results)

    def test_original_calls_cannot_be_replaced(self):
        original = calls()
        original[1]["input"]["op"] = "relink"
        with self.assertRaises(AssertionError):
            links.validate_calls(original)

    def test_no_output_fields_or_errors_ignored(self):
        original = calls()
        for changed in [dict(diagram={"shapes": []}, error=""),
                        dict(diagram={"shapes": [], "connections": []}, error="failed")]:
            results = [changed, original[1]["result"]]
            self.assertEqual(links.compare(original, results)[0]["outcome"], "mismatch")

    def test_no_unescaping_or_path_normalization(self):
        original = calls()
        results = [copy.deepcopy(c["result"]) for c in original]
        results[1]["value"] = "../child #%.svg"
        self.assertEqual(links.compare(original, results)[0]["outcome"], "mismatch")

    def test_json_types_remain_distinct(self):
        self.assertFalse(links.type_equal({"flag": True}, {"flag": 1}))
        self.assertFalse(links.type_equal(None, []))

    def test_each_node_and_connection_link_is_compared(self):
        original = calls()
        original[0]["input"]["diagram"] = {
            "shapes": [{"link": "root.layers.child"}],
            "connections": [{"link": "root.layers.child"}],
        }
        original[0]["result"]["diagram"] = {
            "shapes": [{"link": "child%20%23%25.svg"}],
            "connections": [{"link": "child%20%23%25.svg"}],
        }
        for field in ["shapes", "connections"]:
            results = [copy.deepcopy(c["result"]) for c in original]
            results[0]["diagram"][field][0]["link"] = "root.layers.child"
            self.assertEqual(links.compare(original, results)[0]["outcome"], "mismatch")


if __name__ == "__main__":
    unittest.main()
