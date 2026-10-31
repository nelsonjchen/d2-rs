#!/usr/bin/env python3
"""Reject unsafe ownership manifests without publishing SVG/PNG boards.

Uses the three original upstream unsafe claims and the same paths for both
CLIs. Complete errors, file inventories and bytes compare without normalization.
Limits: streaming 1 GiB total / 128 MiB file / 20 GiB free, 60 seconds per call.
"""

import argparse
import base64
import hashlib
import importlib.util
import json
import os
import shutil
import xml.etree.ElementTree as ET
from pathlib import Path

import references

ROOT = references.ROOT
MANIFEST = ".d2-board-output-manifest-v1.json"
spec = importlib.util.spec_from_file_location("cli", ROOT / "scripts/cli-scoreboard.py")
cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cli)
cli.TRACE = ROOT


def inventory(root):
    return {
        str(p.relative_to(root)): base64.b64encode(p.read_bytes()).decode()
        for p in root.rglob("*")
        if p.is_file()
        and p.name not in {"go.log", "rust.log"}
        and root / "tmp" not in p.parents
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    references.verify_source()
    runner = args.rust.resolve()
    sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    runner_hash = sha(runner)
    capture = args.capture.resolve()
    capture.mkdir(exist_ok=False)
    fixture = ROOT / "crates/d2-rs-cli/tests/fixtures/board-manifest-api.json"
    calls = json.loads(fixture.read_text())["calls"][:3]
    assert len(calls) == 3
    # The validator accepts either hex spelling. Stale-file retirement
    # compares its lowercase form with the hash computed from disk bytes.
    stale_bytes = b"previously generated\n"
    calls.append(
        {
            "error": "",
            "manifest": {
                "format": "d2-board-output",
                "version": 1,
                "files": [
                    {
                        "path": "stale.svg",
                        "sha256": hashlib.sha256(stale_bytes).hexdigest().upper(),
                    }
                ],
                "directories": [],
            },
        }
    )
    rows = []
    for index, call in enumerate(calls):
        for extension in ["svg", "png"] if index < 3 else ["svg"]:
            work = capture / f"claim-{index}-{extension}"
            work.mkdir()
            (work / "tmp").mkdir()
            source = work / "input.d2"
            source.write_text("root\nlayers: { child: { node } }\n")
            output = work / ("output." + extension)
            tree = output.with_suffix("")
            results = {}
            for implementation, binary in [
                ("go", references.GO_ORACLE),
                ("rust", runner),
            ]:
                tree.mkdir()
                (tree / MANIFEST).write_text(json.dumps(call["manifest"]) + "\n")
                (tree / "sentinel.txt").write_bytes(b"preserve user bytes\n")
                if index == 3:
                    (tree / "stale.svg").write_bytes(stale_bytes)
                before = inventory(work)
                log = work / (implementation + ".log")
                result = cli.bounded_run(
                    [str(binary), str(source), str(output)],
                    dict(os.environ, TMPDIR=str(work / "tmp")),
                    capture,
                    log,
                    60,
                )
                errors = "\n".join(
                    line
                    for line in log.read_text().splitlines()
                    if line.startswith("err:")
                )
                files = inventory(work)
                stage_paths = sorted(p.name for p in work.glob(".d2-board-output-*"))
                results[implementation] = {
                    "run": result,
                    "errors": errors,
                    "files": files,
                    "preserved": files == before,
                    "stages": stage_paths,
                }
                shutil.rmtree(tree)
            expected = results["go"]
            if index < 3:
                assert (
                    expected["run"] == {"exit": 1, "failure": None}
                    and expected["preserved"]
                )
                assert call["error"] in expected["errors"]
            else:
                assert (
                    expected["run"] == {"exit": 0, "failure": None}
                    and not expected["errors"]
                )
                assert "output/stale.svg" not in expected["files"]
                for name in ["output/index.svg", "output/child.svg"]:
                    assert ET.fromstring(
                        base64.b64decode(expected["files"][name])
                    ).tag.endswith("svg")
            assert not expected["stages"]
            rows.append(
                {
                    "name": f"claim-{index}-{extension}",
                    "outcome": "match" if expected == results["rust"] else "mismatch",
                    "results": results,
                }
            )
    assert sha(runner) == runner_hash
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "capture": str(capture),
        "runner": str(runner),
        "runner_sha256": runner_hash,
        "oracle_sha256": sha(references.GO_ORACLE),
        "fixture_sha256": sha(fixture),
        "rows": rows,
        "matches": sum(r["outcome"] == "match" for r in rows),
        "total": len(rows),
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"matches": report["matches"], "total": len(rows)}))


if __name__ == "__main__":
    main()
