#!/usr/bin/env python3
"""Classify pinned internal fixtures and test upstream graph decoding only."""
from collections import Counter
import hashlib
import json
import subprocess
from references import ROOT, REFERENCE, MANIFEST, verify_source

source = verify_source()
# The pinned TALA internal testdata, enumerated from the pristine tree (the
# v0.9.0 inventory formerly committed as tests/upstream/manifest.json).
INTERNAL_ROOTS = ("d2layouts/d2talalayout/internal/testdata",
                  "d2layouts/d2talalayout/internal/engine/testdata/fuzz")
inventory = {"internal_fixtures": [
    {"upstream_path": str(path.relative_to(source)), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    for base in INTERNAL_ROOTS for path in sorted((source / base).rglob("*")) if path.is_file()]}
binary = REFERENCE / "bin/tala-engine-oracle"
rows = []
field_counts = Counter()
for fixture in inventory["internal_fixtures"]:
    path = source / fixture["upstream_path"]
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != fixture["sha256"]:
        raise RuntimeError(f"Fixture checksum mismatch: {path}")
    row = {"upstream_path": fixture["upstream_path"], "sha256": fixture["sha256"],
           "rust_status": "pending_direct_engine_fixture_adapter"}
    if "/fuzz/" in fixture["upstream_path"]:
        row["format"] = "go_fuzz_corpus"
        rows.append(row)
        continue
    value = json.loads(data)
    keys = {key.lower(): key for key in value} if isinstance(value, dict) else {}
    if "nodes" not in keys or "edges" not in keys or "containers" not in keys:
        row.update({"format": "ovg_or_other_json", "top_level_fields": list(value) if isinstance(value, dict) else None})
        rows.append(row)
        continue
    row.update({"format": "layout_graph_json", "role": "expected_output" if ".exp." in path.name else "input_or_regression",
                "top_level_fields": list(value), "nodes": len(value[keys["nodes"]] or []),
                "edges": len(value[keys["edges"]] or [])})
    field_counts.update(value.keys())
    try:
        result = subprocess.run([str(binary), "-operation", "roundtrip"], input=data,
                                capture_output=True, timeout=120)
        row.update({"go_decode_exit": result.returncode,
                    "go_decode_stderr": result.stderr.decode(errors="replace")})
        if result.returncode == 0:
            row["go_canonical_sha256"] = hashlib.sha256(result.stdout).hexdigest()
            canonical = json.loads(result.stdout)
            row["canonical_top_level_fields"] = list(canonical)
    except subprocess.TimeoutExpired:
        row["harness_timeout_seconds"] = 120
    rows.append(row)
report = {"scope": "format inventory and pristine Go decoder coverage; no Rust parity passes",
          "source_commit": MANIFEST["d2"]["commit"],
          "oracle": json.loads(binary.with_suffix(".provenance.json").read_text()),
          "format_counts": dict(Counter(row["format"] for row in rows)),
          "graph_field_presence": dict(field_counts),
          "go_decode_successes": sum(row.get("go_decode_exit") == 0 for row in rows),
          "go_decode_failures": sum(row.get("go_decode_exit", 0) != 0 or "harness_timeout_seconds" in row for row in rows),
          "rust_parity_passes": 0, "fixtures": rows}
report_path = ROOT / "oracle-cache/reports/engine-fixture-inventory.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
verify_source()
print(json.dumps({key: report[key] for key in ("format_counts", "go_decode_successes", "go_decode_failures", "rust_parity_passes")}, indent=2))
