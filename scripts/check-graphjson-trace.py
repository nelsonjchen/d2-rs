#!/usr/bin/env python3
"""Compare engine traces for the pinned v0.9.0 GraphJSON corpus.

The Go oracle and Rust adapter store snapshot arrays in different arena orders.
Normalization keeps event order and every operation value, but orders snapshot
nodes/edges by their durable IDs and removes the serialization-only edge index.
With --include-rng, it also compares each source-wrapped Go math/rand Int63
operation in the same stream order. Adapter-only SelectedResult snapshots are
excluded because this corpus calls Go's internal engine directly; the D2
adapter boundary is compared by check-emitted-trace-parity.py. Event order is
never sorted. Detailed mode compares generic whole-run JSONL hooks; focused
source probes have additional target-specific switches and are not implied.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any, Mapping

from references import ROOT, REFERENCE, digest, verify_oracle, verify_source, read_report

DEFAULT_SEEDS = (0, 2, 7, 42, 123456789)
PRECEDING_CONTEXT_EVENTS = 3
FULL_TRACE_EVENTS = {
    "stage",
    "placement",
    "placement-candidates",
    "placement-score",
    "packing",
    "ovg-topology",
    "route-flavor",
    "tree-trial",
    "tree-selected",
    "source-orientation",
    "gap-connectivity",
    "gap-inner",
    "gap-pass",
    "gap-proposal",
    "gap-trial",
    "rng-init",
    "rng-scope",
    "rng",
}


def events(
    stderr: bytes,
    include_rng: bool = False,
    include_placement: bool = False,
    include_full_trace: bool = False,
) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    # The source trace wraps several layout sources, but only node-placement
    # emits layout draws in this direct engine corpus. Other constructors are
    # independently initialized fields/helpers whose construction order is
    # not an algorithm operation and differs between the Go and Rust structs.
    traced_rng_streams = {"node-placement"}
    for line in stderr.decode(errors="replace").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(value, dict):
            continue
        event_name = value.get("event")
        if include_full_trace and event_name not in FULL_TRACE_EVENTS:
            continue
        if event_name == "rng-init":
            if include_full_trace or (include_rng and value.get("stream") in traced_rng_streams):
                result.append(value)
            continue
        if event_name == "rng-scope":
            if include_rng or include_full_trace:
                result.append(value)
            continue
        if event_name == "rng":
            if include_rng or include_full_trace:
                result.append(value)
            continue
        if event_name == "placement":
            if include_placement or include_full_trace:
                for node in value.get("nodes", []):
                    stable_id = node.pop("stable_id", None)
                    if stable_id is None:
                        raise RuntimeError(
                            "placement trace record is missing its stable numeric node ID"
                        )
                    node["debug_id"] = node["id"]
                    node["id"] = str(stable_id)
                result.append(value)
            continue
        if event_name == "stage":
            # Compound snapshots are internal Rust bookkeeping, not upstream
            # stage boundaries; the direct engine does not emit SelectedResult.
            stage = value.get("stage", "")
            if stage.startswith("Compound/") or stage == "SelectedResult":
                continue
            value.pop("seed", None)
            value["nodes"] = sorted(value.get("nodes", []), key=lambda node: node.get("id", ""))
            value["edges"] = sorted(
                [
                    {key: item for key, item in edge.items() if key != "index"}
                    for edge in value.get("edges", [])
                ],
                key=lambda edge: edge.get("id", ""),
            )
            result.append(value)
            continue
        if include_full_trace and event_name in FULL_TRACE_EVENTS:
            # Operation payload order, including candidates, scores, packing,
            # routing and gap decisions, is algorithmically meaningful.
            result.append(value)
            continue
    return result


def event_counts(trace: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for event in trace:
        name = str(event["event"])
        counts[name] = counts.get(name, 0) + 1
    return dict(sorted(counts.items()))


def sum_event_counts(rows: list[dict[str, Any]], field: str) -> dict[str, int]:
    totals: dict[str, int] = {}
    for row in rows:
        for name, count in row.get(field, {}).items():
            totals[name] = totals.get(name, 0) + count
    return dict(sorted(totals.items()))


def trace_sha256(trace: list[dict[str, Any]]) -> str:
    encoded = json.dumps(trace, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def first_difference(left: Any, right: Any, path: str = "$") -> str | None:
    if type(left) is not type(right):
        return f"{path}: {left!r} != {right!r}"
    if isinstance(left, dict):
        keys = list(dict.fromkeys([*left, *right]))
        for key in keys:
            if key not in left or key not in right:
                return f"{path}.{key}: {left.get(key)!r} != {right.get(key)!r}"
            difference = first_difference(left[key], right[key], f"{path}.{key}")
            if difference:
                return difference
        return None
    if isinstance(left, list):
        for index, (left_item, right_item) in enumerate(zip(left, right)):
            difference = first_difference(left_item, right_item, f"{path}[{index}]")
            if difference:
                return difference
        if len(left) != len(right):
            return f"{path}.length: {len(left)} != {len(right)}"
        return None
    if left != right:
        return f"{path}: {left!r} != {right!r}"
    return None


def first_event_difference(left: list[dict[str, Any]], right: list[dict[str, Any]]) -> dict[str, Any] | None:
    """Keep the first differing events and their immediately preceding trace."""
    for index, (left_event, right_event) in enumerate(zip(left, right)):
        difference = first_difference(left_event, right_event)
        if difference is not None:
            result: dict[str, Any] = {
                "event_index": index,
                "difference": difference,
                "go_event": left_event,
                "rust_event": right_event,
                "preceding_events": [
                    {"event_index": previous_index, "event": left[previous_index]}
                    for previous_index in range(
                        max(0, index - PRECEDING_CONTEXT_EVENTS), index
                    )
                ],
            }
            if left_event.get("event") == "ovg-topology":
                point_prefix = "$.points["
                point_index = None
                if difference.startswith(point_prefix):
                    closing = difference.find("]", len(point_prefix))
                    if closing >= 0:
                        try:
                            point_index = int(difference[len(point_prefix):closing])
                        except ValueError:
                            pass

                def ovg_context(event: dict[str, Any]) -> dict[str, Any]:
                    points = event.get("points", [])
                    start = max(0, (point_index or 0) - 2)
                    end = min(len(points), (point_index or 0) + 3)
                    return {
                        "event": event.get("event"),
                        "scope": event.get("scope"),
                        "box_ids": [box.get("id") for box in event.get("boxes", [])],
                        "point_count": len(points),
                        "ordered_points_sha256": trace_sha256(points),
                        "point_index": point_index,
                        "point_window": [
                            {"index": point, **points[point]}
                            for point in range(start, end)
                        ],
                    }

                result["go_event"] = ovg_context(left_event)
                result["rust_event"] = ovg_context(right_event)
            node_path = "$.nodes["
            if left_event.get("event") == "placement" and difference.startswith(node_path):
                closing = difference.find("]", len(node_path))
                if closing >= 0:
                    try:
                        node_index = int(difference[len(node_path):closing])
                    except ValueError:
                        node_index = -1
                    current_nodes = left_event.get("nodes", [])
                    if 0 <= node_index < len(current_nodes):
                        node_id = str(current_nodes[node_index].get("id", ""))
                        if node_id:
                            result["node_id"] = node_id
                            previous_node_snapshots = []
                            for previous_index in range(index - 1, -1, -1):
                                previous_left = left[previous_index]
                                previous_right = right[previous_index]
                                if previous_left.get("event") != "placement":
                                    continue
                                go_nodes = [
                                    node for node in previous_left.get("nodes", [])
                                    if str(node.get("id", "")) == node_id
                                ]
                                rust_nodes = [
                                    node for node in previous_right.get("nodes", [])
                                    if str(node.get("id", "")) == node_id
                                ]
                                if go_nodes or rust_nodes:
                                    previous_node_snapshots.append({
                                        "event_index": previous_index,
                                        "phase": previous_left.get("phase"),
                                        "root": previous_left.get("root"),
                                        "go_nodes": go_nodes,
                                        "rust_nodes": rust_nodes,
                                    })
                                    if len(previous_node_snapshots) == PRECEDING_CONTEXT_EVENTS:
                                        break
                            result["previous_node_snapshots"] = previous_node_snapshots
            return result
    if len(left) != len(right):
        index = min(len(left), len(right))
        return {
            "event_index": index,
            "difference": f"$.length: {len(left)} != {len(right)}",
            "go_event": left[index] if index < len(left) else None,
            "rust_event": right[index] if index < len(right) else None,
            "preceding_events": [
                {"event_index": previous_index, "event": left[previous_index]}
                for previous_index in range(
                    max(0, index - PRECEDING_CONTEXT_EVENTS), index
                )
            ],
        }
    return None


def run(
    command: list[str],
    data: bytes,
    env_key: str,
    timeout: int,
    include_rng: bool = False,
    include_placement: bool = False,
    include_full_trace: bool = False,
) -> tuple[int, bytes, bytes]:
    import os

    environment = trace_environment(
        os.environ,
        env_key,
        include_rng=include_rng,
        include_placement=include_placement,
        include_full_trace=include_full_trace,
    )
    completed = subprocess.run(command, input=data, capture_output=True, timeout=timeout, env=environment)
    return completed.returncode, completed.stdout, completed.stderr


def trace_environment(
    source: Mapping[str, str],
    env_key: str,
    *,
    include_rng: bool = False,
    include_placement: bool = False,
    include_full_trace: bool = False,
) -> dict[str, str]:
    """Set equivalent trace switches without suppressing requested operations."""
    environment = {
        key: value
        for key, value in source.items()
        if not key.startswith(("TALA_", "WEFTAN_"))
    }
    environment[env_key] = "1"
    if include_rng or include_full_trace:
        environment[env_key.replace("_TRACE_JSONL", "_TRACE_RNG_JSONL")] = "1"
    if include_placement or include_full_trace:
        prefix = env_key.removesuffix("_TRACE_JSONL")
        environment[f"{prefix}_TRACE_PLACEMENT"] = "1"
        # Candidate/score hooks are guarded by this switch in the Go oracle.
        # Keep it only for snapshot-only comparisons; full traces need the raw
        # placement operations on both implementations.
        if not include_full_trace:
            environment[f"{prefix}_TRACE_PLACEMENT_SNAPSHOTS_ONLY"] = "1"
    if include_full_trace:
        prefix = env_key.removesuffix("_TRACE_JSONL")
        for suffix in (
            "PACKING",
            "GAP",
            "ROUTING",
            "OVG_JSONL",
            "TREE_PLACEMENT",
            "TREE_TRIALS",
        ):
            environment[f"{prefix}_TRACE_{suffix}"] = "1"
    # The normalized trace represents Go's pointer-visible geometry.  Rust's
    # stable arena retains hidden aggregate members, so diagnostic snapshots
    # must project those members through their live vessel before comparison.
    if env_key == "WEFTAN_TRACE_JSONL":
        environment["WEFTAN_TRACE_PROJECTED_POSITIONS"] = "1"
    return environment


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", action="append", type=int, dest="seeds")
    parser.add_argument(
        "--include-rng",
        action="store_true",
        help="compare source-wrapped RNG Int63 events interleaved with stage snapshots",
    )
    parser.add_argument(
        "--include-placement",
        action="store_true",
        help="compare ordered recursive placement snapshots along with stages and RNG",
    )
    parser.add_argument(
        "--include-full-trace",
        action="store_true",
        help="compare generic full-run JSONL families, including candidate/score, tree, OVG, routing, gap, packing, and RNG events",
    )
    parser.add_argument(
        "--fixture",
        action="append",
        default=[],
        help="limit to exact upstream-relative fixture paths; may be repeated",
    )
    parser.add_argument(
        "--all-graphjson",
        action="store_true",
        help="include every pinned layout_graph_json input/regression fixture, not only layout/OVG graph.input.json",
    )
    parser.add_argument(
        "--include-expected-output",
        action="store_true",
        help="with --all-graphjson, also trace post-layout GraphJSON snapshots as re-layout inputs",
    )
    parser.add_argument("--rust-binary", type=Path, help="diagnostic graphjson-layout binary to compare")
    parser.add_argument("--timeout", type=int, default=120, help="per-process timeout in seconds")
    parser.add_argument("--report", type=Path, default=ROOT / "oracle-cache/reports/graphjson-trace-parity.json")
    args = parser.parse_args()
    if args.include_expected_output and not args.all_graphjson:
        parser.error("--include-expected-output requires --all-graphjson")
    source = verify_source()
    seeds = tuple(args.seeds) if args.seeds else DEFAULT_SEEDS
    go = REFERENCE / "bin/tala-engine-oracle-trace"
    go_provenance = verify_oracle(go, trace=True)
    rust_candidates = (
        ROOT / "target/diagnostic/examples/graphjson-layout",
        ROOT / "target/debug/examples/graphjson-layout",
    )
    rust = args.rust_binary or next((path for path in rust_candidates if path.is_file()), None)
    if rust is None:
        raise SystemExit("missing diagnostic graphjson binary; build graphjson-layout with diagnostic-traces")
    if not rust.is_file():
        raise SystemExit(f"diagnostic graphjson binary does not exist: {rust}")
    if args.all_graphjson:
        inventory = read_report("engine-fixture-inventory.json", "scripts/audit-engine-fixtures.py")
        fixture_rows = [
            row for row in inventory["fixtures"]
            if row.get("format") == "layout_graph_json"
            and (args.include_expected_output or row.get("role") != "expected_output")
        ]
        fixtures = []
        for row in fixture_rows:
            fixture = source / row["upstream_path"]
            if hashlib.sha256(fixture.read_bytes()).hexdigest() != row["sha256"]:
                raise RuntimeError(f"fixture checksum mismatch: {fixture}")
            fixtures.append(fixture)
        fixture_set = (
            "all pinned layout_graph_json inputs, regressions, and expected outputs"
            if args.include_expected_output
            else "all pinned layout_graph_json inputs and regressions"
        )
    else:
        fixtures = sorted([
            *source.glob("d2layouts/d2talalayout/internal/testdata/layout/*/graph.input.json"),
            *source.glob("d2layouts/d2talalayout/internal/testdata/ovg/*/graph.input.json"),
        ])
        fixture_set = "layout and OVG graph.input.json fixtures"
    if args.fixture:
        by_relative_path = {
            str(fixture.relative_to(source)): fixture for fixture in fixtures
        }
        missing = sorted(set(args.fixture) - by_relative_path.keys())
        if missing:
            raise SystemExit(f"unknown fixture path(s): {', '.join(missing)}")
        fixtures = [by_relative_path[path] for path in args.fixture]
        fixture_set += "; selected fixtures"
    rows: list[dict[str, Any]] = []
    for seed in seeds:
        for fixture in fixtures:
            data = fixture.read_bytes()
            go_exit, _, go_stderr = run(
                [str(go), "-seed", str(seed)],
                data,
                "TALA_TRACE_JSONL",
                args.timeout,
                args.include_rng,
                args.include_placement,
                args.include_full_trace,
            )
            rust_exit, _, rust_stderr = run(
                [str(rust), str(seed)],
                data,
                "WEFTAN_TRACE_JSONL",
                args.timeout,
                args.include_rng,
                args.include_placement,
                args.include_full_trace,
            )
            go_events = events(
                go_stderr, args.include_rng, args.include_placement, args.include_full_trace
            )
            rust_events = events(
                rust_stderr, args.include_rng, args.include_placement, args.include_full_trace
            )
            difference = None if go_exit == rust_exit == 0 else f"exit codes: {go_exit} != {rust_exit}"
            event_difference = None
            if difference is None:
                event_difference = first_event_difference(go_events, rust_events)
                difference = None if event_difference is None else event_difference["difference"]
            passed = difference is None
            row = {
                "fixture": str(fixture.relative_to(source)),
                "seed": seed,
                "go_exit": go_exit,
                "rust_exit": rust_exit,
                "go_events": len(go_events),
                "rust_events": len(rust_events),
                "go_stage_events": sum(event.get("event") == "stage" for event in go_events),
                "rust_stage_events": sum(event.get("event") == "stage" for event in rust_events),
                "go_placement_events": sum(event.get("event") == "placement" for event in go_events),
                "rust_placement_events": sum(event.get("event") == "placement" for event in rust_events),
                "go_rng_events": sum(event.get("event") == "rng" for event in go_events),
                "rust_rng_events": sum(event.get("event") == "rng" for event in rust_events),
                "go_rng_init_events": sum(event.get("event") == "rng-init" for event in go_events),
                "rust_rng_init_events": sum(event.get("event") == "rng-init" for event in rust_events),
                "go_rng_scope_events": sum(event.get("event") == "rng-scope" for event in go_events),
                "rust_rng_scope_events": sum(event.get("event") == "rng-scope" for event in rust_events),
                "go_trace_sha256": trace_sha256(go_events),
                "rust_trace_sha256": trace_sha256(rust_events),
                "passed": passed,
            }
            if args.include_full_trace:
                row["go_event_counts"] = event_counts(go_events)
                row["rust_event_counts"] = event_counts(rust_events)
            if difference:
                row["first_difference"] = difference
                if event_difference is not None:
                    row["first_difference_context"] = event_difference
            rows.append(row)
            print(f"{row['fixture']} seed {seed}: {'PASS' if passed else 'FAIL'}", flush=True)
    if args.include_full_trace:
        trace_event_families = sorted(FULL_TRACE_EVENTS)
    elif args.include_placement:
        trace_event_families = (
            ["stage", "placement", "rng-init", "rng-scope", "rng"]
            if args.include_rng
            else ["stage", "placement"]
        )
    else:
        trace_event_families = (
            ["stage", "rng-init", "rng-scope", "rng"] if args.include_rng else ["stage"]
        )
    report = {
        "schema_version": 1,
        "kind": (
            "d2-v0.9.0-tala-graphjson-generic-full-run-trace-parity"
            if args.include_full_trace
            else "d2-v0.9.0-tala-graphjson-stage-and-rng-trace-parity"
            if args.include_rng
            else "d2-v0.9.0-tala-graphjson-normalized-trace-parity"
        ),
        "authority": "pristine D2 v0.9.0 with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "fixture_set": fixture_set,
        "seeds": list(seeds),
        "timeout_seconds": args.timeout,
        "fixtures": len(fixtures),
        "matched": sum(row["passed"] for row in rows),
        "total": len(rows),
        "event_families": trace_event_families,
        "normalization": [
            "stable node/edge ID ordering",
            "preserve placement-snapshot owner and node order exactly",
            "drop edge index",
            "drop seed",
            "drop additive Compound/*",
            "exclude adapter-level SelectedResult outside the direct engine boundary",
            "preserve RNG event stream, operation, result, draw count, and interleaving exactly",
            "compare node-placement RNG initialization, seed, and per-stream instance IDs",
            "preserve recursive placement snapshot phases and source traversal order",
            "preserve detailed operation-family event order and payload values",
        ],
        "go_oracle_sha256": digest(go),
        "rust_binary": str(rust.resolve()),
        "rust_binary_sha256": digest(rust),
        "trace_patch_sha256": go_provenance["trace_patch_sha256"],
        "go_event_counts": {
            "stage": sum(row["go_stage_events"] for row in rows),
            "placement": sum(row["go_placement_events"] for row in rows),
            "rng-init": sum(row["go_rng_init_events"] for row in rows),
            "rng-scope": sum(row["go_rng_scope_events"] for row in rows),
            "rng": sum(row["go_rng_events"] for row in rows),
        },
        "rust_event_counts": {
            "stage": sum(row["rust_stage_events"] for row in rows),
            "placement": sum(row["rust_placement_events"] for row in rows),
            "rng-init": sum(row["rust_rng_init_events"] for row in rows),
            "rng-scope": sum(row["rust_rng_scope_events"] for row in rows),
            "rng": sum(row["rust_rng_events"] for row in rows),
        },
        **(
            {
                "go_full_event_counts": sum_event_counts(rows, "go_event_counts"),
                "rust_full_event_counts": sum_event_counts(rows, "rust_event_counts"),
            }
            if args.include_full_trace
            else {}
        ),
        "results": rows,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(f"MATCH {report['matched']}/{report['total']}; wrote {args.report}")
    return int(report["matched"] != report["total"])


if __name__ == "__main__":
    raise SystemExit(main())
