#!/usr/bin/env python3
"""Tests for the GraphJSON engine trace boundary projection."""

from __future__ import annotations

import importlib.util
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-graphjson-trace.py")
SPEC = importlib.util.spec_from_file_location("check_graphjson_trace", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_trace_events_exclude_only_out_of_scope_adapter_stages() -> None:
    import json

    source = b"\n".join(
        json.dumps({"event": "stage", "stage": name, "nodes": [], "edges": []}).encode()
        for name in (
            "Prescale",
            "Compound/EdgeRouting",
            "NodePlacement",
            "SelectedResult",
            "Normalize",
        )
    )
    assert [event["stage"] for event in MODULE.events(source)] == [
        "Prescale",
        "NodePlacement",
        "Normalize",
    ]


def test_rng_events_keep_operation_values_and_interleaving() -> None:
    import json

    stage = {"event": "stage", "stage": "Prescale", "nodes": [], "edges": []}
    rng = {
        "event": "rng",
        "stream": "pipeline-random",
        "operation": "int63",
        "result": "123456789",
        "draw_count": 1,
        "stream_id": 1,
    }
    rng_init = {"event": "rng-init", "stream": "node-placement", "stream_id": 1, "seed": 7}
    rng_scope = {"event": "rng-scope", "stream": "node-placement", "nodes": ["1", "2"]}
    untraced_helper = {"event": "rng-init", "stream": "default", "seed": 1}
    source = b"\n".join(
        json.dumps(value).encode() for value in (untraced_helper, rng_init, rng_scope, stage, rng)
    )

    assert [event["event"] for event in MODULE.events(source)] == ["stage"]
    assert MODULE.events(source, include_rng=True) == [rng_init, rng_scope, stage, rng]


def test_first_event_difference_preserves_both_stage_snapshots() -> None:
    preceding_event = {"event": "stage", "stage": "RouteEdges", "nodes": [], "edges": []}
    go_event = {
        "event": "stage",
        "stage": "ContainerPlacement",
        "nodes": [{"id": "node-7", "y_bits": "0x4098780000000000"}],
    }
    rust_event = {
        "event": "stage",
        "stage": "ContainerPlacement",
        "nodes": [{"id": "node-7", "y_bits": "0x4066800000000000"}],
    }

    difference = MODULE.first_event_difference(
        [preceding_event, go_event], [preceding_event, rust_event]
    )

    assert difference is not None
    assert difference["event_index"] == 1
    assert difference["difference"] == (
        "$.nodes[0].y_bits: '0x4098780000000000' != '0x4066800000000000'"
    )
    assert difference["go_event"] == go_event
    assert difference["rust_event"] == rust_event
    assert difference["preceding_events"] == [
        {"event_index": 0, "event": preceding_event}
    ]


def test_placement_mismatch_includes_prior_snapshots_for_the_node() -> None:
    preceding_event = {
        "event": "placement",
        "phase": "optimized",
        "root": "top",
        "nodes": [{"id": "7", "x_bits": "0x1"}],
    }
    go_event = {
        "event": "placement",
        "phase": "after-orthogonal-0",
        "root": "top",
        "nodes": [{"id": "7", "x_bits": "0x2"}],
    }
    rust_event = {
        "event": "placement",
        "phase": "after-orthogonal-0",
        "root": "top",
        "nodes": [{"id": "7", "x_bits": "0x3"}],
    }

    difference = MODULE.first_event_difference(
        [preceding_event, go_event], [preceding_event, rust_event]
    )

    assert difference is not None
    assert difference["node_id"] == "7"
    assert difference["previous_node_snapshots"] == [
        {
            "event_index": 0,
            "phase": "optimized",
            "root": "top",
            "go_nodes": [{"id": "7", "x_bits": "0x1"}],
            "rust_nodes": [{"id": "7", "x_bits": "0x1"}],
        }
    ]


def test_placement_snapshots_are_opt_in_and_keep_traversal_order() -> None:
    import json

    first = {
        "event": "placement",
        "phase": "placeNodes-entry",
        "nodes": [
            {"id": "b", "stable_id": "20"},
            {"id": "a", "stable_id": "10"},
        ],
    }
    stage = {"event": "stage", "stage": "NodePlacement", "nodes": [], "edges": []}
    second = {
        "event": "placement",
        "phase": "after-split",
        "nodes": [
            {"id": "a", "stable_id": "10"},
            {"id": "b", "stable_id": "20"},
        ],
    }
    source = b"\n".join(json.dumps(value).encode() for value in (first, stage, second))

    assert MODULE.events(source) == [stage]
    assert MODULE.events(source, include_placement=True) == [
        {
            "event": "placement",
            "phase": "placeNodes-entry",
            "nodes": [
                {"id": "20", "debug_id": "b"},
                {"id": "10", "debug_id": "a"},
            ],
        },
        stage,
        {
            "event": "placement",
            "phase": "after-split",
            "nodes": [
                {"id": "10", "debug_id": "a"},
                {"id": "20", "debug_id": "b"},
            ],
        },
    ]


def test_placement_trace_requires_stable_node_ids() -> None:
    import json

    source = json.dumps(
        {"event": "placement", "phase": "initialized", "nodes": [{"id": "same"}]}
    ).encode()
    try:
        MODULE.events(source, include_placement=True)
    except RuntimeError as error:
        assert "stable numeric node ID" in str(error)
    else:
        raise AssertionError("placement traces without stable IDs must be rejected")


def test_full_trace_keeps_operation_family_order_and_payloads() -> None:
    import json

    trace = [
        {"event": "rng-init", "stream": "hierarchy-placement", "seed": 1},
        {"event": "stage", "stage": "Prescale", "seed": 1, "nodes": [], "edges": []},
        {"event": "placement-candidates", "owner": "a", "candidate": 2, "x_bits": "0x3ff0000000000000"},
        {"event": "placement-score", "owner": "a", "score_bits": "0x4000000000000000"},
        {"event": "packing", "phase": "place", "groups": [[1, 2]]},
        {"event": "route-flavor", "edge": 7, "flavor": "orthogonal"},
        {"event": "gap-proposal", "direction": 1, "clearance_bits": "0x3fe0000000000000"},
        {"event": "rng", "stream": "node-placement", "operation": "int63", "result": "5"},
        {"event": "unrelated-debug", "value": "discarded"},
    ]
    source = b"\n".join(json.dumps(event).encode() for event in trace)

    actual = MODULE.events(source, include_full_trace=True)
    expected = [dict(event) for event in trace[:-1]]
    expected[1].pop("seed")

    assert actual == expected
    assert MODULE.event_counts(actual) == {
        "gap-proposal": 1,
        "packing": 1,
        "placement-candidates": 1,
        "placement-score": 1,
        "rng": 1,
        "rng-init": 1,
        "route-flavor": 1,
        "stage": 1,
    }


def test_full_trace_enables_raw_placement_hooks_for_both_oracles() -> None:
    for prefix in ("TALA", "WEFTAN"):
        full = MODULE.trace_environment(
            {"PATH": "/bin", f"{prefix}_TRACE_PLACEMENT_SNAPSHOTS_ONLY": "stale"},
            f"{prefix}_TRACE_JSONL",
            include_full_trace=True,
        )
        assert full["PATH"] == "/bin"
        assert full[f"{prefix}_TRACE_PLACEMENT"] == "1"
        assert f"{prefix}_TRACE_PLACEMENT_SNAPSHOTS_ONLY" not in full
        assert full[f"{prefix}_TRACE_OVG_JSONL"] == "1"
        assert full[f"{prefix}_TRACE_TREE_PLACEMENT"] == "1"
        assert full[f"{prefix}_TRACE_TREE_TRIALS"] == "1"

        snapshots = MODULE.trace_environment(
            {"PATH": "/bin"},
            f"{prefix}_TRACE_JSONL",
            include_placement=True,
        )
        assert snapshots[f"{prefix}_TRACE_PLACEMENT_SNAPSHOTS_ONLY"] == "1"


if __name__ == "__main__":
    test_trace_events_exclude_only_out_of_scope_adapter_stages()
    test_rng_events_keep_operation_values_and_interleaving()
    test_first_event_difference_preserves_both_stage_snapshots()
    test_placement_mismatch_includes_prior_snapshots_for_the_node()
    test_placement_snapshots_are_opt_in_and_keep_traversal_order()
    test_placement_trace_requires_stable_node_ids()
    test_full_trace_keeps_operation_family_order_and_payloads()
    test_full_trace_enables_raw_placement_hooks_for_both_oracles()
