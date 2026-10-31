#!/usr/bin/env python3
"""Compare reconstructed Rust graphjson layouts with the pinned Go oracle.

The graphjson adapter is an in-process public TALA API. This qualification
script still records failures instead of normalizing them away, and remains
separate from the D2 plugin protocol.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

from exact_json import decode_output, difference
from references import ROOT, REFERENCE, verify_source


def field(record: dict, *names: str):
    for name in names:
        if name in record:
            return record[name]
    return None


def normalized(document: dict) -> dict:
    def label(record):
        if record is None:
            return None
        return {
            "position": field(record, "Position", "position"),
            "width": field(record, "Width", "width"),
            "height": field(record, "Height", "height"),
        }

    nodes = []
    for node in document.get("nodes", []):
        icon = field(node, "Icon", "icon")
        nodes.append(
            {
                "id": node["id"],
                "width": node["width"],
                "height": node["height"],
                "top_left": field(node, "topLeft", "top_left"),
                "label": label(field(node, "Label", "label")),
                "icon": None if icon is None else {
                    "position": field(icon, "Position", "position")
                },
            }
        )

    edges = []
    for edge in document.get("edges", []):
        edges.append(
            {
                "id": edge["id"],
                "from": edge["from"],
                "to": edge["to"],
                "points": edge.get("points", []),
                "label_percentage": field(
                    edge, "labelPercentage", "LabelPercentage", "label_percentage"
                ),
                "label": label(field(edge, "Label", "label")),
            }
        )
    # The Go serializer preserves live slice order; the compact Rust adapter
    # walks the input arena. Array order is not layout geometry, so align by
    # stable identity while retaining duplicate IDs and their multiplicity.
    nodes.sort(key=lambda node: node["id"])
    edges.sort(key=lambda edge: edge["id"])
    return {"nodes": nodes, "edges": edges}


def run(command: list[str], data: bytes) -> tuple[int, bytes, bytes]:
    result = subprocess.run(
        command,
        input=data,
        capture_output=True,
        timeout=120,
    )
    return result.returncode, result.stdout, result.stderr


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("seed", nargs="?", type=int, default=1)
    parser.add_argument("--include-exp", action="store_true")
    parser.add_argument("--rust-binary", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    source = verify_source()
    go = REFERENCE / "bin/tala-engine-oracle-trace"
    # The oracle profile is the documented path for the expensive direct
    # engine sweep. Keep the debug path as a fallback so a normal development
    # build remains sufficient.
    rust_candidates = [
        ROOT / "target/oracle/examples/graphjson-layout",
        ROOT / "target/diagnostic/examples/graphjson-layout",
        ROOT / "target/debug/examples/graphjson-layout",
    ]
    rust = args.rust_binary or next((candidate for candidate in rust_candidates if candidate.exists()), None)
    seed = args.seed
    include_exp = args.include_exp
    fixture_roots = [
        ("layout", source / "d2layouts/d2talalayout/internal/testdata/layout"),
        ("ovg", source / "d2layouts/d2talalayout/internal/testdata/ovg"),
    ]
    fixture_names = ["graph.input.json"]
    if include_exp:
        fixture_names.append("graph.exp.json")
    fixtures = sorted(
        (suite, fixture)
        for suite, root in fixture_roots
        for fixture_name in fixture_names
        for fixture in root.glob(f"*/{fixture_name}")
    )
    if rust is None:
        raise SystemExit(
            "missing diagnostic binary; run `cargo build -p d2-tala-rs "
            "--features diagnostic-traces --example graphjson-layout` "
            "(or add `--profile oracle` for the optimized sweep)"
        )

    rows = []
    for suite, fixture in fixtures:
        data = fixture.read_bytes()
        go_exit, go_output, go_error = run([str(go), "-seed", str(seed)], data)
        rust_exit, rust_output, rust_error = run([str(rust), str(seed)], data)
        row = {
            "suite": suite,
            "fixture": str(fixture.relative_to(source)),
            "seed": seed,
            "input_sha256": hashlib.sha256(data).hexdigest(),
            "go_exit": go_exit,
            "rust_exit": rust_exit,
        }
        if go_exit == rust_exit == 0:
            go_value = normalized(decode_output(go_output))
            rust_value = normalized(decode_output(rust_output))
            mismatch = difference(go_value, rust_value)
            row["passed"] = mismatch is None
            if not row["passed"]:
                row["first_mismatch"] = mismatch
                row["go_sha256"] = hashlib.sha256(go_output).hexdigest()
                row["rust_sha256"] = hashlib.sha256(rust_output).hexdigest()
        else:
            row["go_error"] = go_error.decode(errors="replace")
            row["rust_error"] = rust_error.decode(errors="replace")
            row["passed"] = (
                go_exit == rust_exit
                and go_output == rust_output
                and go_error == rust_error
            )
        rows.append(row)
        print(f"{row['fixture']}: {'PASS' if row['passed'] else 'FAIL'}")

    report = {
        "scope": (
            "graphjson reconstruction and layout/OVG routing; "
            "post-layout snapshots are included"
            if include_exp
            else "graphjson reconstruction and layout/OVG routing; not plugin protocol"
        ),
        "source_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "seed": seed,
        "go_oracle": str(go),
        "rust_binary": str(rust),
        "passed": sum(row["passed"] for row in rows),
        "failed": sum(not row["passed"] for row in rows),
        "results": rows,
    }
    report_path = args.report or (
        ROOT / "oracle-cache/reports" / ("graphjson-postlayout-boundary.json" if include_exp else "graphjson-layout-boundary.json")
    )
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, indent=2) + "\n"
    )
    return int(report["failed"] != 0)


if __name__ == "__main__":
    sys.exit(main())
