#!/usr/bin/env python3
"""Differential wire-decoder semantics, including pending exact-error gaps."""
import hashlib
import importlib.util
import json
import subprocess
from references import ROOT, REFERENCE, verify_source
spec=importlib.util.spec_from_file_location("boundaries",ROOT/"scripts/check-boundaries.py")
boundaries=importlib.util.module_from_spec(spec);spec.loader.exec_module(boundaries)
verify_source()
go=REFERENCE/"bin/tala-engine-oracle"
rust=ROOT/"target/debug/examples/graphjson-wire"
rows=[]
for case in json.loads((ROOT/"tests/graphjson/decoder-cases.json").read_text()):
    results={name:subprocess.run(command,input=case["input"].encode(),capture_output=True,timeout=30)
             for name,command in (("go",[str(go),"-operation","wire"]),("rust",[str(rust)]))}
    g,r=results["go"],results["rust"]
    row={"name":case["name"],"go_exit":g.returncode,"rust_exit":r.returncode}
    if g.returncode==r.returncode==0:
        row["difference"]=boundaries.difference(boundaries.decode_output(g.stdout),boundaries.decode_output(r.stdout))
        row["status"]="exact_wire_output" if row["difference"] is None else "output_difference"
    else:
        row.update({"go_error":g.stderr.decode(errors="replace"),"rust_error":r.stderr.decode(errors="replace")})
        row["status"]="exact_rejection" if g.returncode and r.returncode and g.stderr==r.stderr else "error_difference"
    rows.append(row)
report={"scope":"wire decode semantics; graph preflight and arena reconstruction excluded",
        "go_sha256":hashlib.sha256(go.read_bytes()).hexdigest(),"rust_sha256":hashlib.sha256(rust.read_bytes()).hexdigest(),
        "matched":sum(r["status"] in ("exact_wire_output","exact_rejection") for r in rows),
        "remaining":sum(r["status"] not in ("exact_wire_output","exact_rejection") for r in rows),"results":rows}
report_path = ROOT / "oracle-cache/reports/graphjson-decoder-boundary.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report,indent=2)+"\n")
print(json.dumps(report,indent=2))
raise SystemExit(bool(report["remaining"]))
