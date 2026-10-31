#!/usr/bin/env python3
"""Original TALA compound-route refinement calls, through production code.

Placement is source-supplied input. Native replay tests PreserveCompoundRoutes,
not the preceding CompoundCandidate placement/rerouting or its context schedule.
Compare complete ordered geometry/label views and both inputs after the call.
This projection excludes cost caches, pointer identity and direction-map storage.
Node label text and label-less edge percentages remain opaque input metadata.
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
    "TestCompoundCandidatePreservesInteriorBusNestedAndLoopRoutes": 2,
    **{"TestCompoundCandidateKeepsSelectedRouteForUnsafeInterior/" + name: 1 for name in
       ["incomplete", "detached", "escaped", "subpixel-escape", "escaped-curve-control", "escaped-label"]},
    "TestCompoundRefinementPreservesSelectedExternalRoutesAndCurves/selected-polyline": 1,
    "TestCompoundRefinementPreservesSelectedExternalRoutesAndCurves/selected-bezier": 1,
    "TestCompoundCandidatePreservesEnclosedBezierAndUnassignedEdgeIDs/assigned-edge-ids": 1,
    "TestCompoundCandidatePreservesEnclosedBezierAndUnassignedEdgeIDs/unassigned-edge-ids": 1,
    "TestCompoundCandidateReconsidersNodeLabelAtReroutedPort": 1,
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

    for payload in result.values():
        geometry(payload["view"])
    assert same_json(request, result), "transport changed JSON values"
    return result


def compare(calls, results):
    actual = {}
    for call in calls:
        assert set(call) == {"test", "call", "input", "result"}
        actual.setdefault(call["test"], []).append(call["call"])
        request = call["input"]
        assert set(request) == {"before", "selected"}, "unexpected input/expected data"
        for payload in request.values():
            assert set(payload) == {"wire", "view"}
            assert set(payload["view"]) == {"nodes", "edges"}
            assert len(payload["view"]["nodes"]) == 9 and len(payload["view"]["edges"]) == 6
        assert set(call["result"]) == {"graph", "error", "before_after", "selected_after"}
    assert set(actual) == set(TESTS), "missing/extra original test"
    assert all(actual[test] == list(range(count)) for test, count in TESTS.items()), "missing/extra original call"
    rows = []
    for test in dict.fromkeys(t.split("/")[0] for t in TESTS):
        indexes = [i for i, c in enumerate(calls) if c["test"].split("/")[0] == test]
        matched = [i for i in indexes if len(results) == len(calls) and same_json(results[i], calls[i]["result"])]
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
    hook = ROOT / "tools/tala-interior-scoreboard/hook_test.go"
    names = list(dict.fromkeys(test.split("/")[0] for test in TESTS))
    originals = {str(Path(PACKAGE) / name): (source / PACKAGE / name).read_bytes()
                 for name in ["compound_interior_routes_test.go", "compound_routes.go", "compound_candidate.go"]}
    bindings = dict(source_commit=references.SOURCE_COMMIT, package=PACKAGE, hook_sha256=sha(hook),
                    sources={path: hashlib.sha256(data).hexdigest() for path, data in originals.items()})
    env = dict(os.environ, GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
               GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off",
               TMPDIR=str(capture / "tmp"), D2RS_TALA_INTERIOR_CAPTURE=str(capture / "calls"))
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "tmp").mkdir()
        target = references.TRACE_CHECKOUT / PACKAGE / "compound_interior_routes_test.go"
        helper = target.parent / "d2rs_interior_test.go"
        original = originals[str(Path(PACKAGE) / target.name)]
        assert target.read_bytes() == original and not helper.exists(), "concurrent source mutation"
        text = original.decode()
        assert text.count("PreserveCompoundRoutes(") == 3
        assert text.count("compoundWithPreservedRoutes(") == 8
        try:
            text = text.replace("func compoundWithPreservedRoutes(ctx context.Context", "func compoundWithPreservedRoutes(t *testing.T, ctx context.Context")
            text = text.replace("compoundWithPreservedRoutes(t.Context()", "compoundWithPreservedRoutes(t, t.Context()")
            # The two excluded cancellation tests are not executed; their calls need no change.
            text = text.replace("compoundWithPreservedRoutes(ctx,", "compoundWithPreservedRoutes(t, ctx,").replace("compoundWithPreservedRoutes(probe,", "compoundWithPreservedRoutes(t, probe,")
            target.write_text(text.replace("PreserveCompoundRoutes(", "d2rsPreserveOriginal(t, "))
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
    env.update(D2RS_TALA_INTERIOR_REQUEST=str(request), D2RS_TALA_INTERIOR_RESPONSE=str(response))
    native = cli.bounded_run([str(runner), "--exact", "internal::compound_routes::interior_api_tests::replay",
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
