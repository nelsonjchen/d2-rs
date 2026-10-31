#!/usr/bin/env python3
"""Compare the multiboard PDF corpus across pinned D2 theme IDs."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile


ROOT = Path(__file__).resolve().parents[1]
THEME_IDS = [0, 1, 3, 4, 5, 6, 7, 8, 100, 101, 102, 103, 104, 105, 200, 201, 300, 301, 302, 303]
SOURCE = ROOT / "tests/d2/pdf-multiboard-links.d2"
PDF_CHECK_SPEC = importlib.util.spec_from_file_location(
    "pdf_parity", ROOT / "scripts/check-pdf-parity.py"
)
assert PDF_CHECK_SPEC and PDF_CHECK_SPEC.loader
PDF_CHECK = importlib.util.module_from_spec(PDF_CHECK_SPEC)
PDF_CHECK_SPEC.loader.exec_module(PDF_CHECK)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--source-go",
        type=Path,
        default=ROOT / ".reference/bin/d2-v0.9.0-source-go1.27",
    )
    parser.add_argument(
        "--official",
        type=Path,
        default=ROOT / ".reference/bin/d2-v0.9.0-official",
    )
    parser.add_argument("--rust", type=Path, default=ROOT / "target/release/d2-rs")
    parser.add_argument("--layout", choices=("tala-rs", "dagre", "elk"), default="tala-rs")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "oracle-cache/reports/pdf-multiboard-all-themes.json",
    )
    args = parser.parse_args()
    if args.dpi <= 0:
        parser.error("--dpi must be positive")
    for binary in (args.source_go, args.official, args.rust):
        if not binary.is_file():
            parser.error(f"binary does not exist: {binary}")
    if not SOURCE.is_file():
        parser.error(f"fixture does not exist: {SOURCE}")
    if not shutil.which("pdftoppm"):
        parser.error("pdftoppm is required to rasterize PDF pages")

    references = (
        ("source_go1.27", args.source_go),
        ("official", args.official),
    )
    themes = []
    with tempfile.TemporaryDirectory(prefix="d2-rs-pdf-theme-matrix-") as temporary:
        temporary_root = Path(temporary)
        for theme_id in THEME_IDS:
            reference_rows = {}
            for reference_name, go_binary in references:
                row = PDF_CHECK.run_pair(
                    go_binary,
                    args.rust,
                    SOURCE,
                    args.layout,
                    [f"--theme={theme_id}"],
                    temporary_root / f"theme-{theme_id}" / reference_name,
                    args.dpi,
                )
                row["name"] = "multi-board-links"
                reference_rows[reference_name] = row
            themes.append({"theme_id": theme_id, "references": reference_rows})

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-pdf-multiboard-theme-matrix",
        "source": str(SOURCE),
        "source_sha256": PDF_CHECK.sha256(SOURCE),
        "layout": args.layout,
        "dpi": args.dpi,
        "rasterizer": "pdftoppm",
        "normalization": "PDF bytes and metadata ignored; rendered page dimensions and RGB pixels compared exactly",
        "references": {
            name: {
                "path": str(binary.resolve()),
                "sha256": PDF_CHECK.sha256(binary),
                "go_version": "1.27.0" if name == "source_go1.27" else None,
            }
            for name, binary in references
        },
        "rust_binary": str(args.rust.resolve()),
        "rust_binary_sha256": PDF_CHECK.sha256(args.rust),
        "themes": themes,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    total = len(THEME_IDS) * len(references)
    matched = sum(
        bool(row.get("matched"))
        for theme in themes
        for row in theme["references"].values()
    )
    print(json.dumps({"matched": matched, "total": total, "output": str(args.output)}))
    return 0 if matched == total else 1


if __name__ == "__main__":
    raise SystemExit(main())
