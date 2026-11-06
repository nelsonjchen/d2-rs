#!/usr/bin/env python3
"""Compare complete e2e SVG output inventories with the pinned Windows Go CLI.

Run on Linux with Wine and cross-built Windows binaries. Absolute arguments
use Wine's Z: mapping and forward slashes, which Go preserves for imports.
"""
from __future__ import annotations

import argparse
import collections
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import threading

import references

ROOT = references.ROOT
UPSTREAM = (ROOT / references.MANIFEST["d2"]["checkout"]).resolve()
MAX_FILE = 128 * 1024 * 1024
MAX_TOTAL = 1024 * 1024 * 1024


def windows_path(path: Path) -> str:
    return "Z:" + path.resolve().as_posix()


def inventory(directory: Path) -> tuple[dict[str, str], int]:
    files = {}
    size = 0
    for path in sorted(directory.rglob("*")):
        if not path.is_file():
            continue
        length = path.stat().st_size
        if length > MAX_FILE:
            raise ValueError(f"output exceeds {MAX_FILE} bytes: {path.name}")
        size += length
        data = path.read_bytes()
        if path.suffix == ".svg":
            # Normalize renderer stamps on SVG tags, preserving label text.
            data = re.sub(
                rb'(<svg\b[^>]*?\s)(data-d2-version|d2Version)="[^"]*"',
                lambda match: match[1] + match[2] + b'=""',
                data,
            )
        files[path.relative_to(directory).as_posix()] = hashlib.sha256(data).hexdigest()
    return files, size


def run_case(args, case: dict, engine: str) -> dict:
    row = {"name": case["name"], "engine": engine}
    if getattr(args, "budget_exhausted", False):
        return {**row, "outcome": "output-budget-exceeded"}
    expected = UPSTREAM / "e2etests/testdata" / case["name"] / engine / "sketch.exp.svg"
    error = case["expErr"] or (case["talaLayoutError"] if engine == "tala" else "")
    error = error or {"dagre": case["dagreFeatureError"], "elk": case["elkFeatureError"]}.get(engine, "")
    if case["hasMtexts"]:
        return {**row, "outcome": "inapplicable-measured-texts"}
    if not error and not expected.is_file():
        return {**row, "outcome": "no-golden"}
    with tempfile.TemporaryDirectory(prefix="d2rs-win-e2e-") as scratch:
        work = Path(scratch)
        source = work / "input.d2"
        source.write_text(case["script"])
        flags = ["--pad=0", "--bundle=false", "--no-xml-tag", "--omit-version=false",
                 "--tala-seeds=1", f"--layout={engine}", f"--timeout={args.timeout}"]
        if case["themeID"] is not None:
            flags.append(f"--theme={case['themeID']}")
        if expected.is_file() and ' d2Version="' in expected.read_text().split(">", 1)[0]:
            flags.append("--animate-interval=1000")
        results = {}
        total = 0
        for name, binary in [("go", args.go), ("rust", args.rust)]:
            output = work / name
            output.mkdir()
            try:
                process = subprocess.run(
                    [args.wine, windows_path(binary), *flags, windows_path(source),
                     windows_path(output / "output.svg")],
                    capture_output=True, timeout=args.timeout + 15,
                    env={**os.environ, "WINEDEBUG": "-all"},
                )
                files, size = inventory(output)
                total += size
                if getattr(args, "budget_lock", None) is not None:
                    with args.budget_lock:
                        args.output_bytes += size + len(process.stdout) + len(process.stderr)
                        if args.output_bytes > MAX_TOTAL:
                            args.budget_exhausted = True
                    if args.budget_exhausted:
                        return {**row, "outcome": "output-budget-exceeded"}
                results[name] = {"exit": process.returncode, "files": files,
                                 "stdout": process.stdout.decode(errors="replace"),
                                 "stderr": process.stderr.decode(errors="replace")}
            except (subprocess.TimeoutExpired, OSError, ValueError) as exc:
                return {**row, "outcome": "process-failed", "binary": name, "error": str(exc)}
        if total > MAX_TOTAL:
            return {**row, "outcome": "output-budget-exceeded"}
        go, rust = results["go"], results["rust"]
        row["bytes"] = total
        if go["exit"] != rust["exit"]:
            outcome = "exit-mismatch"
        elif go["exit"]:
            outcome = "error-match" if go["stderr"] == rust["stderr"] and go["stdout"] == rust["stdout"] else "error-mismatch"
            if not error or error not in go["stderr"]:
                outcome = "unexpected-error"
        elif not go["files"]:
            outcome = "inapplicable-empty-cli-output" if not case["script"].strip() and not rust["files"] else "missing-output"
        elif error:
            outcome = "missing-error"
        else:
            outcome = "svg-match" if go["files"] == rust["files"] and go["stdout"] == rust["stdout"] else "svg-mismatch"
        row["outcome"] = outcome
        if outcome not in {"svg-match", "error-match", "inapplicable-empty-cli-output"}:
            row["results"] = results
            if getattr(args, "witnesses_dir", None) is not None:
                args.witnesses_dir.mkdir(parents=True, exist_ok=True)
                witness = Path(tempfile.mkdtemp(
                    prefix=case["name"].replace("/", "-") + "-" + engine + "-",
                    dir=args.witnesses_dir,
                ))
                shutil.copytree(work, witness, dirs_exist_ok=True)
                row["witness"] = str(witness)
        return row


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/x86_64-pc-windows-gnu/diagnostic/d2-rs.exe")
    parser.add_argument("--wine", default="wine")
    parser.add_argument("--cases", type=Path, default=references.E2E_CASES)
    parser.add_argument("--output", type=Path, default=references.REPORTS / "windows-e2e.json")
    parser.add_argument("--witnesses-dir", type=Path)
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--filter", default="")
    args = parser.parse_args()
    args.budget_lock = threading.Lock()
    args.output_bytes = 0
    args.budget_exhausted = False
    if args.witnesses_dir is None:
        args.witnesses_dir = args.output.parent / (args.output.stem + "-witnesses")
    references.verify_source()
    metadata = subprocess.check_output(["go", "version", "-m", str(args.go)], text=True)
    if f"vcs.revision={references.SOURCE_COMMIT}" not in metadata or "vcs.modified=true" in metadata or references.MANIFEST["d2"]["go_toolchain"] not in metadata.splitlines()[0] or "GOOS=windows" not in metadata or "GOARCH=amd64" not in metadata:
        parser.error("Go binary must be built from the pin with the pinned toolchain for windows/amd64")
    version = subprocess.check_output([args.wine, windows_path(args.go), "--version"], env={**os.environ, "WINEDEBUG": "-all"}, timeout=45).decode().strip()
    if version != "v0.9.0":
        parser.error(f"Go binary has unexpected version {version!r}")
    cases = [json.loads(path.read_text()) for path in sorted(args.cases.glob("*.json"))]
    if not cases:
        parser.error(f"no cases in {args.cases}")
    jobs = [(case, engine) for case in cases if not args.filter or re.search(args.filter, case["name"])
            for engine in (["dagre"] if case["justDagre"] else ["dagre", "elk", "tala"])]
    if not jobs:
        parser.error("no selected cases")
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        rows = list(pool.map(lambda job: run_case(args, *job), jobs))
    summary = dict(collections.Counter(row["outcome"] for row in rows))
    report = {"go": str(args.go), "rust": str(args.rust), "source_commit": references.SOURCE_COMMIT,
              "go_sha256": references.digest(args.go), "rust_sha256": references.digest(args.rust),
              "output_bytes": args.output_bytes, "output_budget": MAX_TOTAL,
              "summary": summary, "rows": rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps(summary, sort_keys=True))
    passing = {"svg-match", "error-match", "inapplicable-empty-cli-output", "inapplicable-measured-texts", "no-golden"}
    return int(any(row["outcome"] not in passing for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
