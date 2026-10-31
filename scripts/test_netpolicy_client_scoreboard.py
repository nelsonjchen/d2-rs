"""Fail-closed gates for original netpolicy client-boundary observations."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "netpolicy_client_scoreboard",
    Path(__file__).with_name("netpolicy-client-scoreboard.py"),
)
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class CaptureIntegrity(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.capture = Path(self.tmp.name)
        (self.capture / "go").mkdir()
        (self.capture / "source.json").write_text(json.dumps(scoreboard.source()))
        (self.capture / "inventory.json").write_text(json.dumps(scoreboard.TESTS))
        (self.capture / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        self.events = {t: "pass" for t in scoreboard.TESTS}
        for i, (name, count) in enumerate(scoreboard.TESTS.items()):
            operations = []
            for index in range(count):
                test = (
                    f"{name}/case_{index}"
                    if name == "TestNonPublicAddressRanges"
                    else name
                )
                self.events[test] = "pass"
                operations.append(
                    {
                        "test": test,
                        "request": {"op": "public", "address": "8.8.8.8"},
                        "got": True,
                    }
                )
            (self.capture / "go" / f"{i}.json").write_text(
                json.dumps({"name": name, "test": name, "operations": operations})
            )
        self.seal()

    def seal(self):
        (self.capture / "go.jsonl").write_text(
            "\n".join(
                json.dumps({"Test": t, "Action": a}) for t, a in self.events.items()
            )
        )
        (self.capture / "hashes.json").write_text(
            json.dumps(scoreboard.base.hashes(self.capture))
        )

    def test_complete(self):
        self.assertEqual(len(scoreboard.verify(self.capture)), 4)

    def test_changed_bytes(self):
        next((self.capture / "go").glob("*.json")).write_text("{}")
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_missing_or_duplicate_function_resealed(self):
        file = next((self.capture / "go").glob("*.json"))
        original = file.read_bytes()
        file.unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)
        file.write_bytes(original)
        file.with_name("duplicate.json").write_bytes(original)
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_missing_operation_resealed(self):
        file = next((self.capture / "go").glob("*.json"))
        value = json.loads(file.read_text())
        value["operations"].pop()
        file.write_text(json.dumps(value))
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_failed_assertion_or_subtest(self):
        self.events["TestNonPublicAddressRanges/public_8.8.8.8"] = "fail"
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_timeout_and_budget_fail(self):
        for reason in ["timeout", "budget-exceeded"]:
            with self.subTest(reason=reason):
                (self.capture / "run.json").write_text(
                    json.dumps({"exit": 0, "failure": reason})
                )
                self.seal()
                with self.assertRaises(AssertionError):
                    scoreboard.verify(self.capture)

    def test_stale_source(self):
        with (
            patch.object(scoreboard, "source", return_value={}),
            self.assertRaises(AssertionError),
        ):
            scoreboard.verify(self.capture)

    def test_missing_native_output_is_not_match(self):
        cases = scoreboard.verify(self.capture)
        with self.assertRaises(AssertionError):
            scoreboard.score(cases, [])

    def test_error_or_history_changes_fail_exactly(self):
        case = {
            "name": "dial",
            "operations": [
                {
                    "got": {
                        "error_kind": "blocked",
                        "hits": ["/first", "/second"],
                        "body": "normal transport",
                    }
                }
            ],
        }
        self.assertEqual(
            scoreboard.score(
                [case],
                [
                    [
                        {
                            "error_kind": "blocked",
                            "hits": ["/first", "/second"],
                            "body": "normal transport",
                        }
                    ]
                ],
            )[0]["outcome"],
            "match",
        )
        for actual in [
            {
                "error_kind": "other",
                "hits": ["/first", "/second"],
                "body": "normal transport",
            },
            {
                "error_kind": "blocked",
                "hits": ["/first", "/second", "/private"],
                "body": "normal transport",
            },
        ]:
            self.assertEqual(
                scoreboard.score([case], [[actual]])[0]["outcome"], "mismatch"
            )


if __name__ == "__main__":
    unittest.main()
