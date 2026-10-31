#!/usr/bin/env python3
"""Run the pinned TALA trace corpus and record a reproducible parity report.

The report deliberately keeps the fixture/seed matrix explicit.  A successful
run means the release Rust binary produced the same public graph and the
diagnostic build produced the same normalized operation trace as the
instrumented v0.9.0 Go oracle for every matrix entry.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

from exact_json import decode_output, difference

from references import ROOT, digest, verify_source


DEFAULT_SEEDS = (1, 2, 3, 5, 42)


def run(command: list[str]) -> str:
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(command)}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result.stdout.strip()


def selected_flavor_route_count(witness: Path, implementation: str, flavor: str) -> int | None:
    """Count routes assigned to a worker, returning None when inventory is unavailable."""
    path = witness / f"{implementation}.routing.json"
    try:
        events = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(events, list):
        return None
    route_count = 0
    for event in events:
        if not isinstance(event, dict):
            return None
        if event.get("event") == "route-flavor" and event.get("flavor") == flavor:
            routes = event.get("routes")
            if not isinstance(routes, list):
                return None
            route_count += len(routes)
    return route_count


def capture_options(args: argparse.Namespace) -> list[str]:
    """Capture only the trace families required by this parity mode."""
    if getattr(args, "swap_ports", False):
        options = []
    elif args.search_flavor:
        # Route-search parity needs the assigned route inventory, search
        # decisions, and optional heap mutations. Capturing placement snapshots,
        # packing, and every RNG draw here creates huge redundant witnesses.
        options = ["--routing"]
    elif getattr(args, "sized_calls", False) or getattr(args, "sized_results", False):
        options = []
    else:
        options = ["--placement", "--packing", "--routing", "--rng"]
    if args.gap:
        options.append("--gap")
    if args.search_flavor:
        options.extend(["--search-flavor", args.search_flavor])
    if args.search_heap:
        options.append("--search-heap")
    if args.search_costs:
        options.append("--search-costs")
    if getattr(args, "sized_calls", False):
        options.append("--sized-calls")
    if getattr(args, "sized_results", False):
        options.append("--sized-results")
    if getattr(args, "swap_ports", False):
        options.append("--swap-ports")
    return options


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", action="append", type=int, dest="seeds")
    parser.add_argument("--fixture", action="append", dest="fixtures")
    parser.add_argument("--gap", action="store_true", help="include gap transaction boundary events")
    parser.add_argument(
        "--search-flavor",
        choices=["ShortestToLongest", "LongestToShortest", "Default", "TopDownLeftRight"],
        help="qualify ordered route-search events for one worker instead of the full emitted trace",
    )
    parser.add_argument(
        "--fixtures-dir",
        type=Path,
        default=ROOT / "tests/d2",
        help="directory containing the requested .d2 fixtures",
    )
    parser.add_argument(
        "--report",
        type=Path,
        default=ROOT / "oracle-cache/reports/layout-multiseed-parity.json",
    )
    parser.add_argument("--search-event", choices=["all", "route-search-pop", "route-search-neighbor", "route-search-cost"], default="all")
    parser.add_argument("--search-heap", action="store_true",
                        help="also compare ordered heap push/pop/decrease operations")
    parser.add_argument("--search-costs", action="store_true",
                        help="also compare route cost-component breakdowns in search order")
    parser.add_argument("--sized-calls", action="store_true",
                        help="compare sized-optimizer before/after shuffle node order")
    parser.add_argument("--sized-results", action="store_true",
                        help="compare sized-optimizer move decisions and full position snapshots")
    parser.add_argument("--swap-ports", action="store_true",
                        help="compare ordered port grouping, candidate, decision, refinement, and rollback events")
    args = parser.parse_args()
    if args.search_heap and args.search_flavor is None:
        parser.error("--search-heap requires --search-flavor")
    if args.search_costs and args.search_flavor is None:
        parser.error("--search-costs requires --search-flavor")
    if (args.sized_calls or args.sized_results) and args.search_flavor is not None:
        parser.error("sized optimizer trace modes cannot be combined with --search-flavor")
    if args.swap_ports and (
        args.gap or args.search_flavor is not None or args.search_heap or args.search_costs
        or args.sized_calls or args.sized_results
    ):
        parser.error("--swap-ports is a focused trace mode and cannot be combined with other trace modes")
    verify_source()

    seeds = tuple(args.seeds) if args.seeds else DEFAULT_SEEDS
    fixture_names = tuple(args.fixtures) if args.fixtures else tuple(
        path.name for path in sorted(args.fixtures_dir.glob("*.d2"))
    )
    rows: list[dict[str, object]] = []
    for seed in seeds:
        for fixture in fixture_names:
            try:
                output = run(
                    [
                        "uv",
                        "run",
                        "--no-project",
                        "python",
                        "scripts/capture-layout-trace.py",
                        fixture,
                        "--fixtures-dir",
                        str(args.fixtures_dir),
                        "--seed",
                        str(seed),
                        *capture_options(args),
                    ]
                )
            except RuntimeError as error:
                # Some corpus files intentionally exercise the source
                # renderer's pre-measure rejection boundary. They cannot
                # produce a TALA graph or operation trace. Keep them visible
                # in the matrix and require the dedicated source-error gate
                # to qualify their diagnostics rather than treating them as
                # a failed layout comparison.
                if "d2-oracle', 'measure'" not in str(error):
                    rows.append({"fixture": fixture, "seed": seed,
                                 "status": "capture_failed", "reason": str(error)})
                    print(f"{fixture} seed {seed}: CAPTURE FAILED: {error}", flush=True)
                    continue
                row = {
                    "fixture": fixture,
                    "seed": seed,
                    "status": "inapplicable",
                    "reason": "v0.9.0 oracle rejected the source before a measured graph existed",
                }
                rows.append(row)
                print(f"{fixture} seed {seed}: INAPPLICABLE (pre-measure rejection)")
                continue
            witness = Path(output)
            comparison_result = subprocess.run(
                [
                    "uv",
                    "run",
                    "--no-project",
                    "python",
                    "scripts/compare-layout-trace.py",
                    str(witness / "go.stderr"),
                    str(witness / "rust.stderr"),
                    *( [] if args.swap_ports else ["--normalize-scoped-ids", "--project-owner-snapshots"] ),
                    *(
                        (["--event-prefix", "route-search-"] if args.search_event == "all" else ["--event", args.search_event])
                        if args.search_flavor
                        else ["--event-prefix", "sized-"] if args.sized_calls and args.sized_results
                        else ["--event", "sized-optimize"] if args.sized_calls
                        else ["--event", "sized-node-result"] if args.sized_results
                        else ["--event-prefix", "swap-port-"] if args.swap_ports
                        else []
                    ),
                ], cwd=ROOT, text=True, capture_output=True, check=False,
            )
            comparison = comparison_result.stdout.strip()
            insufficient = "INSUFFICIENT EVIDENCE" in comparison_result.stderr
            search_trace_matched = comparison_result.returncode == 0
            heap_trace_matched = True
            heap_comparison = None
            heap_result = None
            if args.search_heap:
                heap_result = subprocess.run(
                    [
                        "uv", "run", "--no-project", "python", "scripts/compare-layout-trace.py",
                        str(witness / "go.stderr"), str(witness / "rust.stderr"),
                        "--event", "route-search-heap",
                    ], cwd=ROOT, text=True, capture_output=True, check=False,
                )
                heap_comparison = heap_result.stdout.strip()
                insufficient = insufficient or "INSUFFICIENT EVIDENCE" in heap_result.stderr
                heap_trace_matched = heap_result.returncode == 0
            trace_matched = search_trace_matched and heap_trace_matched
            public_difference = difference(
                decode_output((witness / "go.stdout.json").read_bytes()),
                decode_output((witness / "rust.stdout.json").read_bytes()),
            )
            manifest = json.loads((witness / "manifest.json").read_text())
            ordinary_identical = all(
                details["public_bytes_identical"]
                for details in manifest["oracles"].values()
            )
            missing_gap_events = args.gap and any(
                details.get("gap_events", 0) == 0 for details in manifest["oracles"].values()
            )
            missing_cost_breakdown_events = args.search_costs and any(
                details.get("cost_breakdown_events", 0) == 0
                for details in manifest["oracles"].values()
            )
            missing_sized_call_events = args.sized_calls and any(
                details.get("sized_call_events", 0) == 0
                for details in manifest["oracles"].values()
            )
            missing_sized_result_events = args.sized_results and any(
                details.get("sized_result_events", 0) == 0
                for details in manifest["oracles"].values()
            )
            missing_swap_port_events = args.swap_ports and any(
                details.get("swap_port_events", 0) == 0
                for details in manifest["oracles"].values()
            )
            insufficient = (
                insufficient or missing_gap_events or missing_cost_breakdown_events
                or missing_sized_call_events or missing_sized_result_events
                or missing_swap_port_events
            )
            matched = trace_matched and not insufficient and public_difference is None and ordinary_identical
            no_search_edges = (
                args.search_flavor is not None
                and insufficient
                and manifest.get("measured_edge_count") == 0
                and all(details.get("search_events") == 0 for details in manifest["oracles"].values())
                and (not args.search_heap or all(details.get("heap_events") == 0 for details in manifest["oracles"].values()))
                and (not args.search_costs or all(details.get("cost_breakdown_events") == 0 for details in manifest["oracles"].values()))
                and public_difference is None and ordinary_identical
            )
            flavor_route_counts = None
            no_selected_flavor_work = False
            flavor_route_inventory_diverges = False
            if args.search_flavor is not None:
                flavor_route_counts = {
                    name: selected_flavor_route_count(witness, name, args.search_flavor)
                    for name in ("go", "rust")
                }
                go_routes = flavor_route_counts["go"]
                rust_routes = flavor_route_counts["rust"]
                flavor_route_inventory_diverges = (
                    go_routes is not None and rust_routes is not None and go_routes != rust_routes
                )
                no_selected_flavor_work = (
                    insufficient
                    and all(details.get("search_events") == 0 for details in manifest["oracles"].values())
                    and (not args.search_heap or all(details.get("heap_events") == 0 for details in manifest["oracles"].values()))
                    and (not args.search_costs or all(details.get("cost_breakdown_events") == 0 for details in manifest["oracles"].values()))
                    and go_routes == 0
                    and rust_routes == 0
                    and public_difference is None
                    and ordinary_identical
                )
            inapplicable = no_search_edges or no_selected_flavor_work
            status = (
                "inapplicable" if inapplicable
                else "matched" if matched
                else "divergent" if flavor_route_inventory_diverges
                else "insufficient_evidence" if insufficient and public_difference is None and ordinary_identical
                else "divergent"
            )
            row = {
                "fixture": fixture,
                "seed": seed,
                "public_graph_exact": public_difference is None,
                "public_graph_difference": public_difference,
                "status": status,
                "trace": comparison,
                "heap_trace": heap_comparison,
                "comparison_exit": comparison_result.returncode,
                "heap_comparison_exit": heap_result.returncode if heap_result else None,
                "comparison_stderr": comparison_result.stderr.strip(),
                "public_bytes_identical": all(
                    details["public_bytes_identical"]
                    for details in manifest["oracles"].values()
                ),
                "raw_rng_init_events": {
                    name: details.get("rng_init_events", 0)
                    for name, details in manifest["oracles"].items()
                },
                "cost_breakdown_events": {
                    name: details.get("cost_breakdown_events", 0)
                    for name, details in manifest["oracles"].items()
                },
                "sized_call_events": {
                    name: details.get("sized_call_events", 0)
                    for name, details in manifest["oracles"].items()
                },
                "sized_result_events": {
                    name: details.get("sized_result_events", 0)
                    for name, details in manifest["oracles"].items()
                },
                "swap_port_events": {
                    name: details.get("swap_port_events", 0)
                    for name, details in manifest["oracles"].items()
                },
                "swap_port_event_counts": {
                    name: details.get("swap_port_event_counts", {})
                    for name, details in manifest["oracles"].items()
                },
                "event_count": int(comparison.split()[1]) if search_trace_matched else None,
                "heap_event_count": int(heap_comparison.split()[1]) if heap_result and heap_result.returncode == 0 else None,
                "witness": str(witness.relative_to(ROOT)),
            }
            if args.gap:
                row["gap_coverage"] = {
                    name: {key: details.get(key, {}) for key in ("gap_event_counts", "gap_decision_counts", "gap_inner_decision_counts")}
                    for name, details in manifest["oracles"].items()
                }
            if missing_gap_events:
                row["reason"] = "requested gap trace emitted no gap events in at least one implementation"
            if missing_cost_breakdown_events:
                row["reason"] = "requested route-cost breakdown emitted no events in at least one implementation"
            if missing_sized_call_events:
                row["reason"] = "requested sized-optimizer trace emitted no events in at least one implementation"
            if missing_sized_result_events:
                row["reason"] = "requested sized-optimizer move trace emitted no events in at least one implementation"
            if missing_swap_port_events:
                row["reason"] = "requested port-swap trace emitted no events in at least one implementation"
            if no_search_edges:
                row["reason"] = "measured graph contains no edges; routing search is inapplicable"
                row["measured_edge_count"] = 0
                row["measured_sha256"] = manifest.get("measured_sha256")
            elif no_selected_flavor_work:
                row["reason"] = f"no routes were assigned to the {args.search_flavor} worker in either implementation"
            elif flavor_route_inventory_diverges:
                row["reason"] = f"{args.search_flavor} worker route inventory differs between implementations"
            elif heap_result is not None and heap_result.returncode != 0 and not insufficient:
                row["reason"] = f"heap trace differs: {heap_comparison or heap_result.stderr.strip()}"
            if flavor_route_counts is not None:
                row["selected_flavor_route_counts"] = flavor_route_counts
            rows.append(row)
            print(f"{fixture} seed {seed}: {row.get('reason') or comparison or comparison_result.stderr.strip()}", flush=True)
            if public_difference is not None:
                print(f"  PUBLIC GRAPH DIVERGENCE: {public_difference}")

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-tala-multiseed-trace-parity",
        "authority": "pristine D2 v0.9.0 with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "seeds": list(seeds),
        "search_flavor": args.search_flavor,
        "search_heap": args.search_heap,
        "search_costs": args.search_costs,
        "sized_calls": args.sized_calls,
        "sized_results": args.sized_results,
        "swap_ports": args.swap_ports,
        "gap_trace": args.gap,
        "event_scope": (
            args.search_event if args.search_flavor
            else "sized-*" if args.sized_calls and args.sized_results
            else "sized-optimize" if args.sized_calls
            else "sized-node-result" if args.sized_results
            else "swap-port-*" if args.swap_ports
            else "all normalized emitted events"
        ),
        "trace_normalizations": [
            "RNG constructor events, stream scopes, and every draw remain in ordered comparison",
            "for the six upstream orthogonal callbacks that serialize root.Graph, compare every shared Go/Rust owner-snapshot record, verify all initialized optimizer nodes are present on both sides, and report context-only nodes",
            "stage and placement snapshot nodes are compared by stable node ID and edges by source ordinal and endpoint IDs, excluding adapter-local edge IDs; candidate and decision order remains exact",
            "when requested, route-search cost-component breakdowns remain interleaved in exact event order",
            "sized optimizer order events use Go DebugID semantics: external D2 ID when present, numeric TALA ID for synthetic nodes",
            "sized optimizer move results compare per-node decisions and graph positions in exact event order",
            "port-swap events compare stable edge/node IDs, raw source iteration order, and exact IEEE-754 route-coordinate bits without reordering",
        ],
        "raw_rng_init_event_totals": {
            name: sum(row.get("raw_rng_init_events", {}).get(name, 0) for row in rows)
            for name in ("go", "rust")
        },
        "fixtures": list(fixture_names),
        "matched": sum(row.get("status") == "matched" for row in rows),
        "inapplicable": sum(row.get("status") == "inapplicable" for row in rows),
        "divergent": sum(row.get("status") == "divergent" for row in rows),
        "insufficient_evidence": sum(row.get("status") == "insufficient_evidence" for row in rows),
        "capture_failed": sum(row.get("status") == "capture_failed" for row in rows),
        "total": len(seeds) * len(fixture_names),
        "release_binary_sha256": digest(ROOT / "target/release/examples/tala-oracle"),
        "diagnostic_binary_sha256": digest(ROOT / "target/diagnostic/examples/tala-oracle"),
        "results": rows,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    if report["matched"] + report["inapplicable"] != report["total"]:
        raise SystemExit(
            "parity matrix has nonpassing rows: "
            f"{report['divergent']} divergent, "
            f"{report['insufficient_evidence']} insufficient-evidence, "
            f"{report['capture_failed']} capture-failed"
        )
    print(
        f"MATCH {report['matched']}/{report['total']} "
        f"({report['inapplicable']} inapplicable); wrote {args.report}"
    )


if __name__ == "__main__":
    main()
