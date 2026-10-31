#!/usr/bin/env python3
"""Compare supported raw inline Markdown against the pinned D2 oracle.

The SVG wrappers and theme CSS are intentionally different between the Go and
Rust renderers.  This check compares the observable native Markdown contract:
successful compilation, canvas/label dimensions, text-run order and geometry,
and strikethrough primitives, while rejecting a foreignObject fallback.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests/d2/markdown-raw-inline.d2"
GO = ROOT / ".reference/bin/d2-v0.9.0-source-go1.27"
RUST = ROOT / "target/release/d2-rs"
NUMBER = r"[-+0-9.eE]+"


def floats(value: str) -> tuple[float, ...]:
    return tuple(float(part) for part in re.findall(NUMBER, value))


def markdown_geometry(svg: str) -> dict[str, object]:
    outer = re.search(r'<svg[^>]*viewBox="([^"]+)"', svg)
    inner = re.search(
        r'<svg x="([^"]+)" y="([^"]+)" width="([^"]+)" height="([^"]+)" viewBox="([^"]+)" overflow="hidden">',
        svg,
    )
    text_runs = [
        (float(x), float(y), text)
        for x, y, text in re.findall(
            r'<text x="([^"]+)" y="([^"]+)"[^>]*>([^<]*)</text>', svg
        )
    ]
    strike_rects = [
        tuple(float(value) for value in (x, y, width, height))
        for x, y, width, height in re.findall(
            rf'<rect x="({NUMBER})" y="({NUMBER})" width="({NUMBER})" height="(1\.000)"[^>]*/>',
            svg,
        )
    ]
    if outer is None or inner is None:
        raise ValueError("missing D2 SVG or native Markdown viewport")
    return {
        "outer_viewbox": floats(outer.group(1)),
        # The Go renderer keeps the diagram's negative viewBox origin on the
        # inner SVG, while Rust translates that origin into the outer canvas.
        # The observable Markdown viewport is its width and height.
        "inner_viewport": tuple(float(value) for value in inner.groups()[2:4]),
        "inner_viewbox": floats(inner.group(5)),
        "text_runs": text_runs,
        "strikethrough": strike_rects,
        "foreign_object": "<foreignObject" in svg,
    }


def close(left: object, right: object) -> bool:
    if isinstance(left, (tuple, list)) and isinstance(right, (tuple, list)):
        return len(left) == len(right) and all(close(a, b) for a, b in zip(left, right))
    if isinstance(left, float) and isinstance(right, float):
        return abs(left - right) <= 0.001
    return left == right


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--go", type=Path, default=GO)
    parser.add_argument("--rust", type=Path, default=RUST)
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    parser.add_argument(
        "--output", type=Path, default=ROOT / "oracle-cache/reports/markdown-raw-inline-parity.json"
    )
    args = parser.parse_args()
    if not args.go.is_file() or not args.rust.is_file():
        parser.error("build the pinned Go and Rust release binaries first")

    source = args.fixture.read_bytes()
    with tempfile.TemporaryDirectory(prefix="d2-rs-raw-inline-") as directory:
        root = Path(directory)
        rows = {}
        for name, binary in (("go", args.go), ("rust", args.rust)):
            output = root / f"{name}.svg"
            result = subprocess.run(
                [str(binary), str(args.fixture), str(output)],
                capture_output=True,
                check=False,
            )
            rows[f"{name}_exit"] = result.returncode
            rows[f"{name}_stderr"] = result.stderr.decode(errors="replace").strip()
            if result.returncode == 0:
                rows[name] = markdown_geometry(output.read_text(encoding="utf-8"))

    go_geometry = rows.get("go")
    rust_geometry = rows.get("rust")
    comparable = go_geometry is not None and rust_geometry is not None
    matched = comparable and all(
        close(go_geometry[key], rust_geometry[key])
        for key in ("outer_viewbox", "inner_viewport", "inner_viewbox", "text_runs", "strikethrough")
    ) and not go_geometry["foreign_object"] and not rust_geometry["foreign_object"]
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-markdown-raw-inline-parity",
        "reference": {
            "d2_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
            "go_toolchain": "go1.27.0",
            "go_binary": str(args.go),
            "rust_binary": str(args.rust),
        },
        "fixture": str(args.fixture),
        "source_sha256": hashlib.sha256(source).hexdigest(),
        "matched": bool(matched),
        "comparison": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"matched": bool(matched), "fixture": str(args.fixture)}))
    return 0 if matched else 1


if __name__ == "__main__":
    raise SystemExit(main())
