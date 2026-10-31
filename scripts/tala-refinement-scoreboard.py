#!/usr/bin/env python3
"""Original TALA optional-refinement calls, through production selection code.

The native replay treats graph/sequence/binding payloads as opaque values. This
qualifies acceptance, fallback, complete publication and context precedence;
it does not qualify graph serialization, routing geometry or Go pointer identity.
On an error, both adapters expose no published value (Rust's Result boundary).
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
PACKAGE = "d2layouts/d2talalayout"
TESTS = {
    "TestOptionalSeedRefinementKeepsSelectedPlacementDespiteHigherScore": 2,
    "TestOptionalSeedRefinementFailurePreservesCompleteSelectedResult/ordinary_error": 1,
    "TestOptionalSeedRefinementFailurePreservesCompleteSelectedResult/panic": 1,
    "TestOptionalSeedRefinementFailurePreservesCompleteSelectedResult/canceled": 1,
    "TestOptionalSeedRefinementFailurePreservesCompleteSelectedResult/wrapped_cancellation": 1,
    "TestOptionalSeedRefinementFailurePreservesCompleteSelectedResult/deadline": 1,
    "TestOptionalSeedRefinementCallerCancellationAlwaysPropagates/success": 1,
    "TestOptionalSeedRefinementCallerCancellationAlwaysPropagates/error": 1,
    "TestOptionalSeedRefinementCallerCancellationAlwaysPropagates/panic": 1,
}

_spec = importlib.util.spec_from_file_location("language_scoreboard", ROOT / "scripts/language-scoreboard.py")
_language = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_language)
same_json = _language.same_json


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compare(calls, results):
    actual = {}
    for call in calls:
        assert set(call) == {"test", "call", "input", "result"}
        actual.setdefault(call["test"], []).append(call["call"])
        request = call["input"]
        assert set(request) == {"ordinary", "incumbent", "candidate", "build_error",
                               "panic", "context_before", "context_after"}, "unexpected input/expected data"
        assert type(request["ordinary"]) is bool and type(request["panic"]) is bool
        assert request["context_before"] == "<nil>"
        assert request["context_after"] in {"<nil>", "context canceled"}
        for payload in [request["incumbent"], request["candidate"]]:
            if payload is not None:
                assert set(payload) == {"graph", "penalty", "area", "sequence_edges", "bindings"}
                assert payload["bindings"] == [None, None, None]
        assert set(call["result"]) == {"selected", "error", "incumbent_after"}
    assert set(actual) == set(TESTS), "missing/extra original test"
    assert all(actual[test] == list(range(count)) for test, count in TESTS.items()), "missing/extra original call"
    first = [c["input"] for c in calls if c["test"] == next(iter(TESTS))]
    assert [r["ordinary"] for r in first] == [False, True]
    assert all(r["candidate"]["penalty"] > r["incumbent"]["penalty"] for r in first)
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
    hook = ROOT / "tools/tala-refinement-scoreboard/hook_test.go"
    names = list(dict.fromkeys(test.split("/")[0] for test in TESTS))
    originals = {str(Path(PACKAGE) / name): (source / PACKAGE / name).read_bytes()
                 for name in ["refinement_test.go", "layout.go", "result.go"]}
    bindings = dict(source_commit=references.SOURCE_COMMIT, package=PACKAGE, hook_sha256=sha(hook),
                    sources={path: hashlib.sha256(data).hexdigest() for path, data in originals.items()})
    env = dict(os.environ, GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
               GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off",
               TMPDIR=str(capture / "tmp"), D2RS_TALA_REFINEMENT_CAPTURE=str(capture / "calls"))
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "tmp").mkdir()
        target = references.TRACE_CHECKOUT / PACKAGE / "refinement_test.go"
        helper = target.parent / "d2rs_refinement_test.go"
        original = originals[str(Path(PACKAGE) / target.name)]
        assert target.read_bytes() == original and not helper.exists(), "concurrent source mutation"
        text = original.decode()
        assert text.count("refineSeedResult(") == 3 and text.count("considerSeedCandidate(") == 1
        try:
            target.write_text(text.replace("refineSeedResult(", "d2rsRefineOriginal(t, ")
                             .replace("considerSeedCandidate(", "d2rsConsiderOriginal(t, "))
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
    request.write_text(json.dumps([c["input"] for c in calls]) + "\n")
    env.update(D2RS_TALA_REFINEMENT_REQUEST=str(request), D2RS_TALA_REFINEMENT_RESPONSE=str(response))
    native = cli.bounded_run([str(runner), "--exact", "internal::snapshot::refinement_api_tests::replay",
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
