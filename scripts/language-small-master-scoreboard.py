#!/usr/bin/env python3
"""Four original master API functions, full results and original assertions.

Fresh writers only. Budgets: 1 GiB total, 128 MiB/file, 20 GiB free.
"""
import argparse
import collections
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("internal", ROOT / "scripts/language-ir-internal-master-scoreboard.py")
internal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(internal)
master = internal.master
sha = internal.sha
TESTS = {
    "d2graph/grid_diagram_test.go": {"TestGridCapacityLimits": 5},
    "d2ir/index_test.go": {"TestEnsureFieldCreateWithoutCompiler": 1},
    "d2ir/substitution_cycle_test.go": {"TestCompositeContainsNodeHandlesExistingCycle": 2},
    "lib/textmeasure/markdown_svg_test.go": {"TestDangerousLinkPolicy": 34},
}
HOOKS = {"d2graph": "grid", "d2ir": "ir", "lib/textmeasure": "links"}


def inventory():
    return [{"path": p, "test": t} for p, tests in TESTS.items() for t in tests]


def source():
    return {"source_commit": references.SOURCE_COMMIT,
        "tests": {p: sha(ROOT / references.MANIFEST["d2"]["checkout"] / p) for p in TESTS},
        "hooks": {name: sha(ROOT / f"tools/language-master-scoreboard/small_{name}_hook_test.go") for name in HOOKS.values()}}


def hashes(capture):
    paths = [capture / name for name in ("inventory.json", "go.jsonl", "run.json", "source.json")]
    paths += sorted((capture / "go").glob("*.json"))
    return {str(p.relative_to(capture)): sha(p) for p in paths}


def verify(capture):
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 * 1024**2 and sum(sizes) <= 1024**3, "capture budget exceeded"
    assert json.loads((capture / "source.json").read_text()) == source(), "source changed"
    assert json.loads((capture / "run.json").read_text()) == {"exit": 0, "failure": None}, "Go tests failed"
    assert json.loads((capture / "inventory.json").read_text()) == inventory(), "inventory changed"
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture), "capture changed"
    assert all(p.is_file() and p.suffix == ".json" for p in (capture / "go").iterdir()), "unexpected capture output"
    cases = [json.loads(p.read_text()) for p in (capture / "go").glob("*.json")]
    names = [c["name"] for c in cases]
    # Grid names are original leaf subtests, each with one call.
    grid_names = ["exact_dimension_limit", "over_dimension_limit", "exact_cell_limit", "over_cell_limit", "integer_overflow"]
    expected = {"TestGridCapacityLimits/" + n + "#0" for n in grid_names}
    expected |= {f"{t}#{i}" for p, tests in TESTS.items() if not p.startswith("d2graph/") for t, count in tests.items() for i in range(count)}
    assert len(names) == len(set(names)) and set(names) == expected, "missing/extra/duplicate original call"
    events = master.module("small_events", "cli-scoreboard.py").events(capture / "go.jsonl")
    assert all(events.get(c["test"]) == "pass" for c in cases), "original assertion did not pass"
    assert all(events.get(r["test"]) == "pass" for r in inventory()), "original function did not pass"
    for c in cases:
        assert c["test"] == c["name"].split("#")[0], "test changed"
        op, got = c["request"]["op"], c["got"]
        assert op in {"grid", "ensure", "contains", "danger", "safe"}, "unknown API"
        if op == "grid":
            assert set(got) == {"capacity", "error"} and type(got["capacity"]) is int and isinstance(got["error"], str), "incomplete grid result"
        elif op == "ensure":
            assert set(got) == {"fields", "error"} and isinstance(got["fields"], list) and isinstance(got["error"], str), "incomplete IR result"
        else:
            assert type(got) is (str if op == "safe" else bool), "incomplete API result"
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    originals = {p: (pinned / p).read_bytes() for p in TESTS}
    helpers = [references.TRACE_CHECKOUT / package / "d2rs_small_test.go" for package in HOOKS]
    assert all(not p.exists() for p in helpers), "hook already installed"
    assert all((references.TRACE_CHECKOUT / p).read_bytes() == data for p, data in originals.items()), "original tests instrumented"
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "inventory.json").write_text(json.dumps(inventory(), indent=2) + "\n")
    env = dict(os.environ, D2RS_MASTER_LANGUAGE_DUMP=str(capture / "go"), GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
    env.pop("TESTDATA_ACCEPT", None)
    env.pop("TA", None)
    try:
        replacements = {
            "d2graph/grid_diagram_test.go": [("GridCapacity(tc.rows, tc.columns)", "d2rsSmallGrid(t, tc.rows, tc.columns)")],
            "d2ir/index_test.go": [("m.EnsureField(key, nil, true, nil)", "d2rsSmallEnsure(t, m, key, nil, true, nil)")],
            "d2ir/substitution_cycle_test.go": [("compositeContainsNode(root,", "d2rsSmallContains(t, root,")],
            "lib/textmeasure/markdown_svg_test.go": [("textmeasure.IsDangerousLink(link)", "d2rsSmallDanger(t, link)"), ("textmeasure.SafeMarkdownLink(link)", "d2rsSmallSafe(t, link)")],
        }
        for p, data in originals.items():
            text = data.decode()
            for old, new in replacements[p]:
                assert old in text, old
                text = text.replace(old, new)
            (references.TRACE_CHECKOUT / p).write_text(text)
        for package, name in HOOKS.items():
            (references.TRACE_CHECKOUT / package / "d2rs_small_test.go").write_bytes((ROOT / f"tools/language-master-scoreboard/small_{name}_hook_test.go").read_bytes())
        cli = master.module("small_capture", "cli-scoreboard.py")
        run = cli.bounded_run(["go", "test", *["./" + p for p in HOOKS], "-run", "^(" + "|".join(r["test"] for r in inventory()) + ")$", "-json", "-count=1"], env, capture, capture / "go.jsonl", 600)
    finally:
        for p, data in originals.items():
            (references.TRACE_CHECKOUT / p).write_bytes(data)
        for p in helpers:
            p.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(run, indent=2) + "\n")
    assert run == {"exit": 0, "failure": None}, run
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)


def score(capture, runner, output):
    cases = verify(capture)
    runner_hash = sha(runner)
    cli = master.module("small_run", "cli-scoreboard.py")
    lang = master.module("small_json", "language-scoreboard.py")
    native = output.parent / (output.stem + "-native")
    native.mkdir(parents=True, exist_ok=False)
    rows = []
    for case in sorted(cases, key=lambda c: c["name"]):
        name = internal.hashlib.sha256(case["name"].encode()).hexdigest()
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
                assert set(actual) == {"got", "observations"}
                got = actual["got"]
                row["outcome"] = "unsupported" if isinstance(got, dict) and "unsupported" in got else "match" if lang.same_json(case["got"], got) else "mismatch"
                if row["outcome"] == "mismatch":
                    row["differences"] = lang.differences(case["got"], got)
            except (AssertionError, KeyError, ValueError, TypeError):
                row["outcome"] = "invalid-output"
        rows.append(row)
    assert sha(runner) == runner_hash, "runner changed"
    report = {"source_commit": references.SOURCE_COMMIT, "runner_sha256": runner_hash, "capture_sha256": sha(capture / "hashes.json"), "summary": dict(collections.Counter(r["outcome"] for r in rows)), "rows": rows}
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
        assert args.runner, "native test runner required"
        score(capture, args.runner, args.output)


if __name__ == "__main__":
    main()
