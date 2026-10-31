#!/usr/bin/env python3
"""Compare native interpolation and shared-copy operations with pinned d2ast."""
import json
import subprocess

from references import ROOT, REFERENCE, verify_source, digest

source = verify_source()
go = REFERENCE / "bin/d2-oracle"
rust = ROOT / "target/debug/examples/ast-interpolation-oracle"
left = json.loads(subprocess.check_output([str(go), "ast-interpolation"], input=b""))
right = json.loads(subprocess.check_output([str(rust)]))
report = {
    "boundary": "interpolation JSON, scalar prefix, child count, coalescing and copy aliasing",
    "source_sha256": digest(source / "d2ast/d2ast.go"),
    "oracle_sha256": digest(go), "rust_sha256": digest(rust),
    "cases": len(left), "matched": left == right,
    "reference": left, "rust": right, "parser_parity": False,
}
report_path = ROOT / "oracle-cache/reports/ast-interpolation-parity.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({key: value for key, value in report.items() if key not in ("reference", "rust")}, indent=2))
raise SystemExit(left != right)
