#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Compare the raw ELK wire corpus with pinned elk-go and record exact output parity."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

from exact_json import decode_output, difference
from references import MANIFEST, REFERENCE, ROOT, verify_source

GO_ORACLE = REFERENCE / "bin/elk-wire-oracle"
GO_BUILD_RECORD = REFERENCE / "elk-wire-oracle-build.json"
RUST_BINARY = ROOT / "target/debug/examples/elk-wire"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def normalize_omitted_root_origin(go_output: dict, rust_output: dict) -> None:
    """Treat an omitted default root origin like elk-go's explicit (0, 0)."""
    for coordinate in ("x", "y"):
        go_value = go_output.get(coordinate)
        rust_value = rust_output.get(coordinate)
        if coordinate not in go_output and coordinate in rust_output and rust_value == 0:
            go_output[coordinate] = 0
        if coordinate not in rust_output and coordinate in go_output and go_value == 0:
            rust_output[coordinate] = 0


def default_fixtures() -> list[Path]:
    return sorted(
        path
        for path in (ROOT / "tests/elk").glob("*.json")
        if not path.name.endswith("-go-v090.json")
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--fixture",
        action="append",
        type=Path,
        help="wire graph to compare; repeat to select several (default: tests/elk/*.json inputs)",
    )
    parser.add_argument(
        "--report",
        type=Path,
        default=ROOT / "oracle-cache/reports/elk-wire-parity.json",
        help="write the machine-readable comparison report here",
    )
    args = parser.parse_args()

    source = verify_source()
    if not GO_ORACLE.is_file() or not GO_BUILD_RECORD.is_file():
        raise SystemExit("Missing pinned raw ELK oracle; run scripts/build-elk-oracle.py first")

    build_record = json.loads(GO_BUILD_RECORD.read_text())
    go_build_info = build_record.get("build_info", "")
    if build_record.get("source_commit") != MANIFEST["d2"]["commit"]:
        raise SystemExit("ELK oracle build record points at a different D2 source commit")
    if build_record.get("go_toolchain") != MANIFEST["d2"]["go_toolchain"]:
        raise SystemExit("ELK oracle build record uses a different Go toolchain")
    if build_record.get("binary_sha256") != sha256(GO_ORACLE):
        raise SystemExit("ELK oracle binary does not match its verified build record")
    if MANIFEST["d2"]["go_toolchain"] not in go_build_info:
        raise SystemExit("ELK oracle build record does not confirm the pinned Go toolchain")
    if "github.com/d2lang/elk-go\tv0.2.0\t" not in go_build_info:
        raise SystemExit("ELK oracle build record does not confirm elk-go v0.2.0")

    subprocess.run(
        ["cargo", "build", "-p", "d2-rs-elk", "--example", "elk-wire"],
        cwd=ROOT,
        check=True,
    )
    if not RUST_BINARY.is_file():
        raise SystemExit(f"Rust ELK wire example was not produced: {RUST_BINARY}")

    fixtures = args.fixture or default_fixtures()
    results = []
    for fixture_arg in fixtures:
        fixture = fixture_arg if fixture_arg.is_absolute() else ROOT / fixture_arg
        payload = fixture.read_bytes()
        go = subprocess.run(
            [str(GO_ORACLE)],
            input=payload,
            capture_output=True,
            cwd=source,
            timeout=120,
        )
        rust = subprocess.run(
            [str(RUST_BINARY)],
            input=payload,
            capture_output=True,
            cwd=ROOT,
            timeout=180,
        )
        row = {
            "fixture": str(fixture.relative_to(ROOT)),
            "input_sha256": hashlib.sha256(payload).hexdigest(),
            "go_exit": go.returncode,
            "rust_exit": rust.returncode,
        }
        if go.returncode == 0 and rust.returncode == 0:
            go_output = decode_output(go.stdout)
            rust_output = decode_output(rust.stdout)
            normalize_omitted_root_origin(go_output, rust_output)
            mismatch = difference(go_output, rust_output)
            row["passed"] = mismatch is None
            if mismatch is not None:
                row["difference"] = mismatch
        else:
            row["passed"] = False
            row["go_error"] = go.stderr.decode(errors="replace")
            row["rust_error"] = rust.stderr.decode(errors="replace")
        results.append(row)
        print(f"{row['fixture']}: {'PASS' if row['passed'] else 'FAIL'}")
        if "difference" in row:
            print(json.dumps(row["difference"], sort_keys=True))

    report = {
        "scope": "complete raw ELK JSON output, including hierarchy, boxes, labels, ports, and routed sections",
        "source_commit": MANIFEST["d2"]["commit"],
        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
        "elk_go_module": "github.com/d2lang/elk-go v0.2.0",
        "go_oracle_sha256": sha256(GO_ORACLE),
        "oracle_build_record_sha256": sha256(GO_BUILD_RECORD),
        "rust_toolchain": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "normalization": "Only an omitted root x/y is filled with zero when the other output explicitly has zero.",
        "passed": sum(row["passed"] for row in results),
        "failed": sum(not row["passed"] for row in results),
        "results": results,
    }
    report_path = args.report if args.report.is_absolute() else ROOT / args.report
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    verify_source()
    print(f"{report['passed']} passed, {report['failed']} failed; report: {report_path}")
    return int(report["failed"] != 0)


if __name__ == "__main__":
    sys.exit(main())
