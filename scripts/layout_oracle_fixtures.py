#!/usr/bin/env python3
"""Whole-D2 layout oracle fixtures (recipes in tools/fixtures/layout-oracle.toml
and the `tala-tree-sentinel-hierarchy` recipe in tools/fixtures/tala-oracle.toml).

Run only through the runner, e.g.
`uv run --no-project python scripts/regen-fixtures.py --only 'layout-*'`.

Every subcommand builds tools/d2-layout-oracle against the runner's patched
upstream tree ($D2RS_UPSTREAM, the manifest pin) with a small -overlay on
package d2layouts (see that program), compiles hand-written D2 sources with
d2lib and an in-process engine, runs twice requiring identical output, and
writes only its staged outputs and its workdir.

- `sequence`  sequence-diagram targets with edge reference scopes and source
              lines (group labels, note and lifeline order), the failing
              object-position validation of sequence scopes, and the
              validation message formats.
- `grid`      fixed-size 2x2 grids: every object box (decimal and bits).
- `elk-nested-sequence-grid`  ELK target boxes of a sequence inside a grid
              cell; the native adapter graph comes from a seed.
- `tala-tree-sentinel`  TALA seed-1 target boxes of a retained tree sentinel
              inside a container; the native adapter graph comes from a seed.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys

from fixture_recipe import go_env, workdir
from references import ROOT, SOURCE_COMMIT
from tala_oracle_fixtures import build, run_twice, sha256, upstream, write_json

LAYOUT = "crates/d2-rs-layout/tests/fixtures/"
SEEDS = ROOT / "tools/fixtures/seeds"
POSITIONS = "d2layouts/d2layouts.go"


def oracle() -> Path:
    source = (upstream() / POSITIONS).read_text()
    original = "func validateObjectPositions(objects []*d2graph.Object) error {"
    assert source.count(original) == 1, f"{POSITIONS}: validateObjectPositions not found exactly once"
    overlay = {
        POSITIONS: source.replace(original, original.replace("validateObjectPositions", "d2rsValidateObjectPositions")),
        "d2layouts/d2rs_positions.go": (ROOT / "tools/d2-layout-oracle/overlay/d2rs_positions.go").read_text(),
    }
    return build("tools/d2-layout-oracle", "d2-layout-oracle", overlay)


def run(binary: Path, requests: list[dict]) -> dict[str, dict]:
    stdout = run_twice([str(binary)], (json.dumps(requests, indent=1) + "\n").encode())
    rows = json.loads(stdout)
    assert [r["name"] for r in rows] == [r["name"] for r in requests]
    return {r["name"]: r for r in rows}


def sequence(_args) -> None:
    binary = oracle()
    sources = json.loads((SEEDS / "layout-sequence-sources.json").read_text())
    requests = [{"name": f"{group}/{name}", "source": source, "layout": "dagre"}
                for group, cases in sources.items() for name, source in cases.items()]
    requests += [{"name": f"validate/{name}", "validate": [{"id": name, "x": x, "y": y}]}
                 for name, x, y in [("positive", "+Inf", "+Inf"), ("negative", "-Inf", "-0"),
                                    ('quoted"', "1.2345678", "+Inf"), ("nan", "NaN", "NaN"), ("finite", "1", "2")]]
    rows = run(binary, requests)

    def row(group, name):
        result = rows[f"{group}/{name}"]
        assert not result["error"], f"{group}/{name}: {result['error']}"
        return result

    write_json(LAYOUT + "sequence-group-label-adjustment-source-v090.json", [
        {"name": name, "source": source, "shapes": row("group-label-adjustment", name)["shapes"],
         "connections": row("group-label-adjustment", name)["connections"],
         "reference_edges": row("group-label-adjustment", name)["reference_edges"]}
        for name, source in sources["group-label-adjustment"].items()])
    for group, logical in [("note-source-order", "sequence-note-source-order-source-v090.json"),
                           ("lifeline-source-order", "sequence-lifeline-source-order-source-v090.json")]:
        cases = []
        for name, source in sources[group].items():
            result = row(group, name)
            cases.append({"name": name, "shapes": result["shapes"], "connections": result["connections"],
                          "reference_edges": result["reference_edges"],
                          "lines": {**result["lines"], "name": name, "source": source}})
        write_json(LAYOUT + logical, cases)
    cases = []
    for name, source in sources["invalid-positions"].items():
        result = rows[f"invalid-positions/{name}"]
        assert result["error"] and result["positions"] is not None, f"{name}: layout did not fail position validation"
        cases.append({"name": name, "source": source, "positions": result["positions"], "error": result["error"]})
    validator = [rows[f"validate/{name}"]["error"] or f"{name} accepted"
                 for name in ["positive", "negative", 'quoted"', "nan", "finite"]]
    write_json(LAYOUT + "sequence-invalid-positions-source-v090.json",
               {"source_commit": SOURCE_COMMIT, "cases": cases, "validator": validator})


def grid(_args) -> None:
    binary = oracle()
    requests = []
    for width, height in [(200, 200), (400, 300), (80, 80)]:
        for gap in (0, 20, 40):
            for order in ("rows", "columns"):
                axes = "grid-rows: 2; grid-columns: 2" if order == "rows" else "grid-columns: 2; grid-rows: 2"
                requests.append({"name": f"{width}-{height}-{gap}-{order}", "layout": "dagre",
                                 "source": f"grid: {{width: {width}; height: {height}; grid-gap: {gap}; {axes}; "
                                           "a; b; c; d}"})
    rows = run(binary, requests)
    write_json(LAYOUT + "grid-fixed-placement-source-v090.json",
               [{"name": r["name"], "d2": r["source"], "objects": rows[r["name"]]["objects"],
                 "error": rows[r["name"]]["error"]} for r in requests])


def target_boxes(binary: Path, seed: dict, layout: str, seeds=None) -> list[dict]:
    request = {"name": "case", "source": seed["source"], "layout": layout}
    if seeds:
        request["seeds"] = seeds
    result = run(binary, [request])["case"]
    assert not result["error"], result["error"]
    return result["shapes"]


def elk_nested_sequence_grid(_args) -> None:
    seed = json.loads((SEEDS / "layout-elk-nested-sequence-grid.json").read_text())
    shapes = target_boxes(oracle(), seed, "elk")
    write_json(LAYOUT + "elk-nested-sequence-grid-source-v090.json", {
        "source_commit": SOURCE_COMMIT, "source_script": seed["source"], "measured_graph_origin": seed["graph_origin"],
        "graph": seed["graph"],
        "expected": [{"id": s["id"], "pos": s["pos"], "width": s["width"], "height": s["height"]} for s in shapes]})


def tala_tree_sentinel(_args) -> None:
    seed = json.loads((SEEDS / "tala-tree-sentinel-hierarchy.json").read_text())
    shapes = target_boxes(oracle(), seed, "tala", [1])
    write_json("crates/d2-tala-rs/tests/fixtures/tree-sentinel-hierarchy-eligibility-v090.json", {
        "description": seed["description"], "source": seed["source"],
        "oracle": f"D2 {SOURCE_COMMIT} d2lib.Compile, TALA seeds [1]: boxes are the target shape rects "
                  "(top-left x, y, width, height)",
        "graph": seed["graph"],
        "expected_boxes": {s["id"]: [float(s["pos"]["x"]), float(s["pos"]["y"]), float(s["width"]), float(s["height"])]
                           for s in shapes}})


GRID = "d2layouts/d2grid"


def grid_test_capture(test_file: str, replacements: list[tuple[str, str, int]], hook: str, capture_env: str,
                      tests: list[str]) -> tuple[Path, dict[str, str]]:
    """Run upstream d2grid tests with `test_file` rewritten to call the
    scoreboard hook's recorders (go test -overlay; the shared tree is not
    modified). Returns the capture directory and each test's outcome."""
    text = (upstream() / GRID / test_file).read_text()
    for old, new, count in replacements:
        found = text.count(old)
        assert found == count or (count < 0 and found > 0), f"{test_file}: {old!r} occurs {found} times"
        text = text.replace(old, new)
    overlay_dir = workdir() / "grid-overlay"
    overlay_dir.mkdir(parents=True, exist_ok=True)
    (overlay_dir / test_file).write_text(text)
    overlay = workdir() / "grid-overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(upstream() / GRID / test_file): str(overlay_dir / test_file),
        str(upstream() / GRID / "d2rs_capture_test.go"): str(ROOT / hook)}}))
    capture = workdir() / "grid-calls"
    shutil.rmtree(capture, ignore_errors=True)
    events = subprocess.run(
        ["go", "test", "-mod=readonly", "-count=1", "-parallel=1", "-json", "-overlay", str(overlay),
         "-run", "^(" + "|".join(tests) + ")$", f"./{GRID}"],
        cwd=upstream(), env={**go_env(), capture_env: str(capture)}, stdout=subprocess.PIPE, check=True,
        timeout=900).stdout
    outcomes = {}
    for line in events.decode().splitlines():
        if line.startswith("{"):
            event = json.loads(line)
            if event.get("Test") and event.get("Action") in ("pass", "fail", "skip"):
                outcomes[event["Test"]] = event["Action"]
    assert all(outcomes.get(t) == "pass" for t in tests), outcomes
    return capture, dict(sorted(outcomes.items()))


def grid_capacity(_args) -> None:
    """Upstream resource_limits_test.go constructor/capacity/layout calls."""
    counts = {"TestNewGridDiagramRejectsUnsafeCapacity/max_int_panic_regression": 1,
              "TestNewGridDiagramRejectsUnsafeCapacity/twenty_million_allocator_regression": 1,
              "TestGridCapacityBoundaryAndOccupiedStorage": 3}
    hook = "tools/grid-capacity-scoreboard/hook_test.go"
    capture, outcomes = grid_test_capture(
        "resource_limits_test.go",
        [("newGridDiagram(", "d2rsGridConstruct(t, ", -1), ("d2graph.GridCapacity(", "d2rsGridCapacity(t, ", -1),
         ("gd.layoutEvenly(", "d2rsGridLayout(t, gd, ", -1)],
        hook, "D2RS_GRID_CAPACITY_CAPTURE", list(dict.fromkeys(t.split("/")[0] for t in counts)))
    calls = []
    for name, count in counts.items():
        files = sorted((capture / name.replace("/", "_")).glob("*.json"))
        assert len(files) == count, (name, len(files), count)
        calls += [json.loads(p.read_text()) for p in files]
    write_json(LAYOUT + "grid-capacity-master.json", {
        "source_commit": SOURCE_COMMIT, "package": GRID, "hook_sha256": sha256((ROOT / hook).read_bytes()),
        "original_assertions": outcomes, "calls": calls})


# Upstream randomized getBestLayout reference cases kept as regressions.
GRID_SEARCH_CALLS = ["TestGridIntegralSearchMatchesReference/1/columns=true",
                     "TestGridIntegralSearchMatchesReference/36/columns=false",
                     "TestGridIntegralSearchMatchesReference/78/columns=false",
                     "TestGridSearchMatchesReference/22/columns=false",
                     "TestGridSearchMatchesReference/52/columns=true"]


def grid_search(_args) -> None:
    hook = "tools/grid-search-scoreboard/hook_test.go"
    capture, outcomes = grid_test_capture(
        "grid_search_test.go",
        [("got := gd.getBestLayout(target, columns)", "got := d2rsGridSearch(t, gd, target, columns)", 1)],
        hook, "D2RS_GRID_SEARCH_CAPTURE", ["TestGridSearchMatchesReference", "TestGridIntegralSearchMatchesReference"])
    calls = {c["test"]: c for c in (json.loads(p.read_text()) for p in sorted(capture.glob("*.json")))}
    assert len(calls) == 443, len(calls)
    write_json(LAYOUT + "grid-search-master.json", {
        "source_commit": SOURCE_COMMIT, "package": GRID, "hook_sha256": sha256((ROOT / hook).read_bytes()),
        "original_assertions": {t: outcomes[t] for t in GRID_SEARCH_CALLS},
        "calls": [calls[t] for t in GRID_SEARCH_CALLS]})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    for name, func in [("sequence", sequence), ("grid", grid), ("elk-nested-sequence-grid", elk_nested_sequence_grid),
                       ("tala-tree-sentinel", tala_tree_sentinel), ("grid-capacity", grid_capacity),
                       ("grid-search", grid_search)]:
        sub.add_parser(name).set_defaults(func=func)
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    sys.exit(main())
