#!/usr/bin/env python3
"""The capture gate rejects incomplete, failed or changed original-test evidence."""

import binascii
import importlib.util
import json
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "wasm_admission_scoreboard", Path(__file__).with_name("wasm-admission-scoreboard.py")
)
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class CaptureGate(unittest.TestCase):
    expected_count = 5
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.capture = Path(self.temp.name)
        (self.capture / "go").mkdir()
        for i, name in enumerate(
            f"{t}#{i}"
            for t, n in {t: n for t, n in scoreboard.COUNTS.items()}.items()
            for i in range(n)
        ):
            (self.capture / "go" / f"{i}.json").write_text(json.dumps({
                "name": name, "test": name.rsplit("#", 1)[0], "request": "{}",
                "got": {"error": {"message": "test error", "code": 400}},
            }))
        for name, value in [
            ("source.json", scoreboard.source()),
            ("inventory.json", {t: n for t, n in scoreboard.COUNTS.items()}),
            ("run.json", {"exit": 0, "failure": None}),
        ]:
            (self.capture / name).write_text(json.dumps(value))
        (self.capture / "go.jsonl").write_text(
            "\n".join(
                json.dumps({"Test": t, "Action": "pass"}) for t in scoreboard.TESTS
            )
        )
        self.seal()

    def seal(self):
        (self.capture / "hashes.json").write_text(
            json.dumps(scoreboard.hashes(self.capture))
        )

    def test_complete_capture(self):
        self.assertEqual(len(scoreboard.verify(self.capture)), self.expected_count)

    def test_missing_even_with_updated_hashes(self):
        next((self.capture / "go").glob("*.json")).unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_failed_upstream(self):
        (self.capture / "run.json").write_text(json.dumps({"exit": 1, "failure": None}))
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_changed_bytes(self):
        (self.capture / "go.jsonl").write_text("")
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_missing_original_assertions(self):
        (self.capture / "go.jsonl").write_text("")
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_extra_call_even_with_updated_hashes(self):
        (self.capture / "go" / "extra.json").write_text(
            json.dumps({"name": scoreboard.TESTS[0] + "#999"})
        )
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_changed_function_inventory(self):
        inventory = dict(scoreboard.COUNTS)
        inventory[scoreboard.TESTS[0]] -= 1
        (self.capture / "inventory.json").write_text(json.dumps(inventory))
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_empty_or_corrupt_svg_never_passes(self):
        for data in ["", "garbage", "PHN2Zz4=", "bm90IHN2Zw=="]:
            with self.subTest(data=data), self.assertRaises((AssertionError, binascii.Error, ET.ParseError)):
                scoreboard.validate_response({"data": data})

    def test_error_code_and_complete_response_are_required(self):
        for response in [{}, {"error": {"message": "error"}},
                         {"error": {"message": "error", "code": "400"}},
                         {"data": "", "error": {"message": "error", "code": 400}}]:
            with self.subTest(response=response), self.assertRaises(AssertionError):
                scoreboard.validate_response(response)


if __name__ == "__main__":
    unittest.main()
