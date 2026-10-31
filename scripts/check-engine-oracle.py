#!/usr/bin/env python3
"""Validate the direct Go oracle against upstream's exact seed goldens.

This qualifies a reference tool, not any Rust implementation.
"""
import hashlib
import json
import re
import subprocess
from references import ROOT, REFERENCE, MANIFEST, verify_source

source = verify_source()
binary = REFERENCE / "bin/tala-engine-oracle"
test_path = source / "d2layouts/d2talalayout/internal/engine/layout_seed_regression_test.go"
test_text = test_path.read_text()
cases = re.findall(r'fixture: "([^"]+)", seed: (-?\d+), sha256: "([0-9a-f]{64})"', test_text)
if not cases:
    raise RuntimeError("No upstream seed goldens found")
results = []
for name, seed, expected in cases:
    fixture = source / "d2layouts/d2talalayout/internal/testdata/layout" / name / "graph.input.json"
    data = fixture.read_bytes()
    row = {"fixture": str(fixture.relative_to(source)), "seed": int(seed),
           "input_sha256": hashlib.sha256(data).hexdigest(), "expected_sha256": expected}
    try:
        result = subprocess.run([str(binary), "-seed", seed], input=data,
                                capture_output=True, timeout=120)
        actual = hashlib.sha256(result.stdout).hexdigest()
        row.update({"exit": result.returncode, "actual_sha256": actual,
                    "stderr": result.stderr.decode(errors="replace"),
                    "passed": result.returncode == 0 and actual == expected})
    except subprocess.TimeoutExpired:
        row.update({"passed": False, "harness_timeout_seconds": 120})
    results.append(row)
    print(f"{name} seed {seed}: {'PASS' if row['passed'] else 'FAIL'}", flush=True)
report = {"scope": "Go reference qualification only; no Rust parity claim",
          "source_commit": MANIFEST["d2"]["commit"],
          "test_source_sha256": hashlib.sha256(test_path.read_bytes()).hexdigest(),
          "oracle": json.loads(binary.with_suffix(".provenance.json").read_text()),
          "passed": sum(row["passed"] for row in results),
          "failed": sum(not row["passed"] for row in results), "results": results}
report_path = ROOT / "oracle-cache/reports/engine-oracle-qualification.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
verify_source()
raise SystemExit(bool(report["failed"]))
