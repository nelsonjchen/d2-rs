#!/usr/bin/env python3
"""Compare source-shaped ASCII fixtures with the pinned v0.9.0 oracle."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
ORACLE = ROOT / ".reference/bin/d2-v0.9.0-source-go1.27"
# Qualification runs against the optimized release binary.  The debug target
# is intentionally disposable during workspace cleanup and may not exist.
RUST = ROOT / "target/release/d2-rs"
REPORT = ROOT / "oracle-cache/reports/ascii-parity.json"

SHAPES = [
    "rectangle",
    "page",
    "hexagon",
    "person",
    "stored_data",
    "cylinder",
    "package",
    "parallelogram",
    "queue",
    "step",
    "callout",
    "document",
    "diamond",
    "oval",
    "circle",
    "cloud",
]


def run(binary: Path, source: bytes, mode: str | None = None) -> bytes:
    command = [str(binary), "--stdout-format=ascii", "-"]
    if mode is not None:
        command.insert(-1, f"--ascii-mode={mode}")
    result = subprocess.run(
        command,
        input=source,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        timeout=30,
    )
    if result.returncode:
        raise RuntimeError(
            f"{binary} failed ({result.returncode}): {result.stderr.decode(errors='replace')}"
        )
    return result.stdout


CHARSET_CASES = {
    "simple-edge": b"a -> b\n",
    "markdown-strikethrough": (
        b"hey: |md\n"
        b"before ~~strikethrough~~ after\n"
        b"|\n"
    ),
}


def main() -> int:
    for binary in (ORACLE, RUST):
        if not binary.is_file():
            raise SystemExit(f"missing binary: {binary}; build d2-rs first")
    rows = []
    for shape in SHAPES:
        source = f"a: {{shape: {shape}}}\n".encode()
        go = run(ORACLE, source)
        rust = run(RUST, source)
        rows.append(
            {
                "shape": shape,
                "match": go == rust,
                "go_sha256": hashlib.sha256(go).hexdigest(),
                "rust_sha256": hashlib.sha256(rust).hexdigest(),
                "go_bytes": len(go),
                "rust_bytes": len(rust),
            }
        )
    charset_rows = []
    for name, source in CHARSET_CASES.items():
        for mode in ("extended", "standard", "unknown"):
            go = run(ORACLE, source, mode)
            rust = run(RUST, source, mode)
            charset_rows.append(
                {
                    "case": name,
                    "mode": mode,
                    "match": go == rust,
                    "go_sha256": hashlib.sha256(go).hexdigest(),
                    "rust_sha256": hashlib.sha256(rust).hexdigest(),
                    "go_bytes": len(go),
                    "rust_bytes": len(rust),
                }
            )
    report = {
        "kind": "d2-v0.9.0-ascii-shape-and-charset-parity",
        "oracle": {
            "binary": str(ORACLE),
            "d2_version": "v0.9.0",
            "commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
            "go_toolchain": "go1.27.0",
        },
        "rust_binary": str(RUST),
        "matches": sum(row["match"] for row in rows),
        "total": len(rows),
        "fixtures": rows,
        "charset_matrix": charset_rows,
        "charset_matches": sum(row["match"] for row in charset_rows),
        "charset_total": len(charset_rows),
    }
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                key: report[key]
                for key in ("matches", "total", "charset_matches", "charset_total")
            },
            sort_keys=True,
        )
    )
    return 0 if report["matches"] == report["total"] and report["charset_matches"] == report["charset_total"] else 1


if __name__ == "__main__":
    sys.exit(main())
