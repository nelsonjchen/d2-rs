#!/usr/bin/env python3
"""Seven original internal IR tests, with all Go assertions active.

Budgets retain every returned error (only identical requests are run encoded).
Cancellation compares the ORIGINAL partial/exists assertions; raw states stay
in separate observations. Poll counts/partial IR are not upstream equalities.
Capture budget: 1 GiB total, 128 MiB/file, 20 GiB minimum free space.
"""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("master", ROOT / "scripts/language-master-scoreboard.py")
master = importlib.util.module_from_spec(spec)
spec.loader.exec_module(master)
TESTS = {
    "d2ir/expansion_test.go": {"TestVariableExpansionBudget": 4, "TestCompileArrayStopsOnCancellation": 1},
    "d2ir/glob_expansion_test.go": {"TestGlobExpansionBudgetDefault": 2},
    "d2ir/edge_expansion_test.go": {"TestEdgeExpansionBudget": 2, "TestEdgeExpansionWorkBudget": 3,
        "TestEdgeGlobExpansionObservesCancellation": 1, "TestEdgeSelectorGlobExpansionObservesCancellation": 1},
}
HOOK = ROOT / "tools/language-master-scoreboard/ir_internal_hook_test.go"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory():
    return [{"path": path, "test": test} for path, tests in TESTS.items() for test in tests]


def hashes(capture):
    paths = [capture / name for name in ("inventory.json", "go.jsonl", "run.json", "source.json")]
    paths += sorted((capture / "go").glob("*.json"))
    return {str(path.relative_to(capture)): sha(path) for path in paths}


def verify(capture):
    sizes = [path.stat().st_size for path in capture.rglob("*") if path.is_file()]
    assert sizes and max(sizes) <= 128 * 1024**2 and sum(sizes) <= 1024**3, "capture budget exceeded"
    source = json.loads((capture / "source.json").read_text())
    assert source == {"source_commit": references.SOURCE_COMMIT, "hook_sha256": sha(HOOK),
        "tests": {p: sha(ROOT / references.MANIFEST["d2"]["checkout"] / p) for p in TESTS}}, "source changed"
    assert json.loads((capture / "run.json").read_text()) == {"exit": 0, "failure": None}, "Go tests failed"
    assert json.loads((capture / "inventory.json").read_text()) == inventory(), "test inventory changed"
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture), "capture changed"
    assert all(path.is_file() and path.suffix == ".json" for path in (capture / "go").iterdir()), "unexpected original output"
    cases = [json.loads(path.read_text()) for path in (capture / "go").glob("*.json")]
    names = [case["name"] for case in cases]
    expected = {f"{test}#{i}" for tests in TESTS.values() for test, count in tests.items() for i in range(count)}
    assert len(names) == len(set(names)) and set(names) == expected, "missing/extra/duplicate original session"
    events = master.module("internal_events", "cli-scoreboard.py").events(capture / "go.jsonl")
    assert all(events.get(row["test"]) == "pass" for row in inventory()), "original assertion did not pass"
    for case in cases:
        assert case["test"] == case["name"].split("#")[0], "case test changed"
        request, got = case["request"], case["got"]
        if request["op"] == "budget":
            assert set(got) == {"error", "results", "state"} and isinstance(got["results"], list), "incomplete budget results"
            assert isinstance(got["error"], str) and all(isinstance(error, str) for error in got["results"]), "invalid budget diagnostics"
            assert len(got["results"]) == sum(op["count"] for op in request["operations"]), "missing reservation results"
            assert all(op["count"] > 0 and op["op"] in ("reserve", "field") and isinstance(op["units"], int) for op in request["operations"]), "invalid request run"
            assert (got["state"] is None) == bool(got["error"]), "constructor result missing"
        elif request["op"] == "nil_reserve":
            assert set(got) == {"error"} and got["error"], "nil rejection missing"
        elif request["op"] == "array_cancel":
            assert got == {"canceled": True, "incomplete": True}, "original array contract failed"
            assert set(case["observations"]) == {"calls", "array", "source_values", "used"}, "raw array state missing"
        else:
            assert request["op"] == "edge_cancel" and got == {"observed_during_expansion": True}, "original edge contract failed"
            states = case["observations"]
            assert states and [s["cancel_at"] for s in states] == list(range(request["cancel_start"], request["cancel_start"] + len(states))), "missing edge attempts"
            assert states[-1]["cancel_at"] < request["cancel_end"], "invalid cancellation range"
            assert all(set(s) == {"cancel_at", "calls", "canceled", "used", "ir"} for s in states), "raw edge state missing"
            assert states[-1]["canceled"] and 0 < states[-1]["used"] < 9, "missing cancellation witness"
    return cases


def native_result(case, actual):
    assert set(actual) == {"got", "observations"}, "missing native contract/raw state"
    if "unsupported" in actual["got"]:
        return "unsupported"
    if case["request"]["op"] == "array_cancel":
        state = actual["observations"]
        assert set(state) == {"calls", "array", "source_values", "used"}, "missing native array state"
        values = state["array"]["values"] or []
        assert actual["got"]["incomplete"] == (len(values) < state["source_values"]), "contradictory native array result"
    if case["request"]["op"] == "edge_cancel":
        states = actual["observations"]
        assert states and all(set(s) == {"cancel_at", "calls", "canceled", "used", "ir"} for s in states), "missing native edge state"
        start = case["request"]["cancel_start"]
        assert [s["cancel_at"] for s in states] == list(range(start, start + len(states))), "missing native cancellation attempt"
        assert states[-1]["cancel_at"] < case["request"]["cancel_end"], "invalid native cancellation range"
        observed = any(s["canceled"] and 0 < s["used"] < 9 for s in states)
        assert actual["got"]["observed_during_expansion"] == observed, "contradictory native edge result"
    lang = master.module("internal_compare", "language-scoreboard.py")
    return "match" if lang.same_json(case["got"], actual["got"]) else "mismatch"


def capture_original(capture):
    source = references.verify_source()
    originals = {p: (source / p).read_bytes() for p in TESTS}
    helper = references.TRACE_CHECKOUT / "d2ir/d2rs_ir_internal_test.go"
    assert not helper.exists(), "IR internal hook already installed"
    assert all((references.TRACE_CHECKOUT / p).read_bytes() == data for p, data in originals.items()), "tests already instrumented"
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "inventory.json").write_text(json.dumps(inventory(), indent=2) + "\n")
    env = dict(os.environ, D2RS_MASTER_LANGUAGE_DUMP=str(capture / "go"),
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
    env.pop("TESTDATA_ACCEPT", None)
    env.pop("TA", None)
    try:
        for path, data in originals.items():
            text = data.decode().replace("d2parser.Parse(", "d2rsInternalParse(t, ")
            for kind, symbol in (("variable", "Variable"), ("glob", "Glob"), ("edge", "Edge"), ("work", "EdgeExpansionWork")):
                name = "new" + (symbol + "ExpansionBudget" if kind != "work" else symbol + "Budget") + "("
                text = text.replace(name, f'd2rsInternalBudgetNew(t, "{kind}", ')
            text = text.replace("c.compileArray(dst, array, ast)", "d2rsInternalArray(t, c, dst, array, ast)")
            text = text.replace("compileIndexTestSource(t,", "d2rsInternalBase(t,")
            text = text.replace("d2parser.ParseMapKey(", "d2rsInternalKey(t, ")
            text = text.replace("for cancelAt := 1; cancelAt < 128; cancelAt++ {", "d2rsInternalRange(t, 1, 128)\nfor cancelAt := 1; cancelAt < 128; cancelAt++ {")
            text = text.replace("root.CreateEdge(NewEdgeIDs(key)[0], refctx, c)", "d2rsInternalCreate(t, root, NewEdgeIDs(key)[0], refctx, c)")
            text = text.replace("root.getEdgesForCompile(NewEdgeIDs(key)[0], refctx, c)", "d2rsInternalSelect(t, root, NewEdgeIDs(key)[0], refctx, c)")
            text = text.replace("ReserveVariableExpansionAliases(context.Background(), nil)", "d2rsInternalNilReserve(t, context.Background(), nil, false)")
            text = text.replace("ReserveVariableExpansionCopy(context.Background(), nil)", "d2rsInternalNilReserve(t, context.Background(), nil, true)")
            (references.TRACE_CHECKOUT / path).write_text(text + "\nvar _ = d2parser.Parse // capture import anchor\n")
        helper.write_bytes(HOOK.read_bytes())
        cli = master.module("internal_capture", "cli-scoreboard.py")
        run = cli.bounded_run(["go", "test", "./d2ir", "-run", "^(" + "|".join(r["test"] for r in inventory()) + ")$", "-json", "-count=1"], env, capture, capture / "go.jsonl", 600)
    finally:
        for path, data in originals.items():
            (references.TRACE_CHECKOUT / path).write_bytes(data)
        helper.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(run, indent=2) + "\n")
    assert run == {"exit": 0, "failure": None}, run
    (capture / "source.json").write_text(json.dumps({"source_commit": references.SOURCE_COMMIT,
        "hook_sha256": sha(HOOK), "tests": {p: hashlib.sha256(data).hexdigest() for p, data in originals.items()}}, indent=2) + "\n")
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")


def score(capture, runner, output):
    cases = verify(capture)
    runner_hash = sha(runner)
    cli = master.module("internal_run", "cli-scoreboard.py")
    lang = master.module("internal_json", "language-scoreboard.py")
    native = output.parent / (output.stem + "-native")
    native.mkdir(parents=True, exist_ok=False)
    rows = []
    for case in sorted(cases, key=lambda c: c["name"]):
        name = hashlib.sha256(case["name"].encode()).hexdigest()
        request, result = native / (name + ".request.json"), native / (name + ".result.json")
        request.write_text(json.dumps(case["request"]))
        env = dict(os.environ, D2RS_INTERNAL_REQUEST=str(request.resolve()), D2RS_INTERNAL_RESULT=str(result.resolve()))
        run = cli.bounded_run([str(runner.resolve()), "--exact", "compiler::master_internal::scoreboard_request", "--nocapture"], env, native, native / (name + ".log"), 30)
        row = {"name": case["name"], "test": case["test"], "go_test": "pass"}
        if run != {"exit": 0, "failure": None} or not result.is_file():
            row["outcome"] = run["failure"] or "rust-failed"
        else:
            try:
                actual = json.loads(result.read_text())
                row["outcome"] = native_result(case, actual)
                if row["outcome"] == "mismatch":
                    row["differences"] = lang.differences(case["got"], actual["got"])
            except (AssertionError, KeyError, ValueError, TypeError):
                row["outcome"] = "invalid-output"
        rows.append(row)
    assert sha(runner) == runner_hash, "native runner changed"
    report = {"source_commit": references.SOURCE_COMMIT, "runner_sha256": runner_hash,
        "capture_sha256": sha(capture / "hashes.json"), "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows, "comparison": "exact budget results; original array/edge cancellation predicates; raw states retained"}
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"], flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runner", type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--capture-only", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        capture_original(capture)
    verify(capture)
    if not args.capture_only:
        assert args.runner, "native test executable required"
        score(capture, args.runner, args.output)


if __name__ == "__main__":
    main()
