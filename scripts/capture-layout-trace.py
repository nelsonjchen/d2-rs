#!/usr/bin/env python3
"""Capture unmodified stage JSONL from both diagnostic oracles for a witness.

This is a capture tool, not a parity verdict: inherited Rust events project
cluster state and have additional/missing stages. Raw event order is retained.
"""
import argparse
import json
import os
import re
import subprocess
import tomllib
from pathlib import Path
from references import ROOT, REFERENCE, verify_source, verify_oracle, digest


def parse_connected_go_trace(line):
    match = re.fullmatch(
        r"CONNECTED_STEP_GO start=(\d+):.*? current=(\d+):.*? excluded=(\[.*?\]) enqueued=(\[.*?\]) clusters=(\[.*?\]) containers=(\[.*?\])",
        line,
    )
    if match is None:
        raise ValueError(f"unparseable Go connected-order trace: {line}")
    start, current, excluded, enqueued, clusters, containers = match.groups()
    return {
        "event": "connected-step",
        "start": int(start),
        "current": int(current),
        "excluded": [int(value) for value in re.findall(r"(\d+):", excluded)],
        "enqueued": [int(value) for value in re.findall(r"(\d+):", enqueued)],
        "clusters": [int(value) for value in re.findall(r"-?\d+", clusters)],
        "containers": [int(value) for value in re.findall(r"-?\d+", containers)],
    }


def parse_connected_rust_trace(line):
    match = re.fullmatch(
        r"CONNECTED_STEP_RUST start=(\d+) current=(\d+) excluded=(\[.*?\]) enqueued=(\[.*?\]) clusters=(\[.*?\]) containers=(\[.*?\])",
        line,
    )
    if match is None:
        raise ValueError(f"unparseable Rust connected-order trace: {line}")
    start, current, excluded, enqueued, clusters, containers = match.groups()
    return {
        "event": "connected-step",
        "start": int(start),
        "current": int(current),
        "excluded": [int(value) for value in re.findall(r"-?\d+", excluded)],
        "enqueued": [int(value) for value in re.findall(r"-?\d+", enqueued)],
        "clusters": [int(value) for value in re.findall(r"-?\d+", clusters)],
        "containers": [int(value) for value in re.findall(r"-?\d+", containers)],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture")
    parser.add_argument("--fixtures-dir", type=Path, default=ROOT / "tests/d2")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--placement", action="store_true",
                        help="also capture upstream placement phase snapshots in raw stderr")
    parser.add_argument("--graph-owners", action="store_true",
                        help="capture Graph.Nodes ownership and direct Containers children")
    parser.add_argument("--packing", action="store_true",
                        help="also capture upstream packing groups and choices")
    parser.add_argument("--gap", action="store_true", help="capture ordered gap transaction boundaries")
    parser.add_argument("--routing", action="store_true", help="capture ordered routing flavor proposals")
    parser.add_argument("--rng", action="store_true",
                        help="capture exact primitive RNG draws and draw counts")
    parser.add_argument("--search-flavor", choices=["ShortestToLongest", "LongestToShortest", "Default", "TopDownLeftRight"],
                        help="capture heap pops for one sequential routing worker")
    parser.add_argument("--search-heap", action="store_true",
                        help="capture diagnostic heap push/pop/decrease events")
    parser.add_argument("--search-costs", action="store_true",
                        help="capture cumulative routing cost checkpoints")
    parser.add_argument("--ovg", action="store_true", help="capture raw construction topology for localization")
    parser.add_argument("--search-filters", action="store_true", help="capture Rust filter inputs for localization")
    parser.add_argument("--sized-calls", action="store_true",
                        help="capture the ordered before/after shuffle node lists for the sized optimizer")
    parser.add_argument("--sized-results", action="store_true",
                        help="capture sized-optimizer move decisions and full node-position snapshots")
    parser.add_argument("--connected-order", metavar="NODE_ID", help="capture connected-node BFS and actual cluster/container traversal order; use 'all' or a TALA node ID")
    parser.add_argument("--swap-ports", action="store_true",
                        help="capture ordered port grouping, candidates, decisions, refinements, and rollbacks")
    args = parser.parse_args()
    verify_source()
    oracle = REFERENCE / "bin/d2-oracle"
    oracle_provenance = verify_oracle(oracle)
    trace_provenance = verify_oracle(REFERENCE / "bin/d2-oracle-trace", trace=True)
    # The diagnostic profile is optimized and symbolized (see the workspace
    # Cargo.toml). Keep the older target/oracle/debug location as a fallback
    # for captures produced before the profile was named explicitly.
    rust_trace_candidates = [
        ROOT / "target/diagnostic/examples/tala-oracle",
        ROOT / "target/oracle/debug/examples/tala-oracle",
    ]
    rust_trace_binary = next((path for path in rust_trace_candidates if path.is_file()), None)
    if rust_trace_binary is None:
        raise RuntimeError(
            "missing optimized diagnostic binary; run `cargo build --profile diagnostic "
            "-p d2-rs-lib --features diagnostic-traces --example tala-oracle`"
        )
    fixture = args.fixtures_dir / args.fixture
    if not fixture.is_file():
        raise RuntimeError(f"Fixture not found: {fixture}")
    environment = {k: v for k, v in os.environ.items()
                   if not k.startswith(("TALA_", "WEFTAN_")) and k not in ("DEV_MODE", "TEST_MODE")}
    measured = subprocess.run([str(oracle), "measure", "--path", fixture.name],
                              input=fixture.read_bytes(), capture_output=True, check=True,
                              timeout=120, env=environment).stdout
    search_suffix = f".search-{args.search_flavor}" if args.search_flavor else ""
    gap_suffix = ".gap" if args.gap else ""
    swap_ports_suffix = ".swap-ports" if args.swap_ports else ""
    directory = ROOT / "target/parity-witnesses" / f"{fixture.stem}.seed-{args.seed}{search_suffix}{gap_suffix}{swap_ports_suffix}.trace"
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "measured.json").write_bytes(measured)
    measured_graph = json.loads(measured)
    manifest = {"fixture": fixture.name, "fixture_sha256": digest(fixture),
                "seed": args.seed, "scope": "raw stage capture, not parity", "oracles": {},
                "go_provenance": oracle_provenance, "go_trace_provenance": trace_provenance,
                "search_flavor": args.search_flavor,
                "swap_ports": args.swap_ports,
                "measured_sha256": digest(directory / "measured.json"),
                "measured_edge_count": len(measured_graph["edges"] or [])}
    profiles = tomllib.loads((ROOT / "Cargo.toml").read_text())["profile"]
    manifest["rust_profiles"] = {name: profiles[name] for name in ("release", "diagnostic")}
    manifest["capture_options"] = {name: getattr(args, name) for name in
                                   ("placement", "graph_owners", "packing", "routing", "rng", "ovg", "search_filters", "sized_calls", "sized_results", "search_heap", "search_costs", "gap", "connected_order", "swap_ports")}
    for name, binary, flag in (("go", REFERENCE / "bin/d2-oracle-trace", "TALA_TRACE_JSONL"),
                               ("rust", rust_trace_binary, "WEFTAN_TRACE_JSONL")):
        diagnostic_environment = {**environment, flag: "1"}
        if args.graph_owners:
            diagnostic_environment[
                "TALA_TRACE_GRAPH_OWNERS" if name == "go" else "D2_RS_TRACE_OWNER_GRAPH"
            ] = "1"
        if args.search_filters and name == "rust":
            diagnostic_environment["WEFTAN_TRACE_SEARCH_FILTERS"] = "1"
        if args.sized_calls:
            diagnostic_environment[
                "TALA_TRACE_SIZED_GRAPH_IDS" if name == "go" else "WEFTAN_TRACE_SIZED_STATE_IDS"
            ] = "all"
            diagnostic_environment[
                "TALA_TRACE_SIZED_CALLS" if name == "go" else "WEFTAN_TRACE_SIZED_CALLS"
            ] = "1"
        if args.sized_results:
            diagnostic_environment[
                "TALA_TRACE_SIZED_GRAPH_IDS" if name == "go" else "WEFTAN_TRACE_SIZED_STATE_IDS"
            ] = "all"
            diagnostic_environment[
                "TALA_TRACE_SIZED_MOVE_RESULTS" if name == "go" else "WEFTAN_TRACE_SIZED_MOVE_RESULTS"
            ] = "1"
        if args.connected_order:
            diagnostic_environment[
                "TALA_TRACE_CONNECTED_ORDER" if name == "go" else "D2RS_TRACE_CONNECTED_ORDER"
            ] = args.connected_order
        if args.swap_ports:
            diagnostic_environment[
                "TALA_TRACE_SWAP_PORTS_JSONL" if name == "go" else "D2RS_TRACE_SWAP_PORTS_JSONL"
            ] = "1"
        if args.ovg:
            diagnostic_environment["TALA_TRACE_OVG_JSONL" if name == "go" else "WEFTAN_TRACE_OVG_JSONL"] = "1"
        if args.search_flavor:
            diagnostic_environment["TALA_TRACE_SEARCH_FLAVOR" if name == "go" else "WEFTAN_TRACE_SEARCH_FLAVOR"] = args.search_flavor
        if args.search_heap:
            diagnostic_environment["TALA_TRACE_SEARCH_HEAP" if name == "go" else "WEFTAN_TRACE_SEARCH_HEAP"] = "1"
        if args.search_costs:
            diagnostic_environment["TALA_TRACE_SEARCH_COSTS" if name == "go" else "WEFTAN_TRACE_SEARCH_COSTS"] = "1"
        if args.placement:
            diagnostic_environment[
                "TALA_TRACE_PLACEMENT" if name == "go" else "WEFTAN_TRACE_PLACEMENT"
            ] = "1"
        if args.packing:
            diagnostic_environment[
                "TALA_TRACE_PACKING" if name == "go" else "WEFTAN_TRACE_PACKING"
            ] = "1"
        if args.gap:
            diagnostic_environment["TALA_TRACE_GAP" if name == "go" else "WEFTAN_TRACE_GAP"] = "1"
        if args.routing:
            diagnostic_environment["TALA_TRACE_ROUTING" if name == "go" else "WEFTAN_TRACE_ROUTING"] = "1"
        if args.rng:
            diagnostic_environment[
                "TALA_TRACE_RNG_JSONL" if name == "go" else "WEFTAN_TRACE_RNG_JSONL"
            ] = "1"
            # Keep the existing human-readable shuffle markers alongside the
            # primitive JSONL stream. They make it possible to attribute a
            # draw-count difference to a specific operation while the JSONL
            # remains the machine-comparable record.
            diagnostic_environment[
                "TALA_TRACE_RNG" if name == "go" else "WEFTAN_TRACE_RNG"
            ] = "1"
        result = subprocess.run([str(binary), "layout", "--tala-seeds", str(args.seed)],
                                input=measured, capture_output=True, timeout=120,
                                env=diagnostic_environment)
        (directory / f"{name}.stdout.json").write_bytes(result.stdout)
        (directory / f"{name}.stderr").write_bytes(result.stderr)
        events = []
        placement_events = []
        candidate_events = []
        score_events = []
        packing_events = []
        routing_events = []
        rng_events = []
        rng_init_events = []
        search_events = []
        heap_events = []
        cost_breakdown_events = []
        sized_call_events = []
        sized_result_events = []
        gap_events = []
        connected_events = []
        swap_port_events = []
        for line in result.stderr.splitlines():
            line = line.decode("utf-8", errors="replace")
            try:
                event = json.loads(line)
            except ValueError:
                event = None
            if isinstance(event, dict) and event.get("event") == "stage":
                events.append(event)
            if isinstance(event, dict) and event.get("event") == "placement":
                placement_events.append(event)
            if isinstance(event, dict) and event.get("event") == "placement-candidates":
                candidate_events.append(event)
            if isinstance(event, dict) and event.get("event") == "placement-score":
                score_events.append(event)
            if isinstance(event, dict) and event.get("event") == "packing":
                packing_events.append(event)
            if isinstance(event, dict) and event.get("event") == "route-flavor":
                routing_events.append(event)
            if isinstance(event, dict) and event.get("event") in {
                "route-search-neighbor", "route-search-cost", "route-search-pop"
            }:
                search_events.append(event)
            if isinstance(event, dict) and event.get("event") == "route-search-heap":
                heap_events.append(event)
            if isinstance(event, dict) and event.get("event") == "route-search-cost-breakdown":
                cost_breakdown_events.append(event)
            if isinstance(event, dict) and event.get("event") == "sized-optimize":
                sized_call_events.append(event)
            if isinstance(event, dict) and event.get("event") == "sized-node-result":
                sized_result_events.append(event)
            if isinstance(event, dict) and str(event.get("event", "")).startswith("gap-"):
                gap_events.append(event)
            if isinstance(event, dict) and str(event.get("event", "")).startswith("swap-port-"):
                swap_port_events.append(event)
            if args.connected_order and line.startswith("CONNECTED_STEP_GO "):
                connected_events.append(parse_connected_go_trace(line))
            if args.connected_order and line.startswith("CONNECTED_STEP_RUST "):
                connected_events.append(parse_connected_rust_trace(line))
            if isinstance(event, dict) and event.get("event") == "rng":
                rng_events.append(event)
            if isinstance(event, dict) and event.get("event") == "rng-init":
                rng_init_events.append(event)
        (directory / f"{name}.gap.json").write_text(json.dumps(gap_events, indent=2) + "\n")
        (directory / f"{name}.search.json").write_text(json.dumps(search_events, indent=2) + "\n")
        (directory / f"{name}.heap.json").write_text(json.dumps(heap_events, indent=2) + "\n")
        (directory / f"{name}.cost-breakdown.json").write_text(json.dumps(cost_breakdown_events, indent=2) + "\n")
        (directory / f"{name}.sized-calls.json").write_text(json.dumps(sized_call_events, indent=2) + "\n")
        (directory / f"{name}.sized-results.json").write_text(json.dumps(sized_result_events, indent=2) + "\n")
        (directory / f"{name}.routing.json").write_text(json.dumps(routing_events, indent=2) + "\n")
        (directory / f"{name}.rng.json").write_text(json.dumps(rng_events, indent=2) + "\n")
        (directory / f"{name}.connected.json").write_text(json.dumps(connected_events, indent=2) + "\n")
        (directory / f"{name}.swap-ports.json").write_text(json.dumps(swap_port_events, indent=2) + "\n")
        (directory / f"{name}.stages.json").write_text(json.dumps(events, indent=2) + "\n")
        (directory / f"{name}.placement.json").write_text(json.dumps(placement_events, indent=2) + "\n")
        (directory / f"{name}.candidates.json").write_text(json.dumps(candidate_events, indent=2) + "\n")
        (directory / f"{name}.scores.json").write_text(json.dumps(score_events, indent=2) + "\n")
        (directory / f"{name}.packing.json").write_text(json.dumps(packing_events, indent=2) + "\n")
        base_binary = oracle if name == "go" else ROOT / "target/release/examples/tala-oracle"
        base = subprocess.run([str(base_binary), "layout", "--tala-seeds", str(args.seed)],
                              input=measured, capture_output=True, timeout=120, env=environment)
        identical = base.returncode == result.returncode == 0 and base.stdout == result.stdout
        manifest["oracles"][name] = {"sha256": digest(binary), "exit": result.returncode,
                                     "base_sha256": digest(base_binary),
                                     "public_bytes_identical": identical,
                                     "placement_events": len(placement_events),
                                     "candidate_events": len(candidate_events),
                                     "score_events": len(score_events),
                                     "packing_events": len(packing_events),
                                     "routing_events": len(routing_events),
                                     "rng_events": len(rng_events),
                                     "rng_init_events": len(rng_init_events),
                                     "gap_events": len(gap_events),
                                     "gap_event_counts": {kind: sum(event["event"] == kind for event in gap_events)
                                                          for kind in sorted({event["event"] for event in gap_events})},
                                     "gap_decision_counts": {kind: sum(event.get("phase") == "decision" and event.get("decision") == kind for event in gap_events)
                                                             for kind in sorted({event["decision"] for event in gap_events if event.get("phase") == "decision"})},
                                     "gap_inner_decision_counts": {kind: sum(event.get("event") == "gap-inner" and event.get("decision") == kind for event in gap_events)
                                                                   for kind in sorted({event["decision"] for event in gap_events if event.get("event") == "gap-inner"})},
                                     "search_events": len(search_events),
                                     "heap_events": len(heap_events),
                                     "cost_breakdown_events": len(cost_breakdown_events),
                                     "sized_call_events": len(sized_call_events),
                                     "sized_result_events": len(sized_result_events),
                                     "swap_port_events": len(swap_port_events),
                                     "swap_port_event_counts": {
                                         kind: sum(event["event"] == kind for event in swap_port_events)
                                         for kind in sorted({event["event"] for event in swap_port_events})
                                     },
                                     "stages": [event["stage"] for event in events]}
        (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        if result.returncode or not events or not identical:
            raise RuntimeError(f"{name}: unsuccessful trace capture; inspect {directory}")
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(directory)


if __name__ == "__main__":
    main()
