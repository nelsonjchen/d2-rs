#!/usr/bin/env python3
"""Original IR copy reservations, node costs and post-Compile budget accounting.

Only the trace checkout is instrumented; all original assertions stay active.
Capture budget: 1 GiB total / 128 MiB file / 20 GiB minimum free space.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("master", ROOT / "scripts/language-master-scoreboard.py")
master = importlib.util.module_from_spec(spec)
spec.loader.exec_module(master)
RELATIVE = "d2ir/expansion_test.go"
TESTS = {"TestVariableExpansionCopyUnitsIncludeSlices": 6,
         "TestVariableExpansionCopyReservationsAreCumulative": 1,
         "TestImportedArraySpreadChargesInsertedMembers": 2}


def hashes(capture):
    paths = [capture / name for name in ("inventory.json", "go.jsonl", "run.json", "source.json")]
    paths += sorted((capture / "go").glob("*.json"))
    assert len(paths) > 4, "empty IR copy capture"
    return {str(path.relative_to(capture)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


def verify(capture):
    sizes = [path.stat().st_size for path in capture.rglob("*") if path.is_file()]
    assert sizes and max(sizes) <= 128 * 1024**2 and sum(sizes) <= 1024**3, "capture budget exceeded"
    assert all(path.is_file() and path.suffix == ".json" for path in (capture / "go").iterdir()), "unexpected source output"
    source = json.loads((capture / "source.json").read_text())
    assert source["source_commit"] == references.SOURCE_COMMIT, "source pin changed"
    pinned = ROOT / references.MANIFEST["d2"]["checkout"] / RELATIVE
    assert source["test_sha256"] == hashlib.sha256(pinned.read_bytes()).hexdigest(), "source test changed"
    hook = ROOT / "tools/language-master-scoreboard/ir_copy_hook_test.go"
    assert source["hook_sha256"] == hashlib.sha256(hook.read_bytes()).hexdigest(), "source hook changed"
    assert json.loads((capture / "run.json").read_text()) == {"exit": 0, "failure": None}, "original tests failed"
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture), "capture missing, extra or changed"
    calls = [json.loads(path.read_text()) for path in (capture / "go").glob("*.json")]
    names = [case["name"] for case in calls]
    assert len(names) == len(set(names)) and set(names) == {f"{test}#{i}" for test, count in TESTS.items() for i in range(count)}, "missing, extra or duplicate original IR copy session"
    events = master.module("copy_events", "cli-scoreboard.py").events(capture / "go.jsonl")
    assert all(events.get(test) == "pass" for test in TESTS), "original IR copy assertion failed"
    assert json.loads((capture / "inventory.json").read_text()) == [{"path": RELATIVE, "test": test} for test in TESTS], "original inventory changed"
    for case in calls:
        assert case["test"] == case["name"].split("#")[0], "session test changed"
        if case["test"] == "TestVariableExpansionCopyUnitsIncludeSlices":
            assert case["request"]["op"] == "node_units" and "node" in case["request"], "missing original node"
            assert set(case["got"]) == {"units"} and isinstance(case["got"]["units"], int), "missing unit result"
        else:
            assert case["request"]["op"] == "compile"
            assert isinstance(case["request"]["copies"], list)
            expected_copies = 2 if case["test"] == "TestVariableExpansionCopyReservationsAreCumulative" else 0
            assert len(case["request"]["copies"]) == expected_copies, "missing original copy operation"
            assert all(copy == {"context": "background"} for copy in case["request"]["copies"]), "unknown original copy context"
            assert len(case["got"]["results"]) == 1 + len(case["request"]["copies"]), "missing reservation result"
            for result in case["got"]["results"]:
                assert set(result) == {"ir", "imports", "error", "error_kind", "limit", "used"}, "incomplete IR copy result"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--runner", type=Path, default=ROOT / "target/diagnostic/examples/master-ir-copy-scoreboard")
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--capture-only", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        original = (references.verify_source() / RELATIVE).read_bytes()
        target = references.TRACE_CHECKOUT / RELATIVE
        helper = references.TRACE_CHECKOUT / "d2ir/d2rs_ir_copy_test.go"
        hook = ROOT / "tools/language-master-scoreboard/ir_copy_hook_test.go"
        assert target.read_bytes() == original and not helper.exists(), "unexpected IR copy instrumentation"
        (capture / "go").mkdir(parents=True, exist_ok=False)
        (capture / "inventory.json").write_text(json.dumps([{"path": RELATIVE, "test": test} for test in TESTS], indent=2) + "\n")
        env = dict(os.environ, D2RS_MASTER_LANGUAGE_DUMP=str(capture / "go"),
                   GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
        env.pop("TESTDATA_ACCEPT", None)
        env.pop("TA", None)
        try:
            text = original.decode().replace("d2parser.Parse(", "d2rsCopyParse(t, ")
            text = re.sub(r"(?<![\w.])Compile\(", "d2rsCopyCompile(t, ", text)
            text = text.replace("ReserveVariableExpansionCopy(", "d2rsCopyReserve(t, ")
            text = text.replace("variableExpansionNodeUnits(", "d2rsCopyUnits(t, ")
            text += "\nvar _ = d2parser.Parse // capture import anchor\n"
            target.write_text(text)
            helper.write_bytes(hook.read_bytes())
            cli = master.module("copy_capture", "cli-scoreboard.py")
            run = cli.bounded_run(["go", "test", "./d2ir", "-run", "^(" + "|".join(dict.fromkeys(test.split("/")[0] for test in TESTS)) + ")$", "-json", "-count=1"], env, capture, capture / "go.jsonl", 600)
        finally:
            target.write_bytes(original)
            helper.unlink(missing_ok=True)
        (capture / "run.json").write_text(json.dumps(run, indent=2) + "\n")
        assert run == {"exit": 0, "failure": None}, run
        (capture / "source.json").write_text(json.dumps({"source_commit": references.SOURCE_COMMIT, "test_sha256": hashlib.sha256(original).hexdigest(), "hook_sha256": hashlib.sha256(hook.read_bytes()).hexdigest()}, indent=2) + "\n")
        (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)
    if not args.capture_only:
        master.score(capture, args.runner, args.output, package="d2ir")


if __name__ == "__main__":
    main()
