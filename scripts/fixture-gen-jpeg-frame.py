"""Input generator for recipe mathtext-img-jpeg-frame (tools/fixtures/mathtext-img.toml).

Run as `uv run --no-project --with pillow==13.0.0 python scripts/fixture-gen-jpeg-frame.py`
(the recipe pins the Pillow version: its libjpeg-turbo output is part of the
input bytes). Writes $D2RS_RECIPE_OUTPUT: a JSON list of {"Name", "Data"} JPEG
files for Go's image/jpeg.Decode (tools/fixtures/programs/mathtext-img-jpeg-frame):

* Pillow encodings of a fixed gradient at 1x1, 8x8, 13x17 and 32x24 in L, RGB
  (4:4:4, 4:2:2, 4:2:0) and CMYK, baseline and progressive, quality 30/85/100;
* ten hand-built 33x25 baseline files with 4:1:1, 4:2:1, 2:1/1:1/2:1 and other
  sampling layouts, each without and with a one-MCU restart interval;
* RGB files whose components are tagged R, G, B without a JFIF segment, and a
  CMYK file whose Adobe segment declares YCCK (transform 2);
* prefixes of eight of those files (0..19, 32, 64, 128, 256, len-2, len-1;
  a prefix past the end is the whole file);
* a lone segment of each of twelve markers with declared lengths 0..64.
"""
from __future__ import annotations

import io
import json
import sys
from pathlib import Path

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402


def gradient(width: int, height: int, mode: str) -> Image.Image:
    image = Image.new(mode, (width, height))
    for y in range(height):
        for x in range(width):
            channels = [(31 * x) % 256, (43 * y) % 256, (19 * (x + y)) % 256, (13 * x + 29 * y) % 256]
            image.putpixel((x, y), (31 * x + 7 * y) % 256 if mode == "L" else tuple(channels[:len(mode)]))
    return image


def encode(image: Image.Image, **options) -> bytes:
    out = io.BytesIO()
    image.save(out, "JPEG", **options)
    return out.getvalue()


# Hand-built 33x25 baseline files (identity quantization, minimal Huffman
# tables); the name suffix is the case number of the original capture.
SPECIALS = [
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103014100021100031100ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffda000c03010002000300003f003bf1a81a81a81ec11e039f1a81a81a81702e074e35035035034034039f1a81a8"
        "1a81702e074e35035035034034039f1a81a81a81702e074e35035035034034039f1a81a81a81702e1fffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103014100021100031100ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffdd00040001ffda000c03010002000300003f003bf1a81a81a81ec11e0f29ff00ffd032e3503503503f02610f29"
        "ff00ffd1238d40d40d411204ca1f29ff00ffd22c06a06a06a08a82680f29ff00ffd33603503503504641360729ff00ff"
        "d43c81a81a81a823809bc329ff00ffd53e81a81a81a826004e6129ff00ffd64400d40d40d413182760ffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103014200021100031100ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffda000c03010002000300003f003bf1a81a81a81a81a81a81a81ec11e04bfc6a06a06a06a06a06a06a05c0b8201"
        "c6a06a06a06a06a06a06a068068097f8d40d40d40d40d40d40d40b8170ffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103014200021100031100ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffdd00040001ffda000c03010002000300003f003bf1a81a81a81a81a81a81a81ec11e0f29ff00ffd032e3503503"
        "503503503503503f02610f29ff00ffd1238d40d40d40d40d40d40d411204ca1f29ff00ffd22c06a06a06a06a06a06a06"
        "a08a82680fffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103012100021100032100ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffda000c03010002000300003f003bf1a81ec11e03502bc6a05c0af1a815e3502e0578d40cf8d409033e3502bc6a"
        "05c0af1a815e3502e0578d40cf8d409033e3502bc6a05c0af1a815e3502e0578d40cf8d409033e3502bc6a05c0af1a81"
        "5e3502e0578d43ffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103012100021100032100ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffdd00040001ffda000c03010002000300003f003bf1a81ec11e035029ff00ffd032e3503f0261035029ff00ffd1"
        "2bc6a0888264035029ff00ffd2238d411204ca06a129ff00ffd32c06a08a8268035029ff00ffd435035046013581a87f"
        "29ff00ffd536035046413601a87f29ff00ffd63c81a823809bc0d43f29ff00ffd73e01a823e09c80d43f29ff00ffd03e"
        "81a826004e606a1f29ff00ffd14400d413182760350f29ff00ffd244c0d413302790350fffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103011100022100031200ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffda000c03010002000300003f003bf1ec0d411e03502e0578d40af1a81702bc6a0578d409033e35033e3502e057"
        "8d40af1a81702bc6a0578d43ffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103011100022100031200ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffdd00040001ffda000c03010002000300003f003bf1ec0d411e035029ff00ffd032e3f01a8261035029ff00ffd1"
        "2bc8881a8264035029ff00ffd22391203504ca06a129ff00ffd32c08a81a8268035029ff00ffd43504600d413581a87f"
        "ffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103011200021100031100ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffda000c03010002000300003f003bf1a81ec11e02bc6a05c0b815e3502e05c0af1a81702e0578d40b81703af1a8"
        "19719715e3502e05c0af1a81702e0578d40b81702bc6a05c0b87ffd9"
    ),
    (
        "ffd8ffdb0043000101010101010101010101010101010101010101010101010101010101010101010101010101010101"
        "0101010101010101010101010101010101010101010101ffc00011080019002103011200021100031100ffc400360000"
        "000000110000000000000000000000000102030405060708090a0b0c0d0e0f1010010000000000000000000000000000"
        "0000ffdd00040001ffda000c03010002000300003f003bf1a81ec11e0f29ff00ffd032e3503f02610f29ff00ffd12bc6"
        "a08882640f29ff00ffd2240d411404ce1f29ff00ffd334035045c1350729ff00ffd4238d411204ca1f29ff00ffd52c06"
        "a08a82680f29ff00ffd63503504601358729ff00ffd73c01a823609b8329ff00ffd03d81a823c09c43ffd9"
    ),
]


def retag_rgb(data: bytes) -> bytes:
    """Drop the JFIF APP0 segment and tag components 1, 2, 3 as R, G, B."""
    assert data[2:4] == b"\xff\xe0"
    data = data[:2] + data[4 + int.from_bytes(data[4:6], "big"):]
    out = bytearray(data)
    for marker, first, stride in ((b"\xff\xc0", 10, 3), (b"\xff\xda", 5, 2)):
        at = out.index(marker)
        for k, tag in enumerate(b"RGB"):
            assert out[at + first + stride * k] == k + 1
            out[at + first + stride * k] = tag
    return bytes(out)


def main() -> None:
    files: list[tuple[str, bytes]] = []
    for width, height in ((1, 1), (8, 8), (13, 17), (32, 24)):
        for mode in ("L", "RGB", "CMYK"):
            for subsampling in ((0, 1, 2) if mode == "RGB" else (0,)):
                for progressive in (False, True):
                    for quality in (30, 85, 100):
                        options = dict(quality=quality, progressive=progressive)
                        if mode == "RGB":
                            options["subsampling"] = subsampling
                        files.append((f"{width}x{height}-{mode}-{subsampling}-{progressive}-{quality}",
                                      encode(gradient(width, height, mode), **options)))
    files += [(f"special-sampling-restart-{120 + k}", bytes.fromhex(h)) for k, h in enumerate(SPECIALS)]
    for subsampling, tag in ((0, 66), (1, 72), (2, 78)):
        files.append((f"RGB-tagged-{tag}", retag_rgb(encode(gradient(13, 17, "RGB"), quality=30, subsampling=subsampling))))
    cmyk = bytearray(encode(gradient(32, 24, "CMYK"), quality=85))
    assert cmyk[2:4] == b"\xff\xee" and cmyk[17] == 0
    cmyk[17] = 2  # Adobe transform: YCCK
    files.append(("YCbCrK-adobe2", bytes(cmyk)))
    for index in (61, 67, 82, 85, 120, 121, 124, 133):
        data = files[index][1]
        for length in sorted({*range(20), 32, 64, 128, 256, len(data) - 2, len(data) - 1}):
            files.append((f"truncated-{index}-{length}", data[:length]))
    for marker in (0x00, 0x01, 0xC3, 0xCC, 0xE0, 0xEE, 0xFE, 0xDB, 0xC4, 0xDA, 0xC0, 0xDD):
        for length in (0, 1, 2, 3, 5, 8, 17, 64):
            segment = bytes([0xFF, marker]) + length.to_bytes(2, "big") + b"\x00" * max(0, length - 2)
            files.append((f"marker-{marker}-{length}", b"\xff\xd8" + segment + b"\xff\xd9"))
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(
        json.dumps([{"Name": name, "Data": list(data)} for name, data in files], separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
