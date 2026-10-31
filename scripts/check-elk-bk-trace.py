#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Compare source and Rust BK candidates for a nested external-edge witness."""

import json
import argparse
import os
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests/elk/d2-hierarchy-balanced-bk-external-port.json"
GO_ORACLE = ROOT / ".reference/bin/elk-wire-trace-oracle"
RUST_EXAMPLE = [
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


def f64_hex(value: float) -> str:
    return struct.pack(">d", value).hex()


def rust_f64_hex(bits: int) -> str:
    return int(bits).to_bytes(8, "big").hex()


def original_node_names(graph: dict) -> dict[str, str]:
    """ELK's imported model reserves node:0 for the root, then walks preorder."""
    result = {}
    next_id = 1

    def visit(nodes: list[dict]) -> None:
        nonlocal next_id
        for node in nodes:
            result[f"node:{next_id}"] = node["id"]
            next_id += 1
            visit(node.get("children", []))

    visit(graph.get("children", []))
    return result


def go_candidates(events: list[dict], id_to_name: dict[str, str]) -> dict:
    groups = {}
    directions = {1: "Down", 2: "Up"}
    horizontals = {1: "Right", 2: "Left"}
    for event in events:
        if "candidate" not in event or "nodes" not in event:
            continue
        nodes = {}
        for node in event["nodes"]:
            stable_id = node.get("stable_id")
            name = id_to_name.get(stable_id)
            if name is None and stable_id and stable_id.startswith("lport:"):
                # External-port dummies are part of the P4 result, not just
                # scaffolding. Their coordinates feed the parent boundary
                # ports and must remain in the candidate comparison.
                name = stable_id
            if name is None:
                continue
            nodes[name] = {
                "placed_y": None
                if node.get("placed_y") is None
                else f64_hex(node["placed_y"]),
                "inner_shift": None
                if node.get("inner_shift") is None
                else f64_hex(node["inner_shift"]),
                "block_size": None
                if node.get("block_size") is None
                else f64_hex(node["block_size"]),
                "root": id_to_name.get(node.get("root_stable_id")),
                "align": id_to_name.get(node.get("align_stable_id")),
            }
        if not nodes:
            continue
        scope_key = tuple(sorted(nodes))
        groups.setdefault(scope_key, []).append(
            {
                "direction": f"{directions[event['vertical']]}/{horizontals[event['horizontal']]}",
                "nodes": nodes,
            }
        )
    return groups


def rust_candidates(events: list[dict], original_names: set[str]) -> dict:
    candidates = []
    current = None
    for event in events:
        if event.get("phase") != "placement":
            continue
        operation = event.get("operation")
        path = event.get("path", "")
        if operation == "bk_candidate":
            if current is not None:
                candidates.append(current)
            direction = event.get("value", "").removeprefix("direction=").split(";", 1)[0]
            current = {"direction": direction, "nodes": {}}
            continue
        if current is None or "].nodes[" not in path:
            continue
        node_and_field = path.split("].nodes[", 1)[1]
        name, _, field = node_and_field.partition("]")
        if field.startswith("."):
            field = field[1:]
        node = current["nodes"].setdefault(name, {})
        if operation == "bk_candidate_position":
            node["placed_y"] = rust_f64_hex(event["float_bits"])
        elif operation == "bk_candidate_inner_shift":
            node["inner_shift"] = rust_f64_hex(event["float_bits"])
        elif operation == "bk_candidate_block_size":
            node["block_size"] = rust_f64_hex(event["float_bits"])
        elif operation == "bk_candidate_alignment" and field in ("root", "align"):
            node[field] = event.get("value")
    if current is not None:
        candidates.append(current)

    groups = {}
    for candidate in candidates:
        nodes = {
            name: {
                "placed_y": fields.get("placed_y"),
                "inner_shift": fields.get("inner_shift"),
                "block_size": fields.get("block_size"),
                "root": fields.get("root")
                if fields.get("root") in original_names
                or str(fields.get("root", "")).startswith("lport:")
                else None,
                "align": fields.get("align")
                if fields.get("align") in original_names
                or str(fields.get("align", "")).startswith("lport:")
                else None,
            }
            for name, fields in candidate["nodes"].items()
            if name in original_names or name.startswith("lport:")
        }
        if not nodes:
            continue
        scope_key = tuple(sorted(nodes))
        groups.setdefault(scope_key, []).append(
            {"direction": candidate["direction"], "nodes": nodes}
        )
    return groups


def go_selected_placement(events: list[dict], id_to_name: dict[str, str]) -> dict:
    for event in events:
        if event.get("stage") != "brandes-koepf":
            continue
        return {
            name: {
                "x": f64_hex(node["x"]),
                "y": f64_hex(node["y"]),
            }
            for node in event.get("nodes", [])
            if (name := id_to_name.get(node.get("stable_id"))) is not None
        }
    raise ValueError("Go trace did not record the selected post-BK placement")


def rust_selected_placement(events: list[dict], selected_names: set[str]) -> dict:
    # Go emits a separate post-BK snapshot for each graph scope. Rust's nested
    # P4 trace uses the same bottom-up order but its arena snapshot contains
    # every scope; the root P4 snapshot comes later and must not be compared
    # with Go's first child-scope result.
    nested_events = [
        event
        for event in events
        if event.get("phase") == "nested" and event.get("operation") == "brandes_koepf"
    ]
    root_events = [
        event
        for event in events
        if event.get("phase") == "placement" and event.get("operation") == "brandes_koepf"
    ]
    selected_events = nested_events if nested_events else root_events
    positions = {}
    # Arena snapshots are emitted as one event per field. Keep the first x/y
    # observed for each node: the first nested BK batch is the bottom-up child
    # scope corresponding to Go's first post-BK scope snapshot.
    for event in selected_events:
        path = event.get("path", "")
        for node_id in selected_names:
            prefix = f"arena.nodes["
            if not path.startswith(prefix) or f"]({node_id})." not in path:
                continue
            axis = path.rsplit(".", 1)[-1]
            if axis in ("x", "y") and event.get("float_bits") is not None:
                positions.setdefault(node_id, {}).setdefault(
                    axis, rust_f64_hex(event["float_bits"])
                )
            break
    return {
        node_id: position
        for node_id, position in positions.items()
        if set(position) == {"x", "y"}
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    args = parser.parse_args()
    if not GO_ORACLE.is_file():
        print(
            "Build the pinned Go trace oracle first: "
            "uv run --no-project python scripts/build-elk-trace-oracle.py",
            file=sys.stderr,
        )
        return 2
    fixture = args.fixture if args.fixture.is_absolute() else ROOT / args.fixture
    graph = json.loads(fixture.read_text())
    source = fixture.read_bytes()
    go = subprocess.run(
        [str(GO_ORACLE)],
        input=source,
        capture_output=True,
        env={**os.environ, "D2_ELK_TRACE": "1"},
        check=True,
        timeout=60,
    )
    rust = subprocess.run(
        RUST_EXAMPLE,
        input=source,
        capture_output=True,
        cwd=ROOT,
        check=True,
        timeout=180,
    )
    go_events = [json.loads(line) for line in go.stderr.decode().splitlines() if line.startswith("{")]
    rust_events = json.loads(rust.stderr.decode().splitlines()[-1])
    id_to_name = original_node_names(graph)
    expected = go_candidates(go_events, id_to_name)
    actual = rust_candidates(rust_events, set(id_to_name.values()))
    go_placement = go_selected_placement(go_events, id_to_name)
    rust_placement = rust_selected_placement(rust_events, set(go_placement))
    candidates_match = expected == actual
    placement_matches = go_placement == rust_placement
    if not candidates_match:
        for scope in sorted(set(expected) | set(actual)):
            if expected.get(scope) != actual.get(scope):
                print(json.dumps({"scope": scope, "go": expected.get(scope), "rust": actual.get(scope)}, indent=2))
    if not placement_matches:
        print(json.dumps({"selected_bk_placement": {"go": go_placement, "rust": rust_placement}}, indent=2))
    if not candidates_match or not placement_matches:
        return 1
    print(
        f"{sum(map(len, expected.values()))} exact normalized BK candidate snapshots "
        f"across {len(expected)} graph scopes; selected post-BK coordinates match "
        f"for {len(go_placement)} nodes"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
