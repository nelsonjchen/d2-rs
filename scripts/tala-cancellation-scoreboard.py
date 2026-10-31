#!/usr/bin/env python3
"""Original compound-cancellation calls with exact geometry/cancellation-cause/input comparison.

Go's normal probe calibrates Context.Err checks. Native calibration instead
counts its existing compound stage/publication boundaries (cfg(test) observer).
The three fractions are the same, but poll counts/locations are NOT equivalent.
The successful probe also compares the complete refined graph, with no score
rejection in this test adapter. Four cancellations require a null graph, exact
cancellation cause and unchanged original input. Raw complete Go/native
diagnostics stay visible; the original assertions check errors.Is, not prefixes. Metadata exclusions match the interior
suite: node label text, label-less percentage, direction storage, caches and
pointer identity are not qualified. No expected result enters native replay.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
PACKAGE = "d2layouts/d2talalayout/internal/engine"
TESTS = {
    "TestCompoundCandidateCancellationPreservesRoutedInterior": 1,
    "TestCompoundCandidateMidflightCancellationPreservesRoutedInput": 4,
}


_spec = importlib.util.spec_from_file_location("language_scoreboard", ROOT / "scripts/language-scoreboard.py")
_language = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_language)
same_json = _language.same_json


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def native_input(request):
    # Lossless transport spelling only: Go encodes integral float64s as `100`,
    # while serde_json::Value distinguishes `100` from `100.0` in the adapter's
    # reconstruction assertions. The source capture is never rewritten.
    result = json.loads(json.dumps(request))

    def geometry(value):
        if isinstance(value, dict):
            for key, child in value.items():
                if key in {"x", "y", "width", "height", "percentage"}:
                    assert type(child) in {int, float}
                    converted = float(child)
                    assert converted == child, "inexact numeric transport"
                    value[key] = converted
                else:
                    geometry(child)
        elif isinstance(value, list):
            for child in value:
                geometry(child)

    geometry(result["graph"]["view"])
    assert same_json(request, result), "transport changed JSON values"
    return result


def compare(calls, results):
    actual = {}
    for call in calls:
        assert set(call) == {"test", "call", "input", "result", "source_schedule", "diagnostic"}
        actual.setdefault(call["test"], []).append(call["call"])
        request = call["input"]
        assert set(request) == {"graph", "cancel"}, "unexpected input/expected data"
        payload = request["graph"]
        assert set(payload) == {"wire", "view"}
        assert set(payload["view"]) == {"nodes", "edges"}
        assert len(payload["view"]["nodes"]) == 9 and len(payload["view"]["edges"]) == 6
        assert set(call["result"]) == {"graph", "cause", "input_after"}
        index = call["call"]
        if call["test"].endswith("RoutedInterior"):
            assert request["cancel"] == {"kind": "pre"}
            assert call["source_schedule"] == {}
        elif index == 0:
            assert request["cancel"] == {"kind": "probe"}
            assert call["source_schedule"]["limit"] == 0
            assert call["result"]["cause"] == "" and call["result"]["graph"] is not None
        else:
            assert request["cancel"] == {"kind": "fraction", "fraction_index": index-1}
            source = call["source_schedule"]
            assert source["checks_before"] == 0 and source["checks_after"] == source["limit"] >= 2
        if request["cancel"]["kind"] != "probe":
            assert call["result"]["graph"] is None and call["result"]["cause"] == "context canceled"
        assert same_json(call["result"]["input_after"], payload["view"])
    assert set(actual) == set(TESTS), "missing/extra original test"
    assert all(actual[test] == list(range(count)) for test, count in TESTS.items()), "missing/extra original call"
    mid = [call for call in calls if call["test"].endswith("RoutedInput")]
    probe_checks = mid[0]["source_schedule"]["checks_after"]
    limits = [probe_checks//3, 2*probe_checks//3, probe_checks-1]
    assert [call["source_schedule"]["limit"] for call in mid[1:]] == limits
    assert all(same_json(c["input"]["graph"], mid[0]["input"]["graph"]) for c in mid)

    def result_matches(index):
        if len(results) != len(calls):
            return False
        result, call = results[index], calls[index]
        if set(result) != {"result", "diagnostic", "native_schedule"}:
            return False
        if not same_json(result["result"], call["result"]):
            return False
        schedule = result["native_schedule"]
        if set(schedule) != {"checks", "limit", "probe_checks"}:
            return False
        if any(type(value) is not int for value in schedule.values()):
            return False
        kind = call["input"]["cancel"]["kind"]
        if kind == "pre":
            return schedule == {"checks":1, "limit":0, "probe_checks":0}
        if kind == "probe":
            return schedule["checks"] >= 3 and schedule["limit"] == schedule["probe_checks"] == 0
        count = schedule["probe_checks"]
        limit = [count//3, 2*count//3, count-1][call["input"]["cancel"]["fraction_index"]]
        return count >= 6 and schedule["checks"] == schedule["limit"] == limit

    rows = []
    for test in dict.fromkeys(t.split("/")[0] for t in TESTS):
        indexes = [i for i, c in enumerate(calls) if c["test"].split("/")[0] == test]
        matched = [i for i in indexes if result_matches(i)]
        rows.append(dict(test=test, calls=len(indexes), matched_calls=len(matched),
                         outcome="match" if len(matched) == len(indexes) else "mismatch"))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-test", required=True, type=Path)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    source = references.verify_source()
    spec = importlib.util.spec_from_file_location("cli", ROOT / "scripts/cli-scoreboard.py")
    cli = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(cli)
    capture, runner = args.capture.resolve(), args.native_test.resolve()
    runner_hash = sha(runner)
    hook = ROOT / "tools/tala-cancellation-scoreboard/hook_test.go"
    names = list(dict.fromkeys(test.split("/")[0] for test in TESTS))
    originals = {str(Path(PACKAGE) / name): (source / PACKAGE / name).read_bytes()
                 for name in ["compound_interior_routes_test.go", "compound_routes.go", "compound_candidate.go"]}
    bindings = dict(source_commit=references.SOURCE_COMMIT, package=PACKAGE, hook_sha256=sha(hook),
                    sources={path: hashlib.sha256(data).hexdigest() for path, data in originals.items()})
    env = dict(os.environ, GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
               GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off",
               TMPDIR=str(capture / "tmp"), D2RS_TALA_CANCELLATION_CAPTURE=str(capture / "calls"))
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "tmp").mkdir()
        target = references.TRACE_CHECKOUT / PACKAGE / "compound_interior_routes_test.go"
        helper = target.parent / "d2rs_cancellation_test.go"
        original = originals[str(Path(PACKAGE) / target.name)]
        assert target.read_bytes() == original and not helper.exists(), "concurrent source mutation"
        text = original.decode()
        assert text.count("compoundWithPreservedRoutes(ctx, f.graph)") == 2
        assert text.count("compoundWithPreservedRoutes(probe, f.graph)") == 1
        try:
            text = text.replace("compoundWithPreservedRoutes(ctx, f.graph)", "d2rsCompoundOriginal(t, ctx, f.graph)")
            text = text.replace("compoundWithPreservedRoutes(probe, f.graph)", "d2rsCompoundOriginal(t, probe, f.graph)")
            target.write_text(text)
            helper.write_bytes(hook.read_bytes())
            result = cli.bounded_run(["go", "test", "./" + PACKAGE, "-run", "^(" + "|".join(names) + ")$",
                                      "-json", "-count=1", "-parallel=1"], env, capture, capture / "go.jsonl", 600)
        finally:
            target.write_bytes(original)
            helper.unlink(missing_ok=True)
        assert result == dict(exit=0, failure=None), result
        assertions = cli.events(capture / "go.jsonl")
        assert all(assertions.get(test) == "pass" for test in set(TESTS) | set(names)), assertions
        paths = sorted((capture / "calls").rglob("*.json")) + [capture / "go.jsonl"]
        manifest = dict(bindings, go=result, original_assertions=assertions,
                        captured_sha256={str(p.relative_to(capture)): sha(p) for p in paths})
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    manifest = json.loads((capture / "manifest.json").read_text())
    assert all(manifest.get(k) == v for k, v in bindings.items()), "changed source/hook"
    assert manifest["go"] == dict(exit=0, failure=None)
    assert all(manifest["original_assertions"].get(test) == "pass" for test in set(TESTS) | set(names))
    paths = sorted((capture / "calls").rglob("*.json")) + [capture / "go.jsonl"]
    assert {str(p.relative_to(capture)): sha(p) for p in paths} == manifest["captured_sha256"], "changed capture inventory/bytes"
    # Explicit original call order, independent of filesystem traversal order.
    calls = [json.loads(p.read_text()) for p in paths if p.suffix == ".json"]
    order = {test: i for i, test in enumerate(TESTS)}
    calls.sort(key=lambda c: (order.get(c["test"], len(order)), c["call"]))
    compare(calls, [])
    replay = capture / ("replay-" + args.output.stem)
    replay.mkdir(exist_ok=False)
    request, response = replay / "input.json", replay / "output.json"
    request.write_text(json.dumps([native_input(c["input"]) for c in calls]) + "\n")
    env.update(D2RS_TALA_CANCELLATION_REQUEST=str(request), D2RS_TALA_CANCELLATION_RESPONSE=str(response))
    native = cli.bounded_run([str(runner), "--exact", "internal::snapshot::compound_cancellation_api_tests::replay",
                              "--ignored", "--nocapture"], env, replay, replay / "native.log", 600)
    results = json.loads(response.read_text()) if native == dict(exit=0, failure=None) and response.exists() else []
    assert sha(runner) == runner_hash, "native executable changed"
    rows = compare(calls, results)
    report = dict(bindings, capture=str(capture), replay=str(replay), runner=str(runner),
                  runner_sha256=runner_hash, rows=rows, calls=calls, results=results,
                  original_assertions=manifest["original_assertions"], native=native,
                  results_sha256=sha(response) if response.exists() else None,
                  controls=dict(total_bytes=1 << 30, file_bytes=128 << 20,
                                minimum_free_bytes=20 << 30, deadline_seconds=600), projection=__doc__)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(rows, indent=2))
    return int(any(row["outcome"] != "match" for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
