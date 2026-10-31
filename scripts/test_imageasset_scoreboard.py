import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

from fixture_recipe import cached_fixture  # oracle-cache artifact (docs/FIXTURES.md)

spec = importlib.util.spec_from_file_location(
    "imageasset_scoreboard", Path(__file__).with_name("imageasset-scoreboard.py")
)
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class ImageAssetGateTests(unittest.TestCase):
    def test_svg_suites_require_every_original_call_and_reject_other_suite_reuse(self):
        source = scoreboard.references.verify_source()
        for name, inventory, fixture in [
            ("svg", scoreboard.SVG_TESTS, "imageasset-svg-master.jsonl"),
            (
                "svg-codecs",
                scoreboard.SVG_CODEC_TESTS,
                "imageasset-svg-codecs-master.jsonl",
            ),
            ("http", scoreboard.HTTP_TESTS, "imageasset-http-master.jsonl"),
            (
                "http-headers",
                scoreboard.HTTP_HEADER_TESTS,
                "imageasset-headers-master.jsonl",
            ),
            ("cache", scoreboard.CACHE_TESTS, "imageasset-cache-master.jsonl"),
            (
                "cache-local",
                scoreboard.LOCAL_CACHE_TESTS,
                "imageasset-cache-local-master.jsonl",
            ),
            (
                "cache-namespace",
                scoreboard.NAMESPACE_TESTS,
                "imageasset-cache-namespace-master.jsonl",
            ),
        ]:
            with self.subTest(suite=name), tempfile.TemporaryDirectory() as temporary:
                capture = Path(temporary)
                calls = capture / "calls.jsonl"
                calls.write_bytes(
                    cached_fixture(
                        "crates/d2-rs-scenebuild/tests/fixtures/" + fixture
                    ).read_bytes()
                )
                manifest = {
                    "source_commit": scoreboard.references.SOURCE_COMMIT,
                    "suite": name,
                    "outcomes": dict.fromkeys(inventory, "pass"),
                    "test_files": {
                        str(p.relative_to(source)): scoreboard.sha(p)
                        for p in (source / "lib/imageasset").glob("*_test.go")
                    },
                    "files": {"calls.jsonl": scoreboard.sha(calls)},
                }
                path = capture / "manifest.json"
                path.write_text(json.dumps(manifest))
                self.assertEqual(
                    len(scoreboard.verify(capture, inventory)[1]),
                    sum(inventory.values()),
                )
                with self.assertRaises(AssertionError):
                    scoreboard.verify(capture)
                lines = calls.read_text().splitlines()
                for changed in [lines[:-1], lines + lines[:1]]:
                    calls.write_text("\n".join(changed) + "\n")
                    manifest["files"]["calls.jsonl"] = scoreboard.sha(calls)
                    path.write_text(json.dumps(manifest))
                    with self.assertRaises(AssertionError):
                        scoreboard.verify(capture, inventory)

    def test_complete_typed_resources_and_errors(self):
        expected = {"resource": {"data": "abc", "pixel_width": 2}, "assets": 1}
        self.assertTrue(scoreboard.compare(expected, expected))
        for actual in [
            {"resource": {"data": "xyz", "pixel_width": 2}, "assets": 1},
            {**expected, "assets": True},
            {**expected, "extra": 1},
            {"error": "failed"},
            {},
        ]:
            self.assertFalse(scoreboard.compare(expected, actual))

    def test_delta_rejects_changed_inputs_expected_values_and_duplicate_or_missing_cases(
        self,
    ):
        row = {
            "case": "original",
            "input_sha256": "a",
            "expected_sha256": "b",
            "outcome": "mismatch",
        }
        before = {"rows": [row]}
        after = {"rows": [{**row, "outcome": "match"}]}
        self.assertEqual(scoreboard.delta(before, after)["improvements"], ["original"])
        self.assertEqual(scoreboard.delta(after, before)["regressions"], ["original"])
        for rows in [
            [],
            [row, row],
            [{**row, "input_sha256": "c"}],
            [{**row, "expected_sha256": "c"}],
        ]:
            with self.assertRaises(AssertionError):
                scoreboard.delta(before, {"rows": rows})

    def test_tampered_source_inventory_and_failed_go_assertions_cannot_pass(self):
        original = cached_fixture(
            "crates/d2-rs-scenebuild/tests/fixtures/imageasset-local-data-master.jsonl"
        )
        with tempfile.TemporaryDirectory() as temporary:
            capture = Path(temporary)
            (capture / "calls.jsonl").write_bytes(original.read_bytes())
            source = scoreboard.references.verify_source()
            manifest = {
                "source_commit": scoreboard.references.SOURCE_COMMIT,
                "outcomes": dict.fromkeys(scoreboard.TESTS, "pass"),
                "test_files": {
                    str(p.relative_to(source)): scoreboard.sha(p)
                    for p in (source / "lib/imageasset").glob("*_test.go")
                },
                "files": {"calls.jsonl": scoreboard.sha(capture / "calls.jsonl")},
            }
            (capture / "manifest.json").write_text(json.dumps(manifest))
            self.assertEqual(len(scoreboard.verify(capture)[1]), 17)
            for name, changed in [
                (
                    "outcomes",
                    {**manifest["outcomes"], next(iter(scoreboard.TESTS)): "fail"},
                ),
                ("test_files", {}),
            ]:
                (capture / "manifest.json").write_text(
                    json.dumps({**manifest, name: changed})
                )
                with self.assertRaises(AssertionError):
                    scoreboard.verify(capture)
            (capture / "manifest.json").write_text(json.dumps(manifest))
            calls = capture / "calls.jsonl"
            data = calls.read_bytes()
            calls.write_bytes(data + b"\n")
            with self.assertRaises(AssertionError):
                scoreboard.verify(capture)
            calls.write_bytes(data)
            (capture / "extra").touch()
            with self.assertRaises(AssertionError):
                scoreboard.verify(capture)


if __name__ == "__main__":
    unittest.main()
