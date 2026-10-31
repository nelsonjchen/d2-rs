#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Check complete CLI PNG files against pinned release Go v0.9.0 hashes.

Run with uv run --no-project python scripts/check-png-metadata.py --rust <binary>.
Fixtures cover file/stdin/stdout, subtree traversal and --omit-version. Every
PNG must decode and the complete output inventory and file hashes must match.
Generator: .reference/codex-runs/witness-inputs/run40-png-boundaries.py.
"""

import argparse
import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from fixture_recipe import cached_fixture  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    args = parser.parse_args()
    binary = args.rust.resolve()
    spec = importlib.util.spec_from_file_location(
        "png_gate", ROOT / "scripts/check-render-parity.py"
    )
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    cases = json.loads(
        (
            cached_fixture("crates/d2-rs-cli/tests/fixtures/png-metadata-source-v090.json")
        ).read_text()
    )
    assert cases, "empty fixture inventory"
    png_count = 0
    for case in cases:
        with tempfile.TemporaryDirectory(prefix="d2rs-png-metadata-") as temporary:
            directory = Path(temporary)
            source = directory / "input.d2"
            source.write_text(case["source"])
            output = directory / "out.png"
            command = [
                str(binary),
                "--pad=0",
                "--stdout-format=png",
                *case["flags"],
                "-" if case["stdin"] else str(source),
                "-" if case["stdout"] else str(output),
            ]
            process = subprocess.run(
                command,
                check=False,
                cwd=directory,
                capture_output=True,
                timeout=90,
                input=case["source"].encode() if case["stdin"] else None,
            )
            assert process.returncode == 0, (case["name"], process.stderr)
            if case["stdout"]:
                output.write_bytes(process.stdout)
            else:
                assert not process.stdout, case["name"]
            inventory = gate.collect_png_outputs(output)
            files = (
                {"": output}
                if output.is_file()
                else {
                    p.relative_to(output.with_suffix("")).as_posix(): p
                    for p in output.with_suffix("").rglob("*")
                    if p.is_file()
                }
            )
            assert inventory and set(inventory) == set(files) == set(case["sha256"]), (
                case["name"]
            )
            actual = {
                key: hashlib.sha256(path.read_bytes()).hexdigest()
                for key, path in files.items()
            }
            assert actual == case["sha256"], (case["name"], actual)
            png_count += len(files)
    print(
        f"{len(cases)} original Go PNG process contracts / {png_count} complete files passed"
    )


if __name__ == "__main__":
    main()
