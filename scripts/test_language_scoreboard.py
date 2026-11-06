#!/usr/bin/env python3
"""Failure gates for the upstream language scoreboard and delta tool."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("language_scoreboard", Path(__file__).with_name("language-scoreboard.py"))
SCORE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCORE)


class ScoreboardGate(unittest.TestCase):
    def test_exact_structure_distinguishes_null_empty_and_missing(self):
        self.assertTrue(SCORE.differences({"v": None}, {"v": []}))
        self.assertTrue(SCORE.differences({"v": None}, {}))
        self.assertTrue(SCORE.differences([1, 2], [2, 1]))
        self.assertTrue(SCORE.differences({"v": True}, {"v": 1}))
        self.assertTrue(SCORE.differences({"v": False}, {"v": 0}))
        self.assertFalse(SCORE.differences({"a": 1, "b": 2}, {"b": 2, "a": 1}))

    def test_empty_dump_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(AssertionError):
                SCORE.import_dump(Path(directory), Path(directory) / "cases.json")

    def test_text_dump_qualifies_current_go_without_claiming_graph_match(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "d2oracle").mkdir()
            for index, text in enumerate(["expected", "different"]):
                (root / "d2oracle" / f"{index}.json").write_text(json.dumps({
                    "package":"d2oracle", "name":f"TestOnlyText/{index}",
                    "request":{"result_kind":"text", "expected_text":"expected", "expected_error":""},
                    "go":text, "error":"",
                }))
            rows = SCORE.import_dump(root, root / "cases.json")
            self.assertEqual([row["go_reproduces_text"] for row in rows], [True, False])
            self.assertTrue(all(row["go_reproduces_golden"] is None for row in rows))

    def case(self, directory):
        golden = Path(directory) / "case.exp.json"
        golden.write_text('{"ast":null,"err":null}')
        return {"package": "d2parser", "name": "case", "golden": str(golden),
                "golden_sha256": SCORE.digest(golden), "go_reproduces_golden": True,
                "request": {"input": "", "path": "case.d2"}}

    def binary(self, directory, body):
        path = Path(directory) / "runner"
        path.write_text(f"#!{sys.executable}\n" + body)
        path.chmod(0o755)
        return path

    def test_missing_empty_and_invalid_output_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            case = self.case(directory)
            for body, expected in [("pass\n", "native-failed"), ("print('broken')\n", "invalid-output"), ("print('{}')\n", "missing-output")]:
                row = SCORE.score(self.binary(directory, body), case, 2)
                self.assertEqual(row["outcome"], expected)

    def test_timeout_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            row = SCORE.score(self.binary(directory, "import time\ntime.sleep(10)\n"), self.case(directory), .05)
            self.assertEqual(row["outcome"], "timeout")

    def test_matching_rust_cannot_pass_stale_golden(self):
        with tempfile.TemporaryDirectory() as directory:
            case = self.case(directory)
            case["go_reproduces_golden"] = False
            row = SCORE.score(self.binary(directory, "print('{\"value\":{\"ast\":null,\"err\":null}}')\n"), case, 2)
            self.assertEqual(row["outcome"], "stale-golden")

    def test_changed_or_missing_golden_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            case = self.case(directory)
            Path(case["golden"]).write_text("null")
            with self.assertRaises(AssertionError):
                SCORE.score(self.binary(directory, "print('{}')\n"), case, 2)
            Path(case["golden"]).unlink()
            with self.assertRaises(FileNotFoundError):
                SCORE.score(self.binary(directory, "print('{}')\n"), case, 2)

    def text_case(self, directory):
        case = self.case(directory)
        case.update(package="d2oracle", error="", go_reproduces_text=True,
                    go_reproduces_golden=None)
        case["request"].update(result_kind="text", expected_text="x\n", expected_error="")
        return case

    def test_text_match_does_not_bind_historical_graph(self):
        with tempfile.TemporaryDirectory() as directory:
            case = self.text_case(directory)
            # Use a structured emitter to preserve the newline in JSON.
            runner = self.binary(directory, "import json\nprint(json.dumps({'text':'x\\n'}))\n")
            row = SCORE.score(runner, case, 2)
            self.assertEqual(row["outcome"], "unbound-golden")
            self.assertEqual(row["text_outcome"], "text-match")
            case["golden"] = None
            row = SCORE.score(runner, case, 2)
            self.assertEqual(row["outcome"], "no-golden")
            self.assertEqual(row["text_outcome"], "text-match")

    def test_text_only_requires_qualified_go_and_complete_native_output(self):
        with tempfile.TemporaryDirectory() as directory:
            case = self.text_case(directory)
            for actual, expected in [({}, "text-mismatch"), ({"text":"wrong"}, "text-mismatch"),
                                     ({"unsupported":"api"}, "unsupported"),
                                     ({"text":"x\n","error":"unexpected"}, "text-mismatch")]:
                runner = self.binary(directory, f"print({json.dumps(json.dumps(actual))})\n")
                self.assertEqual(SCORE.score(runner, case, 2)["text_outcome"], expected)
            case["go_reproduces_text"] = False
            runner = self.binary(directory, "import json\nprint(json.dumps({'text':'x\\n'}))\n")
            self.assertEqual(SCORE.score(runner, case, 2)["text_outcome"], "stale-text")

    def test_text_only_still_checks_historical_golden_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            case = self.text_case(directory)
            Path(case["golden"]).write_text("null")
            with self.assertRaises(AssertionError):
                SCORE.score(self.binary(directory, "print('{}')\n"), case, 2)
            Path(case["golden"]).unlink()
            with self.assertRaises(FileNotFoundError):
                SCORE.score(self.binary(directory, "print('{}')\n"), case, 2)

    def test_delta_rejects_missing_keys_and_lost_text_matches(self):
        with tempfile.TemporaryDirectory() as directory:
            before = Path(directory) / "before.json"
            after = Path(directory) / "after.json"
            before.write_text(json.dumps({"rows":[{"package":"d2oracle","name":"a","outcome":"unsupported-graph","text_outcome":"text-match"}]}))
            script = Path(__file__).with_name("language-scoreboard-delta.py")
            for rows in [[], [{"package":"d2oracle","name":"a","outcome":"unsupported-graph","text_outcome":"unsupported"}]]:
                after.write_text(json.dumps({"rows":rows}))
                result = subprocess.run([sys.executable,str(script),str(before),str(after)],capture_output=True)
                self.assertNotEqual(result.returncode,0)


if __name__ == "__main__":
    unittest.main()
