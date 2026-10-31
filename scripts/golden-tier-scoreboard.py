#!/usr/bin/env python3
"""Inventory every pinned d2sketch/d2exporter/d2svg golden; qualify Go before Rust.

Captures come from the original upstream test bodies. Exporter comparison
clears exactly the fields its own golden contract clears, without touching
root/legend/arrowhead metadata. SVG tiers (sketch, d2svg dark_theme and
appendix) use only the established e2e SVG XML/version/trailing-newline
allowance; raw witnesses are retained.
"""
from __future__ import annotations

import argparse
import collections
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess

from references import verify_source, digest
import references

ROOT = Path(__file__).resolve().parents[1]
TRACE = references.TRACE_CHECKOUT
UPSTREAM = ROOT.parent / "d2"


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


CLI = module("tier_cli", "cli-scoreboard.py")
E2E = module("tier_e2e", "e2e-scoreboard.py")
LANGUAGE = module("tier_json", "language-scoreboard.py")


def insert(path, old, new):
    text = path.read_text()
    if new not in text:
        assert text.count(old) == 1, (path, old)
        path.write_text(text.replace(old, new))


# Per SVG tier: Go package directory, test file, test function, golden
# directory and basename, and the request recorded beside the upstream
# assertion (each mirrors that test's own RenderOpts/CompileOptions).
SVG_TIERS = {
    "sketch": ("d2renderers/d2sketch", "sketch_test.go", "TestSketch", "sketch",
               '"engine":tc.engine, "theme_id":tc.themeID'),
    # dark_theme_test.go: ThemeID 200, FontFamily HandDrawn, no sketch.
    "dark": ("d2renderers/d2svg/dark_theme", "dark_theme_test.go", "TestDarkTheme", "dark_theme",
             '"theme_id":200, "font_family":"handdrawn"'),
    # appendix_test.go: ThemeID tc.themeID, then appendix.Append.
    "appendix": ("d2renderers/d2svg/appendix", "appendix_test.go", "TestAppendix", "sketch",
                 '"theme_id":tc.themeID, "appendix":true'),
}
TIERS = [*SVG_TIERS, "exporter"]


def install(tier):
    verify_source()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=TRACE, text=True).strip()
    assert revision == references.SOURCE_COMMIT, revision
    if tier in SVG_TIERS:
        relative, test_file, _, basename, fields = SVG_TIERS[tier]
        directory = TRACE / relative
        package = Path(relative).name
    else:
        directory, package = TRACE / "d2exporter", "d2exporter"
    helper = (ROOT / "tools/golden-tier-scoreboard/hook_test.go").read_text()
    (directory / "d2rs_tier_scoreboard_test.go").write_text(helper.replace("PACKAGE", package))
    if tier in SVG_TIERS:
        path = directory / test_file
        marker = f'\terr = diff.Testdata(filepath.Join(dataPath, "{basename}"), ".svg", svgBytes)'
        hook = f'\td2rsTierDump(t, "golden", map[string]interface{{}}{{"tier":"{tier}", "input":tc.script, {fields}}}, diagram, svgBytes)\n'
        insert(path, marker, hook + marker)
    else:
        path = directory / "export_test.go"
        request = 'map[string]interface{}{"tier":"exporter", "input":tc.dsl}'
        marker = '\t// This test is agnostic of layout changes'
        insert(path, marker, f'\td2rsTierDump(t, "full", {request}, got, nil)\n' + marker)
        marker = '\terr = diff.TestdataJSON(filepath.Join("..", "testdata", "d2exporter", t.Name()), got)'
        insert(path, marker, f'\td2rsTierDump(t, "golden", {request}, got, nil)\n' + marker)
    return directory


def exporter_contract(diagram):
    result = copy.deepcopy(diagram)
    for shape in result["shapes"] or []:
        shape.update(pos={"x": 0, "y": 0}, width=0, height=0, labelWidth=0, labelHeight=0)
        # labelPosition is omitempty in Go's target schema.
        shape.pop("labelPosition", None)
    for connection in result["connections"] or []:
        # Connection.LabelPosition is not omitempty (unlike Shape's).
        connection.update(route=[], labelWidth=0, labelHeight=0, labelPosition="")
    return result


def equal(tier, left, right):
    if tier in SVG_TIERS:
        # Reject corrupt XML even if the same corrupt bytes appear twice.
        import xml.etree.ElementTree as ET
        ET.fromstring(left)
        ET.fromstring(right)
        return E2E.normalize(left.decode()) == E2E.normalize(right.decode())
    a, b = json.loads(left), json.loads(right)
    for value in [a, b]:
        if not isinstance(value, dict) or not {"shapes", "connections", "root"} <= value.keys():
            raise ValueError("incomplete target diagram")
    return LANGUAGE.same_json(a, b)


def score_golden(tier, golden, go, native, failure=None):
    """No absent, empty, stale, timed-out or unsupported result can pass."""
    if go is None:
        return "unbound-golden"
    if not go:
        return "go-empty-output"
    try:
        if not equal(tier, golden, go):
            return "go-golden-mismatch"
    except Exception:
        return "go-invalid-output"
    if failure:
        return failure
    if native is None or not native:
        return "rust-missing-output"
    try:
        return "golden-match" if equal(tier, golden, native) else "golden-mismatch"
    except Exception:
        return "rust-invalid-output"


def golden_paths(tier):
    if tier in SVG_TIERS:
        base = UPSTREAM / SVG_TIERS[tier][0] / "testdata"
        return base, sorted(base.rglob(f"{SVG_TIERS[tier][3]}.exp.svg"))
    base = UPSTREAM / "testdata/d2exporter"
    return base, sorted(base.rglob("*.exp.json"))


def exporter_assertions(name, diagram):
    """The two pre-scrub assertions in upstream export_test.go:80-84,101-102."""
    if name == "TestExport/shape/image_dimensions":
        return diagram["shapes"][0]["width"] == 200 and diagram["shapes"][0]["height"] == 230
    if name == "TestExport/shape/sequence_group_position":
        shape = diagram["shapes"][3]
        return shape["id"] == "hey.group" and shape["labelPosition"] == "INSIDE_TOP_LEFT"
    return None


def native_inventory(tier, output):
    required = {"target.json", "sketch.svg" if tier in SVG_TIERS else "golden.json"}
    for filename in required:
        path = output / filename
        if not path.is_file() or not path.stat().st_size:
            return "rust-missing-output"
    actual = {path.name for path in output.iterdir()} - {"request.json", "result.json", "process.log"}
    return None if actual == required else "rust-inventory-mismatch"


def assess(tier, capture, binary):
    base, goldens = golden_paths(tier)
    captured = {}
    for path in sorted((capture / "go").glob("*.golden.json")):
        row = json.loads(path.read_text())
        name = row["name"]
        assert name not in captured, name
        captured[name] = (path, row)
    test_events = CLI.events(capture / "go.log")
    rows = []
    bound = set()
    for golden in goldens:
        relative = golden.relative_to(base).as_posix()
        name = (SVG_TIERS[tier][2] + "/" + str(Path(relative).parent) if tier in SVG_TIERS
                else relative.removesuffix(".exp.json"))
        row = {"name": name, "golden": str(golden.relative_to(ROOT.parent)), "golden_sha256": digest(golden)}
        go, native, failure = None, None, None
        if name in captured:
            bound.add(name)
            path, record = captured[name]
            row["capture"] = str(path.relative_to(ROOT))
            row["go_test"] = test_events.get(name, "missing-event")
            if tier in SVG_TIERS:
                svg = path.with_name(path.name.replace(".golden.json", ".svg"))
                go = svg.read_bytes() if svg.is_file() else b""
            else:
                go = json.dumps(record["go"]).encode()
            output = capture / "rust" / path.stem.removesuffix(".golden")
            result_path = output / "result.json"
            result = json.loads(result_path.read_text()) if result_path.is_file() else {"failure": "rust-missing-output"}
            failure = result.get("failure")
            failure = failure or native_inventory(tier, output)
            actual = output / ("sketch.svg" if tier in SVG_TIERS else "golden.json")
            if actual.is_file():
                native = actual.read_bytes()
            row["native_result"] = result
            if tier == "exporter" and name in {"TestExport/shape/image_dimensions", "TestExport/shape/sequence_group_position"}:
                raw_target = output / "target.json"
                try:
                    matched = (not failure and test_events.get(name + "/assertions") == "pass"
                               and exporter_assertions(name, json.loads(raw_target.read_text())))
                except (OSError, ValueError, KeyError, IndexError, TypeError):
                    matched = False
                row["assertion_outcome"] = "assertion-match" if matched else "assertion-mismatch"
        row["outcome"] = score_golden(tier, golden.read_bytes(), go, native, failure)
        if name not in captured and name in test_events:
            row["outcome"] = "go-" + {"pass": "missing-output", "fail": "failed", "skip": "upstream-skip"}[test_events[name]]
        if row.get("go_test") in {"skip", "missing-event"}:
            row["outcome"] = "go-" + row["go_test"]
        rows.append(row)
    # A new live test without a retained golden is visible, never silently lost.
    for name in sorted(captured.keys() - bound):
        rows.append({"name": name, "outcome": "no-golden"})
    summary = dict(collections.Counter(row["outcome"] for row in rows))
    return {"tier": tier, "binary": str(binary), "binary_sha256": digest(binary),
            "goldens": len(goldens), "live_captures": len(captured), "summary": summary,
            "go_run": json.loads((capture / "go-result.json").read_text()) if (capture / "go-result.json").is_file() else None,
            "assertions": dict(collections.Counter(row["assertion_outcome"] for row in rows if "assertion_outcome" in row)),
            "rows": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tier", choices=TIERS, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/oracle/examples/golden-tier-scoreboard")
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    capture, binary = args.capture.resolve(), args.rust.resolve()
    if not binary.is_file():
        parser.error("build first: cargo build --profile oracle -p d2-rs-cli --example golden-tier-scoreboard")
    if args.reuse:
        manifest = json.loads((capture / "manifest.json").read_text())
        assert manifest["binary_sha256"] == digest(binary), "native binary changed"
        assert manifest["tier"] == args.tier
        for relative, checksum in manifest["files"].items():
            assert digest(capture / relative) == checksum, ("capture changed", relative)
    else:
        assert not capture.exists(), "use a fresh capture path"
        directory = install(args.tier)
        (capture / "go").mkdir(parents=True)
        environment = dict(os.environ, D2RS_TIER_DUMP=str(capture / "go"),
                           GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
                           GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
        for name in ["TESTDATA_ACCEPT", "TA"]:
            environment.pop(name, None)
        package = "./" + str(directory.relative_to(TRACE))
        test_name = SVG_TIERS[args.tier][2] if args.tier in SVG_TIERS else "TestExport"
        command = ["go", "test", package, "-run", f"^{test_name}$", "-count=1", "-json"]
        # Original assertions briefly write .got files beside their goldens.
        # Include that live inventory in the same streaming budget.
        temporary_outputs = directory / "testdata" if args.tier in SVG_TIERS else TRACE / "testdata/d2exporter"
        result = CLI.bounded_run(command, environment, capture, capture / "go.log", 900,
                                 extra_directories=[temporary_outputs])
        (capture / "go-result.json").write_text(json.dumps(result, indent=2) + "\n")
        if result["failure"]:
            raise RuntimeError(result)
        for path in sorted((capture / "go").glob("*.golden.json")):
            record = json.loads(path.read_text())
            output = capture / "rust" / path.stem.removesuffix(".golden")
            output.mkdir(parents=True)
            request = output / "request.json"
            request.write_text(json.dumps(record["request"]) + "\n")
            native = CLI.bounded_run([str(binary), str(request), str(output)],
                                     os.environ.copy(), capture, output / "process.log", 90)
            if native["exit"] and not native["failure"]:
                native["failure"] = "rust-failed"
            target = output / "target.json"
            if args.tier == "exporter" and target.is_file() and not native["failure"]:
                value = exporter_contract(json.loads(target.read_text()))
                (output / "golden.json").write_text(json.dumps(value, indent=2) + "\n")
            (output / "result.json").write_text(json.dumps(native) + "\n")
        manifest = {"tier": args.tier, "binary_sha256": digest(binary),
                    "hook_sha256": digest(ROOT / "tools/golden-tier-scoreboard/hook_test.go"),
                    "source_commit": references.SOURCE_COMMIT,
                    "budget": {"total_bytes": 1 << 30, "file_bytes": 128 << 20, "minimum_free_bytes": 20 << 30}}
        manifest["files"] = {str(path.relative_to(capture)): digest(path)
                             for path in sorted(capture.rglob("*")) if path.is_file()}
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    report = assess(args.tier, capture, binary)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(args.tier, report["summary"], "goldens", report["goldens"], "captures", report["live_captures"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
