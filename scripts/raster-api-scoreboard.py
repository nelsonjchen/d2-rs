#!/usr/bin/env python3
"""Replay Render calls from pinned upstream text/color-font tests.

The original test assertions must pass; complete PNG inventories use a
strict RGBA extension of the raster gate and errors compare exact text. No golden or
production Go source is changed. Use the run45 streaming guard around captures.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import re
import struct
import subprocess
import zlib
from pathlib import Path

import references
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
TESTS = {
    "raster_test.go": [
        "TestTextAnchorsUnderlineAndErrors",
        "TestRenderRejectsFallbackReferenceCountBeforeSliceAllocation",
    ],
    "gradient_test.go": ["TestGradientPaintsTextFillAndStroke"],
    "noto_color_emoji_native_test.go": [
        "TestNotoColorEmojiCOLRv1RendersPaletteAndDeterministically",
        "TestNotoColorEmojiColorLayersChargeStructuralNodes",
        "TestNotoColorEmojiCOLRv1SoftLightGlyphRenders",
    ],
}
EXPECTED_COUNTS = {
    "TestGradientPaintsTextFillAndStroke": 1,
    "TestNotoColorEmojiCOLRv1RendersPaletteAndDeterministically": 1,
    "TestNotoColorEmojiCOLRv1SoftLightGlyphRenders": 1,
    "TestNotoColorEmojiColorLayersChargeStructuralNodes": 2,
    "TestRenderRejectsFallbackReferenceCountBeforeSliceAllocation": 1,
    "TestTextAnchorsUnderlineAndErrors": 2,
    "TestTextAnchorsUnderlineAndErrors/missing_complex_script": 1,
    "TestTextAnchorsUnderlineAndErrors/missing_glyph": 1,
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def decode_rgba(path):
    """Strict RGB/RGBA gate, adapted from check-render-parity.decode_png.

    That comparator intentionally accepts opaque RGB only. These upstream
    API tests render onto transparent backgrounds, so retain and compare all
    four channels, including RGB values under zero alpha.
    """
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    offset, compressed, dimensions = 8, bytearray(), None
    saw_idat = closed_idat = saw_iend = False
    while offset < len(data):
        if len(data) - offset < 12:
            raise ValueError("truncated PNG chunk")
        length = struct.unpack_from(">I", data, offset)[0]
        if length > len(data) - offset - 12:
            raise ValueError("truncated PNG payload")
        kind = data[offset + 4 : offset + 8]
        payload = data[offset + 8 : offset + 8 + length]
        if any(not (65 <= b <= 90 or 97 <= b <= 122) for b in kind) or kind[2] & 0x20:
            raise ValueError("invalid chunk type")
        if (
            zlib.crc32(kind + payload) & 0xFFFFFFFF
            != struct.unpack_from(">I", data, offset + 8 + length)[0]
        ):
            raise ValueError("invalid PNG CRC")
        offset += length + 12
        if dimensions is None and kind != b"IHDR":
            raise ValueError("missing initial IHDR")
        if kind == b"IHDR":
            if dimensions is not None or length != 13:
                raise ValueError("invalid IHDR")
            width, height, depth, color, compression, filtering, interlace = (
                struct.unpack(">IIBBBBB", payload)
            )
            if (
                not width
                or not height
                or depth != 8
                or color not in (2, 6)
                or (compression, filtering, interlace) != (0, 0, 0)
            ):
                raise ValueError("unsupported PNG format")
            dimensions = width, height
            channels = 3 if color == 2 else 4
        elif kind == b"IDAT":
            if closed_idat:
                raise ValueError("nonconsecutive IDAT")
            saw_idat = True
            compressed.extend(payload)
        elif kind == b"IEND":
            if length or not saw_idat or offset != len(data):
                raise ValueError("invalid IEND/trailing data")
            saw_iend = True
            break
        else:
            if saw_idat:
                closed_idat = True
            if not kind[0] & 0x20 or kind == b"tRNS":
                raise ValueError("unsupported critical/transparency chunk")
    if not saw_iend:
        raise ValueError("incomplete PNG")
    stride = width * channels + 1
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, stride * height + 1)
    if (
        not decoder.eof
        or decoder.unused_data
        or decoder.unconsumed_tail
        or len(raw) != stride * height
    ):
        raise ValueError("incomplete/trailing zlib or wrong scanline length")
    if any(raw[y * stride] > 4 for y in range(height)):
        raise ValueError("invalid PNG filter")
    with Image.open(path) as image:
        image.load()
        if image.size != dimensions or image.mode != (
            "RGB" if channels == 3 else "RGBA"
        ):
            raise ValueError("decoded PNG mode/dimensions differ")
        return dimensions, image.convert("RGBA").tobytes()


def compare_frames(expected, actual):
    try:
        left, right = decode_rgba(expected), decode_rgba(actual)
        return {"matched": left == right}
    except (OSError, ValueError, zlib.error) as error:
        return {"matched": False, "error": str(error)}


def capture_go(capture):
    source = references.verify_source()
    trace = references.TRACE_CHECKOUT
    assert (
        subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=trace, text=True
        ).strip()
        == references.SOURCE_COMMIT
    )
    package = trace / "d2renderers/d2raster"
    originals = {}
    helpers = [
        package / "d2rs_raster_api_hook_test.go",
        package / "d2rs_raster_api_scene_test.go",
    ]
    assert not any(p.exists() for p in helpers), "capture helper already installed"
    sources = {}
    try:
        for filename, names in TESTS.items():
            path = package / filename
            original = path.read_text()
            # Refuse a modified oracle test, including another capture session.
            assert original == (source / "d2renderers/d2raster" / filename).read_text()
            sources[filename] = digest(path)
            originals[path] = original
            text = original
            for name in names:
                match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
                assert match, name
                body, count = re.subn(r"\bRender\(", "d2rsRasterRender(t, ", match[0])
                assert count, name
                text = text[: match.start()] + body + text[match.end() :]
            path.write_text(text)
        helpers[0].write_bytes(
            (ROOT / "tools/raster-api-scoreboard/hook_test.go").read_bytes()
        )
        helpers[1].write_text(
            (ROOT / "tools/scene-probe/scenejson.go")
            .read_text()
            .replace("package main", "package d2raster", 1)
        )
        env = os.environ.copy()
        env.update(
            D2RS_RASTER_API_CAPTURE=str(capture),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        pattern = "^(" + "|".join(n for names in TESTS.values() for n in names) + ")$"
        with (capture / "go-events.jsonl").open("w") as log:
            result = subprocess.run(
                [
                    "go",
                    "test",
                    "-json",
                    "./d2renderers/d2raster",
                    "-run",
                    pattern,
                    "-count=1",
                ],
                cwd=trace,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                timeout=600,
                check=False,
            )
        events = []
        for line in (capture / "go-events.jsonl").read_text().splitlines():
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
        outcomes = {
            e["Test"]: e["Action"]
            for e in events
            if "Test" in e and e.get("Action") in ("pass", "fail", "skip")
        }
        inputs = sorted((capture / "go").rglob("case.json"))
        assert result.returncode == 0 and inputs, "Go tests failed or no captures"
        assert all(
            outcomes.get(n) == "pass" for names in TESTS.values() for n in names
        ), outcomes
        for path in inputs:
            assert outcomes.get(json.loads(path.read_text())["test"]) == "pass"
        assert (
            dict(collections.Counter(json.loads(p.read_text())["test"] for p in inputs))
            == EXPECTED_COUNTS
        )
        files = {
            str(p.relative_to(capture)): digest(p)
            for p in capture.rglob("*")
            if p.is_file()
        }
        manifest = {
            "source_commit": references.SOURCE_COMMIT,
            "tests": TESTS,
            "sources": sources,
            "outcomes": outcomes,
            "files": files,
            "hook_sha256": digest(ROOT / "tools/raster-api-scoreboard/hook_test.go"),
            "scene_encoder_sha256": digest(ROOT / "tools/scene-probe/scenejson.go"),
        }
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    finally:
        for path, original in originals.items():
            path.write_text(original)
        for path in helpers:
            path.unlink(missing_ok=True)


def capture_cases(capture, expected_counts=None):
    cases = sorted((capture / "go").rglob("case.json"))
    counts = EXPECTED_COUNTS if expected_counts is None else expected_counts
    assert (
        cases
        and dict(collections.Counter(json.loads(p.read_text())["test"] for p in cases))
        == counts
    )
    expected = {
        p.parent / name
        for p in cases
        for name in ("case.json", "board.scene.json", "board.options.json")
    }
    for path in cases:
        outcomes = [
            path.parent / name
            for name in ("board.png", "board.err")
            if (path.parent / name).is_file()
        ]
        assert len(outcomes) == 1, path
        expected.update(outcomes)
    assert {p for p in (capture / "go").rglob("*") if p.is_file()} == expected
    return cases


def score(capture, rust, output):
    manifest = json.loads((capture / "manifest.json").read_text())
    assert manifest["source_commit"] == references.SOURCE_COMMIT
    for relative, expected in manifest["files"].items():
        assert digest(capture / relative) == expected, relative
    native = output.with_suffix("")
    assert not native.exists(), "native output must be fresh"
    native.mkdir(parents=True)
    cases = capture_cases(capture)
    requests = [
        {"go": str(p.parent), "out": str(native / p.parent.relative_to(capture / "go"))}
        for p in cases
    ]
    result = subprocess.run(
        [str(rust)],
        input="\n".join(json.dumps(r) for r in requests) + "\n",
        text=True,
        capture_output=True,
        timeout=600,
        check=False,
    )
    (native / "rust.log").write_text(result.stdout + result.stderr)
    rows = []
    for path in cases:
        relative = path.parent.relative_to(capture / "go")
        go = path.parent
        out = native / relative
        png, err = go / "board.png", go / "board.err"
        row = {"board": str(relative), "outcome": "mismatch"}
        assert png.is_file() != err.is_file(), relative
        assert manifest["outcomes"].get(json.loads(path.read_text())["test"]) == "pass"
        if png.exists():
            comparison = compare_frames(png, out / "board.png")
            row["kind"] = "pixels"
            row["outcome"] = (
                "match"
                if comparison["matched"] and not (out / "board.err").exists()
                else "mismatch"
            )
            if row["outcome"] != "match":
                row["detail"] = comparison
                if (out / "board.err").exists():
                    row["error"] = (out / "board.err").read_text()
        else:
            row["kind"] = "error"
            row["outcome"] = (
                "match"
                if (out / "board.err").is_file()
                and err.read_bytes() == (out / "board.err").read_bytes()
                and not (out / "board.png").exists()
                else "mismatch"
            )
            row["expected"] = err.read_text()
            if (out / "board.err").exists():
                row["actual"] = (out / "board.err").read_text()
        rows.append(row)
    expected_paths = {
        str(Path(r["out"]).relative_to(native) / "board.png")
        for r in requests
        if (Path(r["go"]) / "board.png").exists()
    }
    actual_paths = {str(p.relative_to(native)) for p in native.rglob("*.png")}
    extras = sorted(actual_paths - expected_paths)
    for path in extras:
        rows.append({"board": path, "outcome": "rust-extra"})
    allowed = {
        Path(r["out"]) / name for r in requests for name in ("board.png", "board.err")
    }
    allowed.add(native / "rust.log")
    for path in sorted(
        p
        for p in native.rglob("*")
        if p.is_file() and p not in allowed and p.suffix != ".png"
    ):
        rows.append({"board": str(path.relative_to(native)), "outcome": "rust-extra"})
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "capture": str(capture),
        "native": str(native),
        "rust_sha256": digest(rust),
        "rust_exit": result.returncode,
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    output.write_text(json.dumps(report, indent=1) + "\n")
    print(report["summary"])
    assert result.returncode == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--rust", type=Path, default=ROOT / "target/diagnostic/examples/raster-probe"
    )
    parser.add_argument("--capture-only", action="store_true")
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        assert not capture.exists(), "capture must be fresh"
        capture.mkdir(parents=True)
        capture_go(capture)
    if not args.capture_only:
        assert args.output
        score(capture, args.rust.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
