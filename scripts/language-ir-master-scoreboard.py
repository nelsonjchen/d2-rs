#!/usr/bin/env python3
"""Original master IR tests: bounded capture, full IR/import/error replay.

Each original Go assertion must pass. Work-only budget primitives without a
Compile call remain explicitly uncovered; no summary substitutes for full IR.
Capture budget: 1 GiB total, 128 MiB per file, 20 GiB minimum volume free.
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
RELATIVE = "d2ir/glob_expansion_test.go"
SUITES = {"glob": RELATIVE, "variable": "d2ir/expansion_test.go", "edge": "d2ir/edge_expansion_test.go",
          "import": "d2ir/import_test.go", "import-depth": "d2ir/import_depth_test.go"}

def capture_hashes(capture):
    paths = [capture / "inventory.json", capture / "go.jsonl", capture / "run.json", capture / "source.json", *sorted((capture / "go").glob("*.json"))]
    assert len(paths) > 4, "empty IR capture"
    return {str(path.relative_to(capture)):hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}

def verify_capture(capture):
    source = json.loads((capture / "source.json").read_text())
    assert source["source_commit"] == references.SOURCE_COMMIT, "source pin changed"
    relative = source.get("test_path", RELATIVE)
    assert relative in SUITES.values(), "unexpected source test"
    pinned = (ROOT / references.MANIFEST["d2"]["checkout"] / relative).resolve()
    assert source["test_sha256"] == hashlib.sha256(pinned.read_bytes()).hexdigest(), "source test changed"
    assert json.loads((capture / "run.json").read_text()) == {"exit":0,"failure":None}, "source capture failed"
    assert json.loads((capture / "hashes.json").read_text()) == capture_hashes(capture), "capture missing, extra or changed"
    names = [json.loads(path.read_text())["name"] for path in (capture / "go").glob("*.json")]
    assert len(names) == len(set(names)), "duplicate captured call"
    if relative in (SUITES["import"], SUITES["import-depth"]):
        expected = ({"TestCanceledContextPropagatesFromImportedParse#0", "TestNilFSDeniesImports#0"}
                    if relative == SUITES["import"] else
                    {"TestImportDepthLimit#0", "TestImportDepthBoundary#0", "TestImportDepthBoundary#1"})
        assert set(names) == expected, "missing or extra original import call"
        events = master.module("import_events", "cli-scoreboard.py").events(capture / "go.jsonl")
        assert all(events.get(name.split("#")[0]) == "pass" for name in names), "original import assertion failed"

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--runner", type=Path, default=ROOT / "target/oracle/examples/master-ir-scoreboard")
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--capture-only", action="store_true")
    parser.add_argument("--suite", choices=tuple(SUITES), default="glob")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        relative = SUITES[args.suite]
        source = references.verify_source() / relative
        target = references.TRACE_CHECKOUT / relative
        original = target.read_bytes()
        assert original == source.read_bytes(), "IR test already instrumented"
        helper = references.TRACE_CHECKOUT / "d2ir/d2rs_ir_glob_test.go"
        assert not helper.exists(), "IR hook already installed"
        text = original.decode()
        names = re.findall(r"func (Test\w+)\(t \*testing.T\)", text)
        (capture / "go").mkdir(parents=True, exist_ok=False)
        tests = [{"path":relative,"test":name} for name in names]
        (capture / "inventory.json").write_text(json.dumps(tests, indent=2) + "\n")
        env = dict(os.environ, D2RS_MASTER_LANGUAGE_DUMP=str(capture / "go"),
                   GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"), GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
        env.pop("TESTDATA_ACCEPT", None); env.pop("TA", None)
        external = args.suite in ("import", "import-depth")
        if external:
            text = text.replace("d2ir.Compile(", "d2rsIRCompile(t, ")
        else:
            text = re.sub(r"\bCompile\(ast,", "d2rsIRCompile(t, ast,", text)
        text = text.replace("d2parser.Parse(", "d2rsIRParse(t, ")
        target.write_text(text + "\nvar _ = d2parser.Parse // capture import anchor\n")
        hook = "ir_import_hook_test.go" if external else "ir_glob_hook_test.go"
        helper.write_bytes((ROOT / "tools/language-master-scoreboard" / hook).read_bytes())
        cli = master.module("ir_capture", "cli-scoreboard.py")
        try:
            run = cli.bounded_run(["go", "test", "./d2ir", "-run", "^(" + "|".join(names) + ")$", "-json", "-count=1"], env, capture, capture / "go.jsonl", 600)
        finally:
            target.write_bytes(original)
            helper.unlink()
        (capture / "run.json").write_text(json.dumps(run, indent=2) + "\n")
        assert not run["failure"] and run["exit"] == 0, run
        (capture / "source.json").write_text(json.dumps({"source_commit":references.SOURCE_COMMIT,"test_path":relative,"test_sha256":hashlib.sha256(original).hexdigest()}, indent=2)+"\n")
        (capture / "hashes.json").write_text(json.dumps(capture_hashes(capture),indent=2)+"\n")
    verify_capture(capture)
    if args.capture_only:
        return
    master.score(capture, args.runner, args.output, package="d2ir")
    report = json.loads(args.output.read_text())
    if json.loads((capture / "source.json").read_text()).get("test_path") == "d2ir/expansion_test.go":
        # A Compile snapshot does not exercise these later internal assertions.
        report["uncaptured_assertions"] = [
            {"test":"TestVariableExpansionCopyReservationsAreCumulative", "contract":"two cumulative ReserveVariableExpansionCopy calls"},
            {"test":"TestImportedArraySpreadChargesInsertedMembers", "contract":"spread and non-spread budget-used difference is 21"},
        ]
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    captured = {row["test"] for row in report["rows"]}
    print("IR functions:", len(captured), "uncovered:", len(report["uncovered_tests"]))

if __name__ == "__main__": main()
