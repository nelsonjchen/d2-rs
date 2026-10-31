#!/usr/bin/env python3
"""Compare translated AST map insertion and key helpers with pinned d2ast."""
import json
import subprocess

from references import ROOT, REFERENCE, verify_source, digest

source = verify_source()
go = REFERENCE / "bin/d2-oracle"
rust = ROOT / "target/debug/examples/ast-key-helpers-oracle"
left = json.loads(subprocess.check_output([str(go), "ast-key-helpers"], input=b""))
right = json.loads(subprocess.check_output([str(rust)]))
report = {
    "boundary": "Map insertion identity and fresh storage, 225 ordered key comparisons per equality method, glob locations",
    "oracle_sha256": digest(go), "rust_sha256": digest(rust),
    "source_sha256": digest(source / "d2ast/d2ast.go"),
    "cases": len(left), "matched": left == right,
    "reference": left, "rust": right,
    "parser_parity": False,
}
report_path = ROOT / "oracle-cache/reports/ast-key-helpers-parity.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({key: value for key, value in report.items() if key not in ("reference", "rust")}, indent=2))
raise SystemExit(left != right)
