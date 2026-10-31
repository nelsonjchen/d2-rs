#!/usr/bin/env python3
"""Shared CLI collision admission on complete SVG/PNG output inventories.

Pinned Go must reject each input without creating outputs. Native exit, full
error lines and output bytes compare, using the same input/output paths.
Raw logs remain. Each call has streaming 1 GiB / 128 MiB file / 20 GiB free
limits and a 60-second deadline. No generator-version or path normalization.
"""

import argparse
import base64
import hashlib
import importlib.util
import json
import os
import shutil
from pathlib import Path

import references

ROOT = references.ROOT
spec = importlib.util.spec_from_file_location("cli", ROOT / "scripts/cli-scoreboard.py")
cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cli)
cli.TRACE = ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    references.verify_source()
    runner = args.rust.resolve()
    digest = hashlib.sha256(runner.read_bytes()).hexdigest()
    capture = args.capture.resolve()
    capture.mkdir(exist_ok=False)
    rows = []
    for name, first, second in [
        ("accent", "é", "e\u0301"),
        ("fold", "Straße", "STRASSE"),
    ]:
        for extension in ["svg", "png"]:
            work = capture / (name + "-" + extension)
            work.mkdir()
            source = work / "input.d2"
            output = work / ("output." + extension)
            source.write_text(
                f'root\nlayers: {{\n  "{first}": {{ a }}\n  "{second}": {{ b }}\n}}\n'
            )
            results = {}
            for implementation, binary in [
                ("go", references.GO_ORACLE),
                ("rust", runner),
            ]:
                log = work / (implementation + ".log")
                result = cli.bounded_run(
                    [str(binary), str(source), str(output)],
                    dict(os.environ, TMPDIR=str(work)),
                    capture,
                    log,
                    60,
                )
                raw = log.read_text()
                # The CLI's error transport is explicit; success/timing logs
                # are retained but are not errors in either implementation.
                errors = "\n".join(
                    line for line in raw.splitlines() if line.startswith("err:")
                )
                outputs = {
                    str(p.relative_to(work)): base64.b64encode(p.read_bytes()).decode()
                    for p in work.rglob("*")
                    if p.is_file() and p != source and p.suffix != ".log"
                }
                results[implementation] = {
                    "run": result,
                    "errors": errors,
                    "outputs": outputs,
                }
                # Disposable outputs have been retained byte-for-byte above.
                for generated in [output, output.with_suffix("")]:
                    if generated.is_dir():
                        shutil.rmtree(generated)
                    elif generated.exists():
                        generated.unlink()
            expected = results["go"]
            assert expected["run"] == {"exit": 1, "failure": None}
            assert (
                "resolve to the same output path" in expected["errors"]
                and not expected["outputs"]
            )
            rows.append(
                {
                    "name": name + "-" + extension,
                    "outcome": "match" if expected == results["rust"] else "mismatch",
                    "results": results,
                }
            )
    assert hashlib.sha256(runner.read_bytes()).hexdigest() == digest
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "capture": str(capture),
        "runner": str(runner),
        "runner_sha256": digest,
        "oracle_sha256": hashlib.sha256(references.GO_ORACLE.read_bytes()).hexdigest(),
        "rows": rows,
        "matches": sum(r["outcome"] == "match" for r in rows),
        "total": len(rows),
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"matches": report["matches"], "total": report["total"]}))


if __name__ == "__main__":
    main()
