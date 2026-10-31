#!/usr/bin/env python3
"""Compare the unsupported native-Markdown HTML error with D2 v0.9.0."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests/d2/markdown-raw-html-unsupported.d2"
OFFICIAL = ROOT / ".reference/bin/d2-v0.9.0-official"
GO = ROOT / ".reference/bin/d2-v0.9.0-source-go1.27"
RUST = ROOT / "target/release/d2-rs"
EXPECTED_DETAIL = "native Markdown SVG does not support HTML element <u>"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--official", type=Path, default=OFFICIAL)
    parser.add_argument("--go", type=Path, default=GO)
    parser.add_argument("--rust", type=Path, default=RUST)
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    parser.add_argument(
        "--output", type=Path, default=ROOT / "oracle-cache/reports/markdown-raw-html-error-parity.json"
    )
    args = parser.parse_args()
    binaries = {
        "official_release": args.official,
        "go_1_27_source": args.go,
        "rust_release": args.rust,
    }
    missing = [str(path) for path in binaries.values() if not path.is_file()]
    if missing:
        parser.error(f"missing binary: {', '.join(missing)}")
    source = args.fixture.read_bytes()
    runs = {}
    for name, binary in binaries.items():
        completed = subprocess.run(
            [str(binary), str(args.fixture), "-"],
            cwd=ROOT,
            capture_output=True,
            check=False,
        )
        runs[name] = {
            "exit_code": completed.returncode,
            "stdout": completed.stdout.decode("utf-8", errors="replace"),
            "stderr": completed.stderr.decode("utf-8", errors="replace"),
        }
    expected_suffix = (EXPECTED_DETAIL + "\n").encode()
    identical_cli_results = len(
        {
            (run["exit_code"], run["stdout"], run["stderr"])
            for run in runs.values()
        }
    ) == 1
    result = {
        "fixture": str(args.fixture),
        "source_sha256": hashlib.sha256(source).hexdigest(),
        "runs": runs,
        "identical_cli_results": identical_cli_results,
        "matched": (
            identical_cli_results
            and all(run["exit_code"] == 1 for run in runs.values())
            and all(run["stdout"] == "" for run in runs.values())
            and all(
                run["stderr"].startswith("err: failed to compile ")
                and run["stderr"].endswith(EXPECTED_DETAIL + "\n")
                for run in runs.values()
            )
        ),
    }
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-markdown-raw-html-error-parity",
        "reference": {
            "commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
            "go_toolchain": "go1.27.0",
            "binaries": {
                name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                for name, path in binaries.items()
            },
        },
        "result": result,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"matched": result["matched"], "error": EXPECTED_DETAIL}))
    return 0 if result["matched"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
