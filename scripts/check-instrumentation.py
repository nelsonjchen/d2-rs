#!/usr/bin/env python3
"""Verify stage instrumentation preserves pristine oracle public output."""
import json
import os
import subprocess
from references import ROOT, REFERENCE, verify_source, digest

verify_source()
oracle = REFERENCE / "bin/d2-oracle"
traced = REFERENCE / "bin/d2-oracle-trace"
results = []
environment = {key: value for key, value in os.environ.items()
               if not key.startswith(("TALA_", "WEFTAN_")) and key not in ("DEV_MODE", "TEST_MODE")}
for fixture in sorted((ROOT / "tests/d2").glob("*.d2")):
    base_measure = subprocess.run(
        [str(oracle), "measure", "--path", fixture.name],
        input=fixture.read_bytes(), capture_output=True, timeout=120, env=environment,
    )
    trace_measure = subprocess.run(
        [str(traced), "measure", "--path", fixture.name],
        input=fixture.read_bytes(), capture_output=True, timeout=120, env=environment,
    )
    if base_measure.returncode or trace_measure.returncode:
        # Raw HTML that the native SVG path does not support is intentionally a
        # measurement-time rejection. Keep this in the transparency report and
        # compare the complete deterministic error boundary instead of letting
        # subprocess.check_call turn it into a harness failure.
        rejected = (
            base_measure.returncode == trace_measure.returncode
            and base_measure.stdout == trace_measure.stdout
            and base_measure.stderr == trace_measure.stderr
        )
        results.append({
            "fixture": fixture.name,
            "seed": 1,
            "public_bytes_identical": rejected,
            "measurement_rejected": True,
            "stage_events": 0,
            "base_exit": base_measure.returncode,
            "trace_exit": trace_measure.returncode,
            "measurement_stderr_identical": base_measure.stderr == trace_measure.stderr,
        })
        continue
    measured = base_measure.stdout
    base = subprocess.run([str(oracle), "layout", "--tala-seeds", "1"], input=measured,
                          capture_output=True, timeout=120, env=environment)
    instrumented = subprocess.run([str(traced), "layout", "--tala-seeds", "1"], input=measured,
                                 capture_output=True, timeout=120,
                                 env={**environment, "TALA_TRACE_JSONL": "1"})
    events = []
    for line in instrumented.stderr.splitlines():
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict) and value.get("event") == "stage":
            events.append(value)
    # A successful pair must have exact public bytes and actual trace events.
    passed = base.returncode == instrumented.returncode == 0 and base.stdout == instrumented.stdout and bool(events)
    results.append({"fixture": fixture.name, "seed": 1, "public_bytes_identical": passed,
                    "stage_events": len(events), "base_exit": base.returncode,
                    "trace_exit": instrumented.returncode})
report = {"scope": "Go stage-instrumentation transparency only; not Rust trace parity",
          "base_sha256": digest(oracle), "trace_sha256": digest(traced), "results": results}
report_path = ROOT / "oracle-cache/reports/instrumentation-baseline.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
raise SystemExit(any(not row["public_bytes_identical"] for row in results))
