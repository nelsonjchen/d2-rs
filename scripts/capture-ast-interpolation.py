#!/usr/bin/env python3
"""Capture upstream interpolation semantics before translating shared storage."""
import json
import subprocess

from references import ROOT, REFERENCE, verify_source, digest

source = verify_source()
oracle = REFERENCE / "bin/d2-oracle"
cases = json.loads(subprocess.check_output([str(oracle), "ast-interpolation"], input=b""))
report = {
    "source_sha256": digest(source / "d2ast/d2ast.go"),
    "oracle_sha256": digest(oracle),
    "cases": cases,
    "rust_status": "pending_interpolation_nodes_and_shared_copy_storage",
    "rust_passing_cases": 0,
}
report_path = ROOT / "oracle-cache/reports/ast-interpolation-reference.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(f"Captured {len(cases)} source cases; Rust interpolation parity remains pending.")
