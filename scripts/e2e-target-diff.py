#!/usr/bin/env python3
"""Diff the Rust target byte stream of one e2e case against its board.exp.json.

Needs a d2-rs CLI built with `--features diagnostic-traces` (the
D2_RS_TRACE_TARGET_BYTES hook). Single-board cases only. Shapes and
connections are matched by id and compared field by field, so the report
names the earliest diverging objects rather than an opaque hash.
"""

from __future__ import annotations

import argparse
import references
import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = ROOT.parent / "d2"


def split_json_stream(text: str) -> list:
    decoder = json.JSONDecoder()
    values, index = [], 0
    while index < len(text):
        value, index = decoder.raw_decode(text, index)
        values.append(value)
    return values


def order_diff(kind: str, expected: list, actual: list) -> str:
    index = next((i for i, (a, b) in enumerate(zip(expected, actual)) if a != b), min(len(expected), len(actual)))
    return (f"{kind} ids/order: first difference at {index} of exp {len(expected)} / rust {len(actual)}: "
            f"exp {expected[index:index + 3]} rust {actual[index:index + 3]}")


def diff(path: str, expected, actual, out: list, limit: int) -> None:
    if len(out) >= limit:
        return
    if isinstance(expected, dict) and isinstance(actual, dict):
        for key in list(expected) + [k for k in actual if k not in expected]:
            if key not in actual:
                out.append(f"{path}.{key}: missing in rust (exp {json.dumps(expected[key])[:120]})")
            elif key not in expected:
                out.append(f"{path}.{key}: extra in rust ({json.dumps(actual[key])[:120]})")
            else:
                diff(f"{path}.{key}", expected[key], actual[key], out, limit)
    elif isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            out.append(f"{path}: len exp {len(expected)} rust {len(actual)}")
        for index, (left, right) in enumerate(zip(expected, actual)):
            diff(f"{path}[{index}]", left, right, out, limit)
    elif expected != actual:
        out.append(f"{path}: exp {json.dumps(expected)[:160]} rust {json.dumps(actual)[:160]}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("name", help="case name, e.g. stable/direction")
    parser.add_argument("--engine", default="dagre")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs-traces")
    parser.add_argument("--cases", type=Path, default=references.E2E_CASES)
    parser.add_argument("--limit", type=int, default=40)
    args = parser.parse_args()
    case = json.loads((args.cases / (args.name.replace("/", "__") + ".json")).read_text())
    expected = json.loads((UPSTREAM / "e2etests/testdata" / args.name / args.engine / "board.exp.json").read_text())
    with tempfile.TemporaryDirectory(prefix="d2rs-diff-") as scratch:
        source = Path(scratch) / "input.d2"
        source.write_text(case["script"])
        trace = Path(scratch) / "target.bytes"
        command = [str(args.rust), "--pad=0", "--bundle=false", "--tala-seeds=1", f"--layout={args.engine}",
                   str(source), str(Path(scratch) / "out.svg")]
        if case["themeID"] is not None:
            command.append(f"--theme={case['themeID']}")
        process = subprocess.run(command, capture_output=True, env={"D2_RS_TRACE_TARGET_BYTES": str(trace), "PATH": "/usr/bin:/bin"})
        if not trace.is_file():
            print("no target trace", process.stderr.decode()[-800:])
            return 1
        values = split_json_stream(trace.read_text())
    shapes, connections, root = values[0] or [], values[1] or [], values[2]
    out: list[str] = []
    expected_shapes = {shape["id"]: shape for shape in expected.get("shapes") or []}
    actual_shapes = {shape["id"]: shape for shape in shapes}
    if list(expected_shapes) != list(actual_shapes):
        out.append(order_diff("shape", list(expected_shapes), list(actual_shapes)))
    for key, shape in expected_shapes.items():
        if key in actual_shapes:
            diff(f"shape[{key}]", shape, actual_shapes[key], out, args.limit)
    expected_connections = {c["id"]: c for c in expected.get("connections") or []}
    actual_connections = {c["id"]: c for c in connections}
    if list(expected_connections) != list(actual_connections):
        out.append(order_diff("connection", list(expected_connections), list(actual_connections)))
    for key, connection in expected_connections.items():
        if key in actual_connections:
            diff(f"conn[{key}]", connection, actual_connections[key], out, args.limit)
    diff("root", expected.get("root"), root, out, args.limit)
    print("\n".join(out) if out else "target identical")
    return 0 if not out else 2


if __name__ == "__main__":
    sys.exit(main())
