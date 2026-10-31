#!/usr/bin/env python3
"""Qualify AST exact rational storage/JSON, independently of numeric parsing."""
import json
import subprocess
from references import ROOT, REFERENCE, verify_source, digest

source = verify_source()
go = REFERENCE / "bin/d2-oracle"
rust = ROOT / "target/debug/examples/ast-numbers-oracle"
requests = [{"raw": "nil", "numerator": None, "denominator": "1"}]
for numerator in [0, 1, -1, 2, 10, 2**53 + 1, -(2**128 + 17), 10**200 + 3]:
    for denominator in [1, -1, 2, -6, 10**150 + 7]:
        requests.append({"raw": "001.00e+0", "numerator": str(numerator), "denominator": str(denominator)})
payload = json.dumps(requests).encode()
left = json.loads(subprocess.check_output([str(go), "ast-numbers"], input=payload))
right = json.loads(subprocess.check_output([str(rust)], input=payload))
report = {
    "boundary": "AST Number exact rational storage, JSON and raw scalar spelling",
    "oracle_sha256": digest(go), "rust_sha256": digest(rust),
    "source_sha256": digest(source / "d2ast/d2ast.go"),
    "cases": len(requests), "matched": left == right,
    "requests": requests, "reference": left, "rust": right,
    "numeric_parser_parity": False,
}
report_path = ROOT / "oracle-cache/reports/ast-number-parity.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({k: v for k, v in report.items() if k not in ("requests", "reference", "rust")}, indent=2))
raise SystemExit(left != right)
