#!/usr/bin/env python3
"""Original assertions, complete reservations and raw cancellation witnesses gate passing."""
import copy
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("internal", Path(__file__).with_name("language-ir-internal-master-scoreboard.py"))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)
LOGICAL = "crates/d2-rs-language/tests/fixtures/master-ir-internal.json"


def cached_fixture():
    """The regenerated oracle artifact (recipe lang-master-ir-internal, docs/FIXTURES.md)."""
    root = Path(os.environ.get("D2RS_FIXTURES_DIR") or scoreboard.ROOT / "oracle-cache/fixtures")
    path = (root if root.is_absolute() else scoreboard.ROOT / root) / LOGICAL
    if not path.is_file():
        raise AssertionError(f"missing {path}: run `uv run --no-project python scripts/regen-fixtures.py --only {LOGICAL}`")
    return path


class InternalContracts(unittest.TestCase):
    def fixture(self, path):
        fixture = json.loads(cached_fixture().read_text())
        (path / "go").mkdir()
        for case in fixture["cases"]:
            name = scoreboard.hashlib.sha256(case["name"].encode()).hexdigest()
            (path / "go" / (name + ".json")).write_text(json.dumps(case))
        (path / "inventory.json").write_text(json.dumps(scoreboard.inventory()))
        (path / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        (path / "go.jsonl").write_text("\n".join(json.dumps({"Action": "pass", "Test": row["test"]}) for row in scoreboard.inventory()))
        (path / "source.json").write_text(json.dumps({"source_commit": scoreboard.references.SOURCE_COMMIT,
            "hook_sha256": scoreboard.sha(scoreboard.HOOK), "tests": {p: scoreboard.sha(scoreboard.ROOT / scoreboard.references.MANIFEST["d2"]["checkout"] / p) for p in scoreboard.TESTS}}))
        self.seal(path)

    def seal(self, path):
        (path / "hashes.json").write_text(json.dumps(scoreboard.hashes(path)))

    def test_complete_original_sessions_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            self.fixture(path)
            self.assertEqual(len(scoreboard.verify(path)), 14)

    def test_inventory_source_assertions_and_process_failure(self):
        for mutation in ("missing", "extra", "duplicate", "event", "failed", "pin", "hook", "hash", "unexpected"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                case = next((path / "go").glob("*.json"))
                if mutation == "missing":
                    case.unlink()
                elif mutation in ("extra", "duplicate"):
                    data = json.loads(case.read_text())
                    if mutation == "extra":
                        data["name"] += "extra"
                    (path / "go/extra.json").write_text(json.dumps(data))
                elif mutation in ("event", "hash"):
                    (path / "go.jsonl").write_text("")
                elif mutation == "failed":
                    (path / "run.json").write_text(json.dumps({"exit": -15, "failure": "budget-exceeded"}))
                elif mutation in ("pin", "hook"):
                    p = path / "source.json"
                    data = json.loads(p.read_text())
                    data["source_commit" if mutation == "pin" else "hook_sha256"] = "wrong"
                    p.write_text(json.dumps(data))
                else:
                    (path / "go/unexpected.log").write_text("extra")
                if mutation != "hash":
                    self.seal(path)
                with self.assertRaises(AssertionError):
                    scoreboard.verify(path)

    def test_missing_reservations_and_cancellation_witnesses_fail(self):
        for mutation in ("reservation", "runs", "state", "array", "edge", "attempt"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                op = "array_cancel" if mutation == "array" else "edge_cancel" if mutation in ("edge", "attempt") else "budget"
                p = next(p for p in (path / "go").glob("*.json") if json.loads(p.read_text())["request"]["op"] == op and (op != "budget" or json.loads(p.read_text())["got"]["results"]))
                data = json.loads(p.read_text())
                if mutation == "reservation":
                    data["got"]["results"].pop()
                elif mutation == "runs":
                    data["request"]["operations"][0]["count"] = 0
                elif mutation == "state":
                    data["got"]["state"] = None
                elif mutation == "array":
                    data["observations"] = None
                elif mutation == "edge":
                    data["observations"][-1]["used"] = 9
                else:
                    data["observations"].pop(0)
                p.write_text(json.dumps(data))
                self.seal(path)
                with self.assertRaises((AssertionError, TypeError)):
                    scoreboard.verify(path)

    def test_native_raw_witnesses_and_exact_budget_results(self):
        cases = json.loads(cached_fixture().read_text())["cases"]
        for case in cases:
            actual = {key: copy.deepcopy(case[key]) for key in ("got", "observations")}
            self.assertEqual(scoreboard.native_result(case, actual), "match")
            if case["request"]["op"] in ("array_cancel", "edge_cancel"):
                actual["observations"] = None
                with self.assertRaises((AssertionError, TypeError)):
                    scoreboard.native_result(case, actual)
            elif case["request"]["op"] == "budget" and actual["got"]["results"]:
                actual["got"]["results"][-1] = "different error"
                self.assertEqual(scoreboard.native_result(case, actual), "mismatch")


if __name__ == "__main__":
    unittest.main()
