#!/usr/bin/env python3
"""Compare complete original graphjson output without projection or sorting.

The current compact Rust diagnostic adapter is not a complete graphjson codec.
Missing metadata, field spellings and array order therefore remain failures.
This gate does not replace internal geometry/operation localization tools.
"""
from __future__ import annotations
import argparse
import json
import subprocess
from pathlib import Path
from exact_json import decode_output, difference
from references import ROOT, MANIFEST, REPORTS, digest, read_report, verify_source


def compare(source_exit, source_out, source_err, native_exit, native_out, native_err,
            source_timeout=False, native_timeout=False):
    if source_timeout or native_timeout:
        return {"passed": False, "kind": "timeout"}
    if source_exit != 0 or native_exit != 0:
        matched = (source_exit == native_exit == 1 and source_out == native_out == b""
                   and source_err == native_err and bool(source_err))
        return {"passed": matched, "kind": "error"}
    if source_err != native_err:
        return {"passed": False, "kind": "stderr"}
    try:
        left, right = decode_output(source_out), decode_output(native_out)
        for value in (left, right):
            if not isinstance(value, dict) or not isinstance(value.get("nodes"), list) or not isinstance(value.get("edges"), list):
                raise ValueError("missing complete node/edge inventories")
            # Reject NaN/Infinity accepted by Python's permissive JSON decoder.
            json.dumps(value, allow_nan=False)
        mismatch = difference(left, right)
        return {"passed": mismatch is None, "kind": "output", "first_mismatch": mismatch}
    except (ValueError, TypeError, OverflowError) as error:
        return {"passed": False, "kind": "invalid_output", "error": str(error)}


def capture(command, data, timeout):
    try:
        p = subprocess.run(command, input=data, capture_output=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr, False
    except subprocess.TimeoutExpired as error:
        return 124, error.stdout or b"", error.stderr or b"", True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-binary", type=Path, required=True)
    parser.add_argument("--rust-binary", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--seed", action="append", type=int)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--fixture", action="append")
    args = parser.parse_args()
    source = verify_source()
    inventory = REPORTS / "engine-fixture-inventory.json"
    fixtures = [r for r in read_report(inventory.name, "scripts/audit-engine-fixtures.py")["fixtures"]
                if r.get("format") == "layout_graph_json" and r.get("role") != "expected_output"]
    if args.fixture:
        by_path = {r["upstream_path"]: r for r in fixtures}
        fixtures = [by_path[name] for name in args.fixture]
    if not fixtures:
        raise RuntimeError("empty fixture inventory")
    args.evidence.mkdir(parents=True, exist_ok=False)
    source_binary, native_binary = args.source_binary.resolve(), args.rust_binary.resolve()
    bindings = {"source_binary_sha256": digest(source_binary), "native_binary_sha256": digest(native_binary),
                "fixture_inventory_sha256": digest(inventory), "checker_sha256": digest(Path(__file__)),
                "comparator_sha256": digest(ROOT / "scripts/exact_json.py")}
    rows = []
    for seed in args.seed or [1]:
        for i, fixture in enumerate(fixtures):
            input_path = source / fixture["upstream_path"]
            if digest(input_path) != fixture["sha256"]:
                raise RuntimeError(f"changed fixture: {input_path}")
            data = input_path.read_bytes()
            outputs = [capture([str(source_binary), "-seed", str(seed)], data, args.timeout),
                       capture([str(native_binary), str(seed)], data, args.timeout)]
            row = {"fixture": fixture["upstream_path"], "seed": seed, "input_sha256": fixture["sha256"]}
            for label, output in zip(("source", "native"), outputs):
                row[label + "_exit"], row[label + "_timeout"] = output[0], output[3]
                for name, content in zip(("stdout", "stderr"), output[1:3]):
                    path = args.evidence / f"{seed}-{i:03d}.{label}.{name}"
                    path.write_bytes(content)
                    row[label + "_" + name] = {"path": str(path.resolve()), "sha256": digest(path)}
            a, b = outputs
            row.update(compare(*a[:3], *b[:3], a[3], b[3]))
            rows.append(row)
            print(f"seed {seed} {fixture['upstream_path']}: {row['kind']} {'PASS' if row['passed'] else 'FAIL'}", flush=True)
    verify_source()
    if bindings["source_binary_sha256"] != digest(source_binary) or bindings["native_binary_sha256"] != digest(native_binary):
        raise RuntimeError("binary changed during capture")
    report = {"scope": "complete ordered graphjson serialization and exact error boundary; no projections",
              "source_commit": MANIFEST["d2"]["commit"], "bindings": bindings,
              "total": len(rows), "passed": sum(r["passed"] for r in rows),
              "failed": sum(not r["passed"] for r in rows), "results": rows}
    (args.evidence / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: report[k] for k in ("total", "passed", "failed")}))
    return int(bool(report["failed"]))

if __name__ == "__main__":
    raise SystemExit(main())
