import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "fallback_scoreboard", Path(__file__).with_name("scenebuild-fallback-scoreboard.py")
)
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class FallbackGateTests(unittest.TestCase):
    def test_delta_rejects_input_or_inventory_changes_and_reports_regression(self):
        row = {
            "case": "a",
            "input_sha256": "input",
            "expected_sha256": "expected",
            "outcome": "mismatch",
        }
        before = {"rows": [row]}
        after = {"rows": [{**row, "outcome": "match"}]}
        self.assertEqual(scoreboard.delta(before, after)["improvements"], ["a"])
        self.assertEqual(scoreboard.delta(after, before)["regressions"], ["a"])
        for changed in ({**row, "input_sha256": "changed"}, {**row, "case": "b"}):
            with self.assertRaises(AssertionError):
                scoreboard.delta(before, {"rows": [changed]})

    def test_complete_scene_values_and_asset_hashes(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            expected = root / "expected.json"
            actual = root / "native" / "expected.json"
            actual.parent.mkdir()
            value = {
                "glyph": {"X": 1, "Asset": "font:fallback:abc"},
                "data_sha256": "abc",
            }
            expected.write_text(json.dumps(value))
            actual.write_text(json.dumps(value, indent=1))
            self.assertTrue(scoreboard.compare(expected, actual, 0))
            actual.write_text("scene: unsupported")
            self.assertFalse(scoreboard.compare(expected, actual, 0))
            for changed in (
                {**value, "data_sha256": "def"},
                {**value, "extra": 0},
                {**value, "glyph": {"X": True, "Asset": "font:fallback:abc"}},
            ):
                actual.write_text(json.dumps(changed))
                self.assertFalse(scoreboard.compare(expected, actual, 0))

    def test_failed_missing_extra_and_corrupt_outputs(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            expected = root / "expected.err"
            actual = root / "native" / "expected.err"
            actual.parent.mkdir()
            expected.write_text("complete error\n")
            self.assertFalse(scoreboard.compare(expected, actual, 0))
            actual.write_bytes(expected.read_bytes())
            self.assertTrue(scoreboard.compare(expected, actual, 0))
            self.assertFalse(scoreboard.compare(expected, actual, 1))
            actual.write_text("complete error")
            self.assertFalse(scoreboard.compare(expected, actual, 0))
            actual.write_bytes(expected.read_bytes())
            (actual.parent / "extra").touch()
            self.assertFalse(scoreboard.compare(expected, actual, 0))

    def capture(self, root):
        for test, count in scoreboard.TESTS.items():
            for index in range(count):
                directory = root / "go" / test / str(index)
                directory.mkdir(parents=True)
                (directory / "input.json").write_text(json.dumps({"test": test}))
                (directory / "expected.json").write_text('{"Root": null}')
        manifest = {
            "source_commit": scoreboard.references.SOURCE_COMMIT,
            "outcomes": dict.fromkeys(scoreboard.TESTS, "pass"),
            "files": {
                str(p.relative_to(root)): scoreboard.digest(p)
                for p in root.rglob("*")
                if p.is_file()
            },
        }
        (root / "manifest.json").write_text(json.dumps(manifest))
        return manifest

    def test_source_inventory_and_hashes(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            self.capture(root)
            self.assertEqual(len(scoreboard.verify_capture(root)), 4)
            input_path = next((root / "go").rglob("input.json"))
            original = input_path.read_bytes()
            input_path.write_bytes(original + b" ")
            with self.assertRaises(AssertionError):
                scoreboard.verify_capture(root)
            input_path.write_bytes(original)
            extra = root / "unexpected"
            extra.touch()
            with self.assertRaises(AssertionError):
                scoreboard.verify_capture(root)
            extra.unlink()
            input_path.unlink()
            with self.assertRaises(AssertionError):
                scoreboard.verify_capture(root)

    def test_ambiguous_expected_and_failed_go_cannot_pass(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            manifest = self.capture(root)
            test = next(iter(scoreboard.TESTS))
            manifest["outcomes"][test] = "fail"
            (root / "manifest.json").write_text(json.dumps(manifest))
            with self.assertRaises(AssertionError):
                scoreboard.verify_capture(root)
            manifest["outcomes"][test] = "pass"
            extra = root / "go" / test / "0" / "expected.err"
            extra.write_text("error")
            manifest["files"][str(extra.relative_to(root))] = scoreboard.digest(extra)
            (root / "manifest.json").write_text(json.dumps(manifest))
            with self.assertRaises(AssertionError):
                scoreboard.verify_capture(root)


if __name__ == "__main__":
    unittest.main()
