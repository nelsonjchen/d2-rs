#!/usr/bin/env python3
"""Compare layout boundary snapshots, not operation-level trace parity.

Each selected phase must occur exactly once, in order, for these single-root
fixtures. All object identities and x/y/width/height IEEE-754 bits are compared.
No coordinate rounding, object filtering, or float normalization is performed.
With --routes, explicit EDGESET markers and all edge identities, edge order,
point counts, point order and x/y IEEE-754 bits are also required and compared.
With --final, also require the final snapshot after route curve conversion.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import struct
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURES = ("icon_positions", "sequence-trace-order")
PHASES = (("rank_nodes", "dagro"), ("rank_spacing", "rank-spacing"),
          ("source_cross", "cross-spacing"), ("fit", "fit"))
FIELDS = ("x", "y", "width", "height")
SCOPE = "four boundary snapshots, not operation-level trace parity"
TRACE_FLAGS = ("DAGRE_DEBUG_PHASE", "DAGRE_TRACE_OBJECTS", "DAGRO_TRACE_RANKS", "DAGRO_TRACE_ORDER", "DAGRO_TRACE_POS")


class TraceError(ValueError):
    pass


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def phase_pairs(include_final: bool = False, rust_rank_phase: str = "rank_nodes") -> tuple[tuple[str, str], ...]:
    phases = ((rust_rank_phase, "dagro"), *PHASES[1:])
    return (*phases, ("final", "final")) if include_final else phases


def comparison_scope(include_final: bool = False) -> str:
    return "five boundary snapshots, not operation-level trace parity" if include_final else SCOPE


def quoted_string(text: str, context: str) -> tuple[str, str]:
    try:
        value, end = json.JSONDecoder().raw_decode(text)
    except json.JSONDecodeError as error:
        raise TraceError(f"{context}: invalid JSON quoted identity (unsupported Go/Rust escapes are not normalized): {error.msg}") from error
    if not isinstance(value, str) or not text.startswith('"'):
        raise TraceError(f"{context}: expected a JSON quoted identity")
    if end < len(text) and not text[end].isspace():
        raise TraceError(f"{context}: missing separator after quoted identity")
    return value, text[end:].strip()


def parse_object(text: str, engine: str, context: str) -> tuple[str, dict]:
    identity, remainder = quoted_string(text, context)
    columns = remainder.split(maxsplit=4)
    if len(columns) != 5:
        raise TraceError(f"{context}: expected four coordinates and an exact bit payload")
    displays, tail = columns[:4], columns[4]
    if engine == "go":
        if not tail.startswith("parent="):
            raise TraceError(f"{context}: expected Go parent identity")
        _, tail = quoted_string(tail[len("parent="):], context)
    match = re.fullmatch(r"bits=([0-9a-fA-F]{16}(?:,[0-9a-fA-F]{16}){3})", tail)
    if not match:
        raise TraceError(f"{context}: expected four 16-digit IEEE-754 bit values")
    bits = [int(value, 16) for value in match.group(1).split(",")]
    for field, display, value in zip(FIELDS, displays, bits):
        try:
            numeric = float(display)
        except ValueError as error:
            raise TraceError(f"{context}: invalid {field} coordinate {display!r}") from error
        decoded = struct.unpack(">d", struct.pack(">Q", value))[0]
        # Textual NaNs cannot represent payload/sign, so retain their exact
        # supplied bits while checking that the diagnostic really printed NaN.
        if math.isnan(numeric) and math.isnan(decoded):
            continue
        if struct.unpack(">Q", struct.pack(">d", numeric))[0] != value:
            raise TraceError(f"{context}: {field} decimal text disagrees with its IEEE-754 bits")
    return identity, {"bits": bits, "display": displays}


def parse_trace(stderr: str, engine: str, *, include_final: bool = False,
                rust_rank_phase: str = "rank_nodes") -> dict[str, dict[str, dict]]:
    if engine not in ("rust", "go"):
        raise ValueError(f"Unknown engine {engine}")
    expected = [pair[engine == "go"] for pair in phase_pairs(include_final, rust_rank_phase)]
    prefix = engine.upper()
    phase_prefix, object_prefix = f"{prefix}_PHASE ", f"{prefix}_OBJ "
    stages = {}
    current = None
    for number, line in enumerate(stderr.splitlines(), 1):
        context = f"{engine} stderr line {number}"
        if line.startswith(phase_prefix):
            current = line[len(phase_prefix):].strip()
            if current not in expected:
                continue
            if current in stages:
                raise TraceError(f"{context}: duplicate required phase {current!r}; only single-root traces are supported")
            stages[current] = {}
        elif line.startswith(object_prefix):
            if current is None:
                raise TraceError(f"{context}: object appeared before its phase")
            if current not in expected:
                continue  # Other explicitly named phases are outside this gate.
            identity, values = parse_object(line[len(object_prefix):], engine, context)
            if identity in stages[current]:
                raise TraceError(f"{context}: duplicate object {identity!r} in phase {current!r}")
            stages[current][identity] = values
    missing = [phase for phase in expected if phase not in stages]
    if missing:
        raise TraceError(f"{engine}: missing required phases: {missing}")
    if list(stages) != expected:
        raise TraceError(f"{engine}: required phases are out of order: {list(stages)}")
    empty = [phase for phase, objects in stages.items() if not objects]
    if empty:
        raise TraceError(f"{engine}: empty required snapshots provide no object evidence: {empty}")
    return stages


def parse_edge(text: str, context: str) -> dict:
    identity, remainder = quoted_string(text, context)
    if not identity:
        raise TraceError(f"{context}: empty canonical edge identity")
    match = re.fullmatch(r"point_count=(0|[1-9][0-9]*) bits=(.*)", remainder)
    if not match:
        raise TraceError(f"{context}: expected point_count=N and an exact route bit payload")
    count, payload = int(match.group(1)), match.group(2)
    points = []
    if payload:
        for point in payload.split(";"):
            if not re.fullmatch(r"[0-9a-fA-F]{16},[0-9a-fA-F]{16}", point):
                raise TraceError(f"{context}: each route point requires two 16-digit IEEE-754 bit values")
            points.append(tuple(int(value, 16) for value in point.split(",")))
    if len(points) != count:
        raise TraceError(f"{context}: point_count={count} disagrees with {len(points)} route points")
    return {"identity": identity, "points": points}


def parse_route_trace(stderr: str, engine: str, *, include_final: bool = False,
                      rust_rank_phase: str = "rank_nodes") -> dict[str, list[dict]]:
    if engine not in ("rust", "go"):
        raise ValueError(f"Unknown engine {engine}")
    expected = [pair[engine == "go"] for pair in phase_pairs(include_final, rust_rank_phase)]
    prefix = engine.upper()
    phase_prefix = f"{prefix}_PHASE "
    edgeset_marker, edge_marker = f"{prefix}_EDGESET", f"{prefix}_EDGE"
    stages, counts, identities = {}, {}, {}
    current = None
    for number, line in enumerate(stderr.splitlines(), 1):
        context = f"{engine} stderr line {number}"
        if line.startswith(phase_prefix):
            current = line[len(phase_prefix):].strip()
            if current not in expected:
                continue
            if current in stages:
                raise TraceError(f"{context}: duplicate required phase {current!r}; only single-root traces are supported")
            stages[current] = []
            identities[current] = set()
        elif line.startswith(edge_marker):
            if current is None:
                raise TraceError(f"{context}: route evidence appeared before its phase")
            if current not in expected:
                continue
            if line.startswith(edgeset_marker):
                if current in counts:
                    raise TraceError(f"{context}: duplicate EDGESET marker in phase {current!r}")
                match = re.fullmatch(r" count=(0|[1-9][0-9]*)", line[len(edgeset_marker):])
                if not match:
                    raise TraceError(f"{context}: expected EDGESET count=N")
                counts[current] = int(match.group(1))
            else:
                if not line.startswith(edge_marker + " "):
                    raise TraceError(f"{context}: malformed EDGE record")
                edge = parse_edge(line[len(edge_marker) + 1:], context)
                if edge["identity"] in identities[current]:
                    raise TraceError(f"{context}: duplicate edge identity {edge['identity']!r} in phase {current!r}")
                identities[current].add(edge["identity"])
                stages[current].append(edge)
    missing = [phase for phase in expected if phase not in stages]
    if missing:
        raise TraceError(f"{engine}: missing required route phases: {missing}")
    if list(stages) != expected:
        raise TraceError(f"{engine}: required route phases are out of order: {list(stages)}")
    for phase, edges in stages.items():
        if phase not in counts:
            raise TraceError(f"{engine}: missing EDGESET marker in phase {phase!r}; absent routes are not evidence of an edgeless graph")
        if counts[phase] != len(edges):
            raise TraceError(f"{engine}: EDGESET count={counts[phase]} disagrees with {len(edges)} edges in phase {phase!r}")
    return stages


def split_pass_traces(stderr: str, engine: str, *, include_final: bool = False,
                      rust_rank_phase: str = "rank_nodes") -> list[str]:
    """Split a diagnostic stream at each complete Dagre pass's first boundary.

    Nested grid orchestration can invoke the Dagre adapter more than once.
    Intermediate Rust-only phases remain inside each slice and are ignored by
    the ordinary single-pass parsers.
    """
    if engine not in ("rust", "go"):
        raise ValueError(f"Unknown engine {engine}")
    expected = [pair[engine == "go"] for pair in phase_pairs(include_final, rust_rank_phase)]
    first_marker = f"{engine.upper()}_PHASE {expected[0]}"
    chunks: list[str] = []
    current: list[str] | None = None
    for line in stderr.splitlines(keepends=True):
        if line.rstrip("\r\n") == first_marker:
            if current is not None:
                chunks.append("".join(current))
            current = [line]
        elif current is not None:
            current.append(line)
    if current is not None:
        chunks.append("".join(current))
    if not chunks:
        # Reuse the strict parser's useful missing-phase diagnostic.
        parse_trace(stderr, engine, include_final=include_final, rust_rank_phase=rust_rank_phase)
        raise TraceError(f"{engine}: no complete Dagre pass found")
    return chunks


def strip_identity_prefix(stages: dict[str, dict[str, dict]], prefix: str) -> dict[str, dict[str, dict]]:
    """Map absolute Rust IDs to source-relative IDs in extracted subgraphs."""
    if not prefix:
        return stages
    normalized = {}
    for phase, objects in stages.items():
        renamed = {}
        for identity, value in objects.items():
            stable_id = identity[len(prefix):] if identity.startswith(prefix) else identity
            if stable_id in renamed:
                raise TraceError(f"identity prefix {prefix!r} collapses multiple objects to {stable_id!r}")
            renamed[stable_id] = value
        normalized[phase] = renamed
    return normalized


def strip_edge_identity_prefix(stages: dict[str, list[dict]], prefix: str) -> dict[str, list[dict]]:
    """Strip an extracted-scope prefix from both endpoints of every route ID.

    Edge identities encode UTF-8 byte lengths, so parse and rebuild that
    framing instead of applying a textual replacement to the whole identity.
    """
    if not prefix:
        return stages
    normalized = {}
    for phase, edges in stages.items():
        renamed = []
        seen = set()
        for edge in edges:
            identity = edge["identity"]
            encoded = identity.encode("utf-8")
            separator = b" -> "
            ordinal_marker = b" #"
            source_length_end = encoded.find(b":")
            if source_length_end < 1:
                raise TraceError(f"malformed canonical edge identity {identity!r}")
            source_length_text = encoded[:source_length_end]
            if not re.fullmatch(rb"(?:0|[1-9][0-9]*)", source_length_text):
                raise TraceError(f"malformed canonical edge identity {identity!r}")
            source_length = int(source_length_text)
            source_start = source_length_end + 1
            source_end = source_start + source_length
            if source_end > len(encoded):
                raise TraceError(f"malformed canonical edge identity {identity!r}")
            if encoded[source_end:source_end + len(separator)] != separator:
                raise TraceError(f"malformed canonical edge identity {identity!r}")
            target_length_start = source_end + len(separator)
            target_length_end = encoded.find(b":", target_length_start)
            if target_length_end < target_length_start + 1:
                raise TraceError(f"malformed canonical edge identity {identity!r}")
            target_length_text = encoded[target_length_start:target_length_end]
            if not re.fullmatch(rb"(?:0|[1-9][0-9]*)", target_length_text):
                raise TraceError(f"malformed canonical edge identity {identity!r}")
            target_length = int(target_length_text)
            target_start = target_length_end + 1
            target_end = target_start + target_length
            if target_end > len(encoded) or encoded[target_end:target_end + len(ordinal_marker)] != ordinal_marker:
                raise TraceError(f"malformed canonical edge identity {identity!r}")
            ordinal = encoded[target_end + len(ordinal_marker):]
            if not re.fullmatch(rb"(?:0|[1-9][0-9]*)", ordinal):
                raise TraceError(f"malformed canonical edge identity {identity!r}")
            try:
                source = encoded[source_start:source_end].decode("utf-8")
                target = encoded[target_start:target_end].decode("utf-8")
            except UnicodeDecodeError as error:
                raise TraceError(f"malformed UTF-8 in canonical edge identity {identity!r}") from error
            source = source.removeprefix(prefix)
            target = target.removeprefix(prefix)
            renamed_identity = (
                f"{len(source.encode('utf-8'))}:{source} -> "
                f"{len(target.encode('utf-8'))}:{target} #{ordinal.decode('ascii')}"
            )
            if renamed_identity in seen:
                raise TraceError(
                    f"identity prefix {prefix!r} collapses multiple route identities in phase {phase!r}"
                )
            seen.add(renamed_identity)
            renamed.append({**edge, "identity": renamed_identity})
        normalized[phase] = renamed
    return normalized


def first_route_difference(rust_edges: list[dict], go_edges: list[dict]) -> dict | None:
    rust_ids = [edge["identity"] for edge in rust_edges]
    go_ids = [edge["identity"] for edge in go_edges]
    if set(rust_ids) != set(go_ids):
        return {"kind": "edge_membership", "missing_in_rust": sorted(set(go_ids) - set(rust_ids)),
                "extra_in_rust": sorted(set(rust_ids) - set(go_ids))}
    for index, (rust_edge, go_edge) in enumerate(zip(rust_edges, go_edges)):
        if rust_edge["identity"] != go_edge["identity"]:
            return {"kind": "edge_order", "edge_index": index,
                    "rust_edge": rust_edge["identity"], "go_edge": go_edge["identity"]}
        identity = go_edge["identity"]
        if len(rust_edge["points"]) != len(go_edge["points"]):
            return {"kind": "route_point_count", "edge": identity, "edge_index": index,
                    "rust_point_count": len(rust_edge["points"]), "go_point_count": len(go_edge["points"])}
        for point_index, (rust_point, go_point) in enumerate(zip(rust_edge["points"], go_edge["points"])):
            for axis, field in enumerate(("x", "y")):
                if rust_point[axis] != go_point[axis]:
                    return {"kind": "route_coordinate_bits", "edge": identity, "edge_index": index,
                            "point_index": point_index, "field": field,
                            "rust_bits": f"{rust_point[axis]:016x}", "go_bits": f"{go_point[axis]:016x}"}
    return None


def first_difference(rust: dict, go: dict, *, rust_routes: dict | None = None, go_routes: dict | None = None,
                     include_final: bool = False, rust_rank_phase: str = "rank_nodes") -> dict | None:
    if (rust_routes is None) != (go_routes is None):
        raise ValueError("Both route snapshots are required for route comparison")
    for rust_phase, go_phase in phase_pairs(include_final, rust_rank_phase):
        rust_objects, go_objects = rust[rust_phase], go[go_phase]
        phase = {"rust_phase": rust_phase, "go_phase": go_phase}
        if rust_objects.keys() != go_objects.keys():
            return {**phase, "kind": "object_membership",
                    "missing_in_rust": sorted(go_objects.keys() - rust_objects.keys()),
                    "extra_in_rust": sorted(rust_objects.keys() - go_objects.keys())}
        for identity, go_object in go_objects.items():
            rust_object = rust_objects[identity]
            for index, field in enumerate(FIELDS):
                if rust_object["bits"][index] != go_object["bits"][index]:
                    return {**phase, "kind": "coordinate_bits", "object": identity, "field": field,
                            "rust_bits": f"{rust_object['bits'][index]:016x}", "go_bits": f"{go_object['bits'][index]:016x}",
                            "rust_display": rust_object["display"][index], "go_display": go_object["display"][index]}
        if rust_routes is not None:
            difference = first_route_difference(rust_routes[rust_phase], go_routes[go_phase])
            if difference:
                return {**phase, **difference}
    return None


def run(binary: Path, fixture: Path, output: Path, engine: str, timeout: float) -> tuple[dict, str]:
    environment = {key: value for key, value in os.environ.items() if key not in TRACE_FLAGS}
    flag = "DAGRE_DEBUG_PHASE" if engine == "rust" else "DAGRE_TRACE_OBJECTS"
    environment[flag] = "1"
    try:
        completed = subprocess.run([str(binary), "--layout=dagre", str(fixture), str(output)],
                                   cwd=ROOT, env=environment, capture_output=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"exit_code": None, "failure": str(error)}, ""
    metadata = {"exit_code": completed.returncode,
                "stdout_sha256": hashlib.sha256(completed.stdout).hexdigest(),
                "stderr_sha256": hashlib.sha256(completed.stderr).hexdigest(), "trace_flag": flag}
    try:
        stderr = completed.stderr.decode("utf-8")
    except UnicodeDecodeError as error:
        metadata["failure"] = f"stderr is not valid UTF-8: {error}"
        return metadata, ""
    if completed.returncode:
        metadata["failure"] = f"process exited {completed.returncode}"
        metadata["stderr"] = stderr
    return metadata, stderr


def compare_fixture(rust_binary: Path, go_binary: Path, fixture: Path, temporary: Path, timeout: float, *, routes: bool = False,
                    include_final: bool = False, rust_pass: int | None = None, go_pass: int | None = None,
                    rust_id_prefix: str = "", rust_rank_phase: str = "rank_nodes") -> dict:
    result = {"fixture": str(fixture), "status": "failed"}
    try:
        result["fixture_sha256"] = digest(fixture)
    except OSError as error:
        return {**result, "reason": f"could not read fixture: {error}"}
    traces = {}
    result["runs"] = {}
    for engine, binary in (("rust", rust_binary), ("go", go_binary)):
        metadata, traces[engine] = run(binary, fixture, temporary / f"{engine}.svg", engine, timeout)
        result["runs"][engine] = metadata
    failures = {engine: metadata["failure"] for engine, metadata in result["runs"].items() if "failure" in metadata}
    if failures:
        return {**result, "reason": "process failure", "failures": failures}
    try:
        if rust_pass is None:
            rust = parse_trace(traces["rust"], "rust", include_final=include_final,
                               rust_rank_phase=rust_rank_phase)
            rust_routes = parse_route_trace(traces["rust"], "rust", include_final=include_final,
                                            rust_rank_phase=rust_rank_phase) if routes else None
            rust_pass_count = 1
            rust_pass_index = None
        else:
            rust_passes = split_pass_traces(traces["rust"], "rust", include_final=include_final,
                                            rust_rank_phase=rust_rank_phase)
            rust_pass_count = len(rust_passes)
            if rust_pass < 0 or rust_pass >= rust_pass_count:
                return {**result, "reason": f"Rust Dagre pass {rust_pass} is outside 0..{rust_pass_count}",
                        "rust_pass_count": rust_pass_count}
            selected_trace = rust_passes[rust_pass]
            rust = parse_trace(selected_trace, "rust", include_final=include_final,
                               rust_rank_phase=rust_rank_phase)
            rust_routes = parse_route_trace(selected_trace, "rust", include_final=include_final,
                                            rust_rank_phase=rust_rank_phase) if routes else None
            rust_pass_index = rust_pass
        if go_pass is None:
            go = parse_trace(traces["go"], "go", include_final=include_final)
            go_routes = parse_route_trace(traces["go"], "go", include_final=include_final) if routes else None
            go_pass_count = 1
            go_pass_index = None
        else:
            go_passes = split_pass_traces(traces["go"], "go", include_final=include_final)
            go_pass_count = len(go_passes)
            if go_pass < 0 or go_pass >= go_pass_count:
                return {**result, "reason": f"Go Dagre pass {go_pass} is outside 0..{go_pass_count}",
                        "go_pass_count": go_pass_count}
            selected_trace = go_passes[go_pass]
            go = parse_trace(selected_trace, "go", include_final=include_final)
            go_routes = parse_route_trace(selected_trace, "go", include_final=include_final) if routes else None
            go_pass_index = go_pass
        rust = strip_identity_prefix(rust, rust_id_prefix)
        if rust_routes is not None:
            rust_routes = strip_edge_identity_prefix(rust_routes, rust_id_prefix)
    except TraceError as error:
        return {**result, "reason": str(error)}
    difference = first_difference(rust, go, rust_routes=rust_routes, go_routes=go_routes,
                                  include_final=include_final, rust_rank_phase=rust_rank_phase)
    return {**result, "status": "divergent" if difference else "matched", "first_difference": difference,
            "rust_pass_count": rust_pass_count, "rust_pass_selected": rust_pass_index,
            "go_pass_count": go_pass_count, "go_pass_selected": go_pass_index,
            "rust_id_prefix_stripped": rust_id_prefix or None,
            "stages": [{"rust_phase": rp, "go_phase": gp, "rust_objects": len(rust[rp]), "go_objects": len(go[gp]),
                        **({"rust_edges": len(rust_routes[rp]), "go_edges": len(go_routes[gp]),
                            "rust_route_points": sum(len(edge["points"]) for edge in rust_routes[rp]),
                            "go_route_points": sum(len(edge["points"]) for edge in go_routes[gp])} if routes else {})}
                       for rp, gp in phase_pairs(include_final, rust_rank_phase)]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixtures", nargs="*", default=list(DEFAULT_FIXTURES), help="fixture names under tests/d2 or explicit .d2 paths")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--go", type=Path, default=ROOT / ".reference/bin/d2-v0.9.0-dagre-trace")
    parser.add_argument("--timeout", type=float, default=60, help="seconds per process invocation")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--summary", action="store_true")
    parser.add_argument("--routes", action="store_true", help="also require and compare every ordered edge and route point in each snapshot")
    parser.add_argument("--final", dest="include_final", action="store_true", help="also require the final object/route snapshot after curve conversion")
    parser.add_argument("--rust-pass", type=int, help="select a zero-based Rust pass when nested orchestration emits repeated stage groups")
    parser.add_argument("--go-pass", type=int, help="select a zero-based Go pass when nested orchestration emits repeated stage groups")
    parser.add_argument("--rust-rank-phase", choices=("rank_nodes", "raw_rank_nodes"), default="rank_nodes",
                        help="choose Rust's Dagre boundary corresponding to Go's dagro snapshot")
    parser.add_argument("--rust-id-prefix", default="", help="strip a known extracted-scope prefix from Rust object IDs before comparison")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    binaries = {"rust": args.rust.resolve(), "go": args.go.resolve()}
    try:
        provenance = {engine: {"path": str(path), "sha256": digest(path)} for engine, path in binaries.items()}
    except OSError as error:
        parser.error(f"could not hash executable: {error}")
    results = []
    with tempfile.TemporaryDirectory(prefix="d2-rs-dagre-stages-") as directory:
        for index, fixture in enumerate(args.fixtures):
            candidate = Path(fixture)
            if not candidate.is_absolute() and candidate.parent == Path("."):
                candidate = ROOT / "tests/d2" / (fixture if candidate.suffix == ".d2" else f"{fixture}.d2")
            temporary = Path(directory) / str(index)
            temporary.mkdir()
            results.append(compare_fixture(binaries["rust"], binaries["go"], candidate.resolve(), temporary, args.timeout,
                                           routes=args.routes, include_final=args.include_final, rust_pass=args.rust_pass,
                                           go_pass=args.go_pass,
                                           rust_id_prefix=args.rust_id_prefix,
                                           rust_rank_phase=args.rust_rank_phase))
    provenance_errors = []
    for engine, path in binaries.items():
        try:
            if digest(path) != provenance[engine]["sha256"]:
                provenance_errors.append(f"{engine} executable changed during comparison")
        except OSError as error:
            provenance_errors.append(f"{engine} executable unreadable after comparison: {error}")
    scope = comparison_scope(args.include_final)
    report = {"schema_version": 1, "scope": scope, "kind": "d2-v0.9.0-dagre-boundary-bits",
              "binaries": provenance, "timeout_seconds": args.timeout,
              "routes_checked": args.routes,
              "final_checked": args.include_final,
              "edge_identity": "opaque JSON-quoted UTF-8 byte-length-prefixed identity: len(src):src -> len(dst):dst #ordinal; authored endpoint direction, zero-based ordinal per ordered endpoint pair" if args.routes else None,
              "phase_mapping": [{"rust": rp, "go": gp} for rp, gp in phase_pairs(args.include_final, args.rust_rank_phase)],
              "rust_pass_selected": args.rust_pass,
              "go_pass_selected": args.go_pass,
              "rust_rank_phase": args.rust_rank_phase,
              "rust_id_prefix_stripped": args.rust_id_prefix or None,
              "provenance_errors": provenance_errors, "results": results,
              **{status: sum(row["status"] == status for row in results) for status in ("matched", "divergent", "failed")}}
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(rendered)
    if args.summary:
        print(f"Dagre boundary bits: {report['matched']} matched, {report['divergent']} divergent, {report['failed']} failed"
              + (f", {len(provenance_errors)} provenance failures" if provenance_errors else "")
              + (f"; wrote {args.output}" if args.output else "")
              + ("; ordered routes checked" if args.routes else "")
              + f" ({scope})")
    else:
        print(rendered, end="")
    return int(bool(report["divergent"] or report["failed"] or provenance_errors))


if __name__ == "__main__":
    raise SystemExit(main())
