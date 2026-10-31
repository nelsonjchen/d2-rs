#!/usr/bin/env python3
"""Original isometric watch tests, with complete SVG bytes and errors.

Runs the original test twice, first in Go and then against the native CLI.
Every original assertion, timeout and source write stays active. Both builds
use the explicit v0.9.0 version stamp of the pinned CLI oracle; no SVG bytes
are normalized. Cyclic Go pointer trees remain explicitly unsupported.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import xml.etree.ElementTree as ET

import references

ROOT = references.ROOT
TEST = "TestIsometricWatchCLIRecompilesAndReloadsSourceMode"
LAYOUT = "TestBuiltinLayoutFlagOrder"
PREVIEWS = {"TestIsometricWatchPreviewPreservesBoardsAndNavigation": 3,
            "TestIsometricWatchPreviewRejectsInvalidTrees": 2,
            "TestIsometricWatchGIFPreviewUsesStaticSingleTheme": 2}
ON = "vars: {d2-config: {isometric: true}}\n"
OFF = "vars: {d2-config: {isometric: false}}\n"
SOURCES = [ON + "a -> b\n", ON + "a -> b -> c\n", OFF + "a -> b -> c\n", ON + "a -> b -> c\n"]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def decode(value):
    return base64.b64decode(value, validate=True)


def type_equal(a, b):
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(type_equal(a[k], b[k]) for k in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(type_equal(x, y) for x, y in zip(a, b))
    return a == b


def validate(stages):
    assert len(stages) == 4, "missing/extra original stages"
    for i, stage in enumerate(stages):
        assert set(stage) == {"test", "source", "isometric", "svg", "outputs"}
        assert stage["test"] == TEST
        assert decode(stage["source"]) == SOURCES[i].encode(), "changed original input"
        assert type(stage["isometric"]) is bool and stage["isometric"] == (i != 2)
        svg = decode(stage["svg"])
        assert svg and ET.fromstring(svg).tag == "{http://www.w3.org/2000/svg}svg", "invalid SVG"
        assert (b"D2 isometric diagram" in svg) == stage["isometric"]
        assert set(stage["outputs"]) == {"output.svg"}, "missing/extra output inventory"
        artifact = stage["outputs"]["output.svg"]
        assert set(artifact) == {"data", "mode"}
        assert type(artifact["mode"]) is int and 0 <= artifact["mode"] <= 0o777
        assert decode(artifact["data"]) == svg, "incomplete or changed file"
    assert stages[1]["svg"] == stages[3]["svg"], "restored mode changed bytes"
    assert stages[0]["svg"] != stages[1]["svg"], "source edit did not render"


def compare(go, native, execution, events):
    validate(go)
    try:
        assert execution == {"exit": 0, "failure": None}, "native execution failed"
        assert events.get(TEST) == "pass", "original native assertion did not pass"
        validate(native)
    except (AssertionError, ValueError, ET.ParseError, TypeError, KeyError) as err:
        return [dict(test=TEST, outcome="native-failed", stages=4, matched_stages=0, error=str(err))]
    matched = sum(type_equal(a, b) for a, b in zip(go, native))
    return [dict(test=TEST, outcome="match" if matched == 4 else "mismatch", stages=4, matched_stages=matched)]


def instrument(original):
    text = original.decode()
    assert text.count("Run: Run, PWD: dir,") == 1
    assert text.count("\t\t\t\treturn data") == 1
    text = text.replace("Run: Run, PWD: dir,", "Run: d2rsWatchReloadRun, PWD: dir,")
    assert text.count("renderIsometricPreview(") == 5
    text = text.replace("renderIsometricPreview(", "d2rsWatchPreview(t, ")
    return text.replace("\t\t\t\treturn data", "\t\t\t\td2rsWatchReloadRecord(t, dir, data, isometric)\n\t\t\t\treturn data")


def compare_previews(calls, results, execution):
    assert len(calls) == sum(PREVIEWS.values()), "missing/extra original preview calls"
    for test, count in PREVIEWS.items():
        assert sum(c["test"].split("/")[0] == test for c in calls) == count
    for call in calls:
        result, request = call["result"], call["input"]
        assert result["tree_after"] == request["tree"] and result["render_after"] == request["render"], "Go mutated its input"
        if not result["error"]:
            assert ET.fromstring(result["svg"]).tag == "{http://www.w3.org/2000/svg}svg"
    rows = []
    for test, count in PREVIEWS.items():
        pairs = [(c, results[i]) for i, c in enumerate(calls) if c["test"].split("/")[0] == test] if len(results) == len(calls) else []
        matched = sum(type_equal(c["result"], r) for c, r in pairs)
        unsupported = sum(isinstance(r, dict) and "unsupported" in r for _, r in pairs)
        outcome = ("native-failed" if execution != {"exit": 0, "failure": None} or not pairs else
                   "match" if matched == count else "unsupported" if matched + unsupported == count else "mismatch")
        rows.append(dict(test=test, calls=count, matched_calls=matched, unsupported_calls=unsupported, outcome=outcome))
    return rows


def validate_layout(calls):
    assert len(calls) == 6, "missing/extra original layout invocations"
    for i, engine in enumerate(["dagre", "elk", "tala"]):
        before, after = calls[2*i:2*i+2]
        for call in [before, after]:
            assert call["test"] == LAYOUT + "/" + engine
            assert call["error"] == call["stdout"] == ""
            assert decode(call["files"]["input.d2"]["data"]) == b"a -> b\na -> c"
            for name, file in call["files"].items():
                assert type(file["mode"]) is int
                if name.endswith(".svg"):
                    assert ET.fromstring(decode(file["data"])).tag == "{http://www.w3.org/2000/svg}svg"
        assert set(before["files"]) == ({"input.d2", "before.svg"} if i == 0 else {"input.d2", "before.svg", "after.svg"})
        assert set(after["files"]) == {"input.d2", "before.svg", "after.svg"}
        assert after["files"]["before.svg"] == after["files"]["after.svg"], "layout flag order changed output"


def compare_layout(go, native, execution, events):
    validate_layout(go)
    try:
        assert execution == {"exit": 0, "failure": None}
        assert events.get(LAYOUT) == "pass"
        validate_layout(native)
    except (AssertionError, ValueError, ET.ParseError, TypeError, KeyError) as err:
        return dict(test=LAYOUT, calls=6, matched_calls=0, outcome="native-failed", error=str(err))
    matched = sum(type_equal(a, b) for a, b in zip(go, native))
    return dict(test=LAYOUT, calls=6, matched_calls=matched, outcome="match" if matched == 6 else "mismatch")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", required=True, type=Path)
    parser.add_argument("--native-test", required=True, type=Path)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source = references.verify_source()
    runner, capture, output = args.rust.resolve(), args.capture.resolve(), args.output.resolve()
    native_test = args.native_test.resolve()
    assert not output.exists(), "fresh report required"
    capture.mkdir(exist_ok=False)
    hook = ROOT / "tools/watch-reload-scoreboard/hook_test.go"
    source_test = source / "d2cli/isometric_watch_test.go"
    original = source_test.read_bytes()
    layout_original = (source / "d2cli/layout_test.go").read_bytes()
    target = references.TRACE_CHECKOUT / "d2cli/isometric_watch_test.go"
    layout_target = target.parent / "layout_test.go"
    helper = target.parent / "d2rs_watch_reload_test.go"
    assert target.read_bytes() == original and layout_target.read_bytes() == layout_original and not helper.exists(), "concurrent trace mutation"
    spec = importlib.util.spec_from_file_location("cli", ROOT / "scripts/cli-scoreboard.py")
    cli = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(cli)
    digest = sha(runner)
    test_digest = sha(native_test)
    runs = {}
    try:
        target.write_text(instrument(original))
        text = layout_original.decode()
        assert text.count("Run: Run,") == 1 and text.count("return stdout.String(), err") == 1
        text = text.replace("Run: Run,", "Run: d2rsWatchReloadRun,")
        layout_target.write_text(text.replace("return stdout.String(), err", "d2rsLayoutRecord(t, directory, args, stdout.String(), err)\n\treturn stdout.String(), err"))
        helper.write_bytes(hook.read_bytes())
        for side in ["go", "native"]:
            work = capture / side
            work.mkdir()
            (work / "tmp").mkdir()
            env = {k: v for k, v in os.environ.items() if not k.startswith(("D2_", "D2RS_"))}
            env.update(GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
                       GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off",
                       TMPDIR=str(work / "tmp"), D2RS_WATCH_CAPTURE=str(work),
                       D2RS_WATCH_BINARY=str(runner) if side == "native" else "")
            names = [TEST, LAYOUT, *PREVIEWS] if side == "go" else [TEST, LAYOUT]
            execution = cli.bounded_run(["go", "test", "./d2cli", "-run", "^(" + "|".join(names) + ")$", "-json",
                "-count=1", "-parallel=1", "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0"],
                env, capture, work / "go.jsonl", 600)
            stages = json.loads((work / "stages.json").read_text()) if (work / "stages.json").is_file() else []
            runs[side] = dict(execution=execution, events=cli.events(work / "go.jsonl"), stages=stages)
            runs[side]["layouts"] = json.loads((work / "layouts.json").read_text()) if (work / "layouts.json").is_file() else []
            if side == "go":
                assert execution == {"exit": 0, "failure": None}, execution
                assert runs[side]["events"].get(TEST) == "pass", runs[side]["events"]
                assert all(runs[side]["events"].get(t) == "pass" for t in PREVIEWS)
                assert runs[side]["events"].get(LAYOUT) == "pass"
                validate_layout(runs[side]["layouts"])
                validate(stages)
    finally:
        target.write_bytes(original)
        layout_target.write_bytes(layout_original)
        helper.unlink(missing_ok=True)
    assert sha(runner) == digest, "changed native runner"
    rows = compare(runs["go"]["stages"], runs["native"]["stages"], runs["native"]["execution"], runs["native"]["events"])
    rows.append(compare_layout(runs["go"]["layouts"], runs["native"]["layouts"], runs["native"]["execution"], runs["native"]["events"]))
    calls = json.loads((capture / "go/previews.json").read_text())
    replay = capture / "preview-replay"
    replay.mkdir()
    (replay / "tmp").mkdir()
    request, response = replay / "request.json", replay / "response.json"
    request.write_text(json.dumps([c["input"] for c in calls]) + "\n")
    env = dict(os.environ, TMPDIR=str(replay / "tmp"), D2RS_WATCH_REQUEST=str(request), D2RS_WATCH_RESPONSE=str(response))
    cli.TRACE = ROOT
    native = cli.bounded_run([str(native_test), "--ignored", "--exact", "isometric_cli::watch_preview::api_tests::replay", "--test-threads=1"], env, capture, replay / "native.log", 120)
    results = json.loads(response.read_text()) if native == {"exit": 0, "failure": None} and response.is_file() else []
    assert sha(native_test) == test_digest, "changed native test runner"
    rows += compare_previews(calls, results, native)
    report = dict(source_commit=references.SOURCE_COMMIT, package="d2cli", rows=rows,
        rust=str(runner), rust_sha256=digest, capture=str(capture), runs=runs,
        native_test=str(native_test), native_test_sha256=test_digest,
        preview_calls=calls, preview_results=results, preview_execution=native,
        source_sha256=sha(source_test), hook_sha256=sha(hook),
        layout_test_sha256=sha(source / "d2cli/layout_test.go"),
        captured_sha256={str(p.relative_to(capture)): sha(p) for p in capture.rglob("*") if p.is_file()},
        controls=dict(total_bytes=1 << 30, file_bytes=128 << 20, minimum_free_bytes=20 << 30,
                      subprocess_seconds=600, original_watch_seconds=45, update_seconds=12, shutdown_seconds=10),
        version_stamp="v0.9.0 on both builds; exact SVG bytes",
        scope="All four original watch functions plus original built-in layout flag-order test captured with assertions active. Four source-mode SVG file reloads and six layout invocations; explicit SIGINT process/API cancellation transport. Raw targets adapt from an identity graph into owned JSON for the shared production preview copy/relink/painter; cyclic Go pointer targets stay unsupported. Complete SVG/error/tree/options bytes; no normalization. Mutable layout resolvers, compiler board planning and raw Go pointer admission are separate contracts.")
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(rows, indent=2))
    return int(any(row["outcome"] not in {"match", "unsupported"} for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
