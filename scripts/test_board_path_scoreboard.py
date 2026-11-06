#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("paths", Path(__file__).with_name("board-path-scoreboard.py"))
paths = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paths)


def calls():
    names = ["TestBoardOutputComponent"] * 62 + [
        "TestValidateBoardOutputPathsRejectsCollisions/case_insensitive_filesystem",
        "TestValidateBoardOutputPathsRejectsCollisions/same_escaped_component",
    ] + ["TestValidateBoardOutputPathsRejectsFileDirectoryPrefixCollision"] * 2 + ["TestAppendBoardOutputNameStaysContained"] * 9
    return [dict(test=name, input=dict(op="component", bytes=[255] if i in [60, 61] else [97]) if i < 62
                 else dict(op="validate") if i < 66 else dict(op="append_name" if i < 73 else "within" if i == 73 else "append_component"), result=dict(value="component") if i < 62 else dict(error="collision"))
            for i, name in enumerate(names)]


class Gates(unittest.TestCase):
    def test_complete_results_match(self):
        original = calls()
        self.assertTrue(all(r["outcome"] == "match" for r in paths.compare(original, [c["result"] for c in original])))

    def test_missing_or_extra_response_rejected(self):
        original = calls()
        for results in [[], [c["result"] for c in original][:-1], [c["result"] for c in original] + [{}]]:
            with self.assertRaises(AssertionError):
                paths.compare(original, results)

    def test_missing_original_session_rejected(self):
        original = calls()
        original[63]["test"] = original[62]["test"]
        with self.assertRaises(AssertionError):
            paths.validate_calls(original)

    def test_complete_errors_compare_without_normalization(self):
        original = calls()
        results = [c["result"] for c in original]
        results[-1] = dict(error="different collision")
        self.assertEqual(paths.compare(original, results)[-1]["outcome"], "mismatch")

    def test_missing_containment_operation_rejected(self):
        original = calls()
        original[-1]["input"]["op"] = "append_name"
        with self.assertRaises(AssertionError):
            paths.validate_calls(original)

    def test_full_containment_paths_and_errors_compare(self):
        original = calls()
        for i in range(66, 75):
            results = [c["result"] for c in original]
            results[i] = dict(value="different", error="")
            self.assertEqual(paths.compare(original, results)[-1]["outcome"], "mismatch")

    def test_invalid_utf8_stays_nonpassing(self):
        original = calls()
        results = [c["result"] for c in original]
        for i in [60, 61]:
            results[i] = dict(unsupported="native board names are UTF-8 strings")
        row = paths.compare(original, results)[0]
        self.assertEqual((row["outcome"], row["matched_calls"], row["unsupported_calls"]), ("unsupported", 60, 2))

    def test_valid_utf8_cannot_be_excluded(self):
        original = calls()
        results = [c["result"] for c in original]
        results[0] = dict(unsupported="native board names are UTF-8 strings")
        self.assertEqual(paths.compare(original, results)[0]["outcome"], "mismatch")


if __name__ == "__main__":
    unittest.main()
