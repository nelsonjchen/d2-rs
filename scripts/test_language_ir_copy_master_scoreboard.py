#!/usr/bin/env python3
"""Complete original copy sessions and successful source assertions are mandatory."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("copy", Path(__file__).with_name("language-ir-copy-master-scoreboard.py"))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class CaptureIntegrity(unittest.TestCase):
    def seal(self, path):
        (path / "hashes.json").write_text(json.dumps(scoreboard.hashes(path)))

    def fixture(self, path):
        (path / "go").mkdir()
        (path / "inventory.json").write_text(json.dumps([{"path": scoreboard.RELATIVE, "test": test} for test in scoreboard.TESTS]))
        for test, count in scoreboard.TESTS.items():
            for i in range(count):
                copies = [{"context": "background"}] * (2 if test == "TestVariableExpansionCopyReservationsAreCumulative" else 0)
                case = {"name": f"{test}#{i}", "test": test, "request": {"op": "compile", "copies": []},
                        "got": {"results": [{"ir": None, "imports": None, "error": "", "error_kind": "", "limit": 10, "used": 0}]}}
                if test == "TestVariableExpansionCopyUnitsIncludeSlices":
                    case["request"] = {"op": "node_units", "node": {"values": []}}
                    case["got"] = {"units": 1}
                else:
                    case["request"]["copies"] = copies
                    case["got"]["results"] *= 1 + len(copies)
                name = scoreboard.hashlib.sha256(case["name"].encode()).hexdigest()
                (path / "go" / (name + ".json")).write_text(json.dumps(case))
        (path / "go.jsonl").write_text("\n".join(json.dumps({"Action": "pass", "Test": test}) for test in scoreboard.TESTS))
        (path / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        pinned = scoreboard.ROOT / scoreboard.references.MANIFEST["d2"]["checkout"] / scoreboard.RELATIVE
        hook = scoreboard.ROOT / "tools/language-master-scoreboard/ir_copy_hook_test.go"
        (path / "source.json").write_text(json.dumps({"source_commit": scoreboard.references.SOURCE_COMMIT,
            "test_sha256": scoreboard.hashlib.sha256(pinned.read_bytes()).hexdigest(),
            "hook_sha256": scoreboard.hashlib.sha256(hook.read_bytes()).hexdigest()}))
        self.seal(path)

    def test_complete_inventory_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            self.fixture(path)
            scoreboard.verify(path)

    def test_missing_extra_duplicate_calls_fail(self):
        for change in ("missing", "extra", "duplicate"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                case = next(p for p in (path / "go").glob("*.json") if json.loads(p.read_text())["test"] == "TestVariableExpansionCopyReservationsAreCumulative")
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

    def test_mutations_typed_errors_assertions_and_hashes_are_required(self):
        for change in ("copies", "lost_operation", "error_kind", "assertion", "budget", "hash", "pin"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                case = next(p for p in (path / "go").glob("*.json") if json.loads(p.read_text())["test"] == "TestVariableExpansionCopyReservationsAreCumulative")
                data = json.loads(case.read_text())
                if change == "copies":
                    data["request"][change] = None
                elif change == "lost_operation":
                    data["request"]["copies"].pop()
                    data["got"]["results"].pop()
                elif change == "error_kind":
                    del data["got"]["results"][0][change]
                elif change in ("assertion", "hash"):
                    (path / "go.jsonl").write_text("")
                elif change == "budget":
                    (path / "run.json").write_text(json.dumps({"exit": -15, "failure": "budget-exceeded"}))
                else:
                    (path / "source.json").write_text(json.dumps({"source_commit": "wrong"}))
                case.write_text(json.dumps(data))
                if change != "hash":
                    self.seal(path)
                with self.assertRaises(AssertionError):
                    scoreboard.verify(path)


if __name__ == "__main__":
    unittest.main()
