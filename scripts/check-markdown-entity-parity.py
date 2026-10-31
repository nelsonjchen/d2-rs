#!/usr/bin/env python3
"""Compare native Markdown entity text runs with the pinned D2 oracle."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from xml.etree import ElementTree


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests/d2/markdown-entities.d2"
GO = ROOT / ".reference/bin/d2-v0.9.0-source-go1.27"
RUST = ROOT / "target/release/d2-rs"


def text_runs(path: Path) -> list[str]:
    root = ElementTree.parse(path).getroot()
    return [
        "".join(node.itertext())
        for node in root.iter()
        if node.tag.rsplit("}", 1)[-1] == "text"
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--go", type=Path, default=GO)
    parser.add_argument("--rust", type=Path, default=RUST)
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    parser.add_argument(
        "--output", type=Path, default=ROOT / "oracle-cache/reports/markdown-entity-parity.json"
    )
    args = parser.parse_args()
    if not args.go.is_file() or not args.rust.is_file():
        parser.error("build the pinned Go and Rust release binaries first")

    source = args.fixture.read_bytes()
    with tempfile.TemporaryDirectory(prefix="d2-rs-markdown-entity-") as directory:
        temporary = Path(directory)
        go_svg = temporary / "go.svg"
        rust_svg = temporary / "rust.svg"
        go = subprocess.run(
            [str(args.go), str(args.fixture), str(go_svg)],
            capture_output=True,
            text=True,
            check=False,
        )
        rust = subprocess.run(
            [str(args.rust), "--stdout-format=svg", str(args.fixture), "-"],
            capture_output=True,
            check=False,
        )
        rust_svg.write_bytes(rust.stdout)
        row = {
            "fixture": str(args.fixture),
            "source_sha256": hashlib.sha256(source).hexdigest(),
            "go_exit": go.returncode,
            "rust_exit": rust.returncode,
        }
        if go.returncode == 0 and rust.returncode == 0:
            go_runs = text_runs(go_svg)
            rust_runs = text_runs(rust_svg)
            row.update(
                {
                    "go_text_runs": go_runs,
                    "rust_text_runs": rust_runs,
                    "matched": go_runs == rust_runs,
                }
            )
        else:
            row.update({"matched": False, "go_stderr": go.stderr, "rust_stderr": rust.stderr.decode(errors="replace")})

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-markdown-html-entity-text-parity",
        "reference": {
            "commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
            "go_toolchain": "go1.27.0",
            "go_binary": str(args.go),
            "rust_binary": str(args.rust),
        },
        "result": row,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"matched": row["matched"], "text_runs": row.get("go_text_runs", [])}, ensure_ascii=False))
    return 0 if row["matched"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
