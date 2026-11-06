#!/usr/bin/env python3
"""Original assertion, inventory and native writer failure gates."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("remaining", Path(__file__).with_name("language-remaining-master-scoreboard.py"))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class RemainingContracts(unittest.TestCase):
    def fixture(self, path):
        cases = [
            {"name": "TestMarkdownMatcherNoMatchStorageIsSparse#0", "test": "TestMarkdownMatcherNoMatchStorageIsSparse", "request": {"op": "sparse", "input": "${unknown}", "variables": {"one": "1"}, "limit": 32, "runs": 5}, "got": {"result": "${unknown}", "used": 0, "error": "", "terminal_storage_sparse": True, "allocations_within_limit": True}, "observations": {"allocations_per_run": 0}},
            {"name": "TestDecodeRejectsOversizedScript#0", "test": "TestDecodeRejectsOversizedScript", "request": {"op": "encode", "input": "xxxx"}, "got": {"result": "encoded", "error": ""}},
            {"name": "TestDecodeRejectsOversizedScript#1", "test": "TestDecodeRejectsOversizedScript", "request": {"op": "decode", "input": "encoded"}, "got": {"result": "", "error": "size error"}},
        ]
        (path / "go").mkdir()
        for i, case in enumerate(cases):
            (path / f"go/{i}.json").write_text(json.dumps(case))
        (path / "source.json").write_text(json.dumps(scoreboard.source()))
        (path / "inventory.json").write_text(json.dumps(scoreboard.inventory()))
        (path / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        (path / "go.jsonl").write_text("\n".join(json.dumps({"Action": "pass", "Test": test}) for test in scoreboard.TESTS.values()))
        self.seal(path)
        return cases

    def seal(self, path):
        (path / "hashes.json").write_text(json.dumps(scoreboard.hashes(path)))

    def test_complete_original_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            self.fixture(path)
            self.assertEqual(len(scoreboard.verify(path)), 3)

    def test_capture_failure_gates(self):
        for mutation in ("missing", "extra", "duplicate", "assertion", "budget", "pin", "hook", "hash", "unexpected", "partial", "allocation_observation"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                p = path / "go/0.json"
                if mutation == "missing":
                    p.unlink()
                elif mutation in ("extra", "duplicate"):
                    data = json.loads(p.read_text())
                    if mutation == "extra":
                        data["name"] += "extra"
                    (path / "go/extra.json").write_text(json.dumps(data))
                elif mutation in ("assertion", "hash"):
                    (path / "go.jsonl").write_text("")
                elif mutation == "budget":
                    (path / "run.json").write_text(json.dumps({"exit": -15, "failure": "budget-exceeded"}))
                elif mutation in ("pin", "hook"):
                    p = path / "source.json"
                    data = json.loads(p.read_text())
                    data["source_commit" if mutation == "pin" else "hooks"] = "changed"
                    p.write_text(json.dumps(data))
                elif mutation in ("partial", "allocation_observation"):
                    data = json.loads(p.read_text())
                    if mutation == "partial":
                        del data["got"]["terminal_storage_sparse"]
                    else:
                        data["observations"] = {}
                    p.write_text(json.dumps(data))
                else:
                    (path / "go/unexpected.log").write_text("extra")
                if mutation != "hash":
                    self.seal(path)
                with self.assertRaises((AssertionError, KeyError)):
                    scoreboard.verify(path)

    def test_native_failures_and_exact_results(self):
        for mutation, outcome in (("complete", "match"), ("missing", "rust-failed"), ("corrupt", "invalid-output"), ("exit", "rust-failed"), ("timeout", "timeout"), ("allocation", "mismatch"), ("value", "mismatch")):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                cases = self.fixture(path)
                expected = {json.dumps(c["request"], sort_keys=True): c["got"] for c in cases}
                runner = path / "runner"
                runner.write_bytes(b"stable runner")
                original = scoreboard.master.module
                cli = original("remaining_test_cli", "cli-scoreboard.py")

                def run(args, env, *unused, expected=expected, mutation=mutation):
                    request = json.loads(Path(env["D2RS_REMAINING_REQUEST"]).read_text())
                    got = dict(expected[json.dumps(request, sort_keys=True)])
                    if mutation == "value":
                        got = {"changed": True}
                    elif mutation == "allocation" and request["op"] == "sparse":
                        got["allocations_within_limit"] = False
                    result = Path(env["D2RS_REMAINING_RESULT"])
                    if mutation != "missing":
                        result.write_text("{" if mutation == "corrupt" else json.dumps({"got": got, "observations": {"allocations": 5, "runs": 5}}))
                    return {"exit": 1 if mutation in ("exit", "timeout") else 0, "failure": "timeout" if mutation == "timeout" else None}

                def module(name, filename, cli=cli, original=original, run=run):
                    return SimpleNamespace(events=cli.events, bounded_run=run) if filename == "cli-scoreboard.py" else original(name, filename)

                report = path / "result.json"
                with patch.object(scoreboard.master, "module", side_effect=module):
                    scoreboard.score(path, runner, runner, report)
                expected_counts = {"match": 2, "mismatch": 1} if mutation == "allocation" else {outcome: 3}
                self.assertEqual(json.loads(report.read_text())["summary"], expected_counts)


if __name__ == "__main__":
    unittest.main()
