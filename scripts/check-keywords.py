#!/usr/bin/env python3
"""Compare all translated language keyword inventories with pinned Go d2ast."""
import json
import subprocess

from references import ROOT, REFERENCE, verify_source, digest

source = verify_source()
go = REFERENCE / "bin/d2-oracle"
rust = ROOT / "target/debug/examples/keywords-oracle"
left = json.loads(subprocess.check_output([str(go), "keywords"], input=b""))
right = json.loads(subprocess.check_output([str(rust)]))
differences = [name for name in sorted(left.keys() | right.keys())
               if name not in left or name not in right or left[name] != right[name]]
report = {
    "boundary": "d2ast keyword sets, ordered arrays and label-position mapping",
    "comparison": "sets sorted; source array order and exact string values preserved",
    "oracle_sha256": digest(go),
    "rust_sha256": digest(rust),
    "source_sha256": digest(source / "d2ast/keywords.go"),
    "rust_source_sha256": digest(ROOT / "crates/d2-rs-core/src/ast/keywords.rs"),
    "groups": len(left),
    "differing_groups": differences,
    "reference": left,
    "parser_compiler_parity": False,
}
report_path = ROOT / "oracle-cache/reports/keyword-parity.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({key: value for key, value in report.items() if key != "reference"}, indent=2))
raise SystemExit(bool(differences))
