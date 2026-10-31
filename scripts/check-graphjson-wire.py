#!/usr/bin/env python3
"""Compare typed wire serialization only; does not reconstruct engine graphs."""
import hashlib
import json
import subprocess
from pathlib import Path
from references import ROOT, REFERENCE, MANIFEST, verify_source, read_report
import importlib.util

spec = importlib.util.spec_from_file_location("boundaries", ROOT / "scripts/check-boundaries.py")
boundaries = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundaries)
source = verify_source()
go = REFERENCE / "bin/tala-engine-oracle"
rust = ROOT / "target/debug/examples/graphjson-wire"
inventory = read_report("engine-fixture-inventory.json", "scripts/audit-engine-fixtures.py")
rows = []
for fixture in inventory["fixtures"]:
    if fixture["format"] != "layout_graph_json":
        continue
    data = (source / fixture["upstream_path"]).read_bytes()
    if hashlib.sha256(data).hexdigest() != fixture["sha256"]:
        raise RuntimeError("Fixture checksum mismatch")
    row = {"fixture": fixture["upstream_path"], "input_sha256": fixture["sha256"]}
    outputs = {}
    for name, command in (("go", [str(go), "-operation", "wire"]), ("rust", [str(rust)])):
        try:
            result = subprocess.run(command, input=data, capture_output=True, timeout=120)
            row[name + "_exit"] = result.returncode
            if result.returncode:
                row[name + "_stderr"] = result.stderr.decode(errors="replace")
            else:
                outputs[name] = boundaries.decode_output(result.stdout)
        except subprocess.TimeoutExpired:
            row[name + "_timeout_seconds"] = 120
    row["difference"] = boundaries.difference(outputs["go"], outputs["rust"]) if len(outputs) == 2 else {"execution": "failed"}
    row["passed"] = row["difference"] is None
    rows.append(row)
report = {"scope": "canonical-field typed wire serialization boundary; reconstruction and layout are qualified separately",
          "source_commit": MANIFEST["d2"]["commit"],
          "go_sha256": hashlib.sha256(go.read_bytes()).hexdigest(),
          "rust_sha256": hashlib.sha256(rust.read_bytes()).hexdigest(),
          "passed": sum(r["passed"] for r in rows), "failed": sum(not r["passed"] for r in rows),
          "remaining": ["Exact JSON diagnostics", "Full pointer-owned aggregate reconstruction", "Broader direct-engine layout/routing coverage"],
          "results": rows}
report_path = ROOT / "oracle-cache/reports/graphjson-wire-boundary.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
verify_source()
print(json.dumps({k: report[k] for k in ("scope", "passed", "failed")}, indent=2))
for row in rows:
    if not row["passed"]:
        print(json.dumps(row))
raise SystemExit(bool(report["failed"]))
