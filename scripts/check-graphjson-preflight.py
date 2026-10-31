#!/usr/bin/env python3
"""Compare targeted preflight errors and acceptance of the upstream wire corpus.

Go runs its complete codec; Rust runs the serialized preflight boundary. The
public graphjson adapter performs reconstruction after this same validation.
"""
import hashlib
import json
import subprocess
from references import ROOT, REFERENCE, verify_source, read_report

source = verify_source()
go = REFERENCE / "bin/tala-engine-oracle"
rust = ROOT / "target/debug/examples/graphjson-wire"
cases = json.loads((ROOT / "tests/graphjson/preflight-cases.json").read_text())
for fixture in read_report("engine-fixture-inventory.json", "scripts/audit-engine-fixtures.py")["fixtures"]:
    if fixture["format"] == "layout_graph_json":
        data = (source / fixture["upstream_path"]).read_bytes()
        assert hashlib.sha256(data).hexdigest() == fixture["sha256"]
        cases.append({"name": fixture["upstream_path"], "input": data.decode(), "kind": "corpus_acceptance"})
rows = []
for case in cases:
    row = {"name": case["name"], "kind": case.get("kind", "targeted_rejection")}
    outputs = {}
    for name, command in (("go", [str(go), "-operation", "roundtrip"]),
                          ("rust", [str(rust), "--preflight"])):
        try:
            result = subprocess.run(command, input=case["input"].encode(), capture_output=True, timeout=30)
            outputs[name] = result
            row[name + "_exit"] = result.returncode
            if result.returncode:
                row[name + "_error"] = result.stderr.decode(errors="replace")
        except subprocess.TimeoutExpired:
            row[name + "_timeout"] = True
    if len(outputs) != 2:
        row["status"] = "timeout"
    elif row["kind"] == "corpus_acceptance":
        row["status"] = "accepted" if all(r.returncode == 0 for r in outputs.values()) else "acceptance_difference"
    else:
        g, r = outputs["go"], outputs["rust"]
        row["status"] = "exact_rejection" if g.returncode == r.returncode == 1 and g.stderr == r.stderr else "error_difference"
    rows.append(row)
report = {"scope": "serialized preflight acceptance and targeted rejection only; no arena/layout parity",
          "go_sha256": hashlib.sha256(go.read_bytes()).hexdigest(),
          "rust_sha256": hashlib.sha256(rust.read_bytes()).hexdigest(),
          "accepted": sum(r["status"] == "accepted" for r in rows),
          "exact_rejections": sum(r["status"] == "exact_rejection" for r in rows),
          "remaining": sum(r["status"] not in ("accepted", "exact_rejection") for r in rows),
          "limitations": ["Upstream multi-error map traversal order is unqualified", "Full pointer-owned aggregate reconstruction remains pending"],
          "results": rows}
report_path = ROOT / "oracle-cache/reports/graphjson-preflight-boundary.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({k: v for k, v in report.items() if k != "results"}, indent=2))
raise SystemExit(bool(report["remaining"]))
