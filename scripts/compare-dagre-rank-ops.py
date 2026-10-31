#!/usr/bin/env python3
"""Compare detailed Dagro network-simplex operation traces.

This gate compares the pinned Go 1.27.0 D2 v0.9.0 source trace build with the
diagnostic Rust CLI. It requires identical event counts and exact JSON payloads
in event order after stable object-ID translation; float values are
represented by IEEE-754 bit patterns. Both-empty traces are explicitly
inapplicable because neither engine ran network simplex for that fixture.
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
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
GO_PREFIX = "GO_RANK_OP "
RUST_PREFIX = "RUST_RANK_OP "
GO_TRACE_FLAG = "DAGRO_TRACE_RANK_OPS"
RUST_TRACE_FLAG = "DAGRE_TRACE_RANK_OPS"
GO_MAP_PREFIX = "GO_MAP "
GO_MAP_PATTERN = re.compile(r'^GO_MAP id=("(?:\\.|[^"\\])*") object=("(?:\\.|[^"\\])*")$')


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def resolve_fixture(value: str) -> Path:
    path = Path(value)
    if path.is_file():
        return path.resolve()
    fixture = ROOT / "tests/d2" / f"{value}.d2"
    if fixture.is_file():
        return fixture
    raise FileNotFoundError(f"no D2 fixture named {value!r} or file at {value!r}")


def trace_events(stderr: str, prefix: str, fixture: Path, engine: str) -> list[dict[str, Any]]:
    events = []
    for line_number, line in enumerate(stderr.splitlines(), 1):
        if not line.startswith(prefix):
            continue
        try:
            event = json.loads(line[len(prefix):])
        except json.JSONDecodeError as error:
            raise ValueError(f"{engine} trace for {fixture} line {line_number} is invalid JSON: {error}") from error
        if not isinstance(event, dict):
            raise ValueError(f"{engine} trace for {fixture} line {line_number} is not an event object")
        events.append(event)
    return events


def parse_go_object_map(stderr: str, fixture: Path) -> dict[str, str]:
    """Read the adapter's numeric Dagro IDs and their stable D2 object paths."""
    objects: dict[str, str] = {}
    for line_number, line in enumerate(stderr.splitlines(), 1):
        if not line.startswith(GO_MAP_PREFIX):
            continue
        match = GO_MAP_PATTERN.fullmatch(line)
        if match is None:
            raise ValueError(f"Go object map for {fixture} line {line_number} is malformed")
        mapper_id, object_id = (json.loads(value) for value in match.groups())
        if mapper_id in objects:
            raise ValueError(f"Go object map for {fixture} repeats mapper ID {mapper_id!r}")
        objects[mapper_id] = object_id
    return objects


def go_trace_events(stderr: str, fixture: Path) -> list[dict[str, Any]]:
    """Normalize Go events against the mapper table for each layout call.

    A D2 compilation can invoke Dagro more than once. The adapter constructs a
    fresh ObjectMapper for every invocation, so mapper IDs restart at zero;
    the next repeated ID begins a new scope.
    """
    object_map: dict[str, str] = {}
    events: list[dict[str, Any]] = []
    for line_number, line in enumerate(stderr.splitlines(), 1):
        if line.startswith(GO_MAP_PREFIX):
            match = GO_MAP_PATTERN.fullmatch(line)
            if match is None:
                raise ValueError(f"Go object map for {fixture} line {line_number} is malformed")
            mapper_id, object_id = (json.loads(value) for value in match.groups())
            if mapper_id in object_map:
                object_map = {}
            object_map[mapper_id] = object_id
            continue
        if not line.startswith(GO_PREFIX):
            continue
        try:
            event = json.loads(line[len(GO_PREFIX):])
        except json.JSONDecodeError as error:
            raise ValueError(f"Go trace for {fixture} line {line_number} is invalid JSON: {error}") from error
        if not isinstance(event, dict):
            raise ValueError(f"Go trace for {fixture} line {line_number} is not an event object")
        events.append(normalize_go_event_ids(event, object_map, fixture))
    return events


def normalize_go_event_ids(
    value: Any, object_map: dict[str, str], fixture: Path, *, path: str = "$"
) -> Any:
    """Replace Go's numeric object aliases recursively, preserving event order."""
    if isinstance(value, dict):
        normalized = {
            key: normalize_go_event_ids(item, object_map, fixture, path=f"{path}.{key}")
            for key, item in value.items()
        }
        if path == "$":
            # Go serializes undirected tree/leave endpoints in mapper-ID
            # lexical order. Renaming those IDs can reverse the pair, so
            # compare by stable object identity after translation. Directed
            # constraints/candidates remain untouched.
            for pair in [*normalized.get("tree", []), normalized.get("leave")]:
                if isinstance(pair, dict) and pair.get("target", "") < pair.get("source", ""):
                    pair["source"], pair["target"] = pair["target"], pair["source"]
        return normalized
    if isinstance(value, list):
        return [normalize_go_event_ids(item, object_map, fixture, path=f"{path}[{index}]")
                for index, item in enumerate(value)]
    if isinstance(value, str) and value.startswith("object:"):
        mapper_id = value.removeprefix("object:")
        if mapper_id.isdecimal():
            try:
                return f"object:{object_map[mapper_id]}"
            except KeyError as error:
                raise ValueError(
                    f"Go trace for {fixture} references unmapped Dagro object ID {mapper_id!r} at {path}"
                ) from error
    return value


def execute(binary: Path, fixture: Path, engine: str, temp_dir: Path) -> dict[str, Any]:
    environment = {key: value for key, value in os.environ.items()
                   if key not in (GO_TRACE_FLAG, RUST_TRACE_FLAG, "DAGRE_DEBUG_PHASE",
                                  "DAGRE_TRACE_OBJECTS", "DAGRO_TRACE_RANKS",
                                  "DAGRO_TRACE_ORDER", "DAGRO_TRACE_POS")}
    environment[GO_TRACE_FLAG if engine == "go" else RUST_TRACE_FLAG] = "1"
    if engine == "go":
        environment["DAGRE_TRACE_OBJECTS"] = "1"
    output = temp_dir / f"{fixture.stem}-{engine}.svg"
    command = [str(binary), "--layout=dagre", str(fixture), str(output)]
    completed = subprocess.run(command, cwd=ROOT, env=environment, capture_output=True, text=True,
                               timeout=180)
    prefix = GO_PREFIX if engine == "go" else RUST_PREFIX
    events = (go_trace_events(completed.stderr, fixture) if engine == "go"
              else trace_events(completed.stderr, prefix, fixture, engine))
    return {
        "command": command,
        "exit_code": completed.returncode,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
        "events": events,
        "output_sha256": digest(output) if output.is_file() else None,
    }


def first_difference(go_events: list[dict[str, Any]], rust_events: list[dict[str, Any]]) -> dict[str, Any] | None:
    for index, (go_event, rust_event) in enumerate(zip(go_events, rust_events)):
        if go_event != rust_event:
            return {"index": index, "go": go_event, "rust": rust_event}
    if len(go_events) != len(rust_events):
        index = min(len(go_events), len(rust_events))
        return {
            "index": index,
            "go": go_events[index] if index < len(go_events) else None,
            "rust": rust_events[index] if index < len(rust_events) else None,
        }
    return None


def comparison_status(
    go_exit_code: int,
    rust_exit_code: int,
    go_events: list[dict[str, Any]],
    rust_events: list[dict[str, Any]],
    difference: dict[str, Any] | None,
) -> str:
    if go_exit_code != 0 or rust_exit_code != 0:
        return "failed"
    if difference is not None:
        return "divergent"
    if not go_events and not rust_events:
        return "inapplicable"
    return "matched"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixtures", nargs="+", help="fixture names in tests/d2 or explicit .d2 paths")
    parser.add_argument("--go", type=Path, default=ROOT / ".reference/bin/d2-v0.9.0-dagre-trace")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--output", type=Path, help="write a JSON comparison report")
    parser.add_argument("--summary", action="store_true")
    args = parser.parse_args()
    go = args.go.resolve()
    rust = args.rust.resolve()
    for binary in (go, rust):
        if not binary.is_file():
            parser.error(f"executable does not exist: {binary}")

    fixture_results = []
    with tempfile.TemporaryDirectory(prefix="d2-rank-ops-") as temp_name:
        temp_dir = Path(temp_name)
        for item in args.fixtures:
            fixture = resolve_fixture(item)
            go_result = execute(go, fixture, "go", temp_dir)
            rust_result = execute(rust, fixture, "rust", temp_dir)
            difference = first_difference(go_result["events"], rust_result["events"])
            status = comparison_status(go_result["exit_code"], rust_result["exit_code"],
                                       go_result["events"], rust_result["events"], difference)
            passed = status in ("matched", "inapplicable")
            fixture_results.append({
                "fixture": str(fixture.relative_to(ROOT)) if fixture.is_relative_to(ROOT) else str(fixture),
                "fixture_sha256": digest(fixture),
                "status": status,
                "reason": ("Neither engine emitted a network-simplex operation for this fixture."
                          if status == "inapplicable" else None),
                "passed": passed,
                "go_exit_code": go_result["exit_code"],
                "rust_exit_code": rust_result["exit_code"],
                "go_event_count": len(go_result["events"]),
                "rust_event_count": len(rust_result["events"]),
                "first_difference": difference,
                "go_output_sha256": go_result["output_sha256"],
                "rust_output_sha256": rust_result["output_sha256"],
                "go_stderr_tail": go_result["stderr"].splitlines()[-12:],
                "rust_stderr_tail": rust_result["stderr"].splitlines()[-12:],
            })

    report = {
        "schema_version": 1,
        "scope": "network-simplex input, longest-path ranks, feasible tree, low/lim, cut values, selected exchanges, and eligible entering-edge order",
        "go_binary": str(go),
        "go_binary_sha256": digest(go),
        "rust_binary": str(rust),
        "rust_binary_sha256": digest(rust),
        "passed": all(item["passed"] for item in fixture_results),
        "fixtures": fixture_results,
    }
    if args.output:
        destination = args.output if args.output.is_absolute() else ROOT / args.output
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(report, indent=2) + "\n")
    if args.summary:
        counts = {status: sum(item["status"] == status for item in fixture_results)
                  for status in ("matched", "inapplicable", "divergent", "failed")}
        print("Dagre rank operation trace: " + ", ".join(
            f"{counts[status]} {status}" for status in counts if counts[status]
        ))
        for item in fixture_results:
            difference = item["first_difference"]
            where = f" at event {difference['index']}" if difference else ""
            note = f" ({item['reason']})" if item["reason"] else ""
            print(f"  {Path(item['fixture']).name}: Go={item['go_event_count']} Rust={item['rust_event_count']}"
                  f" {item['status']}{where}{note}")
        if args.output:
            print(f"wrote {destination}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
