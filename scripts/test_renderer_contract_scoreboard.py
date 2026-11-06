#!/usr/bin/env python3
"""Failure gates for the original renderer-call capture and replay."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "renderer_contract", Path(__file__).with_name("renderer-contract-scoreboard.py"))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class Gate(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "source.json").write_text(json.dumps(scoreboard.source()))
        for key, suite in scoreboard.SUITES.items():
            root = self.root / key
            (root / "calls").mkdir(parents=True)
            (root / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
            (root / "events.jsonl").write_text("\n".join(
                json.dumps({"Test": test, "Action": "pass"}) for test in suite["tests"]))
            for test, count in suite["tests"].items():
                for i in range(count):
                    row = {"name": f"{test}#{i}", "test": test, "package": suite["package"],
                           "request": {}, "got": {"output": None, "error": ""}}
                    (root / "calls" / f"{test}-{i}.json").write_text(json.dumps(row))
        self.seal()

    def seal(self):
        (self.root / "hashes.json").write_text(json.dumps(scoreboard.hashes(self.root)))

    def test_complete(self):
        self.assertEqual(len(scoreboard.verify(self.root)), 15)

    def test_missing_or_extra_even_resealed(self):
        next((self.root / "animate/calls").glob("*.json")).unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.root)

    def test_duplicate_call_even_resealed(self):
        path = next((self.root / "scene/calls").glob("*.json"))
        (path.parent / "extra.json").write_bytes(path.read_bytes())
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.root)

    def test_changed_bytes(self):
        (self.root / "scene/events.jsonl").write_text("")
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.root)

    def test_original_assertions_required_even_resealed(self):
        (self.root / "scene/events.jsonl").write_text("")
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.root)

    def test_timeout_or_budget_never_pass(self):
        for failure in ["timeout", "budget-exceeded", "free-space-exhausted"]:
            (self.root / "scene/run.json").write_text(json.dumps({"exit": -15, "failure": failure}))
            self.seal()
            with self.assertRaises(AssertionError):
                scoreboard.verify(self.root)

    def test_transform_preserves_literals_and_assertions(self):
        original = 'out, err := Wrap(d) // Wrap(example)\nif err != nil { t.Fatal("Wrap() error") }'
        expected = 'out, err := d2rsWrap(t, d) // Wrap(example)\nif err != nil { t.Fatal("Wrap() error") }'
        self.assertEqual(scoreboard.transform(original, {"Wrap(": "d2rsWrap(t, "}), expected)


if __name__ == "__main__":
    unittest.main()
