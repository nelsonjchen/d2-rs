#!/usr/bin/env python3
"""Compare rendered PDF pages from pinned Go D2 and native d2-rs."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURES = {
    "single-board-title": ROOT / "tests/d2/pdf-single-board-title.d2",
    "multi-board-links": ROOT / "tests/d2/pdf-multiboard-links.d2",
}
RENDER_CHECK_SPEC = importlib.util.spec_from_file_location(
    "render_parity", ROOT / "scripts/check-render-parity.py"
)
assert RENDER_CHECK_SPEC and RENDER_CHECK_SPEC.loader
RENDER_CHECK = importlib.util.module_from_spec(RENDER_CHECK_SPEC)
RENDER_CHECK_SPEC.loader.exec_module(RENDER_CHECK)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pdf_pages(path: Path, prefix: Path, dpi: int) -> list[Path]:
    subprocess.run(
        ["pdftoppm", "-r", str(dpi), "-png", str(path), str(prefix)],
        check=True,
        capture_output=True,
    )
    return sorted(
        prefix.parent.glob(f"{prefix.name}-*.png"),
        key=lambda page: int(page.stem.rsplit("-", 1)[1]),
    )


def run_pair(
    go_binary: Path,
    rust_binary: Path,
    source: Path,
    layout: str,
    common_args: list[str],
    temporary_root: Path,
    dpi: int,
) -> dict[str, object]:
    temporary_root.mkdir(parents=True, exist_ok=True)
    go_pdf = temporary_root / "go.pdf"
    rust_pdf = temporary_root / "rust.pdf"
    go_layout = "tala" if layout == "tala-rs" else layout
    go = subprocess.run(
        [str(go_binary), f"--layout={go_layout}", *common_args, str(source), str(go_pdf)],
        capture_output=True,
        text=True,
        check=False,
    )
    rust = subprocess.run(
        [str(rust_binary), f"--layout={layout}", *common_args, str(source), str(rust_pdf)],
        capture_output=True,
        text=True,
        check=False,
    )
    row: dict[str, object] = {
        "source": str(source),
        "source_sha256": sha256(source),
        "go_exit": go.returncode,
        "rust_exit": rust.returncode,
    }
    if go.returncode or rust.returncode:
        row["matched"] = False
        row["go_stderr"] = go.stderr[-2000:]
        row["rust_stderr"] = rust.stderr[-2000:]
        return row

    go_pages = pdf_pages(go_pdf, temporary_root / "go-page", dpi)
    rust_pages = pdf_pages(rust_pdf, temporary_root / "rust-page", dpi)
    page_rows = []
    for page_index in range(max(len(go_pages), len(rust_pages))):
        if page_index >= len(go_pages) or page_index >= len(rust_pages):
            page_rows.append(
                {
                    "page": page_index + 1,
                    "matched": False,
                    "go_present": page_index < len(go_pages),
                    "rust_present": page_index < len(rust_pages),
                }
            )
            continue
        go_width, go_height, go_pixels = RENDER_CHECK.decode_png(go_pages[page_index])
        rust_width, rust_height, rust_pixels = RENDER_CHECK.decode_png(rust_pages[page_index])
        page_row: dict[str, object] = {
            "page": page_index + 1,
            "go_size": [go_width, go_height],
            "rust_size": [rust_width, rust_height],
            "matched": False,
        }
        if (go_width, go_height) == (rust_width, rust_height):
            pixel_diff = (
                None
                if go_pixels == rust_pixels
                else RENDER_CHECK.summarize_pixel_differences(
                    go_width, go_height, go_pixels, rust_pixels
                )
            )
            page_row["matched"] = pixel_diff is None
            page_row["different_pixels"] = (
                0
                if pixel_diff is None
                else sum(int(item["changed_pixels"]) for item in pixel_diff["rows"])
            )
            if pixel_diff is not None:
                page_row["difference_bounds"] = pixel_diff["bounds"]
                page_row["different_rows"] = len(pixel_diff["rows"])
        page_rows.append(page_row)

    row["go_pages"] = len(go_pages)
    row["rust_pages"] = len(rust_pages)
    row["pages"] = page_rows
    row["matched"] = len(go_pages) == len(rust_pages) and all(
        page["matched"] for page in page_rows
    )
    return row


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--go",
        type=Path,
        default=ROOT / ".reference/bin/d2-v0.9.0-source-go1.27",
    )
    parser.add_argument("--rust", type=Path, default=ROOT / "target/release/d2-rs")
    parser.add_argument(
        "--layout", choices=("tala-rs", "dagre", "elk"), default="dagre"
    )
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument(
        "--arg",
        dest="common_args",
        action="append",
        default=[],
        help="pass the same CLI option to both binaries; repeat as needed",
    )
    parser.add_argument(
        "--output", type=Path, default=ROOT / "oracle-cache/reports/pdf-source-go1.27.json"
    )
    parser.add_argument("fixtures", nargs="*", help="name=path or a default fixture name")
    args = parser.parse_args()
    if args.dpi <= 0:
        parser.error("--dpi must be positive")
    if not args.go.is_file() or not args.rust.is_file():
        parser.error("build the pinned Go and Rust binaries before this check")
    if not shutil.which("pdftoppm"):
        parser.error("pdftoppm is required to rasterize the PDF pages")

    fixtures = DEFAULT_FIXTURES.copy()
    if args.fixtures:
        fixtures = {}
        for value in args.fixtures:
            if "=" in value:
                name, raw_path = value.split("=", 1)
                fixtures[name] = Path(raw_path).resolve()
            elif value in DEFAULT_FIXTURES:
                fixtures[value] = DEFAULT_FIXTURES[value]
            else:
                parser.error(f"unknown fixture {value!r}; use name=path for a custom input")

    with tempfile.TemporaryDirectory(prefix="d2-rs-pdf-parity-") as temporary:
        temporary_root = Path(temporary)
        rows = []
        for name, source in fixtures.items():
            row = run_pair(
                args.go,
                args.rust,
                source,
                args.layout,
                args.common_args,
                temporary_root / name,
                args.dpi,
            )
            row["name"] = name
            rows.append(row)

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-pdf-page-pixel-parity",
        "reference": {
            "d2_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
            "go_version": "1.27.0",
            "go_binary": str(args.go.resolve()),
            "go_binary_sha256": sha256(args.go),
            "rust_binary": str(args.rust.resolve()),
            "rust_binary_sha256": sha256(args.rust),
            "layout": args.layout,
            "arguments": args.common_args,
            "rasterizer": "pdftoppm",
            "dpi": args.dpi,
            "normalization": "PDF bytes and metadata ignored; rendered page dimensions and RGB pixels compared exactly",
        },
        "fixtures": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    matched = sum(bool(row.get("matched")) for row in rows)
    print(json.dumps({"matched": matched, "total": len(rows)}))
    return 0 if matched == len(rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
