#!/usr/bin/env python3
"""Persist strict layout witnesses and compare each top-level graph field.

This localizes observable mismatches; it does not establish stage/trace parity.
No geometry, array ordering or fields are normalized away.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
from references import ROOT, REFERENCE, verify_source, digest

spec = importlib.util.spec_from_file_location("boundaries", ROOT / "scripts/check-boundaries.py")
boundaries = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundaries)


def main():
    verify_source()
    oracle = REFERENCE / "bin/d2-oracle"
    rust = ROOT / "target/release/examples/tala-oracle"
    output_dir = ROOT / "target/parity-witnesses"
    output_dir.mkdir(parents=True, exist_ok=True)
    environment = {k: v for k, v in os.environ.items()
                   if not k.startswith(("TALA_", "WEFTAN_")) and k not in ("DEV_MODE", "TEST_MODE")}
    def run(command, source):
        return subprocess.run([str(v) for v in command], input=source, capture_output=True,
                              timeout=120, env=environment)
    results = []
    for fixture in sorted((ROOT / "tests/d2").glob("*.d2")):
        measurement = run([oracle, "measure", "--path", fixture.name], fixture.read_bytes())
        measurement.check_returncode()
        (output_dir / f"{fixture.stem}.measured.json").write_bytes(measurement.stdout)
        for seed in (1, 2, 3):
            row = {"fixture": fixture.name, "seed": seed, "fields": {}}
            outputs = []
            for name, binary in (("go", oracle), ("rust", rust)):
                result = run([binary, "layout", "--tala-seeds", str(seed)], measurement.stdout)
                prefix = output_dir / f"{fixture.stem}.seed-{seed}.{name}"
                prefix.with_suffix(prefix.suffix + ".json").write_bytes(result.stdout)
                prefix.with_suffix(prefix.suffix + ".stderr").write_bytes(result.stderr)
                row[name + "_exit"] = result.returncode
                outputs.append(json.loads(result.stdout) if result.returncode == 0 else None)
            if all(value is not None for value in outputs):
                left, right = outputs
                for field in sorted(left.keys() | right.keys()):
                    if field not in left or field not in right:
                        row["fields"][field] = {"missing": "go" if field not in left else "rust"}
                    else:
                        row["fields"][field] = boundaries.difference(left[field], right[field], "$." + field)
            results.append(row)
    report = {"scope": "Exact final graph fields; not an intermediate-stage comparison",
              "oracle_sha256": digest(oracle), "rust_sha256": digest(rust), "results": results}
    report_path = ROOT / "oracle-cache/reports/layout-localization.json"
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    for row in results:
        mismatches = {k: v for k, v in row["fields"].items() if v is not None}
        if mismatches or row["go_exit"] or row["rust_exit"]:
            print(json.dumps({**row, "fields": mismatches}))


if __name__ == "__main__":
    main()
