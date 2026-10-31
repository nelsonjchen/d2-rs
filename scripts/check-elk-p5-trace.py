#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Compare ordered P5 orthogonal-layer decisions with pinned elk-go."""

import argparse
import json
import os
import re
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURE = ROOT / "tests/elk/d2-multi-external-fanout-root.json"
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
LAYER_PATH = re.compile(r"graphs\[(\d+)\]\.layers\[(-?\d+)->(\d+)\]")


def float_bits(value: float) -> int:
    return int.from_bytes(struct.pack(">d", value), "big")


def authored_node_names(graph: dict) -> set[str]:
    names = set()

    def visit(nodes: list[dict]) -> None:
        for node in nodes:
            names.add(node["id"])
            visit(node.get("children", []))

    visit(graph.get("children", []))
    return names


def normalize_nodes(nodes: list[dict], authored_names: set[str], rust: bool) -> list[dict]:
    normalized = []
    dummy_index = 0
    for node in nodes:
        raw_name = node["name"] if rust else node["stable_id"]
        if raw_name in authored_names or raw_name.startswith("node:"):
            name = raw_name if rust else node["name"]
        else:
            name = f"dummy:{dummy_index}"
            dummy_index += 1
        if rust:
            x, y, width, height, margin_left, margin_right = node["bits"]
        else:
            x = float_bits(node["x"])
            y = float_bits(node["y"])
            width = float_bits(node["width"])
            height = float_bits(node["height"])
            margin_left = float_bits(node["margin_left"])
            margin_right = float_bits(node["margin_right"])
        normalized.append(
            {
                "name": name,
                "x": x,
                "y": y,
                "width": width,
                "height": height,
                "margin_left": margin_left,
                "margin_right": margin_right,
            }
        )
    return normalized


def go_events(lines: list[str], authored_names: set[str]) -> list[dict]:
    result = []
    for line in lines:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("stage") != "p5-orthogonal-layer":
            continue
        nodes = []
        for node in event["nodes"]:
            stable_id = node["stable_id"]
            if stable_id.startswith("node:"):
                index = int(stable_id.split(":", 1)[1])
                # elk-go's importer assigns authored nodes in preorder. The
                # exact name map is attached below by `main`.
                name = f"node:{index}"
            else:
                name = stable_id
            nodes.append({**node, "name": name})
        result.append(
            {
                "graph": event["graph"],
                "layer_index": event["layer_index"],
                "layer": event["layer"],
                "layer_start_x": float_bits(event["layer_start_x"]),
                "layer_width": float_bits(event["layer_width"]),
                "route_start_x": float_bits(event["route_start_x"]),
                "slots": event["slots"],
                "left_external": event["left_external"],
                "right_external": event["right_external"],
                "routing_width": float_bits(event["routing_width"]),
                "next_x": float_bits(event["next_x"]),
                "nodes": nodes,
            }
        )
    return result


def rust_events(events: list[dict]) -> list[dict]:
    result = []
    for event in events:
        if event.get("operation") != "p5_orthogonal_layer":
            continue
        match = LAYER_PATH.fullmatch(event["path"])
        if not match:
            raise ValueError(f"unrecognized Rust P5 trace path: {event['path']}")
        graph_id, layer_index, _next_layer_index = map(int, match.groups())
        fields = dict(field.split("=", 1) for field in event["value"].split(";"))
        nodes = []
        if fields["nodes"]:
            for value in fields["nodes"].split(","):
                name, *bits = value.rsplit(":", 6)
                if len(bits) != 6:
                    raise ValueError(f"malformed Rust P5 node value: {value}")
                nodes.append({"name": name, "bits": [int(bit, 16) for bit in bits]})
        result.append(
            {
                "graph": graph_id,
                "layer_index": layer_index,
                "layer": int(fields["layer"]),
                "layer_start_x": int(fields["start"], 16),
                "layer_width": int(fields["layer_width"], 16),
                "route_start_x": int(fields["route"], 16),
                "slots": int(fields["slots"]),
                "left_external": fields["left_external"] == "true",
                "right_external": fields["right_external"] == "true",
                "routing_width": int(fields["routing_width"], 16),
                "next_x": int(fields["next"], 16),
                "nodes": nodes,
            }
        )
    return result


def map_go_names(events: list[dict], graph: dict) -> None:
    names = []

    def visit(nodes: list[dict]) -> None:
        for node in nodes:
            names.append(node["id"])
            visit(node.get("children", []))

    visit(graph.get("children", []))
    for event in events:
        for node in event["nodes"]:
            stable_id = node["name"]
            if stable_id.startswith("node:"):
                index = int(stable_id.split(":", 1)[1])
                node["name"] = names[index - 1]


def scope_groups(events: list[dict], rust: bool) -> list[list[dict]]:
    """Group P5 records by one routed graph/component without arena IDs."""
    groups = []
    current = []
    previous_graph = None
    for event in events:
        starts_scope = (
            event["layer_index"] == -1
            if rust
            else previous_graph is not None and event["graph"] != previous_graph
        )
        if starts_scope and current:
            groups.append(current)
            current = []
        current.append(event)
        previous_graph = event["graph"]
    if current:
        groups.append(current)
    return groups


def canonicalize(events: list[dict], authored_names: set[str], rust: bool) -> list[dict]:
    result = []
    for group_index, group in enumerate(scope_groups(events, rust)):
        authored_scope = tuple(
            sorted(
                {
                    node["name"]
                    for event in group
                    for node in event["nodes"]
                    if node["name"] in authored_names
                }
            )
        )
        if not authored_scope:
            raise ValueError(
                f"cannot identify P5 graph scope {group_index} from authored nodes"
            )
        for event in group:
            result.append(
                {
                    # elk-go allocates fresh graph IDs when it splits
                    # disconnected components; Rust keeps each component in
                    # its original graph. The authored-node set is the stable
                    # identity shared by both executions.
                    "graph_scope": authored_scope,
                    # Source layer IDs are global arena allocation indices,
                    # and the Rust arena allocates temporary layers
                    # differently. The per-graph layer index is stable.
                    **{
                        key: value
                        for key, value in event.items()
                        if key not in ("graph", "nodes", "layer")
                    },
                    "nodes": normalize_nodes(event["nodes"], authored_names, rust),
                }
            )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, default=DEFAULT_FIXTURE)
    args = parser.parse_args()
    fixture = args.fixture if args.fixture.is_absolute() else ROOT / args.fixture
    if not GO_ORACLE.is_file():
        print("Build the oracle with scripts/build-elk-trace-oracle.py first", file=sys.stderr)
        return 2

    graph = json.loads(fixture.read_text())
    payload = fixture.read_bytes()
    go = subprocess.run(
        [str(GO_ORACLE)],
        input=payload,
        capture_output=True,
        env={**os.environ, "D2_ELK_TRACE": "1"},
        check=True,
        timeout=60,
    )
    rust = subprocess.run(
        RUST_EXAMPLE,
        input=payload,
        capture_output=True,
        cwd=ROOT,
        check=True,
        timeout=180,
    )
    go_trace = go_events(go.stderr.decode().splitlines(), authored_node_names(graph))
    map_go_names(go_trace, graph)
    rust_lines = [line for line in rust.stderr.decode().splitlines() if line.startswith("[")]
    if not rust_lines:
        raise ValueError("Rust trace output did not contain the JSON event array")
    rust_trace = rust_events(json.loads(rust_lines[-1]))
    names = authored_node_names(graph)
    expected = canonicalize(go_trace, names, rust=False)
    actual = canonicalize(rust_trace, names, rust=True)
    if expected != actual:
        limit = max(len(expected), len(actual))
        for index in range(limit):
            source = expected[index] if index < len(expected) else None
            translated = actual[index] if index < len(actual) else None
            if source != translated:
                print(json.dumps({"event": index, "go": source, "rust": translated}, indent=2))
        return 1
    print(f"{len(expected)} ordered P5 layer events match exactly, including routing slots and float bits")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
