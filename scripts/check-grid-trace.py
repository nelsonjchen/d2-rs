#!/usr/bin/env python3
"""Compare exact D2 v0.9.0 and Rust grid-state snapshots.

All binaries are explicit local oracles: the official release binary, a Go
1.27.0 source build with diagnostic grid hooks, and the diagnostic Rust CLI.
The hooks write only to stderr and this check verifies trace-on/off output
noninterference. It does not claim parity for unmatched intermediate graph
extraction states; those are reported separately from common exact snapshots.
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

from references import ROOT, REFERENCE, digest
from upstream_inputs import pristine_e2e


DEFAULT_FIXTURES = (
    "tests/d2/grid-root-route-trace.d2",
    "tests/d2/grid-direction-left-cell.d2",
    str(pristine_e2e("grid_label_positions.d2")),
    str(pristine_e2e("icons_on_top.d2")),
    str(pristine_e2e("nested_layout_bug.d2")),
)
TRACE_PREFIXES = ("GRID_STATE ", "GRID_LABEL ", "GRID_ROUTE ")
TRACE_ENVIRONMENT_VARIABLES = ("D2_TRACE_GRID_STATE", "D2_TRACE_LAYOUT_FINAL")


def invoke(
    binary: Path,
    fixture: Path,
    output: Path,
    *,
    trace: bool,
    layout: str,
    rust: bool,
    seed: int | None = None,
    require_final_state: bool = False,
) -> dict:
    environment = os.environ.copy()
    inherited_final_state = environment.get("D2_TRACE_LAYOUT_FINAL")
    for variable in TRACE_ENVIRONMENT_VARIABLES:
        environment.pop(variable, None)
    if trace:
        environment["D2_TRACE_GRID_STATE"] = "1"
        if require_final_state:
            environment["D2_TRACE_LAYOUT_FINAL"] = "1"
        elif inherited_final_state is not None:
            environment["D2_TRACE_LAYOUT_FINAL"] = inherited_final_state
    completed = subprocess.run(
        [
            str(binary),
            f"--layout={'tala-rs' if rust and layout == 'tala' else layout}",
            *([] if seed is None else [f"--tala-seeds={seed}"]),
            str(fixture),
            str(output),
        ],
        cwd=ROOT,
        env=environment,
        capture_output=True,
        timeout=120,
    )
    output.with_suffix(output.suffix + ".stdout").write_bytes(completed.stdout)
    output.with_suffix(output.suffix + ".stderr").write_bytes(completed.stderr)
    return {
        "exit_code": completed.returncode,
        "stdout_sha256": hashlib.sha256(completed.stdout).hexdigest(),
        "stderr_sha256": hashlib.sha256(completed.stderr).hexdigest(),
        "svg_sha256": digest(output) if output.exists() else None,
        "trace_environment": {
            variable: environment.get(variable)
            for variable in TRACE_ENVIRONMENT_VARIABLES
        },
        "trace_records": [
            line
            for line in completed.stderr.decode("utf-8", errors="replace").splitlines()
            if line.startswith(TRACE_PREFIXES)
        ],
        "has_trace_when_disabled": (not trace)
        and any(
            line.startswith(TRACE_PREFIXES)
            for line in completed.stderr.decode("utf-8", errors="replace").splitlines()
        ),
    }


def stage_records(run: dict, stage: str) -> Counter[str]:
    return Counter(
        line
        for line in run["trace_records"]
        if line.startswith(("GRID_STATE ", "GRID_ROUTE "))
        and f" stage={stage} " in line
    )


def common_grid_records(go_run: dict, rust_run: dict) -> dict:
    """Compare common grid observations independent of extraction scope labels."""

    def by_node(run: dict) -> Counter[tuple[str, str]]:
        records = Counter()
        for line in run["trace_records"]:
            if not line.startswith("GRID_STATE "):
                continue
            fields = dict(field.split("=", 1) for field in line.split()[1:])
            if fields.get("stage") == "grid" and "node" in fields:
                records[(fields["node"], fields["rect"])] += 1
        return records

    go_records = by_node(go_run)
    rust_records = by_node(rust_run)
    return {
        "common_exact": sum((go_records & rust_records).values()),
        "go_only": [list(item) for item in sorted((go_records - rust_records).elements())],
        "rust_only": [list(item) for item in sorted((rust_records - go_records).elements())],
        "exact_sets": go_records == rust_records,
    }


def common_label_records(go_run: dict, rust_run: dict, stage: str) -> dict:
    """Compare labels by stable node and position across extracted scopes."""

    def by_node(run: dict) -> Counter[tuple[str, str]]:
        records = Counter()
        for line in run["trace_records"]:
            if not line.startswith("GRID_LABEL "):
                continue
            fields = dict(field.split("=", 1) for field in line.split()[1:])
            if fields.get("stage") == stage and "node" in fields:
                records[(fields["node"], fields["position"])] += 1
        return records

    go_records = by_node(go_run)
    rust_records = by_node(rust_run)
    go_nodes = {node for node, _ in go_records}
    rust_nodes = {node for node, _ in rust_records}
    shared_nodes = go_nodes & rust_nodes
    go_comparable = Counter(
        {record: count for record, count in go_records.items() if record[0] in shared_nodes}
    )
    rust_comparable = Counter(
        {record: count for record, count in rust_records.items() if record[0] in shared_nodes}
    )
    return {
        "common_exact": sum((go_comparable & rust_comparable).values()),
        "go_only": [
            list(item) for item in sorted((go_comparable - rust_comparable).elements())
        ],
        "rust_only": [
            list(item) for item in sorted((rust_comparable - go_comparable).elements())
        ],
        "unmatched_go_nodes": sorted(go_nodes - rust_nodes),
        "unmatched_rust_nodes": sorted(rust_nodes - go_nodes),
        "exact_common_nodes": go_comparable == rust_comparable,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixtures", nargs="*", default=DEFAULT_FIXTURES)
    parser.add_argument(
        "--layout",
        choices=("dagre", "tala"),
        default="dagre",
        help="compare D2's Dagre path or bundled TALA path",
    )
    parser.add_argument(
        "--official",
        type=Path,
        default=REFERENCE / "bin/d2-v0.9.0-official",
    )
    parser.add_argument(
        "--go",
        type=Path,
        default=REFERENCE / "bin/d2-v0.9.0-grid-trace-clean",
    )
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument(
        "--report",
        type=Path,
        default=ROOT / "oracle-cache/reports/dagre-grid-trace-parity-20260924.json",
    )
    parser.add_argument("--seed", type=int, help="run one TALA seed separately from the default race")
    parser.add_argument("--require-final-state", action="store_true", help="fail if either final-layout inventory is unavailable")
    parser.add_argument("--output-root", type=Path, default=REFERENCE / "grid-trace-validation", help="directory for raw captures; use a fresh directory to preserve earlier evidence")
    args = parser.parse_args()
    if args.seed is not None and args.layout != "tala":
        parser.error("--seed requires --layout=tala")
    binaries = {"official": args.official, "go": args.go, "rust": args.rust}
    missing = [str(path) for path in binaries.values() if not path.is_file()]
    if missing:
        parser.error(f"missing binary: {', '.join(missing)}")

    validation = args.output_root
    results = []
    overall_passed = True
    for fixture_arg in args.fixtures:
        fixture = (ROOT / fixture_arg).resolve()
        if not fixture.is_file() or not fixture.is_relative_to(ROOT):
            parser.error(f"fixture must be a file within the workspace: {fixture_arg}")
        slug = fixture.relative_to(ROOT).as_posix().replace("/", "_").replace(".", "_")
        if args.seed is not None:
            slug += f"_seed_{args.seed}"
        output_dir = validation / slug
        output_dir.mkdir(parents=True, exist_ok=True)
        runs = {
            "official": invoke(
                args.official, fixture, output_dir / "official.svg",
                trace=False, layout=args.layout, rust=False, seed=args.seed,
                require_final_state=args.require_final_state,
            ),
            "go_trace": invoke(
                args.go, fixture, output_dir / "go-trace.svg",
                trace=True, layout=args.layout, rust=False, seed=args.seed,
                require_final_state=args.require_final_state,
            ),
            "go_plain": invoke(
                args.go, fixture, output_dir / "go-plain.svg",
                trace=False, layout=args.layout, rust=False, seed=args.seed,
                require_final_state=args.require_final_state,
            ),
            "rust_trace": invoke(
                args.rust, fixture, output_dir / "rust-trace.svg",
                trace=True, layout=args.layout, rust=True, seed=args.seed,
                require_final_state=args.require_final_state,
            ),
            "rust_plain": invoke(
                args.rust, fixture, output_dir / "rust-plain.svg",
                trace=False, layout=args.layout, rust=True, seed=args.seed,
                require_final_state=args.require_final_state,
            ),
        }
        all_successful = all(run["exit_code"] == 0 for run in runs.values())
        go_layout_state = stage_records(runs["go_trace"], "layout-final")
        rust_layout_state = stage_records(runs["rust_trace"], "layout-final")
        final_state_available = bool(go_layout_state or rust_layout_state)
        final_state_exact = bool(go_layout_state) and go_layout_state == rust_layout_state
        grid_labels = common_label_records(runs["go_trace"], runs["rust_trace"], "grid")
        final_labels = common_label_records(
            runs["go_trace"], runs["rust_trace"], "layout-final"
        )
        grid_state = common_grid_records(runs["go_trace"], runs["rust_trace"])
        external_routes_exact = stage_records(runs["go_trace"], "external-route") == stage_records(
            runs["rust_trace"], "external-route"
        )
        instrumentation_noninterference = (
            runs["go_trace"]["svg_sha256"] == runs["go_plain"]["svg_sha256"]
            and runs["rust_trace"]["svg_sha256"] == runs["rust_plain"]["svg_sha256"]
            and not runs["go_plain"]["has_trace_when_disabled"]
            and not runs["rust_plain"]["has_trace_when_disabled"]
        )
        release_source_equal = runs["official"]["svg_sha256"] == runs["go_plain"]["svg_sha256"]
        # The small flat witness has one identical grid-scope snapshot. For
        # nested extraction, Go and Rust expose different temporary object
        # sets; common exact records and final state are kept separate.
        grid_stage_exact = grid_state["exact_sets"]
        available_boundaries_exact = (
            final_state_exact if final_state_available else grid_stage_exact
        )
        passed = (
            all_successful
            and (not args.require_final_state or final_state_available)
            and available_boundaries_exact
            and external_routes_exact
            and instrumentation_noninterference
            and release_source_equal
            and grid_labels["exact_common_nodes"]
            and final_labels["exact_common_nodes"]
            and (grid_stage_exact or bool(grid_state["common_exact"]))
        )
        overall_passed = overall_passed and passed
        results.append(
            {
                "fixture": fixture.relative_to(ROOT).as_posix(),
                "fixture_sha256": digest(fixture),
                "passed": passed,
                "release_svg_matches_go_source": release_source_equal,
                "instrumentation_noninterference": instrumentation_noninterference,
                "grid_scope_common_records": grid_state,
                "grid_labels": grid_labels,
                "layout_final_available": final_state_available,
                "layout_final_exact": final_state_exact,
                "layout_final_labels": final_labels,
                "external_routes_exact": external_routes_exact,
                "runs": runs,
            }
        )

    report = {
        "schema_version": 2,
        "kind": "d2-v0.9.0-grid-state-trace-parity",
        "layout": args.layout,
        "seed": args.seed,
        "trace_environment": {
            "inherited": {
                variable: os.environ.get(variable)
                for variable in TRACE_ENVIRONMENT_VARIABLES
            },
            "require_final_state": args.require_final_state,
            "policy": "Both controlled flags are absent in official/plain runs; traced runs enable grid state and enable final state when required, otherwise retaining only an inherited final-state value.",
        },
        "source_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "go_toolchain": "go1.27.0",
        "scope": "bit-exact grid state and route snapshots; extraction-local intermediate sets are reported separately",
        "binaries": {
            name: {"path": str(path), "sha256": digest(path)}
            for name, path in binaries.items()
        },
        "passed": overall_passed,
        "results": results,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": overall_passed, "results": [
        {
            "fixture": result["fixture"],
            "passed": result["passed"],
            "grid_scope_exact": result["grid_scope_common_records"]["exact_sets"],
            "grid_common_records": result["grid_scope_common_records"]["common_exact"],
            "grid_labels_common": result["grid_labels"]["common_exact"],
            "grid_labels_comparable_exact": result["grid_labels"]["exact_common_nodes"],
            "layout_final_available": result["layout_final_available"],
            "layout_final_exact": result["layout_final_exact"],
            "layout_final_labels_exact": result["layout_final_labels"]["exact_common_nodes"],
            "external_routes_exact": result["external_routes_exact"],
        }
        for result in results
    ], "report": str(args.report)}, indent=2))
    return 0 if overall_passed else 1


if __name__ == "__main__":
    sys.exit(main())
