#!/usr/bin/env python3
"""Sweep every pinned graphjson input/regression fixture through TALA.

Expected-output snapshots are intentionally excluded: they are already
materialized layouts and are qualified by the post-layout boundary script,
not by feeding them back into the layout engine.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from references import MANIFEST, REFERENCE, ROOT, digest, verify_source, read_report


def normalized(document: dict) -> dict:
    return {
        "nodes": {
            node["id"]: (
                node["width"],
                node["height"],
                node.get("topLeft", node.get("top_left")),
            )
            for node in document.get("nodes", [])
        },
        "edges": {
            edge["id"]: (
                edge["from"],
                edge["to"],
                edge.get("points", []),
            )
            for edge in document.get("edges", [])
        },
    }


def run(command: list[str], data: bytes, timeout: int) -> tuple[int, bytes, bytes]:
    try:
        result = subprocess.run(command, input=data, capture_output=True, timeout=timeout)
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired as error:
        return 124, b"", f"timeout after {timeout}s: {error}".encode()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--seed",
        action="append",
        type=int,
        dest="seeds",
        help="seed to compare; may be repeated (defaults to 1)",
    )
    parser.add_argument(
        "--fixture",
        action="append",
        default=[],
        help="limit to exact upstream-relative fixture paths; may be repeated",
    )
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--rust-binary", type=Path, help="graphjson-layout binary to compare")
    parser.add_argument("--report", type=Path, default=ROOT / "oracle-cache/reports/graphjson-corpus-parity.json")
    args = parser.parse_args()
    seeds = args.seeds or [1]

    source = verify_source()
    inventory = read_report("engine-fixture-inventory.json", "scripts/audit-engine-fixtures.py")
    fixtures = [
        row for row in inventory["fixtures"]
        if row.get("format") == "layout_graph_json" and row.get("role") != "expected_output"
    ]
    if args.fixture:
        fixtures_by_path = {row["upstream_path"]: row for row in fixtures}
        missing = sorted(set(args.fixture) - fixtures_by_path.keys())
        if missing:
            raise SystemExit(f"unknown fixture path(s): {', '.join(missing)}")
        fixtures = [fixtures_by_path[path] for path in args.fixture]
    go = REFERENCE / "bin" / "tala-engine-oracle-trace"
    rust_candidates = [
        ROOT / "target/diagnostic/examples/graphjson-layout",
        ROOT / "target/oracle/examples/graphjson-layout",
        ROOT / "target/debug/examples/graphjson-layout",
    ]
    rust = args.rust_binary or next((candidate for candidate in rust_candidates if candidate.exists()), None)
    if rust is None:
        raise SystemExit("missing graphjson-layout example; build the diagnostic target first")
    if not rust.is_file():
        raise SystemExit(f"graphjson-layout binary does not exist: {rust}")

    rows = []
    for seed in seeds:
        for fixture in fixtures:
            path = source / fixture["upstream_path"]
            data = path.read_bytes()
            if hashlib.sha256(data).hexdigest() != fixture["sha256"]:
                raise RuntimeError(f"fixture checksum mismatch: {path}")
            go_exit, go_output, go_error = run([str(go), "-seed", str(seed)], data, args.timeout)
            rust_exit, rust_output, rust_error = run([str(rust), str(seed)], data, args.timeout)
            row = {
                "fixture": fixture["upstream_path"],
                "input_sha256": fixture["sha256"],
                "seed": seed,
                "go_exit": go_exit,
                "rust_exit": rust_exit,
            }
            if go_exit == rust_exit == 0:
                try:
                    go_value = json.loads(go_output)
                    rust_value = json.loads(rust_output)
                except json.JSONDecodeError as error:
                    row.update({"passed": False, "error": f"JSON decode: {error}"})
                else:
                    row["passed"] = normalized(go_value) == normalized(rust_value)
                    if not row["passed"]:
                        row["go_sha256"] = hashlib.sha256(go_output).hexdigest()
                        row["rust_sha256"] = hashlib.sha256(rust_output).hexdigest()
            else:
                row.update({
                    "passed": False,
                    "go_error": go_error.decode(errors="replace"),
                    "rust_error": rust_error.decode(errors="replace"),
                })
            rows.append(row)
            print(f"{fixture['upstream_path']} seed {seed}: {'PASS' if row['passed'] else 'FAIL'}", flush=True)

    report = {
        "scope": "pinned GraphJSON layout inputs and regressions; expected-output snapshots excluded",
        "fixture_set": "all pinned fixtures" if not args.fixture else "selected upstream-relative fixtures",
        "fixtures": [fixture["upstream_path"] for fixture in fixtures],
        "source_commit": MANIFEST["d2"]["commit"],
        "seed": seeds[0] if len(seeds) == 1 else None,
        "seeds": seeds,
        "go_oracle_sha256": digest(go),
        "rust_binary": str(rust.resolve()),
        "rust_binary_sha256": digest(rust),
        "passed": sum(row["passed"] for row in rows),
        "failed": sum(not row["passed"] for row in rows),
        "total": len(rows),
        "results": rows,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("scope", "seeds", "passed", "failed", "total")}, indent=2))
    raise SystemExit(report["failed"] != 0)


if __name__ == "__main__":
    main()
