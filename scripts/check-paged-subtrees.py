#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Check selected PDF/PPTX documents against original Go v0.9.0 fixtures.

Run: UV_CACHE_DIR=$PWD/.reference/uv-cache uv run --no-project --with pypdf \
    python scripts/check-paged-subtrees.py --rust target/diagnostic/d2-rs

PDF checks every page's dimensions, drawing stream, used RGB/alpha image
(compressed and decoded) and annotation; dates are outside this contract.
PPTX checks raw archive bytes and every ZIP member, including metadata, media
and navigation.
This supplementary gate does not replace the strict upstream scoreboards.
Fixture generator: .reference/codex-runs/witness-inputs/run39-paged-subtree.py.
"""

import argparse
import hashlib
import json
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

from pypdf import PdfReader

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from fixture_recipe import cached_fixture  # noqa: E402


def sha(data):
    return hashlib.sha256(data).hexdigest()


def inspect(path, fmt):
    if fmt == "pptx":
        with zipfile.ZipFile(path) as z:
            names = z.namelist()
            assert names and len(names) == len(set(names)) and z.testzip() is None
            return {n: sha(z.read(n)) for n in names}
    reader = PdfReader(path, strict=True)
    assert reader.pages

    def value(v):
        from pypdf.generic import IndirectObject

        if isinstance(v, IndirectObject):
            pages = {p.indirect_reference.idnum: i for i, p in enumerate(reader.pages)}
            if v.idnum in pages:
                return {"page": pages[v.idnum]}
            return value(v.get_object())
        if isinstance(v, dict):
            return {str(k): value(v.raw_get(k)) for k in v}
        if isinstance(v, list):
            return [value(x) for x in v]
        if isinstance(v, (str, int, float, bool)) or v is None:
            return v
        return str(v)

    pages = []
    for p in reader.pages:
        ims = []
        for args, op in p.get_contents().operations:
            if op != b"Do":
                continue
            im = p["/Resources"]["/XObject"][args[0]].get_object()
            mask = im.get("/SMask")
            mask = mask.get_object() if mask else None
            ims.append(
                {
                    "name": str(args[0]),
                    "width": im["/Width"],
                    "height": im["/Height"],
                    "rgb": sha(im.get_data()),
                    "compressed": sha(im._data),
                    "alpha": sha(mask.get_data()) if mask else None,
                    "compressed_alpha": sha(mask._data) if mask else None,
                }
            )
        assert len(ims) == 1
        pages.append(
            {
                "box": list(p.mediabox),
                "content": sha(p.get_contents().get_data()),
                "images": ims,
                "annotations": value(p.get("/Annots", [])),
            }
        )
    return pages


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    args = parser.parse_args()
    binary = args.rust.resolve()
    cases = json.loads(
        (
            cached_fixture("crates/d2-rs-cli/tests/fixtures/paged-subtree-source-v090.json")
        ).read_text()
    )
    assert len(cases) == 26
    with tempfile.TemporaryDirectory(prefix="d2-rs-paged-subtree-") as scratch:
        for case in cases:
            name = case["name"] + "-" + case["format"]
            directory = Path(scratch) / name
            directory.mkdir()
            source = directory / "input.d2"
            source.write_text(case["source"])
            output = directory / ("out." + case["format"])
            command = [
                str(binary),
                "--pad=0",
                "--bundle=false",
                "--stdout-format=" + case["format"],
                "--target=" + case["target"],
                "-" if case["stdin"] else str(source),
                "-" if case["stdout"] else str(output),
            ]
            proc = subprocess.run(
                command,
                check=False,
                cwd=directory,
                capture_output=True,
                input=case["source"].encode() if case["stdin"] else None,
                timeout=90,
            )
            if "expected_error" in case:
                assert proc.returncode != 0, name
                expected_error = (
                    f"err: failed to compile input.d2: {case['expected_error']}\n"
                )
                assert proc.stderr.decode() == expected_error, (name, proc.stderr)
                assert not proc.stdout and {p.name for p in directory.iterdir()} == {
                    "input.d2"
                }, name
                print(name, "passed", flush=True)
                continue
            assert proc.returncode == 0, (name, proc.stderr)
            assert {p.name for p in directory.iterdir()} == (
                {"input.d2"} if case["stdout"] else {"input.d2", output.name}
            ), (name, "unexpected output inventory")
            if case["stdout"]:
                assert proc.stdout, (name, "missing stdout")
                output.write_bytes(proc.stdout)
            else:
                assert not proc.stdout, (name, "unexpected stdout")
            assert output.is_file() and 0 < output.stat().st_size < 128 << 20
            assert inspect(output, case["format"]) == case["expected"], name
            if "raw_sha256" in case:
                assert sha(output.read_bytes()) == case["raw_sha256"], (
                    name,
                    "raw archive bytes",
                )
            assert (
                sum(p.stat().st_size for p in Path(scratch).rglob("*") if p.is_file())
                < 64 << 20
            )
            print(name, "passed", flush=True)
    print(f"{len(cases)} original Go paged document contracts passed")


if __name__ == "__main__":
    main()
