#!/usr/bin/env python3
"""Compare raw Dagre Brandes–Köpf compaction coordinates with v0.9.0.

The gate preserves all four raw alignment maps, exact f64 bits, graph
registration identity, and repeated executions. It deliberately compares the
maps before Dagro's global alignment and median-selection stages.
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
    "DAGRE_POS_TRACE",
    "DAGRE_TRACE_OBJECTS",
    "DAGRO_TRACE_ORDER",
    "DAGRO_TRACE_RANKS",
    "DAGRO_TRACE_POS",
)
ALIGNMENTS = ("ul", "ur", "dl", "dr")


class TraceError(ValueError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_go_trace(stderr: str) -> tuple[
    dict[str, dict[int, str]],
    dict[str, dict[int, int]],
    dict[str, set[tuple[int, int]]],
    dict[int, str],
    dict[str, str],
]:
    go_nodes: dict[int, str] = {}
    go_positions: dict[str, dict[int, str]] = {alignment: {} for alignment in ALIGNMENTS}
    go_roots: dict[str, dict[int, int]] = {alignment: {} for alignment in ALIGNMENTS}
    go_root_events: list[tuple[str, str, str]] = []
    go_conflict_events: list[tuple[str, str, str]] = []
    go_conflict_counts: dict[str, int] = {}
    go_objects: dict[str, str] = {}
    phase = ""
    node_pattern = re.compile(r"GO_DAGRO_NODE index=(\d+) id=(\"(?:[^\"\\]|\\.)*\")")
    position_pattern = re.compile(
        r"GO_POS alignment=(ul|ur|dl|dr) id=(\"(?:[^\"\\]|\\.)*\") "
        r"x=(?:[^ ]+) x_bits=([0-9a-f]{16})"
    )
    root_pattern = re.compile(
        r"GO_BK_ROOT alignment=(ul|ur|dl|dr) id=(\"(?:[^\"\\]|\\.)*\") "
        r"root=(\"(?:[^\"\\]|\\.)*\")"
    )
    conflict_pattern = re.compile(
        r"GO_BK_CONFLICT kind=(type1|type2) first=(\"(?:[^\"\\]|\\.)*\") "
        r"second=(\"(?:[^\"\\]|\\.)*\")"
    )
    conflict_end_pattern = re.compile(r"GO_BK_CONFLICT_END kind=(type1|type2) count=(\d+)")
    object_pattern = re.compile(r'GO_MAP id=("(?:[^"\\]|\\.)*") object=("(?:[^"\\]|\\.)*")')
    for line in stderr.splitlines():
        if line.startswith("GO_MAP "):
            match = object_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Go object identity: {line[:180]}")
            encoded_id, encoded_object = match.groups()
            object_id, external_id = json.loads(encoded_id), json.loads(encoded_object)
            if object_id in go_objects and go_objects[object_id] != external_id:
                raise TraceError(f"Go object map ID {object_id!r} changed identity")
            go_objects[object_id] = external_id
            continue
        if line.startswith("GO_DAGRO_PHASE "):
            phase = line.removeprefix("GO_DAGRO_PHASE ")
            continue
        if line.startswith("GO_DAGRO_NODE ") and phase == "position":
            match = node_pattern.match(line)
            if not match:
                raise TraceError(f"malformed Go node registration: {line[:180]}")
            index, encoded_id = match.groups()
            index = int(index)
            identity = json.loads(encoded_id)
            if index in go_nodes and go_nodes[index] != identity:
                raise TraceError(f"Go position registration index {index} changed identity")
            go_nodes[index] = identity
            continue
        if line.startswith("GO_POS "):
            match = position_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Go position event: {line[:180]}")
            alignment, encoded_id, bits = match.groups()
            identity = json.loads(encoded_id)
            # The position-phase registration map is emitted immediately after
            # positionX completes; defer ID normalization until it is parsed.
            go_positions[alignment][identity] = bits  # type: ignore[index]
        if line.startswith("GO_BK_ROOT_ERROR "):
            raise TraceError(f"Go BK root referenced an unregistered node: {line}")
        if line.startswith("GO_BK_ROOT "):
            match = root_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Go BK root event: {line[:180]}")
            alignment, encoded_id, encoded_root = match.groups()
            identity, root = json.loads(encoded_id), json.loads(encoded_root)
            go_root_events.append((alignment, identity, root))
        if line.startswith("GO_BK_CONFLICT_END "):
            match = conflict_end_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Go BK conflict-set end: {line}")
            kind, count = match.groups()
            go_conflict_counts[kind] = int(count)
        elif line.startswith("GO_BK_CONFLICT "):
            match = conflict_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Go BK conflict event: {line[:180]}")
            kind, encoded_first, encoded_second = match.groups()
            go_conflict_events.append(
                (kind, json.loads(encoded_first), json.loads(encoded_second))
            )

    if not go_nodes or sorted(go_nodes) != list(range(len(go_nodes))):
        raise TraceError("Go did not emit a contiguous post-position registration map")
    if not go_objects:
        raise TraceError("Go did not emit D2 ObjectMapper identities")
    index_by_id = {identity: index for index, identity in go_nodes.items()}
    for alignment, identity, root in go_root_events:
        if identity not in index_by_id or root not in index_by_id:
            raise TraceError("Go BK root referenced a node missing from the post-position map")
        index, root_index = index_by_id[identity], index_by_id[root]
        go_roots[alignment][index] = root_index
    if any(not values for values in go_positions.values()):
        raise TraceError("Go position trace did not emit all four alignments")
    if any(not values for values in go_roots.values()):
        raise TraceError("Go BK trace did not emit all four root maps")
    index_by_id = {identity: index for index, identity in go_nodes.items()}
    if set(go_conflict_counts) != {"type1", "type2"}:
        raise TraceError("Go BK trace did not delimit both conflict sets")
    go_conflicts: dict[str, set[tuple[int, int]]] = {"type1": set(), "type2": set()}
    for kind, first, second in go_conflict_events:
        if first not in index_by_id or second not in index_by_id:
            raise TraceError("Go BK conflict references a node missing from the post-position map")
        pair = tuple(sorted((index_by_id[first], index_by_id[second])))
        go_conflicts[kind].add(pair)
    if any(go_conflict_counts[kind] != len(go_conflicts[kind]) for kind in go_conflicts):
        raise TraceError("Go BK conflict-set event count does not match its membership records")
    normalized: dict[str, dict[int, str]] = {}
    for alignment, values in go_positions.items():
        try:
            normalized[alignment] = {index_by_id[identity]: bits for identity, bits in values.items()}
        except KeyError as error:
            raise TraceError(f"Go position references unregistered node {error.args[0]!r}") from error
    return normalized, go_roots, go_conflicts, go_nodes, go_objects


def parse_rust_trace(
    stderr: str,
) -> tuple[
    list[dict[str, dict[int, str]]],
    list[dict[str, dict[int, int]]],
    list[dict[str, set[tuple[int, int]]]],
    list[int],
    list[str | None],
]:
    registrations: dict[int, int] = {}
    external_id_registrations: dict[int, str | None] = {}
    groups: list[dict[str, dict[int, str]]] = []
    root_groups: list[dict[str, dict[int, int]]] = []
    conflict_groups: list[dict[str, set[tuple[int, int]]]] = []
    current: dict[str, dict[int, str]] | None = None
    current_roots: dict[str, dict[int, int]] | None = None
    current_conflicts: dict[str, set[tuple[int, int]]] | None = None
    current_conflict_counts: dict[str, int] = {}
    last_alignment: str | None = None
    last_root_alignment: str | None = None
    node_pattern = re.compile(r"RUST_DAGRE_ORDER_NODE index=(\d+) id=(\d+)")
    object_pattern = re.compile(
        r'RUST_DAGRE_OBJECT index=(\d+) external_id=(null|"(?:[^"\\]|\\.)*")'
    )
    position_pattern = re.compile(
        r"RUST_DAGRE_POS alignment=(ul|ur|dl|dr) index=(\d+) id=(\d+) "
        r"x_bits=([0-9a-f]{16})"
    )
    root_pattern = re.compile(
        r"RUST_BK_ROOT alignment=(ul|ur|dl|dr) index=(\d+) id=(\d+) "
        r"root_index=(\d+) root=(\d+)"
    )
    conflict_pattern = re.compile(
        r"RUST_BK_CONFLICT kind=(type1|type2) first_index=(\d+) second_index=(\d+)"
    )
    conflict_end_pattern = re.compile(r"RUST_BK_CONFLICT_END kind=(type1|type2) count=(\d+)")

    def start_conflict_pass(kind: str) -> None:
        nonlocal current_conflicts, current_conflict_counts
        if kind == "type1" and current_conflicts is not None and set(current_conflict_counts) == {
            "type1",
            "type2",
        }:
            conflict_groups.append(current_conflicts)
            current_conflicts = None
            current_conflict_counts = {}
        if current_conflicts is None:
            current_conflicts = {"type1": set(), "type2": set()}
    for line in stderr.splitlines():
        if line.startswith("RUST_DAGRE_ORDER_NODE "):
            match = node_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Rust node registration: {line}")
            index, identity = map(int, match.groups())
            if index != len(registrations):
                raise TraceError(f"Rust registration indexes are not contiguous at {index}")
            registrations[index] = identity
            continue
        if line.startswith("RUST_DAGRE_OBJECT "):
            match = object_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Rust object identity: {line}")
            index_text, encoded_external_id = match.groups()
            index = int(index_text)
            if index not in registrations:
                raise TraceError(f"Rust object identity references unregistered index {index}")
            external_id_registrations[index] = (
                None if encoded_external_id == "null" else json.loads(encoded_external_id)
            )
            continue
        if line.startswith("RUST_BK_CONFLICT_END "):
            match = conflict_end_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Rust BK conflict-set end: {line}")
            kind, count_text = match.groups()
            start_conflict_pass(kind)
            count = int(count_text)
            if count != len(current_conflicts[kind]):
                raise TraceError(f"Rust {kind} conflict count disagrees with membership events")
            if kind in current_conflict_counts:
                raise TraceError(f"Rust emitted duplicate {kind} conflict-set end")
            current_conflict_counts[kind] = count
            continue
        if line.startswith("RUST_BK_CONFLICT "):
            match = conflict_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Rust BK conflict event: {line[:180]}")
            kind, first_text, second_text = match.groups()
            first, second = int(first_text), int(second_text)
            start_conflict_pass(kind)
            if first not in range(len(registrations)) or second not in range(len(registrations)):
                raise TraceError("Rust BK conflict references an unknown registration index")
            pair = tuple(sorted((first, second)))
            if pair in current_conflicts[kind]:
                raise TraceError(f"Rust emitted duplicate {kind} conflict pair {pair}")
            current_conflicts[kind].add(pair)
            continue
        if not line.startswith("RUST_DAGRE_POS "):
            if not line.startswith("RUST_BK_ROOT "):
                continue
            match = root_pattern.fullmatch(line)
            if not match:
                raise TraceError(f"malformed Rust BK root event: {line[:180]}")
            alignment, index_text, identity_text, root_index_text, root_text = match.groups()
            index, identity = int(index_text), int(identity_text)
            root_index, root_identity = int(root_index_text), int(root_text)
            if registrations.get(index) != identity or registrations.get(root_index) != root_identity:
                raise TraceError(f"Rust BK root index {index}->{root_index} does not match registration")
            if alignment == "ul" and current_roots is not None and last_root_alignment == "dr":
                root_groups.append(current_roots)
                current_roots = None
            if current_roots is None:
                current_roots = {name: {} for name in ALIGNMENTS}
            if index in current_roots[alignment]:
                raise TraceError(f"Rust emitted duplicate {alignment} root for index {index}")
            current_roots[alignment][index] = root_index
            last_root_alignment = alignment
            continue
        match = position_pattern.fullmatch(line)
        if not match:
            raise TraceError(f"malformed Rust position event: {line[:180]}")
        alignment, index_text, identity_text, bits = match.groups()
        index, identity = int(index_text), int(identity_text)
        if registrations.get(index) != identity:
            raise TraceError(f"Rust position index {index} does not match its registered node")
        if alignment == "ul":
            if current is not None and last_alignment == "dr":
                groups.append(current)
                current = None
        if current is None:
            current = {name: {} for name in ALIGNMENTS}
        if index in current[alignment]:
            raise TraceError(f"Rust emitted duplicate {alignment} coordinate for index {index}")
        current[alignment][index] = bits
        last_alignment = alignment
    if current is not None:
        groups.append(current)
    if current_roots is not None:
        root_groups.append(current_roots)
    if current_conflicts is not None:
        if set(current_conflict_counts) != {"type1", "type2"}:
            raise TraceError("Rust BK conflict trace is missing a set terminator")
        conflict_groups.append(current_conflicts)
    if (
        not registrations
        or not groups
        or not root_groups
        or not conflict_groups
        or set(external_id_registrations) != set(registrations)
    ):
        raise TraceError("Rust did not emit a complete registration map and BK trace")
    if any(set(group) != set(ALIGNMENTS) for group in groups):
        raise TraceError("Rust position trace is missing an alignment")
    if any(set(group) != set(ALIGNMENTS) for group in root_groups):
        raise TraceError("Rust BK root trace is missing an alignment")
    return (
        groups,
        root_groups,
        conflict_groups,
        [registrations[index] for index in range(len(registrations))],
        [external_id_registrations[index] for index in range(len(registrations))],
    )


def split_trace_groups(stderr: str, start_prefix: str) -> list[str]:
    """Split one D2 run into independent Dagre graph invocations."""
    groups: list[list[str]] = []
    current: list[str] | None = None
    for line in stderr.splitlines():
        if line.startswith(start_prefix):
            if current is not None:
                groups.append(current)
            current = [line]
        elif current is not None:
            current.append(line)
    if current is not None:
        groups.append(current)
    return ["\n".join(group) for group in groups]


def first_difference(
    expected: dict[str, dict[int, str]], actual: dict[str, dict[int, str]]
) -> dict | None:
    for alignment in ALIGNMENTS:
        expected_nodes = expected[alignment]
        actual_nodes = actual[alignment]
        if expected_nodes.keys() != actual_nodes.keys():
            differing = sorted(expected_nodes.keys() ^ actual_nodes.keys())
            index = differing[0] if differing else None
            return {
                "alignment": alignment,
                "index": index,
                "go_bits": expected_nodes.get(index) if index is not None else None,
                "rust_bits": actual_nodes.get(index) if index is not None else None,
                "reason": "node inventory differs",
            }
        for index in sorted(expected_nodes):
            if expected_nodes[index] != actual_nodes[index]:
                return {
                    "alignment": alignment,
                    "index": index,
                    "go_bits": expected_nodes[index],
                    "rust_bits": actual_nodes[index],
                    "reason": "coordinate bits differ",
                }
    return None


def first_root_difference(
    expected: dict[str, dict[int, int]], actual: dict[str, dict[int, int]]
) -> dict | None:
    for alignment in ALIGNMENTS:
        if expected[alignment].keys() != actual[alignment].keys():
            differing = sorted(expected[alignment].keys() ^ actual[alignment].keys())
            index = differing[0] if differing else None
            return {
                "alignment": alignment,
                "index": index,
                "go_root_index": expected[alignment].get(index) if index is not None else None,
                "rust_root_index": actual[alignment].get(index) if index is not None else None,
                "reason": "root node inventory differs",
            }
        for index in sorted(expected[alignment]):
            if expected[alignment][index] != actual[alignment][index]:
                return {
                    "alignment": alignment,
                    "index": index,
                    "go_root_index": expected[alignment][index],
                    "rust_root_index": actual[alignment][index],
                    "reason": "root assignment differs",
                }
    return None


def compare_fixture(go: Path, rust: Path, fixture: str, timeout: float) -> dict:
    explicit_source = Path(fixture).expanduser()
    source = (
        explicit_source.resolve()
        if explicit_source.is_file()
        else ROOT / "tests/d2" / f"{fixture.removesuffix('.d2')}.d2"
    )
    if not source.is_file():
        return {"fixture": source.name, "status": "failed", "failure": "fixture does not exist"}
    with tempfile.TemporaryDirectory(prefix="d2-rs-bk-trace-") as temporary:
        outputs = {}
        stderr_by_engine = {}
        for engine, binary in (("go", go), ("rust", rust)):
            environment = {key: value for key, value in os.environ.items() if key not in TRACE_FLAGS}
            if engine == "go":
                environment.update({
                    "DAGRO_TRACE_ORDER": "1",
                    "DAGRO_TRACE_POS": "1",
                    "DAGRE_TRACE_OBJECTS": "1",
                })
            else:
                environment.update({
                    "DAGRE_DEBUG_PHASE": "1",
                    "DAGRE_ORDER_TRACE": "1",
                    "DAGRE_POS_TRACE": "1",
                })
            output = Path(temporary) / f"{engine}.svg"
            try:
                completed = subprocess.run(
                    [str(binary), "--layout=dagre", str(source), str(output)],
                    cwd=ROOT,
                    env=environment,
                    capture_output=True,
                    timeout=timeout,
                    check=False,
                )
            except subprocess.TimeoutExpired:
                return {"fixture": source.name, "status": "failed", "failure": f"{engine} timed out"}
            outputs[engine] = {
                "exit_code": completed.returncode,
                "stdout_sha256": hashlib.sha256(completed.stdout).hexdigest(),
                "svg_sha256": sha256(output) if output.is_file() else None,
            }
            stderr_by_engine[engine] = completed.stderr.decode("utf-8", errors="replace")
            if completed.returncode != 0:
                return {
                    "fixture": source.name,
                    "status": "failed",
                    "failure": f"{engine} exited {completed.returncode}",
                    "stderr_tail": stderr_by_engine[engine][-2000:],
                    "runs": outputs,
                }

    try:
        go_traces = [
            parse_go_trace(group)
            for group in split_trace_groups(stderr_by_engine["go"], 'GO_MAP id="0"')
        ]
        rust_traces = [
            parse_rust_trace(group)
            for group in split_trace_groups(
                stderr_by_engine["rust"], "RUST_DAGRE_ORDER_NODE index=0 "
            )
        ]
        if not go_traces or not rust_traces:
            raise TraceError("one engine emitted no complete Dagre position trace group")
    except (TraceError, json.JSONDecodeError) as error:
        return {
            "fixture": source.name,
            "status": "failed",
            "failure": str(error),
            "runs": outputs,
        }

    trace_groups = []
    for trace_index in range(max(len(go_traces), len(rust_traces))):
        if trace_index >= len(go_traces) or trace_index >= len(rust_traces):
            trace_groups.append({"trace_group": trace_index, "exact": False, "reason": "trace group count differs"})
            continue
        go_positions, go_roots, go_conflicts, go_registration, go_objects = go_traces[trace_index]
        (
            rust_groups,
            rust_root_groups,
            rust_conflict_groups,
            rust_registration,
            rust_external_ids,
        ) = rust_traces[trace_index]
        registration_counts_match = len(go_registration) == len(rust_registration)
        source_object_identities_match = registration_counts_match and all(
            rust_external_ids[index] == go_objects.get(identity)
            for index, identity in go_registration.items()
        )
        replays_match = [
            first_difference(go_positions, group) is None for group in rust_groups
        ]
        root_replays_match = [
            first_root_difference(go_roots, group) is None for group in rust_root_groups
        ]
        conflict_replays_match = [
            group == go_conflicts for group in rust_conflict_groups
        ]
        multiplicity_match = len(rust_groups) == 1
        differences = [
            first_difference(go_positions, group) for group in rust_groups
        ]
        root_differences = [
            first_root_difference(go_roots, group) for group in rust_root_groups
        ]
        group_exact = (
            registration_counts_match
            and source_object_identities_match
            and multiplicity_match
            and len(rust_root_groups) == 1
            and len(rust_conflict_groups) == 1
            and all(replays_match)
            and all(root_replays_match)
            and all(conflict_replays_match)
        )
        trace_groups.append({
            "trace_group": trace_index,
            "exact": group_exact,
            "registration_counts_match": registration_counts_match,
            "source_object_identities_match_registration": source_object_identities_match,
            "registration_node_count": len(rust_registration),
            "position_replays_match_go": replays_match,
            "root_replays_match_go": root_replays_match,
            "conflict_replays_match_go": conflict_replays_match,
            "go_conflict_counts": {kind: len(pairs) for kind, pairs in go_conflicts.items()},
            "rust_conflict_counts": [
                {kind: len(pairs) for kind, pairs in group.items()}
                for group in rust_conflict_groups
            ],
            "first_differences": [
                {"trace_group": trace_index, **difference}
                for difference in differences
                if difference is not None
            ],
            "first_root_differences": [
                {"trace_group": trace_index, **difference}
                for difference in root_differences
                if difference is not None
            ],
        })
    group_count_matches = len(go_traces) == len(rust_traces)
    exact = group_count_matches and all(group["exact"] for group in trace_groups)
    return {
        "fixture": source.name,
        "status": "matched" if exact else "divergent",
        "go_alignment_count": len(ALIGNMENTS),
        "go_trace_group_count": len(go_traces),
        "rust_trace_group_count": len(rust_traces),
        "trace_group_count_matches": group_count_matches,
        "trace_groups": trace_groups,
        "registration_node_count": sum(
            len(trace[3]) for trace in rust_traces
        ),
        "runs": outputs,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixtures", nargs="*", default=list(DEFAULT_FIXTURES))
    parser.add_argument("--go", type=Path, default=ROOT / ".reference/bin/d2-v0.9.0-dagre-trace")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--output", type=Path, default=ROOT / "oracle-cache/reports/dagre-position-trace-20260922.json")
    parser.add_argument("--timeout", type=float, default=60.0)
    args = parser.parse_args()
    args.go = args.go.resolve()
    args.rust = args.rust.resolve()
    results = [compare_fixture(args.go, args.rust, fixture, args.timeout) for fixture in args.fixtures]
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-dagre-brandes-koepf-alignment-and-coordinate-trace",
        "source_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "fields": [
            "four vertical-alignment root maps",
            "type-1 and type-2 conflict sets",
            "four raw coordinate maps",
            "exact f64 bits",
            "node identity",
            "replay multiplicity",
        ],
        "binaries": {
            "go": {"path": str(args.go), "sha256": sha256(args.go)},
            "rust": {"path": str(args.rust), "sha256": sha256(args.rust)},
        },
        "matched": sum(result["status"] == "matched" for result in results),
        "divergent": sum(result["status"] == "divergent" for result in results),
        "failed": sum(result["status"] == "failed" for result in results),
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(
        f"Dagre BK position trace: {report['matched']} matched, "
        f"{report['divergent']} divergent, {report['failed']} failed; wrote {args.output}"
    )
    return 1 if report["divergent"] or report["failed"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
