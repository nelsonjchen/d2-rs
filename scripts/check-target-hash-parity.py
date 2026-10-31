#!/usr/bin/env python3
"""Compare D2 target diagram hashes in the pinned Go and Rust SVG outputs."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = {
    "geometry": ROOT.parent / "d2/e2etests-cli/testdata/raster/geometry.d2",
    "animation": ROOT.parent / "d2/e2etests-cli/testdata/raster/animation.d2",
    "local-icon": ROOT.parent / "d2/e2etests-cli/testdata/raster/local-icon.d2",
    "markdown": ROOT.parent / "d2/e2etests-cli/testdata/raster/markdown.d2",
    "unlabelled-connection-link": ROOT.parent / "d2/e2etests-cli/testdata/raster/unlabelled-connection-link.d2",
    "markdown-strikethrough": ROOT / "tests/d2/markdown-strikethrough.d2",
    "markdown-autolink": ROOT / "tests/d2/markdown-autolink.d2",
    "markdown-email": ROOT / "tests/d2/markdown-email.d2",
    "markdown-nested-inline": ROOT / "tests/d2/markdown-nested-inline.d2",
    "nested-order": ROOT / "tests/d2/nested-order.d2",
    "dagre-compound-label-midpoint": ROOT / "tests/d2/dagre-compound-label-midpoint.d2",
    "dagre-compound-wide-label": ROOT / "tests/d2/dagre-compound-wide-label.d2",
}
HASH = re.compile(rb'class="(d2-[0-9]+) d2-svg"')


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_svg(binary: Path, source: Path, rust: bool) -> tuple[int, bytes, str]:
    if rust:
        command = [str(binary), "--stdout-format=svg", str(source), "-"]
    else:
        with tempfile.NamedTemporaryFile(suffix=".svg") as output:
            process = subprocess.run(
                [str(binary), str(source), output.name],
                capture_output=True,
                check=False,
            )
            return process.returncode, Path(output.name).read_bytes(), process.stderr.decode(errors="replace")
    process = subprocess.run(command, capture_output=True, check=False)
    return process.returncode, process.stdout, process.stderr.decode(errors="replace")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--go", type=Path, default=ROOT / ".reference/bin/d2-v0.9.0-source-go1.27")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/release/d2-rs")
    parser.add_argument("--output", type=Path, default=ROOT / "oracle-cache/reports/target-hash-parity.json")
    args = parser.parse_args()
    if not args.go.is_file() or not args.rust.is_file():
        parser.error("build the Go and Rust binaries before running this check")

    rows = []
    for name, source in FIXTURES.items():
        row = {"name": name, "source": str(source), "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest()}
        go_exit, go_svg, go_stderr = run_svg(args.go, source, False)
        rust_exit, rust_svg, rust_stderr = run_svg(args.rust, source, True)
        go_match = HASH.search(go_svg)
        rust_match = HASH.search(rust_svg)
        row.update(
            {
                "go_hash": go_match.group(1).decode() if go_match else None,
                "rust_hash": rust_match.group(1).decode() if rust_match else None,
                "matched": bool(go_match and rust_match and go_match.group(1) == rust_match.group(1)),
                "go_exit": go_exit,
                "rust_exit": rust_exit,
            }
        )
        if go_exit or rust_exit or not go_match or not rust_match:
            row["stderr"] = {"go": go_stderr[-2000:], "rust": rust_stderr[-2000:]}
        rows.append(row)

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-target-hash-parity",
        "reference": {
            "d2_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
            "go_binary": str(args.go),
            "go_binary_sha256": sha256(args.go),
            "rust_binary": str(args.rust),
            "rust_binary_sha256": sha256(args.rust),
            "metric": "inner SVG d2-<fnv32> target identity",
        },
        "fixtures": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"matched": sum(row["matched"] for row in rows), "total": len(rows)}))
    return 0 if all(row["matched"] for row in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
