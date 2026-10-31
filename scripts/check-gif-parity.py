#!/usr/bin/env python3
"""Compare D2 v0.9.0 animated GIF output with its frozen raster fixtures."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
FRAME_RATE = 30
FIXTURE_INTERVALS_MS = (310, 1050)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(command: list[str], *, binary_output: bool = False) -> bytes | str:
    result = subprocess.run(command, capture_output=True)
    if result.returncode:
        stderr = result.stderr.decode("utf-8", errors="replace")
        raise RuntimeError(f"command failed ({result.returncode}): {command}\n{stderr}")
    return result.stdout if binary_output else result.stdout.decode("utf-8", errors="replace")


def subblocks(data: bytes, offset: int) -> tuple[list[bytes], int]:
    blocks = []
    while True:
        if offset >= len(data):
            raise ValueError("truncated GIF sub-block sequence")
        size = data[offset]
        offset += 1
        if size == 0:
            return blocks, offset
        end = offset + size
        if end > len(data):
            raise ValueError("truncated GIF sub-block")
        blocks.append(data[offset:end])
        offset = end


def inspect_gif(path: Path) -> dict[str, Any]:
    data = path.read_bytes()
    if data[:6] not in (b"GIF87a", b"GIF89a") or len(data) < 14:
        raise ValueError(f"{path} is not a complete GIF")

    width = int.from_bytes(data[6:8], "little")
    height = int.from_bytes(data[8:10], "little")
    packed = data[10]
    offset = 13
    if packed & 0x80:
        offset += 3 * (1 << ((packed & 0x07) + 1))

    loop_count = -1
    frame_delays: list[int] = []
    pending_delay = 0
    trailer_seen = False
    while offset < len(data):
        marker = data[offset]
        offset += 1
        if marker == 0x3B:
            trailer_seen = True
            break
        if marker == 0x21:
            if offset >= len(data):
                raise ValueError("truncated GIF extension")
            label = data[offset]
            offset += 1
            if label == 0xF9:
                if offset >= len(data):
                    raise ValueError("truncated GIF graphic control extension")
                size = data[offset]
                offset += 1
                end = offset + size
                if size != 4 or end >= len(data) or data[end] != 0:
                    raise ValueError("invalid GIF graphic control extension")
                pending_delay = int.from_bytes(data[offset + 1 : offset + 3], "little")
                offset = end + 1
            elif label == 0xFF:
                if offset >= len(data):
                    raise ValueError("truncated GIF application extension")
                size = data[offset]
                offset += 1
                end = offset + size
                if end > len(data):
                    raise ValueError("truncated GIF application identifier")
                application = data[offset:end]
                blocks, offset = subblocks(data, end)
                if application in (b"NETSCAPE2.0", b"ANIMEXTS1.0"):
                    for block in blocks:
                        if len(block) >= 3 and block[0] == 1:
                            loop_count = int.from_bytes(block[1:3], "little")
                            break
            else:
                _, offset = subblocks(data, offset)
        elif marker == 0x2C:
            end = offset + 9
            if end > len(data):
                raise ValueError("truncated GIF image descriptor")
            local_packed = data[offset + 8]
            offset = end
            if local_packed & 0x80:
                offset += 3 * (1 << ((local_packed & 0x07) + 1))
            if offset >= len(data):
                raise ValueError("missing GIF LZW minimum code size")
            offset += 1
            _, offset = subblocks(data, offset)
            frame_delays.append(pending_delay)
            pending_delay = 0
        else:
            raise ValueError(f"unexpected GIF block marker {marker:#04x}")

    if not trailer_seen or data[-1] != 0x3B:
        raise ValueError("GIF trailer is missing")
    if width <= 0 or height <= 0 or not frame_delays:
        raise ValueError("GIF has invalid dimensions or no image frames")

    decoded = run(
        [
            "ffmpeg",
            "-v",
            "error",
            "-i",
            str(path),
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ],
        binary_output=True,
    )
    assert isinstance(decoded, bytes)
    bytes_per_frame = width * height * 4
    if len(decoded) != bytes_per_frame * len(frame_delays):
        raise ValueError(
            f"decoded {len(decoded)} RGBA bytes for {len(frame_delays)} "
            f"frames at {width}x{height}"
        )
    frame_hashes = [
        sha256(decoded[offset : offset + bytes_per_frame])
        for offset in range(0, len(decoded), bytes_per_frame)
    ]
    return {
        "width": width,
        "height": height,
        "loop_count": loop_count,
        "delays_cs": frame_delays,
        "frame_hashes": frame_hashes,
        "encoded_sha256": sha256(data),
    }


def compare(expected: dict[str, Any], actual: dict[str, Any]) -> dict[str, Any]:
    same_canvas = (actual["width"], actual["height"]) == (
        expected["width"],
        expected["height"],
    )
    same_frames = actual["frame_hashes"] == expected["frame_hashes"]
    mismatch_frames = []
    for index, (want, got) in enumerate(
        zip(expected["frame_hashes"], actual["frame_hashes"])
    ):
        if want != got:
            mismatch_frames.append(index)
    return {
        "matches": (
            same_canvas
            and same_frames
            and actual["loop_count"] == expected["loop_count"]
            and actual["delays_cs"] == expected["delays_cs"]
        ),
        "matches_canvas": same_canvas,
        "matches_composited_pixels": same_frames,
        "matches_loop_count": actual["loop_count"] == expected["loop_count"],
        "matches_frame_delays": actual["delays_cs"] == expected["delays_cs"],
        "frame_count": len(actual["frame_hashes"]),
        "loop_count": actual["loop_count"],
        "total_duration_cs": sum(actual["delays_cs"]),
        "delays_cs": actual["delays_cs"],
        "mismatching_frames": mismatch_frames,
        "encoded_sha256": actual["encoded_sha256"],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--intervals",
        type=int,
        nargs="+",
        default=FIXTURE_INTERVALS_MS,
        help="D2 animation intervals in milliseconds (default: 310 1050)",
    )
    parser.add_argument(
        "--report",
        type=Path,
        help="write the JSON comparison report to this path",
    )
    parser.add_argument(
        "--pad",
        type=int,
        default=16,
        help="D2 canvas padding in pixels (default: 16)",
    )
    parser.add_argument(
        "--rust-binary",
        type=Path,
        default=ROOT / "target/release/d2-rs",
        help="Rust CLI binary to compare (default: target/release/d2-rs)",
    )
    args = parser.parse_args()

    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        raise RuntimeError("ffmpeg is required to compare decoded GIF frames")
    manifest = json.loads((ROOT / "reference-manifest.json").read_text())
    source_info = manifest["source_builds"]["d2-v0.9.0-go1.27.0"]
    source_binary = ROOT / source_info["binary"]
    release_info = manifest["releases"]["macos-arm64"]
    release_binary = ROOT / ".reference/bin/d2-v0.9.0-official"
    rust_binary = args.rust_binary
    if not rust_binary.is_absolute():
        rust_binary = ROOT / rust_binary
    binaries = {
        "go_source": source_binary,
        "official_release": release_binary,
        "rust": rust_binary,
    }
    for name, binary in binaries.items():
        if not binary.is_file():
            raise RuntimeError(f"missing {name} binary: {binary}")
    try:
        rust_binary_path = str(rust_binary.relative_to(ROOT))
    except ValueError:
        rust_binary_path = str(rust_binary)

    source_digest = sha256(source_binary.read_bytes())
    release_digest = sha256(release_binary.read_bytes())
    if source_digest != source_info["binary_sha256"]:
        raise RuntimeError("Go source binary checksum differs from reference-manifest.json")
    if release_digest != release_info["binary_sha256"]:
        raise RuntimeError("official release binary checksum differs from reference-manifest.json")
    go_build_info = str(run(["go", "version", "-m", str(source_binary)]))
    if source_info["go_toolchain"] not in go_build_info.splitlines()[0]:
        raise RuntimeError("Go source oracle was not built with the pinned Go toolchain")

    rustc = str(run(["rustc", "--version"])).strip()
    ffmpeg_version = str(run([ffmpeg, "-version"])).splitlines()[0]
    fixture_root = ROOT.parent / "d2/e2etests-cli/testdata/raster"
    results: dict[str, Any] = {
        "environment": {
            "d2_commit": manifest["d2"]["commit"],
            "go_toolchain": source_info["go_toolchain"],
            "go_source_binary_sha256": source_digest,
            "official_release_binary_sha256": release_digest,
            "rust_binary_sha256": sha256(rust_binary.read_bytes()),
            "rust_binary_path": rust_binary_path,
            "rustc": rustc,
            "ffmpeg": ffmpeg_version,
            "platform": platform.platform(),
        },
        "settings": {"pad": args.pad, "scale": 1},
        "intervals": {},
    }
    all_match = True
    with tempfile.TemporaryDirectory(prefix="d2-rs-gif-parity-") as temporary:
        temporary_root = Path(temporary)
        for interval_ms in args.intervals:
            fixture = fixture_root / "animation.d2"
            expected_path = fixture_root / f"animation-{interval_ms}.exp.gif"
            if not fixture.is_file() or not expected_path.is_file():
                raise RuntimeError(f"missing pinned GIF fixture at {fixture_root}")
            expected = inspect_gif(expected_path)
            expected["encoded_sha256"] = sha256(expected_path.read_bytes())
            interval_result: dict[str, Any] = {
                "input_sha256": sha256(fixture.read_bytes()),
                "expected_sha256": expected["encoded_sha256"],
                "expected": compare(expected, expected),
                "implementations": {},
            }
            contracts: dict[str, dict[str, Any]] = {}
            for name, binary in binaries.items():
                output = temporary_root / f"{name}-{interval_ms}.gif"
                run(
                    [
                        str(binary),
                        f"--pad={args.pad}",
                        f"--animate-interval={interval_ms}",
                        str(fixture),
                        str(output),
                    ]
                )
                contract = inspect_gif(output)
                contracts[name] = contract
                check = compare(expected, contract)
                interval_result["implementations"][name] = check
                if name != "rust" and not check["matches"]:
                    all_match = False
                if name == "rust" and not check["matches"]:
                    all_match = False
            interval_result["source_and_release_encoded_bytes_match"] = (
                contracts["go_source"]["encoded_sha256"]
                == contracts["official_release"]["encoded_sha256"]
            )
            results["intervals"][str(interval_ms)] = interval_result

    rendered = json.dumps(results, indent=2, sort_keys=True) + "\n"
    if args.report:
        report_path = args.report if args.report.is_absolute() else ROOT / args.report
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(rendered)
    print(rendered, end="")
    return 0 if all_match else 1


if __name__ == "__main__":
    raise SystemExit(main())
