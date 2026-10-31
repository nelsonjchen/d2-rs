#!/usr/bin/env python3
"""The class-index gate must retain every original lookup and assertion."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("class_index", Path(__file__).with_name("language-class-index-master-scoreboard.py"))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class CaptureIntegrity(unittest.TestCase):
    def fixture(self, path):
        (path / "go").mkdir()
        (path / "inventory.json").write_text(json.dumps([{"path": scoreboard.RELATIVE, "test": test} for test in scoreboard.TESTS]))
        for test, count in scoreboard.TESTS.items():
            case = {"name": test + "#0", "test": test, "request": {"queries": ["class"] * count},
                    "got": {"results": [None] * count, "error": "", "error_kind": "", "indexed_roots": 1, "context_checks": None}}
            (path / "go" / (test + ".json")).write_text(json.dumps(case))
        (path / "go.jsonl").write_text("\n".join(json.dumps({"Action": "pass", "Test": test}) for test in scoreboard.TESTS))
        (path / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        pinned = scoreboard.ROOT / scoreboard.references.MANIFEST["d2"]["checkout"] / scoreboard.RELATIVE
        hook = scoreboard.ROOT / "tools/language-master-scoreboard/class_index_hook_test.go"
        (path / "source.json").write_text(json.dumps({"source_commit": scoreboard.references.SOURCE_COMMIT,
            "test_sha256": scoreboard.hashlib.sha256(pinned.read_bytes()).hexdigest(),
            "hook_sha256": scoreboard.hashlib.sha256(hook.read_bytes()).hexdigest()}))
        self.seal(path)

    def seal(self, path):
        (path / "hashes.json").write_text(json.dumps(scoreboard.hashes(path)))

    def test_complete_inventory_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            self.fixture(path)
            scoreboard.verify(path)

    def test_missing_extra_duplicate_session_fails(self):
        for change in ("missing", "extra", "duplicate"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                case = next((path / "go").glob("*.json"))
                if change == "missing":
                    case.unlink()
                else:
                    data = json.loads(case.read_text())
                    if change == "extra":
                        data["name"] += "extra"
                    (path / "go/extra.json").write_text(json.dumps(data))
                self.seal(path)
                with self.assertRaises(AssertionError):
                    scoreboard.verify(path)

    def test_truncated_results_or_queries_and_incomplete_results_fail(self):
        for change in ("results", "queries", "indexed_roots"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                case = next((path / "go").glob("*.json"))
                data = json.loads(case.read_text())
                if change == "queries":
                    data["request"][change].pop()
                elif change == "results":
                    data["got"][change].pop()
                else:
                    del data["got"][change]
                case.write_text(json.dumps(data))
                self.seal(path)
                with self.assertRaises(AssertionError):
                    scoreboard.verify(path)

    def test_failed_assertion_budget_or_changed_capture_cannot_pass(self):
        for change in ("assertion", "budget", "hash"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                if change == "assertion":
                    (path / "go.jsonl").write_text("")
                    self.seal(path)
                elif change == "budget":
                    (path / "run.json").write_text(json.dumps({"exit": -15, "failure": "budget-exceeded"}))
                    self.seal(path)
                else:
                    (path / "go.jsonl").write_text("")
                with self.assertRaises(AssertionError):
                    scoreboard.verify(path)


if __name__ == "__main__":
    unittest.main()
