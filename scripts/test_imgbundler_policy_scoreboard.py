"""Integrity and filesystem admission gates for original local policy captures."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "policy_scoreboard", Path(__file__).with_name("imgbundler-policy-scoreboard.py")
)
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class Integrity(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.capture = Path(self.tmp.name)
        (self.capture / "go").mkdir()
        (self.capture / "fs").mkdir()
        (self.capture / "source.json").write_text(json.dumps(scoreboard.source()))
        (self.capture / "inventory.json").write_text(json.dumps(scoreboard.TESTS))
        (self.capture / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        self.events = {t: "pass" for t in scoreboard.TESTS}
        for name in scoreboard.expected():
            (self.capture / "go" / (name + ".json")).write_text(
                json.dumps({"name": name})
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
        self.assertEqual(len(scoreboard.verify(self.capture)), 8)

    def test_missing_call_even_when_resealed(self):
        next((self.capture / "go").glob("*.json")).unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_modified_capture(self):
        next((self.capture / "go").glob("*.json")).write_text("{}")
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_failed_original_test(self):
        self.events[next(iter(scoreboard.TESTS))] = "fail"
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_timeout(self):
        (self.capture / "run.json").write_text(
            json.dumps({"exit": 0, "failure": "timeout"})
        )
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_refuse_foreign_or_preexisting_paths(self):
        for path in [
            self.capture,
            self.capture / "fs",
            self.capture / "other",
            self.capture / "fs/../other",
        ]:
            with self.subTest(path=path), self.assertRaises(AssertionError):
                scoreboard.materialize(
                    self.capture, {"fixture": str(path), "entries": {}}
                )
        owned = self.capture / "fs/owned"
        owned.mkdir()
        with self.assertRaises(AssertionError):
            scoreboard.materialize(self.capture, {"fixture": str(owned), "entries": {}})

    def test_reject_parent_entries(self):
        with self.assertRaises(AssertionError):
            scoreboard.materialize(
                self.capture,
                {
                    "fixture": str(self.capture / "fs/owned"),
                    "entries": {"../escape": {"body": ""}},
                },
            )


if __name__ == "__main__":
    unittest.main()
