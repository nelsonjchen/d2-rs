#!/usr/bin/env python3
"""Qualify direct graphjson reconstruction on D2's benchmark/regression corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from references import ROOT, REFERENCE, verify_source


def normalized(document: dict) -> dict:
    """Keep semantic geometry while ignoring Go/Rust field spelling."""
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


def run(binary: Path, seed: int, data: bytes, *, oracle: bool) -> tuple[int, bytes, bytes]:
    try:
        result = subprocess.run(
            [str(binary), "-seed", str(seed)] if oracle else [str(binary), str(seed)],
            input=data,
            capture_output=True,
            timeout=300,
        )
    except subprocess.TimeoutExpired as exc:
        # Keep an over-limit run in the qualification report instead of
        # aborting before the remaining fixtures are compared.
        return 124, exc.stdout or b"", b"Timed out after 300 seconds\n"
    return result.returncode, result.stdout, result.stderr


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", action="append", type=int)
    parser.add_argument(
        "--report", type=Path, default=ROOT / "oracle-cache/reports/graphjson-expanded-boundary.json"
    )
    args = parser.parse_args()
    seeds = args.seed or [1]
    source = verify_source()
    go = REFERENCE / "bin/tala-engine-oracle-trace"
    rust_candidates = [
        # This checker compares semantic graph output, not operation traces.
        # The expanded corpus includes intentionally large graphs, so prefer
        # the ordinary optimized binary; diagnostic tracing is qualified by
        # check-graphjson-trace.py and can add enough bookkeeping to hit the
        # semantic check's timeout on large inputs.
        ROOT / "target/release/examples/graphjson-layout",
        ROOT / "target/oracle/examples/graphjson-layout",
        ROOT / "target/diagnostic/examples/graphjson-layout",
        ROOT / "target/debug/examples/graphjson-layout",
    ]
    rust = next((candidate for candidate in rust_candidates if candidate.exists()), None)
    if rust is None:
        raise SystemExit("missing graphjson-layout binary; build the release example first")
    roots = {
        "benchmark": source / "d2layouts/d2talalayout/internal/testdata/benchmark",
        "regression": source / "d2layouts/d2talalayout/internal/testdata/regression",
    }
    fixtures = [
        (suite, fixture)
        for suite, root in roots.items()
        for fixture in sorted(root.glob("*.json"))
    ]
    rows: list[dict[str, object]] = []
    for seed in seeds:
        for suite, fixture in fixtures:
            data = fixture.read_bytes()
            go_exit, go_output, go_error = run(go, seed, data, oracle=True)
            rust_exit, rust_output, rust_error = run(rust, seed, data, oracle=False)
            row: dict[str, object] = {
                "suite": suite,
                "fixture": str(fixture.relative_to(source)),
                "seed": seed,
                "input_sha256": hashlib.sha256(data).hexdigest(),
                "go_exit": go_exit,
                "rust_exit": rust_exit,
            }
            if go_exit == rust_exit == 0:
                go_value = json.loads(go_output)
                rust_value = json.loads(rust_output)
                row["matched"] = normalized(go_value) == normalized(rust_value)
                if not row["matched"]:
                    row["go_sha256"] = hashlib.sha256(go_output).hexdigest()
                    row["rust_sha256"] = hashlib.sha256(rust_output).hexdigest()
                    go_nodes = normalized(go_value)["nodes"]
                    rust_nodes = normalized(rust_value)["nodes"]
                    for node_id in sorted(set(go_nodes) | set(rust_nodes)):
                        if go_nodes.get(node_id) != rust_nodes.get(node_id):
                            row["first_diff"] = f"nodes[{node_id}] {go_nodes.get(node_id)!r} != {rust_nodes.get(node_id)!r}"
                            break
            else:
                row["matched"] = False
                row["go_error"] = go_error.decode(errors="replace")
                row["rust_error"] = rust_error.decode(errors="replace")
            rows.append(row)
            print(f"{row['fixture']} seed {seed}: {'PASS' if row['matched'] else 'FAIL'}")

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-tala-graphjson-expanded-boundary",
        "authority": "pristine D2 v0.9.0 with Go 1.27.0",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "seeds": seeds,
        "suite_counts": {suite: sum(1 for current, _ in fixtures if current == suite) for suite in roots},
        "matched": sum(row["matched"] for row in rows),
        "total": len(rows),
        "go_oracle_sha256": hashlib.sha256(go.read_bytes()).hexdigest(),
        "rust_binary_sha256": hashlib.sha256(rust.read_bytes()).hexdigest(),
        "results": rows,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(f"MATCH {report['matched']}/{report['total']}; wrote {args.report}")
    return int(report["matched"] != report["total"])


if __name__ == "__main__":
    raise SystemExit(main())
