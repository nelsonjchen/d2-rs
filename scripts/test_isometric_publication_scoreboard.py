#!/usr/bin/env python3
"""Complete-call and artifact gates for the original publication tests."""

import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "publication", Path(__file__).with_name("isometric-publication-scoreboard.py")
)
publication = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publication)


def fixture():
    result = {}
    for name in publication.TESTS:
        calls = []
        for i in range(publication.COUNTS[name]):
            nodes = {
                ".": {"type": "directory", "mode": 448},
                "old.svg": {"type": "file", "mode": 384, "bytes": "b2xk"},
            }
            row = {
                "name": name,
                "call": i,
                "before": nodes,
                "after": copy.deepcopy(nodes),
            }
            if name == publication.TESTS[1]:
                row.update(canceled=i == 0, error="context canceled" if i == 0 else "")
            else:
                row.update(
                    source="a",
                    args=["--isometric"],
                    stdout="",
                    failed=(i == 1 if name == publication.TESTS[0] else True),
                )
            calls.append(row)
        result[name] = calls
    return result


class Gates(unittest.TestCase):
    def test_complete_matches(self):
        expected = fixture()
        self.assertTrue(
            all(
                r["outcome"] == "match" for r in publication.compare(expected, expected)
            )
        )

    def test_partial_native_fails(self):
        expected = fixture()
        for name in publication.TESTS:
            actual = copy.deepcopy(expected)
            actual[name].pop()
            self.assertEqual(
                next(
                    r
                    for r in publication.compare(expected, actual)
                    if r["test"] == name
                )["outcome"],
                "mismatch",
            )

    def test_changed_bytes_modes_stdout_failure_and_temporary_entries_fail(self):
        expected = fixture()
        name = publication.TESTS[0]
        for change in ["bytes", "mode", "stdout", "failed", "extra"]:
            actual = copy.deepcopy(expected)
            row = actual[name][0]
            if change in {"bytes", "mode"}:
                row["after"]["old.svg"][change] = 0
            elif change == "stdout":
                row[change] = "bGVhaw=="
            elif change == "failed":
                row[change] = True
            else:
                row["after"][".d2-isometric-leak"] = {
                    "type": "file",
                    "mode": 384,
                    "bytes": "",
                }
            self.assertEqual(
                publication.compare(expected, actual)[0]["outcome"], "mismatch", change
            )

    def test_missing_original_and_duplicate_call_fail(self):
        for change in ["function", "call", "duplicate"]:
            expected = fixture()
            name = publication.TESTS[0]
            if change == "function":
                del expected[name]
            elif change == "call":
                expected[name].pop()
            else:
                expected[name][1]["call"] = 0
            with self.assertRaises(AssertionError):
                publication.compare(expected, fixture())

    def test_missing_native_and_extra_function_fail(self):
        actual = fixture()
        del actual[publication.TESTS[0]]
        self.assertEqual(
            publication.compare(fixture(), actual)[0]["outcome"], "mismatch"
        )
        actual["extra"] = []
        with self.assertRaises(AssertionError):
            publication.compare(fixture(), actual)

    def test_cancellation_error_is_exact(self):
        actual = fixture()
        actual[publication.TESTS[1]][0]["error"] = "canceled"
        self.assertEqual(
            publication.compare(fixture(), actual)[1]["outcome"], "mismatch"
        )


if __name__ == "__main__":
    unittest.main()
