#!/usr/bin/env python3
"""Compare key-path construction, copying and glob inspection with pinned Go."""
import json
import subprocess
from references import ROOT, REFERENCE, verify_source, digest

source = verify_source()
go = REFERENCE / "bin/d2-oracle"
rust = ROOT / "target/debug/examples/key-paths-oracle"
left = json.loads(subprocess.check_output([str(go), "key-paths"], input=b""))
right = json.loads(subprocess.check_output([str(rust)]))
report = {"source_sha256": digest(source / "d2ast/d2ast.go"),
          "oracle_sha256": digest(go), "rust_sha256": digest(rust),
          "cases": len(left), "matched": left == right,
          "reference": left, "rust": right, "parser_parity": False}
report_path = ROOT / "oracle-cache/reports/key-path-parity.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({k: v for k, v in report.items() if k not in ("reference", "rust")}, indent=2))
raise SystemExit(left != right)
