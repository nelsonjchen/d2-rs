"""Failure gates for raw resolver capture and the explicit scheduling contract."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "resolver_scoreboard", Path(__file__).with_name("imgbundler-resolver-scoreboard.py")
)
scoreboard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scoreboard)


class Integrity(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.capture = Path(self.tmp.name)
        (self.capture / "go").mkdir()
        (self.capture / "source.json").write_text(json.dumps(scoreboard.source()))
        (self.capture / "inventory.json").write_text(json.dumps(scoreboard.TESTS))
        (self.capture / "run.json").write_text(json.dumps({"exit": 0, "failure": None}))
        self.events = {t: "pass" for t in scoreboard.TESTS}
        self.events.update(
            {
                "TestInlineRemoteContentTypeIsSafeAndCanonical/" + s: "pass"
                for s in scoreboard.SUBTESTS
            }
        )
        self.events_file()
        for name in scoreboard.expected():
            (self.capture / "go" / (name.replace("/", "_") + ".json")).write_text(
                json.dumps({"name": name})
            )
        self.seal()

    def events_file(self):
        (self.capture / "go.jsonl").write_text(
            "\n".join(
                json.dumps({"Test": t, "Action": a}) for t, a in self.events.items()
            )
        )

    def seal(self):
        (self.capture / "hashes.json").write_text(
            json.dumps(scoreboard.hashes(self.capture))
        )

    def test_complete(self):
        self.assertEqual(len(scoreboard.verify(self.capture)), 13)

    def test_missing_resealed_call(self):
        next((self.capture / "go").glob("*.json")).unlink()
        self.seal()
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_changed_bytes(self):
        next((self.capture / "go").glob("*.json")).write_text("{}")
        with self.assertRaises(AssertionError):
            scoreboard.verify(self.capture)

    def test_failed_subtest(self):
        self.events[
            "TestInlineRemoteContentTypeIsSafeAndCanonical/parameterized_SVG"
        ] = "fail"
        self.events_file()
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

    def test_scheduler_original_contract_is_explicit(self):
        case = {
            "test": "TestResolutionFailureStopsSchedulingNewFetches",
            "got": {"output": "<svg/>"},
        }
        actual = {
            "output": "<svg/>",
            "error": 'imageasset: probe "https://example.com/7": unsupported or malformed image',
            "requests_within_worker_limit": True,
        }
        self.assertEqual(scoreboard.classify(case, actual), "assertion-match")
        for field, value in [
            ("output", ""),
            ("requests_within_worker_limit", False),
            ("error", "context canceled"),
            ("error", ""),
        ]:
            with self.subTest(field=field, value=value):
                self.assertEqual(
                    scoreboard.classify(case, dict(actual, **{field: value})),
                    "mismatch",
                )

    def test_exact_tier_rejects_any_changed_error_or_bytes(self):
        case = {"test": "ordinary", "got": {"output": "<svg/>", "error": "malformed"}}
        self.assertEqual(scoreboard.classify(case, case["got"]), "match")
        self.assertEqual(
            scoreboard.classify(case, {"output": "<svg/>", "error": "other"}),
            "mismatch",
        )


if __name__ == "__main__":
    unittest.main()
