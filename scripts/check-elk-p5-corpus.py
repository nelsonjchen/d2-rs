#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Compare ordered P5 orthogonal-routing traces across the checked-in ELK corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

from references import MANIFEST, REFERENCE, ROOT, verify_source

TRACE_CHECK = ROOT / "scripts/check-elk-p5-trace.py"
TRACE_BUILD_RECORD = REFERENCE / "elk-wire-trace-oracle-build.json"
EVENT_COUNT = re.compile(r"^(\d+) ordered P5 layer events match exactly", re.MULTILINE)


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
        default=ROOT / "oracle-cache/reports/elk-p5-trace-parity.json",
        help="write the machine-readable comparison report here",
    )
    args = parser.parse_args()

    source = verify_source()
    if not TRACE_BUILD_RECORD.is_file():
        raise SystemExit("Missing pinned ELK trace-oracle build record")
    build_record = json.loads(TRACE_BUILD_RECORD.read_text())
    if build_record.get("source_commit") != MANIFEST["d2"]["commit"]:
        raise SystemExit("ELK trace-oracle build record points at a different D2 commit")
    if build_record.get("go_toolchain") != MANIFEST["d2"]["go_toolchain"]:
        raise SystemExit("ELK trace oracle uses a different Go toolchain")

    fixtures = args.fixture or sorted(
        path
        for path in (ROOT / "tests/elk").glob("*.json")
        if not path.name.endswith("-go-v090.json")
    )
    rows = []
    for fixture_arg in fixtures:
        fixture = fixture_arg if fixture_arg.is_absolute() else ROOT / fixture_arg
        result = subprocess.run(
            [
                "uv",
                "run",
                "--no-project",
                "python",
                str(TRACE_CHECK),
                "--fixture",
                str(fixture),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=300,
        )
        output = result.stdout + result.stderr
        match = EVENT_COUNT.search(output)
        row = {
            "fixture": str(fixture.relative_to(ROOT)),
            "input_sha256": hashlib.sha256(fixture.read_bytes()).hexdigest(),
            "passed": result.returncode == 0 and match is not None,
        }
        if match:
            row["p5_layer_events"] = int(match.group(1))
        if not row["passed"]:
            row["exit_code"] = result.returncode
            row["output"] = output[-4000:]
        rows.append(row)
        print(
            f"{row['fixture']}: "
            f"{'PASS' if row['passed'] else 'FAIL'}"
            + (f" ({row['p5_layer_events']} events)" if match else "")
        )

    report = {
        "scope": "ordered elk-go P5 orthogonal-routing decisions, slots, and exact float bits",
        "source_commit": MANIFEST["d2"]["commit"],
        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
        "elk_go_module": "github.com/d2lang/elk-go v0.2.0",
        "trace_oracle_sha256": build_record.get("binary_sha256"),
        "rust_toolchain": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "normalization": (
            "Use each routed scope's sorted authored-node IDs instead of arena graph IDs; "
            "use per-scope layer index instead of global layer ID; identify temporary "
            "dummies by their position in each ordered node snapshot."
        ),
        "passed": sum(row["passed"] for row in rows),
        "failed": sum(not row["passed"] for row in rows),
        "results": rows,
    }
    report_path = args.report if args.report.is_absolute() else ROOT / args.report
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    verify_source()
    print(f"{report['passed']} passed, {report['failed']} failed; report: {report_path}")
    return int(report["failed"] != 0)


if __name__ == "__main__":
    sys.exit(main())
