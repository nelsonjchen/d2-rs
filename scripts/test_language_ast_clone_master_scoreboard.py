#!/usr/bin/env python3
"""A successful capture requires every original call and original assertion."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("clone", Path(__file__).with_name("language-ast-clone-master-scoreboard.py"))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class CaptureIntegrity(unittest.TestCase):
    def fixture(self, path):
        (path / "go").mkdir()
        tests = [{"path": scoreboard.RELATIVE, "test": test} for test in scoreboard.TESTS]
        (path / "inventory.json").write_text(json.dumps(tests))
        for test, count in scoreboard.TESTS.items():
            for index in range(count):
                case = {"name": f"{test}#{index}", "test": test, "got": {"map": None, "error": "context canceled", "error_kind": "canceled"}}
                (path / "go" / f"{test}-{index}.json").write_text(json.dumps(case))
        (path / "go.jsonl").write_text("\n".join(json.dumps({"Action": "pass", "Test": test}) for test in scoreboard.TESTS))
        (path / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        pinned = scoreboard.ROOT / scoreboard.references.MANIFEST["d2"]["checkout"] / scoreboard.RELATIVE
        (path / "source.json").write_text(json.dumps({"source_commit": scoreboard.references.SOURCE_COMMIT,
                                                      "test_sha256": scoreboard.hashlib.sha256(pinned.read_bytes()).hexdigest()}))
        self.seal(path)

    def seal(self, path):
        (path / "hashes.json").write_text(json.dumps(scoreboard.hashes(path)))

    def test_complete_original_inventory_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            self.fixture(path)
            scoreboard.verify(path)

    def test_missing_extra_duplicate_calls_fail_after_resealing(self):
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
                with self.assertRaisesRegex(AssertionError, "original clone call"):
                    scoreboard.verify(path)

    def test_failed_run_or_original_assertion_cannot_pass(self):
        for change in ("budget", "assertion"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                if change == "budget":
                    (path / "run.json").write_text(json.dumps({"exit": -15, "failure": "budget-exceeded"}))
                else:
                    (path / "go.jsonl").write_text("")
                self.seal(path)
                with self.assertRaises(AssertionError):
                    scoreboard.verify(path)

    def test_changed_or_incomplete_result_cannot_pass(self):
        for change in ("changed", "incomplete"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                case = next((path / "go").glob("*.json"))
                data = json.loads(case.read_text())
                del data["got"]["map"]
                case.write_text(json.dumps(data))
                if change == "incomplete":
                    self.seal(path)
                with self.assertRaises(AssertionError):
                    scoreboard.verify(path)


if __name__ == "__main__":
    unittest.main()
