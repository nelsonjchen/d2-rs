#!/usr/bin/env python3
"""Original grid measurement calls: complete intervals and IEEE-754 bits.

Go cache-allocation assertions remain active. Native storage strategy is not
part of this numerical projection, and is never credited as cache parity.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
PACKAGE = "d2layouts/d2grid"
TESTS = {
    "TestGridIntegralMeasurementsMatchAdditionOrder": 100,
    "TestGridIntegralMeasurementBounds": 15,
    "TestGridIntegralSingletonMeasurementsStayLazy": 1,
}
BOUNDARIES = {"zeros", "exact_limit", "exact_limit_with_gaps", "sum_over_limit",
    "gap_sum_over_limit", "size_over_limit", "gap_over_limit", "fractional_size",
    "fractional_gap", "negative_size", "negative_gap", "infinite_size",
    "infinite_gap", "nan_size", "nan_gap"}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compare(calls, results):
    counts = {test: 0 for test in TESTS}
    boundaries = []
    for call in calls:
        assert set(call) == {"test", "input", "result"}
        test = call["test"].split("/")[0]
        assert test in TESTS
        counts[test] += 1
        if test == "TestGridIntegralMeasurementBounds":
            boundaries.append(call["test"].split("/")[1])
        request = call["input"]
        assert set(request) == {"sizes", "gap", "ranges"}, "expected data in request"
        assert request["ranges"] and len(request["ranges"]) == len(call["result"])
        assert all(isinstance(v, str) and len(v) == 16 and int(v, 16) >= 0
                   for v in request["sizes"] + [request["gap"]])
        n = len(request["sizes"])
        if test == "TestGridIntegralSingletonMeasurementsStayLazy":
            assert n == 100000 and request["ranges"] == [[0, 0], [0, 1], [0, 2]]
        else:
            assert 2 <= n <= 81
            exhaustive = [[start, end] for start in range(n) for end in range(start, n + 1)]
            if test == "TestGridIntegralMeasurementsMatchAdditionOrder":
                assert len(request["ranges"]) == 100 + len(exhaustive)
                assert request["ranges"][100:] == exhaustive, "missing original interval"
            else:
                assert request["ranges"] == exhaustive, "missing original boundary interval"
        for interval, result in zip(request["ranges"], call["result"]):
            assert 0 <= interval[0] <= interval[1] <= len(request["sizes"])
            assert len(result) == 4 and result[:2] == interval
            assert all(isinstance(v, str) and len(v) == 16 and int(v, 16) >= 0 for v in result[2:])
    assert counts == TESTS, "incomplete/extra original session inventory"
    assert len(boundaries) == len(BOUNDARIES) and set(boundaries) == BOUNDARIES
    rows = []
    for test in TESTS:
        indexes = [i for i, call in enumerate(calls) if call["test"].split("/")[0] == test]
        matched = [i for i in indexes if len(results) == len(calls)
                   and json.dumps(results[i]) == json.dumps(calls[i]["result"])]
        rows.append(dict(test=test, sessions=len(indexes), measurements=sum(len(calls[i]["result"]) for i in indexes),
            matched_sessions=len(matched), outcome="match" if len(matched) == len(indexes) else "mismatch"))
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
    capture = args.capture.resolve()
    runner = args.native_test.resolve()
    runner_hash = sha(runner)
    hook = ROOT / "tools/grid-measurement-scoreboard/hook_test.go"
    originals = {name: (source / PACKAGE / name).read_bytes() for name in ["grid_integral_test.go", "grid_search.go"]}
    bindings = dict(source_commit=references.SOURCE_COMMIT, package=PACKAGE, hook_sha256=sha(hook),
        sources={str(Path(PACKAGE) / name): hashlib.sha256(data).hexdigest() for name, data in originals.items()})
    env = dict(os.environ, GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod", GOPROXY="off", TMPDIR=str(capture / "tmp"), D2RS_GRID_MEASUREMENT_CAPTURE=str(capture / "calls"))
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "tmp").mkdir()
        target = references.TRACE_CHECKOUT / PACKAGE / "grid_integral_test.go"
        helper = target.parent / "d2rs_grid_measurement_test.go"
        original = originals[target.name]
        assert target.read_bytes() == original and not helper.exists(), "concurrent source mutation"
        old = "got := measurements.get(start, end)"
        assert original.decode().count(old) == 1
        try:
            target.write_text(original.decode().replace(old, "got := d2rsMeasurement(t, measurements, start, end)"))
            helper.write_bytes(hook.read_bytes())
            result = cli.bounded_run(["go", "test", "./" + PACKAGE, "-run", "^(" + "|".join(TESTS) + ")$", "-json", "-count=1", "-parallel=1"], env, capture, capture / "go.jsonl", 600)
        finally:
            target.write_bytes(original)
            helper.unlink(missing_ok=True)
        assert result == dict(exit=0, failure=None), result
        events = cli.events(capture / "go.jsonl")
        assert all(events.get(test) == "pass" for test in TESTS), events
        paths = sorted((capture / "calls").glob("*.json")) + [capture / "go.jsonl"]
        manifest = dict(bindings, go=result, original_assertions=events,
            captured_sha256={str(p.relative_to(capture)): sha(p) for p in paths})
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    manifest = json.loads((capture / "manifest.json").read_text())
    assert all(manifest.get(k) == v for k, v in bindings.items()), "changed source/hook"
    assert manifest["go"] == dict(exit=0, failure=None)
    paths = sorted((capture / "calls").glob("*.json")) + [capture / "go.jsonl"]
    assert {str(p.relative_to(capture)): sha(p) for p in paths} == manifest["captured_sha256"], "changed capture inventory/bytes"
    calls = [json.loads(p.read_text()) for p in paths if p.suffix == ".json"]
    compare(calls, [])
    assert all(manifest["original_assertions"].get(call["test"]) == "pass" for call in calls)
    replay = capture / ("replay-" + args.output.stem)
    replay.mkdir(exist_ok=False)
    request, response = replay / "input.json", replay / "output.json"
    request.write_text(json.dumps([c["input"] for c in calls]) + "\n")
    env.update(D2RS_GRID_MEASUREMENT_REQUEST=str(request), D2RS_GRID_MEASUREMENT_RESPONSE=str(response))
    native = cli.bounded_run([str(runner), "--exact", "grid::measurement_api_tests::replay", "--ignored", "--nocapture"], env, replay, replay / "native.log", 600)
    results = json.loads(response.read_text()) if native == dict(exit=0, failure=None) and response.exists() else []
    assert sha(runner) == runner_hash, "native executable changed"
    rows = compare(calls, results)
    report = dict(bindings, capture=str(capture), replay=str(replay), runner=str(runner), runner_sha256=runner_hash,
        rows=rows, original_assertions=manifest["original_assertions"], native=native,
        session_sha256=[sha(p) for p in paths if p.suffix == ".json"], results_sha256=sha(response) if response.exists() else None,
        controls=dict(total_bytes=1 << 30, file_bytes=128 << 20, minimum_free_bytes=20 << 30, deadline_seconds=600),
        projection=__doc__)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(rows, indent=2))
    return int(any(row["outcome"] != "match" for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
