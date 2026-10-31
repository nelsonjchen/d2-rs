#!/usr/bin/env python3
"""IR corpus integrity gates: retained failures must remain reproducible."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("ir_master", Path(__file__).with_name("language-ir-master-scoreboard.py"))
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)

class Integrity(unittest.TestCase):
    def fixture(self, path):
        (path / "go").mkdir()
        (path / "go/call.json").write_text(json.dumps({"name":"TestExample#0"}))
        (path / "inventory.json").write_text("[]")
        (path / "go.jsonl").write_text("{}")
        (path / "run.json").write_text(json.dumps({"exit":0,"failure":None}))
        pinned = scoreboard.ROOT / scoreboard.references.MANIFEST["d2"]["checkout"] / scoreboard.RELATIVE
        (path / "source.json").write_text(json.dumps({"source_commit":scoreboard.references.SOURCE_COMMIT,"test_sha256":scoreboard.hashlib.sha256(pinned.read_bytes()).hexdigest()}))
        self.seal(path)

    def seal(self, path):
        (path / "hashes.json").write_text(json.dumps(scoreboard.capture_hashes(path)))

    def test_complete_capture_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory); self.fixture(path)
            scoreboard.verify_capture(path)

    def test_missing_changed_or_extra_calls_fail(self):
        for operation in ("missing", "changed", "extra"):
            with self.subTest(operation=operation), tempfile.TemporaryDirectory() as directory:
                path = Path(directory); self.fixture(path)
                if operation == "missing": (path / "go/call.json").unlink()
                elif operation == "changed": (path / "go/call.json").write_text("{}")
                else: (path / "go/extra.json").write_text("{}")
                with self.assertRaises(AssertionError): scoreboard.verify_capture(path)

    def test_failed_run_cannot_be_sealed_as_passing(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory); self.fixture(path)
            (path / "run.json").write_text(json.dumps({"exit":1,"failure":"budget-exceeded"}))
            self.seal(path)
            with self.assertRaises(AssertionError): scoreboard.verify_capture(path)

    def test_pin_and_test_source_must_match(self):
        for field in ("source_commit", "test_sha256"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                path = Path(directory); self.fixture(path)
                source = json.loads((path / "source.json").read_text()); source[field] = "other"
                (path / "source.json").write_text(json.dumps(source)); self.seal(path)
                with self.assertRaises(AssertionError): scoreboard.verify_capture(path)

    def test_duplicate_call_names_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory); self.fixture(path)
            (path / "go/duplicate.json").write_bytes((path / "go/call.json").read_bytes())
            self.seal(path)
            with self.assertRaises(AssertionError): scoreboard.verify_capture(path)

    def test_variable_suite_source_is_verified(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory); self.fixture(path)
            source = json.loads((path / "source.json").read_text())
            source["test_path"] = "d2ir/expansion_test.go"
            pinned = scoreboard.ROOT / scoreboard.references.MANIFEST["d2"]["checkout"] / source["test_path"]
            source["test_sha256"] = scoreboard.hashlib.sha256(pinned.read_bytes()).hexdigest()
            (path / "source.json").write_text(json.dumps(source)); self.seal(path)
            scoreboard.verify_capture(path)
            source["test_path"] = "d2ir/other_test.go"
            (path / "source.json").write_text(json.dumps(source)); self.seal(path)
            with self.assertRaises(AssertionError): scoreboard.verify_capture(path)

    def test_edge_suite_source_is_verified(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory); self.fixture(path)
            source = json.loads((path / "source.json").read_text())
            source["test_path"] = scoreboard.SUITES["edge"]
            pinned = scoreboard.ROOT / scoreboard.references.MANIFEST["d2"]["checkout"] / source["test_path"]
            source["test_sha256"] = scoreboard.hashlib.sha256(pinned.read_bytes()).hexdigest()
            (path / "source.json").write_text(json.dumps(source)); self.seal(path)
            scoreboard.verify_capture(path)

    def test_import_suites_reject_missing_calls_even_after_resealing(self):
        for suite in ("import", "import-depth"):
            with self.subTest(suite=suite), tempfile.TemporaryDirectory() as directory:
                path = Path(directory); self.fixture(path)
                relative = scoreboard.SUITES[suite]
                source = json.loads((path / "source.json").read_text())
                source["test_path"] = relative
                pinned = scoreboard.ROOT / scoreboard.references.MANIFEST["d2"]["checkout"] / relative
                source["test_sha256"] = scoreboard.hashlib.sha256(pinned.read_bytes()).hexdigest()
                (path / "source.json").write_text(json.dumps(source)); self.seal(path)
                with self.assertRaisesRegex(AssertionError, "original import call"):
                    scoreboard.verify_capture(path)

if __name__ == "__main__": unittest.main()
