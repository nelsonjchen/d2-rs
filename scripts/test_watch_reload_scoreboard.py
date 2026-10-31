#!/usr/bin/env python3
"""Failure gates for the original watch reload scoreboard."""
import base64
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("watch_reload", Path(__file__).with_name("watch-reload-scoreboard.py"))
watch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(watch)


def encoded(data):
    return base64.b64encode(data).decode()


def fixture():
    stages = []
    for i, source in enumerate(watch.SOURCES):
        svg = ('<svg xmlns="http://www.w3.org/2000/svg"><title>' +
               ('D2 isometric diagram' if i != 2 else 'regular') +
               '</title><text>' + str(0 if i == 0 else 1) + '</text></svg>').encode()
        stages.append(dict(test=watch.TEST, source=encoded(source.encode()), isometric=i != 2,
                           svg=encoded(svg), outputs={"output.svg": dict(data=encoded(svg), mode=0o600)}))
    return stages


class Gates(unittest.TestCase):
    def setUp(self):
        self.go = fixture()
        self.native = copy.deepcopy(self.go)

    def outcome(self, execution=None, events=None):
        return watch.compare(self.go, self.native, execution or {"exit": 0, "failure": None},
                             events if events is not None else {watch.TEST: "pass"})[0]["outcome"]

    def test_complete_exact(self):
        self.assertEqual(self.outcome(), "match")

    def test_missing_stage(self):
        self.native.pop()
        self.assertEqual(self.outcome(), "native-failed")

    def test_extra_stage(self):
        self.native.append(self.native[-1])
        self.assertEqual(self.outcome(), "native-failed")

    def test_missing_output(self):
        self.native[0]["outputs"].clear()
        self.assertEqual(self.outcome(), "native-failed")

    def test_extra_output(self):
        self.native[0]["outputs"]["extra.svg"] = self.native[0]["outputs"]["output.svg"]
        self.assertEqual(self.outcome(), "native-failed")

    def test_corrupt_svg(self):
        self.native[0]["svg"] = encoded(b"<svg>")
        self.assertEqual(self.outcome(), "native-failed")

    def test_empty_svg(self):
        self.native[0]["svg"] = ""
        self.assertEqual(self.outcome(), "native-failed")

    def test_incomplete_file(self):
        self.native[0]["outputs"]["output.svg"]["data"] = encoded(b"<svg>")
        self.assertEqual(self.outcome(), "native-failed")

    def test_changed_source(self):
        self.native[0]["source"] = encoded(b"wrong")
        self.assertEqual(self.outcome(), "native-failed")

    def test_unchanged_reload(self):
        self.native[1]["svg"] = self.native[0]["svg"]
        self.native[1]["outputs"] = copy.deepcopy(self.native[0]["outputs"])
        self.assertEqual(self.outcome(), "native-failed")

    def test_changed_restore(self):
        self.native[3]["svg"] = self.native[0]["svg"]
        self.native[3]["outputs"] = copy.deepcopy(self.native[0]["outputs"])
        self.assertEqual(self.outcome(), "native-failed")

    def test_svg_difference_is_not_normalized(self):
        svg = watch.decode(self.native[0]["svg"]).replace(b"<text>", b'<text fill="red">')
        self.native[0]["svg"] = encoded(svg)
        self.native[0]["outputs"]["output.svg"]["data"] = encoded(svg)
        self.assertEqual(self.outcome(), "mismatch")

    def test_modes_compare(self):
        self.native[0]["outputs"]["output.svg"]["mode"] = 0o644
        self.assertEqual(self.outcome(), "mismatch")

    def test_bool_mode_rejected(self):
        self.native[0]["outputs"]["output.svg"]["mode"] = True
        self.assertEqual(self.outcome(), "native-failed")

    def test_timeout(self):
        self.assertEqual(self.outcome({"exit": -15, "failure": "timeout"}), "native-failed")

    def test_budget_exhaustion(self):
        self.assertEqual(self.outcome({"exit": -15, "failure": "budget-exceeded"}), "native-failed")

    def test_original_failure(self):
        self.assertEqual(self.outcome(events={watch.TEST: "fail"}), "native-failed")

    def test_missing_original_assertion(self):
        self.assertEqual(self.outcome(events={}), "native-failed")

    def test_go_failure_never_passes(self):
        self.go.clear()
        with self.assertRaises(AssertionError):
            self.outcome()


class PreviewGates(unittest.TestCase):
    def setUp(self):
        self.calls = []
        for test, count in watch.PREVIEWS.items():
            for _ in range(count):
                request = dict(tree={"nodes": []}, render={"Scale": 0.2})
                result = dict(svg='<svg xmlns="http://www.w3.org/2000/svg"/>', error="",
                              tree_after=request["tree"], render_after=request["render"])
                self.calls.append(dict(test=test, input=request, result=result))
        self.results = copy.deepcopy([c["result"] for c in self.calls])

    def rows(self, execution=None):
        return watch.compare_previews(self.calls, self.results, execution or {"exit": 0, "failure": None})

    def test_complete_calls(self):
        self.assertTrue(all(r["outcome"] == "match" for r in self.rows()))

    def test_missing_native_call(self):
        self.results.pop()
        self.assertTrue(all(r["outcome"] == "native-failed" for r in self.rows()))

    def test_extra_native_call(self):
        self.results.append(self.results[-1])
        self.assertTrue(all(r["outcome"] == "native-failed" for r in self.rows()))

    def test_missing_go_call(self):
        self.calls.pop()
        with self.assertRaises(AssertionError):
            self.rows()

    def test_native_error_not_passing(self):
        self.results[0]["error"] = "failure"
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_changed_input_not_passing(self):
        self.results[0]["render_after"] = {"Scale": 1}
        self.assertEqual(self.rows()[0]["outcome"], "mismatch")

    def test_cycle_stays_unsupported(self):
        self.results[4] = {"unsupported": "Go pointer cycle"}
        row = self.rows()[1]
        self.assertEqual(row["outcome"], "unsupported")
        self.assertEqual(row["matched_calls"], 1)

    def test_cycle_cannot_hide_another_mismatch(self):
        self.results[3]["error"] = "different error"
        self.results[4] = {"unsupported": "Go pointer cycle"}
        self.assertEqual(self.rows()[1]["outcome"], "mismatch")

    def test_timeout(self):
        self.assertTrue(all(r["outcome"] == "native-failed" for r in self.rows({"exit": -15, "failure": "timeout"})))

    def test_json_bool_not_number(self):
        self.assertFalse(watch.type_equal({"foo": True}, {"foo": 1}))


class LayoutGates(unittest.TestCase):
    def setUp(self):
        self.calls = []
        files = {"input.d2": dict(data=encoded(b"a -> b\na -> c"), mode=0o600)}
        for engine in ["dagre", "elk", "tala"]:
            svg = encoded(f'<svg xmlns="http://www.w3.org/2000/svg"><title>{engine}</title></svg>'.encode())
            for name in ["before.svg", "after.svg"]:
                files[name] = dict(data=svg, mode=0o600)
                self.calls.append(dict(test=watch.LAYOUT + "/" + engine, args=[engine, name],
                                       stdout="", error="", files=copy.deepcopy(files)))
        self.native = copy.deepcopy(self.calls)

    def row(self, execution=None, events=None):
        return watch.compare_layout(self.calls, self.native, execution or {"exit": 0, "failure": None},
                                    events if events is not None else {watch.LAYOUT: "pass"})

    def test_complete(self):
        self.assertEqual(self.row()["matched_calls"], 6)

    def test_missing(self):
        self.native.pop()
        self.assertEqual(self.row()["outcome"], "native-failed")

    def test_extra_output(self):
        self.native[-1]["files"]["extra.svg"] = self.native[-1]["files"]["after.svg"]
        self.assertEqual(self.row()["outcome"], "native-failed")

    def test_mode_changed(self):
        for call in self.native:
            for name, file in call["files"].items():
                if name.endswith(".svg"):
                    file["mode"] = 0o644
        self.assertEqual(self.row()["outcome"], "mismatch")

    def test_failed_assertion(self):
        self.assertEqual(self.row(events={})["outcome"], "native-failed")

    def test_budget(self):
        self.assertEqual(self.row(execution={"exit": -15, "failure": "budget-exceeded"})["outcome"], "native-failed")


if __name__ == "__main__":
    unittest.main()
