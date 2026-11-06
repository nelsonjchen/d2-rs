"""Shared, byte-exact SVG outcome capture for fuzzing and reduction."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import xml.etree.ElementTree as ET

EMPTY_INVENTORY = hashlib.sha256(b"{}").hexdigest()


def first_error(stderr, directory):
    for line in stderr.splitlines():
        if "err:" in line or "error" in line.lower():
            return re.sub(r"\s+", " ", line.replace(str(directory), "<dir>")).strip()
    return ""


def run(binary, directory, layout, name):
    # Both implementations use the same output stem in separate directories.
    # Board manifests contain relative paths, so their bytes compare directly.
    directory = Path(directory)
    output = directory / name
    output.mkdir()
    shutil.copyfile(directory / "input.d2", output / "input.d2")
    try:
        result = subprocess.run(
            [str(Path(binary).resolve()), f"--layout={layout}", "input.d2", "output.svg"],
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
            if not data:
                raise ValueError(f"empty output: {path.relative_to(output)}")
            if path.suffix == ".svg":
                if ET.fromstring(data).tag.rsplit("}", 1)[-1] != "svg":
                    raise ValueError("not an SVG")
            elif path.suffix == ".json":
                json.loads(data)
            inventory[path.relative_to(output).as_posix()] = hashlib.sha256(data).hexdigest()
        # d2cli/main.go renderWithPNGEncoderToPaths deliberately writes no
        # file for a folder-only diagram, including an empty source. Compare
        # that complete empty inventory too; the fuzzer reports it separately
        # from rendered SVG matches. A missing board on just one side differs.
        if inventory and not any(path.endswith(".svg") for path in inventory):
            raise ValueError("missing SVG outputs")
    except (ValueError, ET.ParseError) as error:
        return "invalid-output", f"err: invalid output: {error}", None
    digest = hashlib.sha256(json.dumps(inventory, sort_keys=True).encode()).hexdigest()
    return 0, stderr, digest
