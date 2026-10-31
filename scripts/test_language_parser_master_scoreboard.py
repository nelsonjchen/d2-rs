#!/usr/bin/env python3
"""Parser capture integrity: no missing leaf, failed assertion or changed bytes pass."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("parser_score", Path(__file__).with_name("language-parser-master-scoreboard.py"))
score = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score)


class Integrity(unittest.TestCase):
    def fixture(self, path):
        (path / "go").mkdir()
        for leaf in ("map", "array"):
            (path / f"go/{leaf}.json").write_text(json.dumps({"name": f"TestDepth/{leaf}#0", "test": f"TestDepth/{leaf}"}))
        (path / "inventory.json").write_text(json.dumps([{"path": score.RELATIVE, "test": "TestDepth"}]))
        (path / "go.jsonl").write_text("\n".join(json.dumps({"Action": "pass", "Test": name}) for name in ("TestDepth/map", "TestDepth/array", "TestDepth")))
        (path / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        pinned = score.ROOT / score.references.MANIFEST["d2"]["checkout"] / score.RELATIVE
        (path / "source.json").write_text(json.dumps({"source_commit": score.references.SOURCE_COMMIT, "test_sha256": score.hashlib.sha256(pinned.read_bytes()).hexdigest()}))
        self.seal(path)

    def seal(self, path):
        (path / "hashes.json").write_text(json.dumps(score.hashes(path)))

    def test_complete_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            self.fixture(path)
            score.verify(path)

    def test_missing_changed_extra_and_duplicate_calls(self):
        for operation in ("missing", "changed", "extra", "duplicate"):
            with self.subTest(operation=operation), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                if operation == "missing":
                    (path / "go/map.json").unlink()
                elif operation == "changed":
                    (path / "go/map.json").write_text("{}")
                else:
                    (path / "go/extra.json").write_bytes((path / "go/map.json").read_bytes())
                    if operation == "duplicate":
                        self.seal(path)
                with self.assertRaises(AssertionError):
                    score.verify(path)

    def test_missing_leaf_fails_even_with_updated_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            self.fixture(path)
            (path / "go/array.json").unlink()
            self.seal(path)
            with self.assertRaisesRegex(AssertionError, "leaf"):
                score.verify(path)

    def test_failed_assertion_budget_and_source_are_not_passing(self):
        for failure in ("assertion", "budget", "pin", "source"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)
                self.fixture(path)
                if failure == "assertion":
                    text = (path / "go.jsonl").read_text().replace('"pass"', '"fail"')
                    (path / "go.jsonl").write_text(text)
                elif failure == "budget":
                    (path / "run.json").write_text(json.dumps({"exit": 1, "failure": "budget-exceeded"}))
                else:
                    source = json.loads((path / "source.json").read_text())
                    source["source_commit" if failure == "pin" else "test_sha256"] = "changed"
                    (path / "source.json").write_text(json.dumps(source))
                self.seal(path)
                with self.assertRaises(AssertionError):
                    score.verify(path)


if __name__ == "__main__":
    unittest.main()
