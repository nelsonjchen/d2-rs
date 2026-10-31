"""Exact-byte and complete-outcome failure gates for first-frame captures."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "raster_images", Path(__file__).with_name("raster-image-scoreboard.py")
)
api = importlib.util.module_from_spec(spec)
spec.loader.exec_module(api)


class ImageGate(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.expected = self.root / "expected.png"
        self.expected.write_bytes(b"complete source PNG bytes")
        self.out = self.root / "native"
        self.out.mkdir()
        self.actual = self.out / "actual.png"

    def test_exact_bytes(self):
        self.actual.write_bytes(self.expected.read_bytes())
        self.assertTrue(api.compare_case(self.expected, self.actual))

    def test_missing(self):
        self.assertFalse(api.compare_case(self.expected, self.actual))

    def test_empty(self):
        self.actual.touch()
        self.assertFalse(api.compare_case(self.expected, self.actual))

    def test_corrupt_or_truncated(self):
        for value in [b"complete source PNG byteX", b"complete source PNG byte"]:
            self.actual.write_bytes(value)
            self.assertFalse(api.compare_case(self.expected, self.actual))

    def test_extra_file(self):
        self.actual.write_bytes(self.expected.read_bytes())
        (self.out / ".DS_Store").touch()
        self.assertFalse(api.compare_case(self.expected, self.actual))

    def test_both_outcome_types(self):
        self.actual.write_bytes(self.expected.read_bytes())
        (self.out / "actual.err").write_text("error")
        self.assertFalse(api.compare_case(self.expected, self.actual))

    def test_failed_process_even_with_correct_output(self):
        self.actual.write_bytes(self.expected.read_bytes())
        self.assertFalse(api.compare_case(self.expected, self.actual, 1))

    def test_delta_regression_and_missing_row(self):
        before = {
            "source_commit": "pin",
            "summary": {"match": 1},
            "rows": [{"board": "a", "outcome": "match"}],
        }
        after = {
            "source_commit": "pin",
            "summary": {"mismatch": 1},
            "rows": [{"board": "a", "outcome": "mismatch"}],
        }
        self.assertEqual(api.delta(before, after)["regressions"], ["a"])
        after["rows"] = []
        with self.assertRaises(AssertionError):
            api.delta(before, after)

    def capture(self):
        capture = self.root / "capture"
        case = capture / "go/source/0"
        case.mkdir(parents=True)
        (case / "case.json").write_text(json.dumps({"test": "source", "format": "png"}))
        (case / "input.bin").write_bytes(b"encoded source")
        (case / "expected.png").write_bytes(self.expected.read_bytes())
        manifest = {
            "source_commit": api.references.SOURCE_COMMIT,
            "outcomes": {"source": "pass"},
            "files": {
                str(p.relative_to(capture)): api.digest(p)
                for p in capture.rglob("*")
                if p.is_file()
            },
        }
        (capture / "manifest.json").write_text(json.dumps(manifest))
        return capture, case

    def test_missing_or_changed_source_input(self):
        capture, case = self.capture()
        with patch.object(api, "TESTS", {"source": 1}):
            (case / "input.bin").write_bytes(b"mutated")
            with self.assertRaises(AssertionError):
                api.score(capture, self.expected, self.root / "report.json")
            (case / "input.bin").unlink()
            with self.assertRaises(AssertionError):
                api.score(capture, self.expected, self.root / "report.json")

    def test_unexpected_source_and_ambiguous_expected_outcome(self):
        capture, case = self.capture()
        (case / "expected.err").write_text("also an error")
        # Even a manifest that includes the extra outcome must fail.
        manifest = json.loads((capture / "manifest.json").read_text())
        manifest["files"][str((case / "expected.err").relative_to(capture))] = (
            api.digest(case / "expected.err")
        )
        (capture / "manifest.json").write_text(json.dumps(manifest))
        with (
            patch.object(api, "TESTS", {"source": 1}),
            self.assertRaises(AssertionError),
        ):
            api.score(capture, self.expected, self.root / "report.json")

    def test_complete_source_and_native_inventory(self):
        capture, _case = self.capture()

        def run(*args, **kwargs):
            for line in kwargs["input"].splitlines():
                request = json.loads(line)
                out = Path(request["out"])
                out.mkdir(parents=True)
                (out / "actual.png").write_bytes(self.expected.read_bytes())
            (out.parent / "extra.png").write_bytes(b"extra")
            return type("Result", (), {"returncode": 0, "stdout": "", "stderr": ""})()

        with (
            patch.object(api, "TESTS", {"source": 1}),
            patch.object(api.subprocess, "run", side_effect=run),
        ):
            report = api.score(capture, self.expected, self.root / "report.json")
        self.assertEqual(report["summary"], {"match": 1, "unexpected-native-file": 1})


if __name__ == "__main__":
    unittest.main()
