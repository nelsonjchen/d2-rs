#!/usr/bin/env python3
"""Compare normalized Dagro crossing-minimization sweeps with d2-rs.

Layer entries are normalized to the Dagro/D2 graph registration ordinal.
The comparison keeps every sweep, exact crossing-cost bits, and node order;
it does not normalize away repeated Rust executions of the helper. Original
objects are cross-checked through Go's per-layout ObjectMapper IDs and Rust's
stable external IDs; their internal arena IDs are different domains.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

from upstream_inputs import pristine_e2e

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURES = (
    "dagre-compound-label-midpoint",
    "dagre-compound-wide-label",
    "nested-left-cluster",
    "sequence-trace-order",
)
TRACE_FLAGS = (
    "DAGRE_DEBUG_PHASE",
    "DAGRE_ORDER_TRACE",
    "DAGRE_TRACE_OBJECTS",
    "DAGRO_TRACE_ORDER",
    "DAGRO_TRACE_RANKS",
    "DAGRO_TRACE_POS",
)


class TraceError(ValueError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_go_trace(stderr: str) -> list[dict]:
    """Parse order sweeps by Dagro invocation, retaining each local ID map."""
    object_pattern = re.compile(
        r'GO_MAP id=("(?:[^"\\]|\\.)*") object=("(?:[^"\\]|\\.)*")'
    )
    node_pattern = re.compile(r'\bindex=(\d+) id=("(?:[^"\\]|\\.)*")')
    event_pattern = re.compile(
        r"GO_DAGRE_ORDER phase=(init|sweep|best) iteration=(-?\d+) "
        r"crossings_bits=(none|[0-9a-f]{16}) layers=(.*)"
    )
    objects_by_id: dict[str, str] = {}
    phase = ""
    pending_node_order: dict[int, str] | None = None
    current_group: dict | None = None
    groups: list[dict] = []

    def finish_group() -> None:
        nonlocal current_group
        if current_group is None:
            return
        events = current_group["events"]
        if not events or events[0]["phase"] != "init" or events[-1]["phase"] != "best":
            raise TraceError("Go order trace group must contain init and best events")
        sweeps = [event["iteration"] for event in events if event["phase"] == "sweep"]
        if sweeps != list(range(len(sweeps))):
            raise TraceError(f"Go sweep iterations are incomplete or unordered: {sweeps}")
        groups.append(current_group)
        current_group = None

    for line in stderr.splitlines():
        if line.startswith("GO_MAP "):
            match = object_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Go object identity: {line[:180]}")
            encoded_id, encoded_object = match.groups()
            object_id, external_id = json.loads(encoded_id), json.loads(encoded_object)
            if object_id in objects_by_id:
                # ObjectMapper IDs restart for each extracted layout graph.
                objects_by_id = {}
            objects_by_id[object_id] = external_id
            continue
        if line.startswith("GO_DAGRO_PHASE "):
            finish_group()
            phase = line.removeprefix("GO_DAGRO_PHASE ")
            pending_node_order = {} if phase == "border-segments" else None
            continue
        if line.startswith("GO_DAGRO_NODE ") and phase == "border-segments":
            match = node_pattern.search(line)
            if not match:
                raise TraceError(f"malformed Go graph-order entry: {line[:180]}")
            index_text, encoded_id = match.groups()
            index, identity = int(index_text), json.loads(encoded_id)
            assert pending_node_order is not None
            if index in pending_node_order:
                raise TraceError(f"Go graph-order index {index} was emitted twice in one phase")
            pending_node_order[index] = identity
            continue
        if not line.startswith("GO_DAGRE_ORDER "):
            continue
        match = event_pattern.fullmatch(line)
        if not match:
            raise TraceError(f"malformed Go order event: {line[:180]}")
        if current_group is None:
            if not pending_node_order or sorted(pending_node_order) != list(range(len(pending_node_order))):
                raise TraceError("Go order event has no contiguous border-segment node map")
            node_ids = [pending_node_order[index] for index in range(len(pending_node_order))]
            index_by_id = {identity: index for index, identity in enumerate(node_ids)}
            if len(index_by_id) != len(node_ids):
                raise TraceError("Go order node map repeats an identity")
            current_group = {"node_ids": node_ids, "objects": objects_by_id.copy(), "events": []}
        phase_name, iteration, crossing_bits, encoded_layers = match.groups()
        layers = json.loads(encoded_layers)
        try:
            normalized = [[index_by_id[identity] for identity in layer] for layer in layers]
        except KeyError as error:
            raise TraceError(f"Go layer references unknown Dagro node {error.args[0]!r}") from error
        current_group["events"].append(
            {
                "phase": phase_name,
                "iteration": int(iteration),
                "crossings_bits": crossing_bits,
                "layers": normalized,
            }
        )
    finish_group()
    return groups


def parse_rust_trace(stderr: str) -> tuple[list[dict], list[list[dict]], list[list[str | None]]]:
    mappings: list[list[int]] = []
    external_id_groups: list[list[str | None]] = []
    events: list[list[dict]] = []
    current_map: list[int] | None = None
    current_external_ids: list[str | None] | None = None
    current_events: list[dict] | None = None
    node_pattern = re.compile(r"RUST_DAGRE_ORDER_NODE index=(\d+) id=(\d+)")
    object_pattern = re.compile(
        r'RUST_DAGRE_OBJECT index=(\d+) external_id=(null|"(?:[^"\\]|\\.)*")'
    )
    layer_pattern = re.compile(
        r"RUST_DAGRE_LAYERS phase=(init|sweep|best) iteration=(-?\d+) "
        r"crossings_bits=(none|[0-9a-f]{16}) layers=(.*)"
    )
    for line in stderr.splitlines():
        if line.startswith("RUST_DAGRE_ORDER_NODE "):
            match = node_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Rust graph-order entry: {line}")
            index, identity = map(int, match.groups())
            if index == 0:
                if current_map is not None:
                    mappings.append(current_map)
                    external_id_groups.append(current_external_ids or [])
                    events.append(current_events or [])
                current_map = []
                current_external_ids = []
                current_events = []
            if current_map is None or index != len(current_map):
                raise TraceError(f"Rust graph-order indexes are not contiguous at {index}")
            current_map.append(identity)
            assert current_external_ids is not None
            current_external_ids.append(None)
            continue
        if line.startswith("RUST_DAGRE_OBJECT "):
            match = object_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Rust object identity: {line}")
            index_text, encoded_external_id = match.groups()
            index = int(index_text)
            if current_map is None or current_external_ids is None or index >= len(current_map):
                raise TraceError(f"Rust object identity references unregistered index {index}")
            current_external_ids[index] = (
                None if encoded_external_id == "null" else json.loads(encoded_external_id)
            )
            continue
        if line.startswith("RUST_DAGRE_LAYERS "):
            match = layer_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Rust layer event: {line[:180]}")
            if current_map is None or current_events is None:
                raise TraceError("Rust layer event appeared before its registration map")
            phase, iteration, crossing_bits, encoded_layers = match.groups()
            layers = json.loads(encoded_layers)
            if any(index < 0 or index >= len(current_map) for layer in layers for index in layer):
                raise TraceError("Rust layer references an unknown normalized node index")
            current_events.append(
                {
                    "phase": phase,
                    "iteration": int(iteration),
                    "crossings_bits": crossing_bits,
                    "layers": layers,
                }
            )
    if current_map is not None:
        mappings.append(current_map)
        external_id_groups.append(current_external_ids or [])
        events.append(current_events or [])
    if not mappings:
        return [], [], []
    if len(mappings) != len(events):
        raise TraceError("Rust emitted incomplete compound-order trace groups")
    if any(not mapping for mapping in mappings):
        raise TraceError("Rust emitted an empty graph-order map")
    return events, mappings, external_id_groups


def compare_trace_groups(
    go_groups: list[dict],
    rust_events: list[list[dict]],
    rust_maps: list[list[int]],
    rust_external_ids: list[list[str | None]],
) -> dict:
    if not go_groups and not rust_events:
        return {
            "status": "inapplicable",
            "reason": "Neither engine emitted a compound-order sweep for this fixture.",
            "group_matches": [],
            "trace_multiplicity_matches": True,
        }
    def identity_key(ids: list[str | None]) -> tuple[str, ...] | None:
        source_objects = tuple(sorted(identity for identity in ids if identity is not None))
        return source_objects or None

    # Separate layout invocations can be nested, and Rust may currently emit
    # an additional enclosing-graph pass. Align exact source-object scopes so
    # those extra calls do not shift every later comparison. The full ordered
    # registration map, every event, and invocation order remain checked;
    # identity keys are only used to pair corresponding invocations.
    rust_by_scope: dict[tuple[str, ...] | None, list[int]] = {}
    for index, identities in enumerate(rust_external_ids):
        rust_by_scope.setdefault(identity_key(identities), []).append(index)
    used_rust: set[int] = set()
    group_matches = []
    group_details = []
    for go_index, go_group in enumerate(go_groups):
        go_ids = go_group["node_ids"]
        go_external_ids = [go_group["objects"].get(node_id) for node_id in go_ids]
        scope_key = identity_key(go_external_ids)
        candidates = rust_by_scope.get(scope_key, [])
        rust_index = next((candidate for candidate in candidates if candidate not in used_rust), None)
        if rust_index is None:
            group_matches.append(False)
            group_details.append({
                "go_group": go_index,
                "matched": False,
                "missing_engine": "rust",
                "go_node_count": len(go_ids),
                "go_object_ids": go_external_ids,
            })
            continue
        used_rust.add(rust_index)
        rust_group = rust_events[rust_index]
        rust_external_ids_for_group = rust_external_ids[rust_index]
        source_identities_match = (
            len(rust_external_ids_for_group) == len(go_ids)
            and all(
                rust_external_ids_for_group[node_index] == go_group["objects"].get(go_id)
                for node_index, go_id in enumerate(go_ids)
            )
        )
        registration_count_match = len(go_ids) == len(rust_maps[rust_index])
        event_match = rust_group == go_group["events"]
        matched = source_identities_match and registration_count_match and event_match
        group_matches.append(matched)
        group_details.append({
            "go_group": go_index,
            "rust_group": rust_index,
            "matched": matched,
            "source_object_identities_match": source_identities_match,
            "registration_count_match": registration_count_match,
            "events_match": event_match,
            "go_node_count": len(go_ids),
            "rust_node_count": len(rust_maps[rust_index]),
            "go_object_ids": go_external_ids,
            "rust_object_ids": rust_external_ids_for_group,
            "go_sweep_count": sum(event["phase"] == "sweep" for event in go_group["events"]),
            "rust_event_count": len(rust_group),
        })
    for rust_index, (rust_group, rust_ids) in enumerate(zip(rust_events, rust_external_ids)):
        if rust_index in used_rust:
            continue
        group_matches.append(False)
        group_details.append({
            "rust_group": rust_index,
            "matched": False,
            "missing_engine": "go",
            "rust_node_count": len(rust_maps[rust_index]),
            "rust_object_ids": rust_ids,
            "rust_event_count": len(rust_group),
        })
    go_scope_sequence = [
        identity_key([group["objects"].get(node_id) for node_id in group["node_ids"]])
        for group in go_groups
    ]
    rust_scope_sequence = [identity_key(ids) for ids in rust_external_ids]
    group_order_matches = go_scope_sequence == rust_scope_sequence
    multiplicity_match = len(go_groups) == len(rust_events)
    exact = (
        multiplicity_match
        and group_order_matches
        and len(used_rust) == len(rust_events)
        and bool(group_matches)
        and all(group_matches)
    )
    return {
        "status": "matched" if exact else "divergent",
        "group_matches": group_matches,
        "groups": group_details,
        "trace_multiplicity_matches": multiplicity_match,
        "group_order_matches": group_order_matches,
        "source_object_identities_match_registration": all(
            detail.get("source_object_identities_match", False)
            for detail in group_details
        ),
        "registration_count_match": all(detail.get("registration_count_match", False) for detail in group_details),
        "normalized_sweep_sequences_match": all(detail.get("events_match", False) for detail in group_details),
        "go_sweep_count": sum(detail.get("go_sweep_count", 0) for detail in group_details),
        "go_order_groups": len(go_groups),
        "rust_order_helper_replays": len(rust_events),
        "go_node_counts": [len(group["node_ids"]) for group in go_groups],
        "rust_node_counts": [len(mapping) for mapping in rust_maps],
    }


def compare_fixture(go: Path, rust: Path, fixture: Path, timeout: float) -> dict:
    with tempfile.TemporaryDirectory(prefix="d2-rs-dagre-order-") as temporary:
        temp = Path(temporary)
        outputs = {}
        traces = {}
        for engine, binary in (("go", go), ("rust", rust)):
            environment = {key: value for key, value in os.environ.items() if key not in TRACE_FLAGS}
            if engine == "go":
                environment.update({"DAGRO_TRACE_ORDER": "1", "DAGRE_TRACE_OBJECTS": "1"})
            else:
                environment.update({"DAGRE_DEBUG_PHASE": "1", "DAGRE_ORDER_TRACE": "1"})
            output = temp / f"{engine}.svg"
            try:
                completed = subprocess.run(
                    [str(binary), "--layout=dagre", str(fixture), str(output)],
                    cwd=ROOT,
                    env=environment,
                    capture_output=True,
                    timeout=timeout,
                )
            except subprocess.TimeoutExpired:
                return {"fixture": fixture.name, "status": "failed", "failure": f"{engine} timed out"}
            outputs[engine] = {
                "exit_code": completed.returncode,
                "stdout_sha256": hashlib.sha256(completed.stdout).hexdigest(),
                "svg_sha256": sha256(output) if output.is_file() else None,
            }
            traces[engine] = completed.stderr.decode("utf-8", errors="replace")
            if completed.returncode != 0:
                return {
                    "fixture": fixture.name,
                    "status": "failed",
                    "failure": f"{engine} exited {completed.returncode}",
                    "stderr": traces[engine][-2000:],
                    "runs": outputs,
                }

    try:
        go_groups = parse_go_trace(traces["go"])
        rust_events, rust_maps, rust_external_ids = parse_rust_trace(traces["rust"])
    except (TraceError, json.JSONDecodeError) as error:
        return {"fixture": fixture.name, "status": "failed", "failure": str(error), "runs": outputs}
    comparison = compare_trace_groups(go_groups, rust_events, rust_maps, rust_external_ids)
    return {
        "fixture": fixture.name,
        "runs": outputs,
        **comparison,
    }


def resolve_fixture(value: str) -> Path:
    path = Path(value)
    if path.is_file():
        return path.resolve()
    for directory in (ROOT / "tests/d2", pristine_e2e()):
        candidate = directory / (value if value.endswith(".d2") else f"{value}.d2")
        if candidate.is_file():
            return candidate.resolve()
    raise FileNotFoundError(f"no Dagre fixture named {value!r} or file at {value!r}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixtures", nargs="*", default=list(DEFAULT_FIXTURES))
    parser.add_argument("--go", type=Path, default=ROOT / ".reference/bin/d2-v0.9.0-dagre-trace")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument("--output", type=Path, default=ROOT / "oracle-cache/reports/dagre-order-trace-20260922.json")
    args = parser.parse_args()
    try:
        fixtures = [resolve_fixture(name) for name in args.fixtures]
    except FileNotFoundError as error:
        parser.error(str(error))
    try:
        binaries = {"go": args.go.resolve(), "rust": args.rust.resolve()}
        provenance = {engine: {"path": str(path), "sha256": sha256(path)} for engine, path in binaries.items()}
    except OSError as error:
        parser.error(f"could not read executable: {error}")
    results = [compare_fixture(binaries["go"], binaries["rust"], fixture, args.timeout) for fixture in fixtures]
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-dagre-order-sweep-trace",
        "source_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "identity": "invocations aligned by stable authored-object scope; within each invocation, exact node registration ordinals and original objects cross-checked via Go ObjectMapper ID and Rust external_id",
        "fields": ["phase", "iteration", "crossing-cost IEEE-754 bits", "every ordered layer node"],
        "scope": "compound and ordinary Dagre ordering sweeps; this is not the full Dagre execution trace",
        "binaries": provenance,
        "results": results,
        "matched": sum(row["status"] == "matched" for row in results),
        "inapplicable": sum(row["status"] == "inapplicable" for row in results),
        "divergent": sum(row["status"] == "divergent" for row in results),
        "failed": sum(row["status"] == "failed" for row in results),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(
        f"Dagre order trace: {report['matched']} matched, {report['inapplicable']} inapplicable, "
        f"{report['divergent']} divergent, {report['failed']} failed; wrote {args.output}"
    )
    return int(bool(report["divergent"] or report["failed"]))


if __name__ == "__main__":
    raise SystemExit(main())
