"""Shared, byte-exact SVG outcome capture for fuzzing and reduction."""
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import xml.etree.ElementTree as ET
import zipfile
import zlib

EMPTY_INVENTORY = hashlib.sha256(b"{}").hexdigest()


def first_error(stderr, directory):
    for line in stderr.splitlines():
        if "err:" in line or "error" in line.lower():
            return re.sub(r"\s+", " ", line.replace(str(directory), "<dir>")).strip()
    return ""


def _validate_output(path, data):
    suffix = path.suffix.lower()
    if not data:
        raise ValueError(f"empty output: {path}")
    if suffix == ".svg":
        if ET.fromstring(data).tag.rsplit("}", 1)[-1] != "svg":
            raise ValueError("not an SVG")
    elif suffix == ".png":
        if not data.startswith(b"\x89PNG\r\n\x1a\n"):
            raise ValueError("not a PNG")
        offset, chunks = 8, []
        while offset + 12 <= len(data):
            size = struct.unpack_from(">I", data, offset)[0]
            end = offset + size + 12
            if end > len(data):
                raise ValueError("truncated PNG")
            kind = data[offset + 4:offset + 8]
            payload = data[offset + 8:offset + 8 + size]
            expected_crc = struct.unpack_from(">I", data, offset + 8 + size)[0]
            if zlib.crc32(kind + payload) != expected_crc:
                raise ValueError("PNG CRC")
            chunks.append(kind)
            offset = end
            if kind == b"IEND":
                break
        if offset != len(data) or not chunks or chunks[0] != b"IHDR" or b"IDAT" not in chunks or chunks[-1] != b"IEND":
            raise ValueError("incomplete PNG")
    elif suffix == ".pdf" and not (
        data.startswith(b"%PDF-") and data.rstrip().endswith(b"%%EOF")
    ):
        raise ValueError("incomplete PDF")
    elif suffix == ".pptx":
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            if archive.testzip() is not None or "[Content_Types].xml" not in archive.namelist():
                raise ValueError("invalid PPTX")
    elif suffix == ".gif" and not (
        data.startswith((b"GIF87a", b"GIF89a")) and data.endswith(b";")
    ):
        raise ValueError("incomplete GIF")
    elif suffix == ".json":
        json.loads(data)
    elif suffix == ".txt":
        data.decode("utf-8")


def run(binary, directory, layout, name, output_format="svg", render_flags=()):
    # Both implementations use the same output stem in separate directories.
    # Board manifests contain relative paths, so their bytes compare directly.
    directory = Path(directory)
    output = directory / name
    output.mkdir()
    shutil.copyfile(directory / "input.d2", output / "input.d2")
    try:
        result = subprocess.run(
            [str(Path(binary).resolve()), f"--layout={layout}", *render_flags,
             "input.d2", f"output.{output_format}"],
            cwd=output, capture_output=True, timeout=20,
            env={**os.environ, "SOURCE_DATE_EPOCH": "946782245"},
        )
    except subprocess.TimeoutExpired:
        return "timeout", "", None
    stderr = result.stderr.decode("utf-8", "replace").replace(str(output), str(directory))
    if result.returncode:
        return result.returncode, stderr, None
    inventory, total = {}, 0
    try:
        for path in sorted(output.rglob("*")):
            if not path.is_file() or path == output / "input.d2":
                continue
            size = path.stat().st_size
            total += size
            if size > 128 * 1024**2 or total > 1024**3:
                raise ValueError("output budget exceeded")
            data = path.read_bytes()
            _validate_output(path, data)
            inventory[path.relative_to(output).as_posix()] = hashlib.sha256(data).hexdigest()
        # d2cli/main.go renderWithPNGEncoderToPaths deliberately writes no
        # file for a folder-only diagram, including an empty source. Compare
        # that complete empty inventory too; the fuzzer reports it separately
        # from rendered SVG matches. A missing board on just one side differs.
        if inventory and not any(path.endswith(f".{output_format}") for path in inventory):
            raise ValueError(f"missing {output_format.upper()} outputs")
    except (ValueError, UnicodeDecodeError, ET.ParseError, zipfile.BadZipFile) as error:
        return "invalid-output", f"err: invalid output: {error}", None
    digest = hashlib.sha256(json.dumps(inventory, sort_keys=True).encode()).hexdigest()
    return 0, stderr, digest
