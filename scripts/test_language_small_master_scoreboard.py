#!/usr/bin/env python3
"""Missing calls, failed assertions and incomplete native output cannot pass."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location("small", Path(__file__).with_name("language-small-master-scoreboard.py"))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class SmallContracts(unittest.TestCase):
    def fixture(self, path):
        cases = json.loads(cached_fixture("crates/d2-rs-language/tests/fixtures/master-small.json").read_text())["cases"]
        (path / "go").mkdir()
        for i, case in enumerate(cases):
            (path / f"go/{i}.json").write_text(json.dumps(case))
        (path / "source.json").write_text(json.dumps(scoreboard.source()))
        (path / "inventory.json").write_text(json.dumps(scoreboard.inventory()))
        (path / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        tests = {case["test"] for case in cases} | {r["test"] for r in scoreboard.inventory()}
        (path / "go.jsonl").write_text("\n".join(json.dumps({"Action": "pass", "Test": test}) for test in tests))
        self.seal(path)
        return cases

    def seal(self, path):
        (path / "hashes.json").write_text(json.dumps(scoreboard.hashes(path)))

    def test_complete_original_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            self.fixture(path)
            self.assertEqual(len(scoreboard.verify(path)), 42)

    def test_capture_failures(self):
        for mutation in ("missing", "extra", "duplicate", "assertion", "budget", "pin", "hook", "hash", "unexpected", "partial"):
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
                elif mutation == "partial":
                    p = next(p for p in (path / "go").glob("*.json") if json.loads(p.read_text())["request"]["op"] == "grid")
                    data = json.loads(p.read_text())
                    del data["got"]["error"]
                    p.write_text(json.dumps(data))
                else:
                    (path / "go/unexpected.log").write_text("extra")
                if mutation != "hash":
                    self.seal(path)
                with self.assertRaises(AssertionError):
                    scoreboard.verify(path)

    def test_native_failures_and_exact_results(self):
        for mutation, outcome in (("complete", "match"), ("missing", "rust-failed"), ("corrupt", "invalid-output"), ("exit", "rust-failed"), ("timeout", "timeout"), ("value", "mismatch")):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                cases = self.fixture(path)
                expected = {json.dumps(c["request"], sort_keys=True): c["got"] for c in cases}
                runner = path / "runner"
                runner.write_bytes(b"stable runner")
                original = scoreboard.master.module
                cli = original("small_test_cli", "cli-scoreboard.py")

                def run(args, env, *unused, expected=expected, mutation=mutation):
                    request = json.loads(Path(env["D2RS_INTERNAL_REQUEST"]).read_text())
                    got = expected[json.dumps(request, sort_keys=True)]
                    if mutation == "value":
                        got = {"changed": True}
                    result = Path(env["D2RS_INTERNAL_RESULT"])
                    if mutation != "missing":
                        result.write_text("{" if mutation == "corrupt" else json.dumps({"got": got, "observations": None}))
                    return {"exit": 1 if mutation in ("exit", "timeout") else 0, "failure": "timeout" if mutation == "timeout" else None}

                def module(name, filename, cli=cli, original=original, run=run):
                    if filename == "cli-scoreboard.py":
                        return SimpleNamespace(events=cli.events, bounded_run=run)
                    return original(name, filename)

                report = path / "result.json"
                with patch.object(scoreboard.master, "module", side_effect=module):
                    scoreboard.score(path, runner, report)
                self.assertEqual(json.loads(report.read_text())["summary"], {outcome: 42})


if __name__ == "__main__":
    unittest.main()
