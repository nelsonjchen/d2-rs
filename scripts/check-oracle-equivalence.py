#!/usr/bin/env python3
"""Verify that the pinned D2 release and source builds are equivalent.

The source build is used for instrumentation and detailed reverse engineering;
the downloaded release asset is the user-facing v0.9.0 oracle.  This check
keeps those roles independent and records whether they produce identical SVG,
PNG bytes, and decoded RGB pixels on the same inputs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import zlib


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_RELEASE = ROOT / ".reference/releases/macos-arm64/d2-v0.9.0/bin/d2"
DEFAULT_SOURCE = ROOT / ".reference/bin/d2-v0.9.0-source-go1.27"
DEFAULT_FIXTURES = {
    "geometry": ROOT.parent / "d2/e2etests-cli/testdata/raster/geometry.d2",
    "animation": ROOT.parent / "d2/e2etests-cli/testdata/raster/animation.d2",
    "local-icon": ROOT.parent / "d2/e2etests-cli/testdata/raster/local-icon.d2",
    "markdown": ROOT.parent / "d2/e2etests-cli/testdata/raster/markdown.d2",
    "unlabelled-connection-link": ROOT.parent
    / "d2/e2etests-cli/testdata/raster/unlabelled-connection-link.d2",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def decode_png(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    offset = 8
    compressed = bytearray()
    width = height = depth = color = None
    while offset < len(data):
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        kind = data[offset + 4 : offset + 8]
        payload = data[offset + 8 : offset + 8 + length]
        offset += length + 12
        if kind == b"IHDR":
            width, height, depth, color, compression, filtering, interlace = struct.unpack(
                ">IIBBBBB", payload
            )
            if (depth, color, compression, filtering, interlace) != (8, 2, 0, 0, 0):
                raise ValueError("only non-interlaced 8-bit RGB PNGs are supported")
        elif kind == b"IDAT":
            compressed.extend(payload)
        elif kind == b"IEND":
            break
    if width is None or height is None:
        raise ValueError(f"{path} has no IHDR")
    stride = width * 3
    raw = zlib.decompress(compressed)
    rows = []
    previous = bytearray(stride)
    cursor = 0
    for _ in range(height):
        filter_type = raw[cursor]
        cursor += 1
        row = bytearray(raw[cursor : cursor + stride])
        cursor += stride
        for index in range(stride):
            left = row[index - 3] if index >= 3 else 0
            up = previous[index]
            upper_left = previous[index - 3] if index >= 3 else 0
            if filter_type == 1:
                row[index] = (row[index] + left) & 255
            elif filter_type == 2:
                row[index] = (row[index] + up) & 255
            elif filter_type == 3:
                row[index] = (row[index] + (left + up) // 2) & 255
            elif filter_type == 4:
                estimate = left + up - upper_left
                pa = abs(estimate - left)
                pb = abs(estimate - up)
                pc = abs(estimate - upper_left)
                predictor = left if pa <= pb and pa <= pc else up if pb <= pc else upper_left
                row[index] = (row[index] + predictor) & 255
            elif filter_type != 0:
                raise ValueError(f"unsupported PNG filter {filter_type}")
        rows.append(row)
        previous = row
    return width, height, b"".join(rows)


def run(binary: Path, source: Path, output: Path) -> tuple[int, bytes]:
    result = subprocess.run(
        [str(binary), str(source), str(output)],
        capture_output=True,
        check=False,
        text=True,
    )
    return result.returncode, result.stderr.encode()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, default=DEFAULT_RELEASE)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--output", type=Path, default=ROOT / "oracle-cache/reports/oracle-equivalence.json")
    parser.add_argument("fixtures", nargs="*", help="name=path or a pinned fixture name")
    args = parser.parse_args()
    if not args.release.is_file() or not args.source.is_file():
        parser.error("fetch/build both D2 v0.9.0 oracle binaries before running this check")

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

    rows = []
    with tempfile.TemporaryDirectory(prefix="d2-rs-oracle-") as temporary:
        temporary_root = Path(temporary)
        for name, source_path in fixtures.items():
            release_svg = temporary_root / f"{name}.release.svg"
            source_svg = temporary_root / f"{name}.source.svg"
            release_png = temporary_root / f"{name}.release.png"
            source_png = temporary_root / f"{name}.source.png"
            release_exit, release_error = run(args.release, source_path, release_svg)
            source_exit, source_error = run(args.source, source_path, source_svg)
            release_png_exit, release_png_error = run(args.release, source_path, release_png)
            source_png_exit, source_png_error = run(args.source, source_path, source_png)
            row = {
                "name": name,
                "source": str(source_path),
                "source_sha256": sha256(source_path),
                "svg_release_exit": release_exit,
                "svg_source_exit": source_exit,
                "png_release_exit": release_png_exit,
                "png_source_exit": source_png_exit,
            }
            if any(
                exit_code != 0
                for exit_code in (release_exit, source_exit, release_png_exit, source_png_exit)
            ):
                row.update(
                    {
                        "matched": False,
                        "release_stderr": (release_error + release_png_error).decode(errors="replace"),
                        "source_stderr": (source_error + source_png_error).decode(errors="replace"),
                    }
                )
                rows.append(row)
                continue
            svg_release_bytes = release_svg.read_bytes()
            svg_source_bytes = source_svg.read_bytes()
            png_release_bytes = release_png.read_bytes()
            png_source_bytes = source_png.read_bytes()
            release_width, release_height, release_pixels = decode_png(release_png)
            source_width, source_height, source_pixels = decode_png(source_png)
            row.update(
                {
                    "svg_exact": svg_release_bytes == svg_source_bytes,
                    "png_bytes_exact": png_release_bytes == png_source_bytes,
                    "png_pixels_exact": (
                        (release_width, release_height, release_pixels)
                        == (source_width, source_height, source_pixels)
                    ),
                    "png_size": [release_width, release_height],
                    "svg_release_sha256": hashlib.sha256(svg_release_bytes).hexdigest(),
                    "svg_source_sha256": hashlib.sha256(svg_source_bytes).hexdigest(),
                    "png_release_sha256": hashlib.sha256(png_release_bytes).hexdigest(),
                    "png_source_sha256": hashlib.sha256(png_source_bytes).hexdigest(),
                }
            )
            row["matched"] = all(
                row[key] for key in ("svg_exact", "png_bytes_exact", "png_pixels_exact")
            )
            rows.append(row)
            print(f"{name}: {'MATCH' if row['matched'] else 'DIFF'}")

    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-release-source-equivalence",
        "authority": "D2 v0.9.0 release asset and source at d5a51743 with Go 1.27.0",
        "d2_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "go_toolchain": "go1.27.0",
        "release_binary": str(args.release),
        "release_binary_sha256": sha256(args.release),
        "source_binary": str(args.source),
        "source_binary_sha256": sha256(args.source),
        "matched": sum(row.get("matched", False) for row in rows),
        "total": len(rows),
        "metric": "exact SVG and PNG bytes plus decoded 8-bit RGB pixels",
        "fixtures": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"MATCH {report['matched']}/{report['total']}; wrote {args.output}")
    return 0 if report["matched"] == report["total"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
