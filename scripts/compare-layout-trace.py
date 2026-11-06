#!/usr/bin/env python3
"""Compare two TALA JSONL traces and report the first semantic divergence.

The capture tool deliberately retains raw stderr because diagnostic text is
useful while debugging. This comparator consumes only JSON objects from those
files. It compares event order, array order, identities, and exact float-bit
strings. The Rust stage event includes a diagnostic-only ``seed`` field that
the Go v0.9.0 event does not, so that one additive field is omitted for stage
comparisons. Raw RNG-constructor events remain strict by default; parity gates
may omit them because Rust helper scopes create extra RNG objects that never
draw. RNG draws and placement scope events remain strict and ordered.
Stable node and source-edge identities are compared in their emitted order.
Only adapter-local edge IDs are omitted; node order, edge order, candidate
lists, and decision events remain strict.

For source-compiled nested diagrams, Go may pass TALA a graph-local ID such as
``r.1`` while Rust retains the fully qualified D2 ID ``directions.r.1``.
``--normalize-scoped-ids`` opts into a per-snapshot one-to-one suffix mapping
for that namespace difference. It does not reorder events or normalize
geometry, candidate order, scores, or any other fields.

The six orthogonal placement callbacks in v0.9.0 walk ``root.Graph`` on the Go
side, while Rust records an owner-scope view. With
``--project-owner-snapshots``, paired ``initialized`` events establish the
active optimizer inventory. Every active node and every node present in both
owner views is compared exactly. Any context node present on only one side
fails the comparison: the option localizes active records but cannot turn an
incomplete owner snapshot into trace parity.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


GO_OWNER_GRAPH_PHASES = {
    "orthogonal-start",
    "orthogonal-after-initialize",
    "orthogonal-after-sizeless",
    "orthogonal-after-transition-compaction",
    "orthogonal-after-sized",
    "orthogonal-after-optimized",
}


def load_events(path: Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for line_number, line in enumerate(path.read_text().splitlines(), 1):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and isinstance(value.get("event"), str):
            events.append(value)
    return events


def comparable(event: dict[str, Any]) -> dict[str, Any]:
    value = json.loads(json.dumps(event))
    if value.get("event") == "stage":
        value.pop("seed", None)
    if value.get("event") in {"stage", "placement"}:
        edges = value.get("edges")
        if isinstance(edges, list):
            for record in edges:
                if isinstance(record, dict):
                    record.pop("id", None)
    return value


def localize_scoped_snapshot_ids(
    left_event: dict[str, Any], right_event: dict[str, Any]
) -> None:
    """Map fully qualified Rust snapshot IDs to Go's graph-local IDs.

    Only an unambiguous, complete one-to-one mapping is accepted. If either
    side lacks the required identity inventory, counts differ, or an identity
    is ambiguous, leave the event untouched so the strict comparison reports
    it. Ordered RNG-scope node and container lists are localized in place,
    never sorted.
    """
    if right_event.get("event") != left_event.get("event"):
        return
    event_type = left_event.get("event")

    left_root = left_event.get("root")
    right_root = right_event.get("root")

    if event_type == "rng-scope":
        left_ids = left_event.get("nodes")
        right_ids = right_event.get("nodes")
        if not isinstance(left_ids, list) or not isinstance(right_ids, list):
            return
        if not all(isinstance(node_id, str) for node_id in left_ids + right_ids):
            return
        mapping = scoped_id_mapping(left_ids, right_ids)
        if mapping is None:
            return
        root_can_localize = add_scoped_root_mapping(
            mapping, left_root, right_root
        )
        left_containers = left_event.get("containers")
        right_containers = right_event.get("containers")
        container_mapping = None
        if isinstance(left_containers, list) and isinstance(right_containers, list):
            if all(
                isinstance(container, dict) and isinstance(container.get("id"), str)
                for container in left_containers + right_containers
            ):
                container_mapping = scoped_id_mapping(
                    [container["id"] for container in left_containers],
                    [container["id"] for container in right_containers],
                )
        # This list records graph order. Localize identities in place without
        # sorting it, so traversal-order differences remain visible.
        right_event["nodes"] = [mapping[node_id] for node_id in right_ids]
        if container_mapping is not None:
            for container in right_containers:
                container["id"] = container_mapping[container["id"]]
        if root_can_localize:
            right_event["root"] = left_root
        return

    if event_type not in {"stage", "placement"}:
        return
    left_nodes = left_event.get("nodes")
    right_nodes = right_event.get("nodes")
    if not isinstance(left_nodes, list) or not isinstance(right_nodes, list):
        return
    if len(left_nodes) != len(right_nodes):
        return
    if not all(
        isinstance(node, dict) and isinstance(node.get("id"), str)
        for node in left_nodes
    ):
        return
    if not all(
        isinstance(node, dict) and isinstance(node.get("id"), str)
        for node in right_nodes
    ):
        return

    mapping = scoped_id_mapping(
        [node["id"] for node in left_nodes],
        [node["id"] for node in right_nodes],
    )
    if mapping is None:
        return

    root_can_localize = add_scoped_root_mapping(mapping, left_root, right_root)

    for node in right_nodes:
        node["id"] = mapping[node["id"]]
    right_edges = right_event.get("edges")
    if isinstance(right_edges, list):
        for edge in right_edges:
            if not isinstance(edge, dict):
                continue
            for endpoint in ("from", "to"):
                value = edge.get(endpoint)
                if isinstance(value, str) and value in mapping:
                    edge[endpoint] = mapping[value]
    if root_can_localize:
        right_event["root"] = left_root


def owner_scope_key(node_id: str, *roots: Any) -> str:
    """Return a node name relative to either spelling of the current root."""
    anchors = sorted(
        {root for root in roots if isinstance(root, str) and root},
        key=len,
        reverse=True,
    )
    for root in anchors:
        if node_id == root:
            return "@root"
        if node_id.startswith(root + "."):
            return node_id[len(root) + 1 :]
    return node_id


def root_names_match(go_root: Any, rust_root: Any) -> bool:
    return (
        isinstance(go_root, str)
        and isinstance(rust_root, str)
        and (
            rust_root == go_root
            or rust_root.endswith("." + go_root)
            or go_root.endswith("." + rust_root)
        )
    )


def unique_scope_keys(
    nodes: Any, *roots: Any
) -> dict[str, dict[str, Any]] | None:
    if not isinstance(nodes, list) or not all(
        isinstance(node, dict) and isinstance(node.get("id"), str) for node in nodes
    ):
        return None
    result: dict[str, dict[str, Any]] = {}
    for node in nodes:
        key = owner_scope_key(node["id"], *roots)
        if key in result:
            return None
        result[key] = node
    return result


def active_scope_inventory(
    go_events: list[dict[str, Any]],
    rust_events: list[dict[str, Any]],
    event_index: int,
) -> set[str] | None:
    """Recover the optimizer's exact node inventory from its `initialized` event."""
    go_start = go_events[event_index].get("phase")
    rust_start = rust_events[event_index].get("phase")
    if go_start not in GO_OWNER_GRAPH_PHASES or rust_start != go_start:
        return None
    start = max(
        (index for index in range(event_index + 1) if go_events[index].get("phase") == "orthogonal-start"),
        default=None,
    )
    if start is None:
        return None
    if (
        rust_events[start].get("phase") != "orthogonal-start"
        or not root_names_match(
            go_events[start].get("root"), rust_events[start].get("root")
        )
        or not root_names_match(
            go_events[start].get("root"), go_events[event_index].get("root")
        )
    ):
        return None
    end = next(
        (
            index
            for index in range(start + 1, len(go_events))
            if go_events[index].get("phase") == "orthogonal-start"
        ),
        len(go_events),
    )
    go_initializers = [
        event
        for event in go_events[start + 1 : end]
        if event.get("phase") == "initialized"
    ]
    rust_initializers = [
        event
        for event in rust_events[start + 1 : end]
        if event.get("phase") == "initialized"
    ]
    if len(go_initializers) != 1 or len(rust_initializers) != 1:
        return None
    go_init, rust_init = go_initializers[0], rust_initializers[0]
    if not root_names_match(go_init.get("root"), rust_init.get("root")):
        return None
    go_nodes = unique_scope_keys(
        go_init.get("nodes"), go_init.get("root"), rust_init.get("root")
    )
    rust_nodes = unique_scope_keys(
        rust_init.get("nodes"), go_init.get("root"), rust_init.get("root")
    )
    if go_nodes is None or rust_nodes is None or go_nodes.keys() != rust_nodes.keys():
        return None
    return set(go_nodes)


def project_owner_snapshot(
    go_event: dict[str, Any],
    rust_event: dict[str, Any],
    active_ids: set[str] | None,
) -> tuple[list[str], list[str]] | None:
    """Project both owner snapshots to shared records, retaining the active scope.

    The six orthogonal callbacks in v0.9.0 serialize ``root.Graph`` even
    though the optimizer operates on a component subgraph. The Rust and Go
    owner views can each contain context records absent from the other. Their
    exact active optimizer inventory comes from the paired `initialized`
    event. Compare every shared record and require all active nodes on both
    sides, while reporting context records unique to either view.
    """
    if (
        go_event.get("event") != "placement"
        or rust_event.get("event") != "placement"
        or go_event.get("phase") not in GO_OWNER_GRAPH_PHASES
        or rust_event.get("phase") != go_event.get("phase")
    ):
        return None
    go_root = go_event.get("root")
    rust_root = rust_event.get("root")
    if active_ids is None or not root_names_match(go_root, rust_root):
        return None
    go_nodes = unique_scope_keys(go_event.get("nodes"), go_root, rust_root)
    rust_nodes = unique_scope_keys(rust_event.get("nodes"), go_root, rust_root)
    if go_nodes is None or rust_nodes is None:
        return None
    shared = go_nodes.keys() & rust_nodes.keys()
    if not active_ids.issubset(shared):
        return None
    go_event["nodes"] = [
        {**node, "id": key} for key, node in go_nodes.items() if key in shared
    ]
    rust_event["nodes"] = [
        {**node, "id": key} for key, node in rust_nodes.items() if key in shared
    ]
    go_event["root"] = rust_event["root"] = "@scope-root"
    go_only = [key for key in go_nodes if key not in rust_nodes]
    rust_only = [key for key in rust_nodes if key not in go_nodes]
    return go_only, rust_only


def scoped_id_mapping(
    left_ids: list[str], right_ids: list[str]
) -> dict[str, str] | None:
    """Build a complete one-to-one mapping from right IDs to left IDs."""
    if len(left_ids) != len(right_ids):
        return None
    mapping: dict[str, str] = {}
    for left_id in left_ids:
        exact = [right_id for right_id in right_ids if right_id == left_id]
        candidates = exact or [
            right_id for right_id in right_ids if right_id.endswith("." + left_id)
        ]
        if len(candidates) != 1:
            return None
        mapping[candidates[0]] = left_id
    if len(mapping) != len(left_ids) or len(set(mapping.values())) != len(left_ids):
        return None
    return mapping


def add_scoped_root_mapping(
    mapping: dict[str, str], left_root: Any, right_root: Any
) -> bool:
    if not isinstance(left_root, str) or not isinstance(right_root, str):
        return False
    if right_root != left_root and not right_root.endswith("." + left_root):
        return False
    existing = mapping.get(right_root)
    if existing is not None and existing != left_root:
        return False
    mapping[right_root] = left_root
    return True


def first_difference(left: Any, right: Any, path: str = "$") -> tuple[str, Any, Any] | None:
    if type(left) is not type(right):
        return path, left, right
    if isinstance(left, dict):
        keys = list(dict.fromkeys([*left.keys(), *right.keys()]))
        for key in keys:
            if key not in left or key not in right:
                return f"{path}.{key}", left.get(key), right.get(key)
            difference = first_difference(left[key], right[key], f"{path}.{key}")
            if difference is not None:
                return difference
        return None
    if isinstance(left, list):
        for index, (left_item, right_item) in enumerate(zip(left, right)):
            difference = first_difference(left_item, right_item, f"{path}[{index}]")
            if difference is not None:
                return difference
        if len(left) != len(right):
            return f"{path}.length", len(left), len(right)
        return None
    if left != right:
        return path, left, right
    return None


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("left", type=Path, help="Go raw stderr or JSONL trace")
    parser.add_argument("right", type=Path, help="Rust raw stderr or JSONL trace")
    selector = parser.add_mutually_exclusive_group()
    selector.add_argument(
        "--event",
        dest="event_type",
        help="compare only one event family, preserving its original order",
    )
    selector.add_argument("--event-prefix", help="compare related event families together in their original order")
    parser.add_argument(
        "--ignore-event",
        action="append",
        default=[],
        help="omit a diagnostic-only event family from the normalized comparison; may be repeated",
    )
    parser.add_argument(
        "--normalize-scoped-ids",
        action="store_true",
        help="map fully qualified Rust snapshot IDs to unambiguous Go graph-local IDs",
    )
    parser.add_argument(
        "--project-owner-snapshots",
        action="store_true",
        help="localize owner snapshots, require equal context inventories, and verify the active optimizer inventory",
    )
    args = parser.parse_args()
    left_events = load_events(args.left)
    right_events = load_events(args.right)
    if args.event_type:
        left_events = [event for event in left_events if event.get("event") == args.event_type]
        right_events = [event for event in right_events if event.get("event") == args.event_type]
    if args.event_prefix:
        left_events = [event for event in left_events if event["event"].startswith(args.event_prefix)]
        right_events = [event for event in right_events if event["event"].startswith(args.event_prefix)]
    ignored_events = set(args.ignore_event)
    if ignored_events:
        left_events = [event for event in left_events if event["event"] not in ignored_events]
        right_events = [event for event in right_events if event["event"] not in ignored_events]
    if not left_events or not right_events:
        family = args.event_type or args.event_prefix or "any"
        raise SystemExit(
            f"INSUFFICIENT EVIDENCE for {family!r} events: "
            f"Go={len(left_events)}, Rust={len(right_events)}; "
            "empty traces do not establish parity"
        )
    if args.normalize_scoped_ids or args.project_owner_snapshots:
        left_events = json.loads(json.dumps(left_events))
        right_events = json.loads(json.dumps(right_events))
        owner_snapshots_compared = 0
        projected_snapshots = 0
        projected_go_nodes = 0
        projected_rust_nodes = 0
        owner_context_sample = None
        active_inventories = (
            [
                active_scope_inventory(left_events, right_events, event_index)
                for event_index in range(min(len(left_events), len(right_events)))
            ]
            if args.project_owner_snapshots
            else []
        )
        for event_index, (left_event, right_event) in enumerate(zip(left_events, right_events)):
            if args.project_owner_snapshots:
                active_ids = active_inventories[event_index]
                owner_phase = left_event.get("phase")
                owner_root = left_event.get("root")
                omitted = project_owner_snapshot(left_event, right_event, active_ids)
                if omitted is not None:
                    owner_snapshots_compared += 1
                    go_only, rust_only = omitted
                    if go_only or rust_only:
                        projected_snapshots += 1
                        projected_go_nodes += len(go_only)
                        projected_rust_nodes += len(rust_only)
                        if owner_context_sample is None:
                            owner_context_sample = (
                                owner_phase,
                                owner_root,
                                go_only[:12],
                                rust_only[:12],
                            )
            if args.normalize_scoped_ids:
                localize_scoped_snapshot_ids(left_event, right_event)
    else:
        owner_snapshots_compared = 0
        projected_snapshots = projected_go_nodes = projected_rust_nodes = 0
    left = [comparable(event) for event in left_events]
    right = [comparable(event) for event in right_events]
    difference = first_difference(left, right)
    if difference is None:
        if projected_go_nodes or projected_rust_nodes:
            phase, root, go_only_ids, rust_only_ids = owner_context_sample
            print(
                "CONTEXT DIVERGENCE: owner snapshots contain one-sided records; "
                f"Go-only={projected_go_nodes}, Rust-only={projected_rust_nodes}, "
                f"snapshots={projected_snapshots}/{owner_snapshots_compared}; "
                f"first phase={phase!r} root={root!r}; "
                f"Go-only IDs={go_only_ids}, Rust-only IDs={rust_only_ids}"
            )
            raise SystemExit(1)
        suffix = (
            f"; verified owner context across {owner_snapshots_compared} "
            "orthogonal snapshots"
            if owner_snapshots_compared else ""
        )
        print(f"MATCH {len(left)} events{suffix}")
        return
    if not args.event_type:
        from collections import Counter

        print("event counts:")
        print(f"  Go:   {dict(sorted(Counter(event['event'] for event in left_events).items()))}")
        print(f"  Rust: {dict(sorted(Counter(event['event'] for event in right_events).items()))}")
        for index, (left_event, right_event) in enumerate(zip(left_events, right_events)):
            if left_event.get("event") != right_event.get("event"):
                print(
                    f"first event-family mismatch at index {index}: "
                    f"Go={left_event.get('event')!r}, Rust={right_event.get('event')!r}"
                )
                break
    path, left_value, right_value = difference
    print(f"DIVERGENCE at {path}")
    print(f"Go:   {json.dumps(left_value, sort_keys=True)}")
    print(f"Rust: {json.dumps(right_value, sort_keys=True)}")
    raise SystemExit(1)


if __name__ == "__main__":
    main()
