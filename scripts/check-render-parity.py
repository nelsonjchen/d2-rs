#!/usr/bin/env python3
"""Compare native PNG output with the pinned Go 1.27 D2 renderer.

This is a semantic raster check, not a byte-for-byte PNG check.  PNG filter
bytes, compression, and ancillary chunks are ignored; decoded RGB pixels and
the canvas dimensions are compared.  The default fixture set combines pinned
upstream raster cases with local regression witnesses. The default Go
executable and source revision come from reference-manifest.json.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import struct
import subprocess
import tempfile
import zlib
from pathlib import Path

import references
from upstream_inputs import pristine_e2e

try:
    from PIL import Image
except ImportError:
    Image = None


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURES = {
    "unavailable-image": ROOT / "tests/d2/unavailable-image.d2",
    "geometry": ROOT.parent / "d2/e2etests-cli/testdata/raster/geometry.d2",
    "raster-diamond": ROOT / "tests/d2/raster-diamond.d2",
    "crowfoot-marker-overflow": ROOT / "tests/d2/crowfoot-marker-overflow.d2",
    "svg-arc-icon": ROOT / "tests/d2/svg-arc-icon.d2",
    "svg-embedded-raster-use": ROOT / "tests/d2/svg-embedded-raster-use.d2",
    "svg-embedded-raster-clip": ROOT / "tests/d2/svg-embedded-raster-clip.d2",
    "svg-embedded-raster-mixed-stroke": ROOT / "tests/d2/svg-embedded-raster-mixed-stroke.d2",
    "svg-embedded-raster-gradient": ROOT / "tests/d2/svg-embedded-raster-gradient.d2",
    "svg-embedded-gif-offset-clip": ROOT / "tests/d2/svg-embedded-gif-offset-clip.d2",
    "svg-embedded-gif-local-use": ROOT / "tests/d2/svg-embedded-gif-local-use.d2",
    "svg-embedded-webp-lossy-use": ROOT / "tests/d2/svg-embedded-webp-lossy-use.d2",
    "svg-embedded-webp-lossless-clip": ROOT / "tests/d2/svg-embedded-webp-lossless-clip.d2",
    "svg-embedded-jpeg-progressive-clip": ROOT / "tests/d2/svg-embedded-jpeg-progressive-clip.d2",
    "svg-embedded-jpeg-ycbcrk-clip": ROOT / "tests/d2/svg-embedded-jpeg-ycbcrk-clip.d2",
    "jpeg-exif-direct": ROOT / "tests/d2/jpeg-exif-direct.d2",
    "animation": ROOT.parent / "d2/e2etests-cli/testdata/raster/animation.d2",
    "local-icon": ROOT.parent / "d2/e2etests-cli/testdata/raster/local-icon.d2",
    "markdown": ROOT.parent / "d2/e2etests-cli/testdata/raster/markdown.d2",
    "unlabelled-connection-link": ROOT.parent / "d2/e2etests-cli/testdata/raster/unlabelled-connection-link.d2",
    "markdown-strikethrough": ROOT / "tests/d2/markdown-strikethrough.d2",
    "markdown-autolink": ROOT / "tests/d2/markdown-autolink.d2",
    "markdown-email": ROOT / "tests/d2/markdown-email.d2",
    "markdown-nested-inline": ROOT / "tests/d2/markdown-nested-inline.d2",
    "markdown-code-link": ROOT / "tests/d2/markdown-code-link.d2",
    "markdown-short": ROOT / "tests/d2/markdown-short.d2",
    "class-shapes": ROOT / "tests/d2/class-shapes.d2",
    "paper-pattern": ROOT / "tests/d2/pattern-paper.d2",
    "shaped-grid-positioning": ROOT / "tests/d2/shaped-grid-positioning.d2",
    "grid-direction-left-cell": ROOT / "tests/d2/grid-direction-left-cell.d2",
    "dagre-compound-label-midpoint": ROOT / "tests/d2/dagre-compound-label-midpoint.d2",
    "dagre-compound-wide-label": ROOT / "tests/d2/dagre-compound-wide-label.d2",
    "sql-table": pristine_e2e("ent2d2_basic.d2"),
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def summarize_pixel_differences(
    width: int, height: int, expected: bytes, actual: bytes
) -> dict[str, object] | None:
    """Retain exact changed-pixel locations and signed RGB deltas."""
    rows: dict[int, dict[str, object]] = {}
    delta_histogram: dict[str, int] = {}
    min_x, min_y = width, height
    max_x = max_y = -1
    for pixel_index in range(width * height):
        offset = pixel_index * 3
        delta = tuple(actual[offset + channel] - expected[offset + channel] for channel in range(3))
        if delta == (0, 0, 0):
            continue
        x = pixel_index % width
        y = pixel_index // width
        min_x, min_y = min(min_x, x), min(min_y, y)
        max_x, max_y = max(max_x, x), max(max_y, y)
        delta_key = ",".join(str(value) for value in delta)
        delta_histogram[delta_key] = delta_histogram.get(delta_key, 0) + 1
        row = rows.setdefault(
            y,
            {"changed_pixels": 0, "x_min": x, "x_max": x, "delta_histogram": {}},
        )
        row["changed_pixels"] = int(row["changed_pixels"]) + 1
        row["x_min"] = min(int(row["x_min"]), x)
        row["x_max"] = max(int(row["x_max"]), x)
        row_histogram = row["delta_histogram"]
        assert isinstance(row_histogram, dict)
        row_histogram[delta_key] = row_histogram.get(delta_key, 0) + 1
    if max_x < 0:
        return None
    return {
        "bounds": [min_x, min_y, max_x + 1, max_y + 1],
        "pixel_delta_histogram": delta_histogram,
        "rows": [{"y": y, **rows[y]} for y in sorted(rows)],
    }


def decode_png(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    offset = 8
    compressed = bytearray()
    width = height = depth = color = None
    saw_idat = False
    closed_idat = False
    saw_iend = False
    saw_plte = False
    while offset < len(data):
        if len(data) - offset < 12:
            raise ValueError(f"{path} has a truncated PNG chunk")
        length = struct.unpack_from(">I", data, offset)[0]
        if length > len(data) - offset - 12:
            raise ValueError(f"{path} has a truncated PNG chunk payload")
        kind = data[offset + 4 : offset + 8]
        if any(not (65 <= byte <= 90 or 97 <= byte <= 122) for byte in kind) or kind[2] & 0x20:
            raise ValueError(f"{path} has an invalid PNG chunk type")
        payload = data[offset + 8 : offset + 8 + length]
        crc = struct.unpack_from(">I", data, offset + 8 + length)[0]
        if zlib.crc32(kind + payload) & 0xFFFFFFFF != crc:
            raise ValueError(f"{path} has an invalid {kind!r} CRC")
        offset += length + 12
        if width is None and kind != b"IHDR":
            raise ValueError(f"{path} does not begin with IHDR")
        if kind == b"IHDR":
            if width is not None or length != 13:
                raise ValueError(f"{path} has an invalid or duplicate IHDR")
            width, height, depth, color, compression, filtering, interlace = struct.unpack(
                ">IIBBBBB", payload
            )
            if width == 0 or height == 0:
                raise ValueError(f"{path} has an empty PNG canvas")
            if (depth, color, compression, filtering, interlace) != (8, 2, 0, 0, 0):
                raise ValueError("only non-interlaced 8-bit RGB PNGs are supported")
        elif kind == b"IDAT":
            if closed_idat:
                raise ValueError(f"{path} has nonconsecutive IDAT chunks")
            saw_idat = True
            compressed.extend(payload)
        elif kind == b"IEND":
            if length != 0 or not saw_idat or offset != len(data):
                raise ValueError(f"{path} has an invalid IEND or trailing data")
            saw_iend = True
            break
        else:
            if saw_idat:
                closed_idat = True
            if kind == b"PLTE":
                if saw_plte or saw_idat or length == 0 or length % 3 or length > 768:
                    raise ValueError(f"{path} has an invalid PLTE")
                saw_plte = True
            elif kind == b"tRNS":
                raise ValueError("RGB transparency is unsupported by this opaque RGB comparator")
            elif not kind[0] & 0x20:
                raise ValueError(f"{path} has unsupported critical chunk {kind!r}")
    if width is None or height is None:
        raise ValueError(f"{path} has no IHDR")
    if not saw_iend:
        raise ValueError(f"{path} has no complete IEND")
    stride = width * 3
    decompressor = zlib.decompressobj()
    raw = decompressor.decompress(compressed, height * (stride + 1) + 1)
    if not decompressor.eof or decompressor.unused_data or decompressor.unconsumed_tail:
        raise ValueError(f"{path} has an incomplete or trailing zlib stream")
    if len(raw) != height * (stride + 1):
        raise ValueError(f"{path} has an incorrect decoded scanline length")
    if Image is not None:
        # Keep the same chunk/CRC/zlib/scanline contract above. Pillow only
        # accelerates unfiltering of the already validated opaque RGB stream.
        if any(raw[y * (stride + 1)] > 4 for y in range(height)):
            raise ValueError("unsupported PNG filter")
        del raw
        # Pinned full-D2 diagrams legitimately exceed Pillow's default pixel
        # threshold; dimensions and decoded byte count were validated above.
        Image.MAX_IMAGE_PIXELS = None
        with Image.open(io.BytesIO(data)) as image:
            if image.mode != "RGB" or image.size != (width, height):
                raise ValueError("decoded PNG mode or dimensions differ")
            return width, height, image.tobytes()
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


def collect_png_outputs(output: Path) -> dict[str, tuple[int, int, bytes]]:
    """Require real, complete single-image or multiboard output before comparing."""
    directory = output.with_suffix("")
    if output.is_file() and directory.exists():
        raise ValueError(f"ambiguous single-image and board outputs: {output}")
    if output.is_file():
        files = {"": output}
    elif directory.is_dir():
        paths = sorted(path for path in directory.rglob("*") if path.is_file())
        manifest = directory / BOARD_OUTPUT_MANIFEST
        paths = [path for path in paths if path != manifest]
        if not paths:
            raise ValueError(f"empty board output directory: {directory}")
        if any(path.suffix.lower() != ".png" for path in paths):
            raise ValueError(f"unexpected non-PNG board artifact: {directory}")
        files = {path.relative_to(directory).as_posix(): path for path in paths}
        frames = {name: decode_png(path) for name, path in files.items()}
        if manifest.is_file():
            frames[BOARD_OUTPUT_MANIFEST] = (0, 0, board_output_manifest_contract(directory, manifest, files))
        return frames
    else:
        raise ValueError(f"missing PNG output or board directory: {output}")
    return {name: decode_png(path) for name, path in files.items()}


# d2cli/board_output.go publishes multiboard trees with this manifest of the
# generated files' SHA-256 hashes. PNG encoder bytes are not a parity contract
# (pixels are), so each side's manifest is verified against its own files and
# only the inventory it declares is compared across engines.
BOARD_OUTPUT_MANIFEST = ".d2-board-output-manifest-v1.json"


def board_output_manifest_contract(directory: Path, manifest: Path, files: dict[str, Path]) -> bytes:
    record = json.loads(manifest.read_text())
    if record.get("format") != "d2-board-output" or record.get("version") != 1:
        raise ValueError(f"unknown board output manifest format: {manifest}")
    listed = {entry["path"]: entry["sha256"] for entry in record.get("files", [])}
    if listed.keys() != files.keys():
        raise ValueError(f"board output manifest inventory differs from files: {manifest}")
    for name, path in files.items():
        if hashlib.sha256(path.read_bytes()).hexdigest() != listed[name]:
            raise ValueError(f"board output manifest hash mismatch for {name}: {manifest}")
    directories = record.get("directories", [])
    if any(not (directory / name).is_dir() for name in directories):
        raise ValueError(f"board output manifest lists a missing directory: {manifest}")
    return json.dumps({"files": sorted(listed), "directories": directories}).encode()


def compare_png_outputs(expected: Path, actual: Path) -> dict[str, object]:
    """Compare every artifact; absent or malformed evidence can never pass."""
    try:
        go_frames = collect_png_outputs(expected)
        rust_frames = collect_png_outputs(actual)
    except (OSError, ValueError, zlib.error) as error:
        return {"matched": False, "error": str(error)}
    if not go_frames or not rust_frames:
        return {"matched": False, "error": "no PNG artifacts to compare"}
    inventories = {"go_artifacts": sorted(go_frames), "rust_artifacts": sorted(rust_frames)}
    if go_frames.keys() != rust_frames.keys():
        return {"matched": False, "error": "PNG artifact inventory differs", **inventories}
    artifacts = []
    for name in sorted(go_frames):
        go_width, go_height, go_pixels = go_frames[name]
        rust_width, rust_height, rust_pixels = rust_frames[name]
        row = {"name": name}
        if name == BOARD_OUTPUT_MANIFEST:
            row["matched"] = go_pixels == rust_pixels
            artifacts.append(row)
            continue
        if (go_width, go_height) != (rust_width, rust_height):
            row.update({"matched": False, "go_size": [go_width, go_height],
                        "rust_size": [rust_width, rust_height]})
        elif go_pixels == rust_pixels:
            row.update({"matched": True, "width": go_width, "height": go_height,
                        "different_pixels": 0, "mean_absolute_channel_error": 0.0})
        else:
            pixel_diff = summarize_pixel_differences(go_width, go_height, go_pixels, rust_pixels)
            deltas = [abs(left - right) for left, right in zip(go_pixels, rust_pixels)]
            row.update({"matched": pixel_diff is None, "width": go_width, "height": go_height,
                        "different_pixels": sum(any(deltas[i:i + 3]) for i in range(0, len(deltas), 3)),
                        "mean_absolute_channel_error": sum(deltas) / len(deltas)})
            if pixel_diff is not None:
                row["pixel_diff"] = pixel_diff
        artifacts.append(row)
    result = {"matched": all(row["matched"] for row in artifacts),
              "artifacts": artifacts, **inventories}
    if len(artifacts) == 1 and artifacts[0]["name"] == "":
        result.update({key: value for key, value in artifacts[0].items() if key != "name"})
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--go",
        type=Path,
        default=references.GO_ORACLE,
    )
    parser.add_argument("--rust", type=Path, default=ROOT / "target/release/d2-rs")
    parser.add_argument(
        "--layout",
        choices=("tala", "tala-rs", "dagre", "elk"),
        help="force the same built-in layout engine in both D2 binaries",
    )
    parser.add_argument(
        "--arg",
        dest="common_args",
        action="append",
        default=[],
        help="pass the same CLI option to both binaries; repeat as needed",
    )
    parser.add_argument("--output", type=Path, default=ROOT / "oracle-cache/reports/render-source-go1.27.json")
    parser.add_argument("--timeout", type=float, default=120, help="maximum seconds per renderer run")
    parser.add_argument("fixtures", nargs="*", help="name=path or a pinned fixture name")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if not args.go.is_file() or not args.rust.is_file():
        parser.error("build the Go and Rust binaries before running this check")

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
    if not fixtures:
        parser.error("the render parity suite must contain at least one fixture")

    rows = []
    with tempfile.TemporaryDirectory(prefix="d2-rs-render-") as temporary:
        temporary_root = Path(temporary)
        for name, source_path in fixtures.items():
            source = source_path.read_bytes()
            case_root = temporary_root / str(len(rows))
            (case_root / "go").mkdir(parents=True)
            (case_root / "rust").mkdir()
            go_png = case_root / "go/output.png"
            rust_png = case_root / "rust/output.png"
            go_command = [str(args.go)]
            rust_command = [str(args.rust)]
            if args.layout:
                # Keep the historical harness spelling as an alias; both
                # pinned CLIs expose the built-in engine as tala.
                layout = "tala" if args.layout == "tala-rs" else args.layout
                go_command.append(f"--layout={layout}")
                rust_command.append(f"--layout={layout}")
            go_command.extend(args.common_args)
            rust_command.extend(args.common_args)
            row = {"name": name, "source": str(source_path), "source_sha256": hashlib.sha256(source).hexdigest()}
            try:
                go = subprocess.run(
                    [*go_command, str(source_path), str(go_png)],
                    capture_output=True, check=False, timeout=args.timeout,
                )
                rust = subprocess.run(
                    [*rust_command, str(source_path), str(rust_png)],
                    capture_output=True, check=False, timeout=args.timeout,
                )
            except (OSError, subprocess.TimeoutExpired) as error:
                row.update({"matched": False, "error": str(error)})
                rows.append(row)
                continue
            if go.returncode or rust.returncode:
                row.update({"matched": False, "go_exit": go.returncode, "rust_exit": rust.returncode})
                rows.append(row)
                continue
            row.update(compare_png_outputs(go_png, rust_png))
            rows.append(row)

    report = {
        "schema_version": 1,
        "kind": "d2-render-pixel-parity",
        "reference": {
            "d2_commit": references.SOURCE_COMMIT,
            "go_binary": str(args.go),
            "go_binary_sha256": sha256(args.go),
            "rust_binary": str(args.rust),
            "rust_binary_sha256": sha256(args.rust),
            "layout": args.layout,
            "arguments": args.common_args,
            "metric": "complete PNG artifact inventory and decoded RGB channel deltas; PNG encoding and metadata are ignored",
            "timeout_seconds": args.timeout,
        },
        "fixtures": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"matched": sum(row.get("matched", False) for row in rows), "total": len(rows)}))
    return 0 if all(row.get("matched", False) for row in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
