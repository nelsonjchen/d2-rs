#!/usr/bin/env python3
"""Original remaining Lane L master tests; full results and allocation assertions.

Capture budget: 1 GiB total, 128 MiB/file, 20 GiB free; fresh writers only.
"""
import argparse
import collections
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("small", ROOT / "scripts/language-small-master-scoreboard.py")
small = importlib.util.module_from_spec(spec)
spec.loader.exec_module(small)
master, sha = small.master, small.sha
TESTS = {
    "lib/textmeasure/substitutions_test.go": "TestMarkdownMatcherNoMatchStorageIsSparse",
    "lib/urlenc/urlenc_test.go": "TestDecodeRejectsOversizedScript",
}
HOOKS = {"lib/textmeasure": "sparse", "lib/urlenc": "urlenc"}


def inventory():
    return [{"path": p, "test": t} for p, t in TESTS.items()]


def source():
    return {"source_commit": references.SOURCE_COMMIT,
        "tests": {p: sha(ROOT / references.MANIFEST["d2"]["checkout"] / p) for p in TESTS},
        "hooks": {name: sha(ROOT / f"tools/language-master-scoreboard/{name}_hook_test.go") for name in HOOKS.values()}}


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
    assert all(p.is_file() and p.suffix == ".json" for p in (capture / "go").iterdir()), "unexpected output"
    cases = [json.loads(p.read_text()) for p in (capture / "go").glob("*.json")]
    names = [c["name"] for c in cases]
    expected = {TESTS[p] + "#0" for p in TESTS} | {"TestDecodeRejectsOversizedScript#1"}
    assert len(names) == len(set(names)) and set(names) == expected, "missing/extra/duplicate original call"
    events = master.module("remaining_events", "cli-scoreboard.py").events(capture / "go.jsonl")
    assert all(events.get(t) == "pass" for t in TESTS.values()), "original assertion did not pass"
    for c in cases:
        assert c["test"] == c["name"].split("#")[0], "test changed"
        got, op = c["got"], c["request"]["op"]
        assert op in {"sparse", "encode", "decode"}, "unknown API"
        assert isinstance(got["result"], str) and isinstance(got["error"], str), "incomplete result"
        if op == "sparse":
            assert set(got) == {"result", "used", "error", "terminal_storage_sparse", "allocations_within_limit"}, "incomplete sparse result"
            assert type(got["used"]) is int and type(got["terminal_storage_sparse"]) is bool and type(got["allocations_within_limit"]) is bool, "invalid sparse result"
            assert isinstance(c["observations"]["allocations_per_run"], (int, float)), "missing allocation observation"
        else:
            assert set(got) == {"result", "error"}, "incomplete codec result"
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    originals = {p: (pinned / p).read_bytes() for p in TESTS}
    helpers = [references.TRACE_CHECKOUT / package / "d2rs_remaining_test.go" for package in HOOKS]
    assert all(not p.exists() for p in helpers), "hook already installed"
    assert all((references.TRACE_CHECKOUT / p).read_bytes() == data for p, data in originals.items()), "original tests instrumented"
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "inventory.json").write_text(json.dumps(inventory(), indent=2) + "\n")
    env = dict(os.environ, D2RS_MASTER_LANGUAGE_DUMP=str(capture / "go"), GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
    env.pop("TESTDATA_ACCEPT", None)
    env.pop("TA", None)
    try:
        for p, data in originals.items():
            text = data.decode()
            if p.startswith("lib/textmeasure/"):
                text = text.replace("matcher, err := newMarkdownSubstitutionMatcher(context.Background(),", "matcher, err := d2rsSparseNew(t, context.Background(),")
                text = text.replace("testing.AllocsPerRun(5, func() {", "d2rsSparseAllocs(t, matcher, input, 5, func() {")
            else:
                text = text.replace("Encode(script)", "d2rsURLEncode(t, script)").replace("Decode(encoded)", "d2rsURLDecode(t, encoded)")
            assert text != data.decode(), "missing insertion point"
            (references.TRACE_CHECKOUT / p).write_text(text)
        for package, name in HOOKS.items():
            (references.TRACE_CHECKOUT / package / "d2rs_remaining_test.go").write_bytes((ROOT / f"tools/language-master-scoreboard/{name}_hook_test.go").read_bytes())
        cli = master.module("remaining_capture", "cli-scoreboard.py")
        run = cli.bounded_run(["go", "test", *["./" + p for p in HOOKS], "-run", "^(" + "|".join(TESTS.values()) + ")$", "-json", "-count=1"], env, capture, capture / "go.jsonl", 600)
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


def score(capture, text_runner, url_runner, output):
    assert not output.exists(), "fresh report required"
    cases = verify(capture)
    runners = {"text": sha(text_runner), "url": sha(url_runner)}
    cli = master.module("remaining_run", "cli-scoreboard.py")
    lang = master.module("remaining_json", "language-scoreboard.py")
    native = output.parent / (output.stem + "-native")
    native.mkdir(parents=True, exist_ok=False)
    rows = []
    for case in sorted(cases, key=lambda c: c["name"]):
        name = small.internal.hashlib.sha256(case["name"].encode()).hexdigest()
        request, result = native / (name + ".request.json"), native / (name + ".result.json")
        request.write_text(json.dumps(case["request"]))
        env = dict(os.environ, D2RS_REMAINING_REQUEST=str(request.resolve()), D2RS_REMAINING_RESULT=str(result.resolve()))
        sparse = case["request"]["op"] == "sparse"
        runner = text_runner if sparse else url_runner
        test = "markdown::substitute::sparse_tests::master_sparse_scoreboard" if sparse else "master_urlenc_scoreboard"
        run = cli.bounded_run([str(runner.resolve()), "--exact", test, "--nocapture"], env, native, native / (name + ".log"), 60)
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
                if sparse:
                    row["observations"] = {"go": case["observations"], "native": actual["observations"]}
            except (AssertionError, KeyError, ValueError, TypeError):
                row["outcome"] = "invalid-output"
        rows.append(row)
    assert sha(text_runner) == runners["text"] and sha(url_runner) == runners["url"], "runner changed"
    report = {"source_commit": references.SOURCE_COMMIT, "runners_sha256": runners, "capture_sha256": sha(capture / "hashes.json"), "summary": dict(collections.Counter(r["outcome"] for r in rows)), "rows": rows}
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"], flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--text-runner", type=Path)
    parser.add_argument("--url-runner", type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--capture-only", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        capture_original(capture)
    verify(capture)
    if not args.capture_only:
        assert args.text_runner and args.url_runner, "native test runners required"
        score(capture, args.text_runner, args.url_runner, args.output)


if __name__ == "__main__":
    main()
