"""Fail-closed gates for the original socket/filesystem policy captures."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "asset_policy", Path(__file__).with_name("asset-policy-scoreboard.py")
)
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class CaptureIntegrity(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.capture = Path(self.tmp.name)
        (self.capture / "go").mkdir()
        self.tier = "localfile"
        (self.capture / "source.json").write_text(
            json.dumps(scoreboard.source(self.tier))
        )
        (self.capture / "inventory.json").write_text(
            json.dumps(scoreboard.TESTS[self.tier])
        )
        (self.capture / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        self.events = {t: "pass" for t in scoreboard.TESTS[self.tier]}
        self.events.update(
            {
                "TestPolicyRejectsFIFOWithoutBlocking/" + name: "pass"
                for name in ["rooted", "unrestricted"]
            }
        )
        for i, name in enumerate(scoreboard.TESTS[self.tier]):
            (self.capture / "go" / f"{i}.json").write_text(
                json.dumps(
                    {
                        "name": name,
                        "operations": [
                            {"request": {"op": "close"}, "got": {"error": ""}}
                        ]
                        * scoreboard.LOCALFILE_OPERATIONS[name],
                    }
                )
            )
        self.seal()

    def seal(self):
        (self.capture / "go.jsonl").write_text(
            "\n".join(
                json.dumps({"Test": t, "Action": a}) for t, a in self.events.items()
            )
        )
        (self.capture / "hashes.json").write_text(
            json.dumps(scoreboard.hashes(self.capture))
        )

    def test_complete(self):
        self.assertEqual(len(scoreboard.verify(self.capture, self.tier)), 8)

    def test_missing_function_resealed(self):
        next((self.capture / "go").glob("*.json")).unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture, self.tier)

    def test_changed_capture(self):
        next((self.capture / "go").glob("*.json")).write_text("{}")
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture, self.tier)

    def test_failed_subtest(self):
        self.events["TestPolicyRejectsFIFOWithoutBlocking/rooted"] = "fail"
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture, self.tier)

    def test_timeout_or_budget_is_failure(self):
        for reason in ["timeout", "budget-exceeded"]:
            with self.subTest(reason=reason):
                (self.capture / "run.json").write_text(
                    json.dumps({"exit": 0, "failure": reason})
                )
                self.seal()
                with self.assertRaises(AssertionError):
                    scoreboard.verify(self.capture, self.tier)

    def test_empty_operations_resealed(self):
        file = next((self.capture / "go").glob("*.json"))
        value = json.loads(file.read_text())
        value["operations"] = []
        file.write_text(json.dumps(value))
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture, self.tier)

    def test_changed_source(self):
        with (
            patch.object(scoreboard, "source", return_value={}),
            self.assertRaises(AssertionError),
        ):
            scoreboard.verify(self.capture, self.tier)

    def test_missing_single_operation_resealed(self):
        file = next((self.capture / "go").glob("*.json"))
        value = json.loads(file.read_text())
        value["operations"].pop()
        file.write_text(json.dumps(value))
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture, self.tier)


if __name__ == "__main__":
    unittest.main()
