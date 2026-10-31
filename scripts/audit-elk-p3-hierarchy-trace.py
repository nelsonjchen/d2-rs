#!/usr/bin/env python3
"""Compare P3 hierarchy scheduling against the pinned ELK source witness.

This focused audit reports differences in schedule, ordered layer starts, RNG
operations, and final geometry. It exits successfully for both matched and
divergent reports; a match covers this witness only, not full ELK or D2 parity.
"""

import argparse
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests/elk/d2-hierarchy-balanced-bk-external-port.json"
GO_ORACLE = ROOT / ".reference/bin/elk-wire-trace-oracle"
RUST_COMMAND = [
    "cargo",
    "run",
    "--quiet",
    "-p",
    "d2-rs-elk",
    "--example",
    "elk-wire",
    "--features",
    "diagnostic-traces",
]


def rust_events(stderr: str) -> list[dict]:
    decoder = json.JSONDecoder()
    for line in stderr.splitlines():
        if not line.startswith("["):
            continue
        try:
            events, end = decoder.raw_decode(line)
        except json.JSONDecodeError:
            continue
        if line[end:].strip():
            continue
        if isinstance(events, list) and all(isinstance(event, dict) for event in events):
            return events
    raise ValueError("Rust diagnostic trace array was not emitted")


def reconstructed_bottom_up_queue(scope_events: list[dict]) -> list[int]:
    """Rebuild initialize()'s prepend queue from its repeated scope snapshots.

    elk-go emits each graph scope while building GraphInfoHolders and again
    after P3 to expose the final order. Only the first snapshot per graph is
    an initialization observation. initialize() prepends eligible BFS
    holders, so the sweep queue is their reverse discovery order.
    """
    seen = set()
    queue = []
    for event in scope_events:
        graph_index = event["graph_index"]
        if graph_index in seen:
            continue
        seen.add(graph_index)
        if event.get("dont_sweep_into"):
            queue.insert(0, graph_index)
    return queue


def component_run_scope_signatures(
    go_trace: list[dict], rust_trace: list[dict], fixture_graph: dict
) -> tuple[list[list[list[str]]], list[list[list[str]]]]:
    """Compare P3's per-component scope membership in source call order.

    The flat component runner invokes P3 independently for each connected
    component. These are not entries in the hierarchy-wide bottom-up queue.
    In the pinned importer, flat authored nodes are added in child order and
    their LNodeIDs are allocated sequentially, so the Go layer IDs map to the
    fixture's child IDs. Rust records the same stable source IDs directly.
    """
    go_children = fixture_graph.get("children", [])
    go_node_names = [node["id"] for node in go_children]

    def component_runs(
        events: list[dict], config_key: str, config_stage: str, scope_key: str, scope_stage: str,
        *, go_source: bool,
    ) -> list[list[list[str]]]:
        config_indices = [
            index for index, event in enumerate(events)
            if event.get(config_key) == config_stage
        ]
        signatures = []
        for run_index, start in enumerate(config_indices):
            end = config_indices[run_index + 1] if run_index + 1 < len(config_indices) else len(events)
            scope = next(
                (event for event in events[start + 1:end] if event.get(scope_key) == scope_stage),
                None,
            )
            if scope is None:
                raise ValueError(f"P3 run {run_index} has no component scope snapshot")
            if go_source:
                layers = [
                    [
                        go_node_names[node_id] if node_id < len(go_node_names)
                        else f"go-node:{node_id}"
                        for node_id in layer["nodes"]
                    ]
                    for layer in scope.get("layers", [])
                ]
            else:
                value = scope.get("value", "")
                match = re.search(r"layers=((?:\[[^\]]*\];?)+);port_orders=", value)
                if not match:
                    raise ValueError(f"P3 run {run_index} has no Rust layer membership: {value}")
                layers = [
                    [node for node in layer.split(",") if node]
                    for layer in re.findall(r"\[([^\]]*)\]", match.group(1))
                ]
            signatures.append(layers)
        return signatures

    return (
        component_runs(go_trace, "stage", "p3-config", "stage", "p3-graph-scope", go_source=True),
        component_runs(
            rust_trace, "operation", "p3_config", "operation", "barycenter_scope_start",
            go_source=False,
        ),
    )


def go_events(stderr: str) -> list[dict]:
    events = []
    for line_number, line in enumerate(stderr.splitlines(), start=1):
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"invalid Go JSONL at line {line_number}: {error}") from error
        if isinstance(event, dict):
            events.append(event)
    return events


def float_bits(value: object) -> str | None:
    if not isinstance(value, (float, int)) or isinstance(value, bool):
        return None
    return f"{struct.unpack('>Q', struct.pack('>d', float(value)))[0]:016x}"


def float32_bits(value: object) -> str | None:
    if not isinstance(value, (float, int)) or isinstance(value, bool):
        return None
    return f"{struct.unpack('>I', struct.pack('>f', float(value)))[0]:08x}"


def normalized_rng_event(event: dict, *, source: str) -> dict:
    operation = event["operation"]
    draw_count = event.get("draw_count")
    if source == "go":
        result = event.get("result")
        if operation == "next_float":
            result = float32_bits(result)
        elif operation == "next_double":
            result = float_bits(result)
        elif isinstance(result, bool):
            result = str(result).lower()
        else:
            result = str(result)
    elif operation in ("next_float", "next_double"):
        raw_bits = event.get("float_bits")
        result = None if raw_bits is None else (
            f"{raw_bits:08x}" if operation == "next_float" else f"{raw_bits:016x}"
        )
    else:
        result = event.get("value")
        result = None if result is None else str(result).lower()
    return {"operation": operation, "draw_count": draw_count, "result": result}


def geometry_snapshot(graph: dict) -> dict:
    def node_snapshot(node: dict) -> dict:
        return {
            "id": node.get("id"),
            "geometry": {
                key: float_bits(node.get(key)) for key in ("x", "y", "width", "height")
            },
            "ports": [
                {
                    "id": port.get("id"),
                    "geometry": {
                        key: float_bits(port.get(key))
                        for key in ("x", "y", "width", "height")
                    },
                }
                for port in node.get("ports", [])
            ],
            "labels": [
                {
                    "text": label.get("text"),
                    "geometry": {
                        key: float_bits(label.get(key))
                        for key in ("x", "y", "width", "height")
                    },
                }
                for label in node.get("labels", [])
            ],
            "children": [node_snapshot(child) for child in node.get("children", [])],
        }

    edges = []
    for edge in graph.get("edges", []):
        sections = []
        for section in edge.get("sections", []):
            points = {}
            for key in ("startPoint", "endPoint"):
                point = section.get(key)
                points[key] = None if point is None else {
                    axis: float_bits(point.get(axis)) for axis in ("x", "y")
                }
            points["bendPoints"] = [
                {axis: float_bits(point.get(axis)) for axis in ("x", "y")}
                for point in section.get("bendPoints", [])
            ]
            sections.append({"points": points})
        edges.append({"id": edge.get("id"), "sections": sections})
    return {
        "geometry": {
            key: float_bits(graph.get(key)) for key in ("width", "height")
        },
        "children": [node_snapshot(node) for node in graph.get("children", [])],
        "edges": edges,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--report",
        type=Path,
        default=ROOT / "target/parity-witnesses/elk-p3-hierarchy-trace.json",
    )
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    args = parser.parse_args()
    if not GO_ORACLE.is_file():
        raise SystemExit("build .reference/bin/elk-wire-trace-oracle first")

    fixture = args.fixture if args.fixture.is_absolute() else ROOT / args.fixture
    source = fixture.read_bytes()
    fixture_graph = json.loads(source)
    environment = {**os.environ, "D2_ELK_TRACE": "1"}
    go = subprocess.run(
        [str(GO_ORACLE)], input=source, capture_output=True, env=environment, check=True, timeout=60
    )
    rust = subprocess.run(
        RUST_COMMAND, input=source, capture_output=True, cwd=ROOT, check=True, timeout=180
    )
    go_trace = go_events(go.stderr.decode())
    rust_trace = rust_events(rust.stderr.decode())
    go_p3_start = next(
        (index for index, event in enumerate(go_trace)
         if event.get("stage") == "p3-phase" and event.get("phase") == "begin"),
        None,
    )
    go_p3_end = next(
        (index for index, event in enumerate(go_trace)
         if event.get("stage") == "p3-phase" and event.get("phase") == "end"),
        None,
    )
    rust_component_p3_start = next(
        (index for index, event in enumerate(rust_trace)
         if event.get("phase") == "ordering" and event.get("operation") == "p3_phase"
         and event.get("value") == "begin"),
        None,
    )
    rust_component_p3_end = next(
        (index for index, event in enumerate(rust_trace)
         if event.get("phase") == "ordering" and event.get("operation") == "p3_phase"
         and event.get("value") == "end"),
        None,
    )
    if rust_component_p3_start is not None and rust_component_p3_end is not None:
        rust_p3_start, rust_p3_end = rust_component_p3_start, rust_component_p3_end
    else:
        rust_p3_start = next(
            (index for index, event in enumerate(rust_trace)
             if event.get("phase") == "ordering" and event.get("operation") == "p3_hierarchy_begin"),
            None,
        )
        rust_p3_end = next(
            (index for index, event in enumerate(rust_trace)
             if event.get("phase") == "ordering" and event.get("operation") == "p3_hierarchy_end"),
            None,
        )
    if go_p3_start is None or go_p3_end is None or rust_p3_start is None or rust_p3_end is None:
        raise ValueError("P3 trace interval markers are missing")
    is_component_pipeline = rust_component_p3_start is not None
    go_p3_trace = go_trace[go_p3_start:go_p3_end + 1]
    rust_p3_trace = rust_trace[rust_p3_start:rust_p3_end + 1]
    go_geometry = geometry_snapshot(json.loads(go.stdout))
    rust_geometry = geometry_snapshot(json.loads(rust.stdout))

    go_scopes = [event for event in go_trace if event.get("stage") == "p3-graph-scope"]
    go_sweep_order = reconstructed_bottom_up_queue(go_scopes)
    go_layer_starts = [
        {key: event[key] for key in ("graph_index", "layer", "randomized", "forward")}
        for event in go_trace
        if event.get("stage") == "p3-layer-start"
    ]
    go_rng = [
        {"operation": event["operation"], "draw_count": event.get("draw_count"), "result": event.get("result")}
        for event in go_p3_trace
        if event.get("stage") == "rng"
        and event.get("operation") in ("next_long", "next_boolean", "next_double", "next_float")
    ]

    rust_scope_order = [
        int(match.group(1))
        for event in rust_trace
        if event.get("operation") == "p3_graph_sweep_start"
        if (match := re.fullmatch(r"graphs\[(\d+)\]", event.get("path", "")))
    ]
    rust_layer_starts = []
    for event in rust_trace:
        if event.get("operation") != "p3_layer_start":
            continue
        values = dict(
            field.split("=", 1)
            for field in event.get("value", "").split(";")
            if "=" in field
        )
        rust_layer_starts.append(
            {
                "graph_index": int(values["graph_index"]),
                "layer": int(values["layer"]),
                "randomized": values["randomized"] == "true",
                "forward": values["forward"] == "true",
            }
        )
    rust_scope_decisions = {}
    for event in rust_trace:
        if event.get("operation") != "barycenter_scope_start":
            continue
        match = re.fullmatch(r"graphs\[(\d+)\]", event.get("path", ""))
        if not match:
            continue
        values = dict(
            field.split("=", 1)
            for field in event.get("value", "").split(";")
            if "=" in field
        )
        rust_scope_decisions[int(match.group(1))] = {
            "dont_sweep_into": values.get("dont_sweep_into") == "true",
            "reason": values.get("reason"),
            "paths_to_random": int(values.get("paths_to_random", "0")),
            "paths_to_hierarchical": int(values.get("paths_to_hierarchical", "0")),
            "threshold_bits": values.get("threshold_bits"),
            "hierarchy_index": values.get("hierarchy_index"),
            "sweep_index": values.get("sweep_index"),
        }
    rust_root_event = next(
        (
            event
            for event in rust_trace
            if event.get("operation") == "barycenter_scope_start"
            and "bfs_graphs=" in event.get("value", "")
        ),
        None,
    )
    rust_root_values = {} if rust_root_event is None else dict(
        field.split("=", 1)
        for field in rust_root_event.get("value", "").split(";")
        if "=" in field
    )
    rust_sweep_queue = [
        int(index)
        for index in rust_root_values.get("sweep_queue", "").split(",")
        if index
    ]
    rust_plan_decisions = {
        int(index): value == "true"
        for index, value in (
            item.split(":", 1)
            for item in rust_root_values.get("eligibility", "").split(",")
            if ":" in item
        )
    }
    go_scope_decisions = {
        event["graph_index"]: {
            "dont_sweep_into": bool(event["dont_sweep_into"]),
            "use_bottom_up": bool(event.get("use_bottom_up", event["dont_sweep_into"])),
        }
        for event in go_scopes
    }
    rust_rng = [
        {"stream": event.get("stream"), "operation": event["operation"], "draw_count": event.get("draw_count"), "result": event.get("value")}
        for event in rust_p3_trace
        if event.get("phase") == "rng"
    ]
    go_randomized_draws = [event for event in go_rng if event["operation"] in ("next_double", "next_float")]
    rust_randomized_draws = [event for event in rust_rng if event["operation"] in ("next_double", "next_float")]
    go_long_count = sum(event["operation"] == "next_long" for event in go_rng)
    rust_long_count = sum(event["operation"] == "next_long" for event in rust_rng)
    compared_rng_operations = {"next_long", "next_boolean", "next_double", "next_float"}
    normalized_go_rng = [
        normalized_rng_event(event, source="go")
        for event in go_rng
        if event["operation"] in compared_rng_operations
    ]
    normalized_rust_rng = [
        normalized_rng_event(event, source="rust")
        for event in rust_p3_trace
        if event.get("phase") == "rng"
        and event.get("operation") in compared_rng_operations
    ]
    rng_sequence_matches = normalized_go_rng == normalized_rust_rng

    p3_configurations_match = None
    complete_component_rng_matches = None
    randomized_boundary_candidates_match = None
    randomized_boundary_orders_match = None
    go_component_scope_signatures = None
    rust_component_scope_signatures = None
    component_scope_sequence_matches = None
    normalized_go_component_rng = []
    normalized_rust_component_rng = []
    go_component_configurations = []
    rust_component_configurations = []
    go_randomized_candidate_scores = []
    rust_randomized_candidate_scores = []
    go_randomized_orders = []
    rust_randomized_orders = []
    if is_component_pipeline:
        go_p3_configs = [
            index for index, event in enumerate(go_trace)
            if event.get("stage") == "p3-config"
        ]
        rust_p3_configs = [
            index for index, event in enumerate(rust_trace)
            if event.get("operation") == "p3_config"
        ]
        if go_p3_configs and rust_p3_configs:
            go_component_trace = go_trace[go_p3_configs[0]:]
            go_component_trace = go_component_trace[:next(
                (index + 1 for index, event in enumerate(go_component_trace)
                 if event.get("stage") == "p3-phase" and event.get("phase") == "end"
                 and sum(item.get("stage") == "p3-config" for item in go_component_trace[:index + 1])
                 == len(go_p3_configs)),
                len(go_component_trace),
            )]
            rust_component_trace = rust_trace[rust_p3_configs[0]:]
            rust_component_trace = rust_component_trace[:next(
                (index + 1 for index, event in enumerate(rust_component_trace)
                 if event.get("operation") == "p3_phase" and event.get("value") == "end"
                 and sum(item.get("operation") == "p3_config" for item in rust_component_trace[:index + 1])
                 == len(rust_p3_configs)),
                len(rust_component_trace),
            )]
            go_component_configurations = [
                {"thoroughness": event["thoroughness"], "cross_min_type": event["cross_min_type"]}
                for event in go_component_trace if event.get("stage") == "p3-config"
            ]
            rust_component_configurations = []
            for event in rust_component_trace:
                if event.get("operation") != "p3_config":
                    continue
                values = dict(
                    field.split("=", 1)
                    for field in event.get("value", "").split(";")
                    if "=" in field
                )
                rust_component_configurations.append({
                    "thoroughness": int(values["thoroughness"]),
                    "cross_min_type": int(values["cross_min_type"]),
                })
            p3_configurations_match = go_component_configurations == rust_component_configurations
            p3_rng_operations = {"set_seed", "next_long", "next_boolean", "next_double", "next_float"}
            raw_normalized_go_component_rng = []
            normalized_go_component_rng = []
            go_component_local_random_reinitializations = []
            between_component_runs = False
            for event in go_component_trace:
                if event.get("stage") == "p3-phase" and event.get("phase") == "end":
                    between_component_runs = True
                if event.get("stage") == "p3-config":
                    between_component_runs = False
                if event.get("stage") != "rng" or event.get("operation") not in p3_rng_operations:
                    continue
                normalized = normalized_rng_event(event, source="go")
                raw_normalized_go_component_rng.append(normalized)
                if (
                    between_component_runs
                    and normalized["operation"] == "set_seed"
                    and normalized["result"] == "1"
                ):
                    go_component_local_random_reinitializations.append(normalized)
                    continue
                normalized_go_component_rng.append(normalized)
            normalized_rust_component_rng = [
                normalized_rng_event(event, source="rust")
                for event in rust_component_trace
                if event.get("phase") == "rng" and event.get("operation") in p3_rng_operations
            ]

            # elk-go's component runner calls prepareGraphRun for each split
            # component. BasePipelineConfiguration re-runs
            # ConfigureGraphProperties, constructing a temporary Random from
            # the graph's configured seed, then runComponents immediately
            # restores the shared root Random before any phase runs. Those
            # temporary generator initializations are visible in the raw Go
            # trace but are not draws from the stream used by layout. Rust
            # keeps the shared stream directly and therefore has no equivalent
            # dead-stream events. Normalize only those source-proven setup
            # events; preserve every operation from the shared stream.
            complete_component_rng_matches = (
                normalized_go_component_rng == normalized_rust_component_rng
            )

            go_randomized = None
            for event in go_component_trace:
                if event.get("stage") == "p3-layer-start":
                    go_randomized = bool(event.get("randomized"))
                elif event.get("stage") == "p3-barycenter" and go_randomized:
                    go_randomized_candidate_scores.append([
                        float_bits(node.get("barycenter"))
                        for node in event.get("nodes", [])
                    ])
                elif event.get("stage") == "p3-layer-order" and go_randomized:
                    go_randomized_orders.append([
                        f"n{node_id}" for node_id in event.get("nodes", [])
                    ])
            rust_randomized = None
            pending_rust_scores = []
            pending_rust_order = []
            for event in rust_component_trace:
                if event.get("phase") == "ordering" and event.get("operation") == "p3_layer_start":
                    if rust_randomized:
                        rust_randomized_candidate_scores.append(pending_rust_scores)
                        rust_randomized_orders.append(pending_rust_order)
                    pending_rust_scores = []
                    pending_rust_order = []
                    values = dict(
                        field.split("=", 1)
                        for field in event.get("value", "").split(";")
                        if "=" in field
                    )
                    rust_randomized = values.get("randomized") == "true"
                elif event.get("phase") == "ordering" and rust_randomized:
                    if event.get("operation") == "candidate":
                        bits = event.get("float_bits")
                        pending_rust_scores.append(
                            None if bits is None else f"{bits:016x}"
                        )
                    elif event.get("operation") == "ordered":
                        pending_rust_order.append(event.get("value"))
            if rust_randomized:
                rust_randomized_candidate_scores.append(pending_rust_scores)
                rust_randomized_orders.append(pending_rust_order)
            randomized_boundary_candidates_match = (
                go_randomized_candidate_scores == rust_randomized_candidate_scores
            )
            randomized_boundary_orders_match = go_randomized_orders == rust_randomized_orders

        (
            go_component_scope_signatures,
            rust_component_scope_signatures,
        ) = component_run_scope_signatures(go_trace, rust_trace, fixture_graph)
        component_scope_sequence_matches = (
            go_component_scope_signatures == rust_component_scope_signatures
        )

    if is_component_pipeline:
        # Component P3 has no hierarchy-wide bottom-up queue. Compare each
        # P3 invocation's source-node membership in call order instead.
        rust_sweep_queue = None
        independent_queue_matches = None
        scope_order_matches = None
    else:
        rust_sweep_queue = rust_scope_order
        independent_queue_matches = go_sweep_order == rust_sweep_queue
        scope_order_matches = go_sweep_order == rust_scope_order
    eligibility_matches = all(
        rust_plan_decisions.get(graph_index) == decision["dont_sweep_into"]
        for graph_index, decision in go_scope_decisions.items()
    )
    final_geometry_matches = go_geometry == rust_geometry
    layer_start_sequence_matches = go_layer_starts == rust_layer_starts
    schedule_matches = (
        component_scope_sequence_matches
        if is_component_pipeline
        else scope_order_matches and independent_queue_matches
    )
    report = {
        "kind": "elk-p3-schedule-investigation",
        "fixture": fixture.relative_to(ROOT).as_posix(),
        "status": "matched" if (
            schedule_matches
            and eligibility_matches
            and layer_start_sequence_matches
            and final_geometry_matches
            and rng_sequence_matches
            and (p3_configurations_match is not False)
            and (complete_component_rng_matches is not False)
            and (randomized_boundary_candidates_match is not False)
            and (randomized_boundary_orders_match is not False)
        ) else "divergent",
        "scope_order": {
            "go_bottom_up_queue": None if is_component_pipeline else go_sweep_order,
            "rust_hierarchy_ordering_calls": None if is_component_pipeline else rust_scope_order,
            "go_component_run_scopes": go_component_scope_signatures,
            "rust_component_run_scopes": rust_component_scope_signatures,
        },
        "comparisons": {
            "scope_order_matches": scope_order_matches,
            "independent_sweep_queue_matches": independent_queue_matches,
            "component_scope_sequence_matches": component_scope_sequence_matches,
            "eligibility_matches": eligibility_matches,
            "layer_start_sequence_matches": layer_start_sequence_matches,
            "final_geometry_matches": final_geometry_matches,
            "rng_sequence_matches": rng_sequence_matches,
            "component_p3_configurations_match": p3_configurations_match,
            "complete_component_rng_matches": complete_component_rng_matches,
            "randomized_boundary_candidates_match": randomized_boundary_candidates_match,
            "randomized_boundary_orders_match": randomized_boundary_orders_match,
        },
        "independent_sweep_queue": {
            "go_bfs_indices": None if is_component_pipeline else go_sweep_order,
            "rust_bfs_indices": rust_sweep_queue,
            "matches": independent_queue_matches,
        },
        "scope_decisions": {
            "go": go_scope_decisions,
            "rust": rust_scope_decisions,
            "rust_hierarchy_plan_by_bfs_index": rust_plan_decisions,
            "eligibility_matches": eligibility_matches,
        },
        "final_geometry": {
            "exact_match": final_geometry_matches,
            "go": go_geometry,
            "rust": rust_geometry,
        },
        "go_layer_starts": go_layer_starts,
        "rust_layer_starts": rust_layer_starts,
        "rng": {
            "go_p3_events": go_rng,
            "rust_p3_events": rust_rng,
            "go_randomized_draw_count": len(go_randomized_draws),
            "rust_randomized_draw_count": len(rust_randomized_draws),
            "go_next_long_count": go_long_count,
            "rust_next_long_count": rust_long_count,
            "normalized_go_sequence": normalized_go_rng,
            "normalized_rust_sequence": normalized_rust_rng,
            "normalized_go_component_sequence": normalized_go_component_rng,
            "normalized_rust_component_sequence": normalized_rust_component_rng,
            "go_component_local_random_reinitializations": go_component_local_random_reinitializations
            if is_component_pipeline else [],
        },
        "component_p3": {
            "go_configurations": go_component_configurations,
            "rust_configurations": rust_component_configurations,
            "go_randomized_candidate_scores": go_randomized_candidate_scores,
            "rust_randomized_candidate_scores": rust_randomized_candidate_scores,
            "go_randomized_orders": go_randomized_orders,
            "rust_randomized_orders": rust_randomized_orders,
        },
        "scope": "diagnostic characterization only; it does not claim ELK or whole-layout parity",
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
