#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Freeze affected native tests and both build profiles for dash/text fixes."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


def inventory(path: Path) -> dict:
    data = path.read_bytes()
    return {"path": str(path.relative_to(ROOT)), "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest()}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suffix", default="20261002")
    parser.add_argument("--label", default="dash-text")
    parser.add_argument("--with-tala-suite", action="store_true",
                        help="also run the complete optimized diagnostic TALA suite against the frozen source")
    parser.add_argument("--tala-example", help="also build and freeze a diagnostic TALA example")
    parser.add_argument("--builds-only", action="store_true",
                        help="freeze builds without running or claiming test suites")
    parser.add_argument("--example-only", action="store_true",
                        help="freeze only the selected diagnostic example for an initial divergence witness")
    args = parser.parse_args()
    assert args.suffix and all(character.isalnum() or character == "-" for character in args.suffix)
    assert args.label and all(character.isalnum() or character == "-" for character in args.label)
    if args.tala_example:
        assert all(character.isalnum() or character in "-_" for character in args.tala_example)
    assert not (args.builds_only and args.with_tala_suite)
    assert not (args.example_only and (args.builds_only or args.with_tala_suite))
    assert not args.example_only or args.tala_example
    OUTPUT = ROOT / f".reference/tala-{args.label}-tests-build-{args.suffix}"
    REPORT = ROOT / f"docs/tala-{args.label}-tests-build-{args.suffix}.json"
    assert not OUTPUT.exists() and not REPORT.exists(), "Evidence capture already exists"
    OUTPUT.mkdir()
    tool_snapshot = OUTPUT / "build-driver-source-at-run.py"
    tool_snapshot.write_bytes(Path(__file__).read_bytes())
    files = subprocess.check_output(["rg", "--files", "crates"], cwd=ROOT, text=True).splitlines()
    files += ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE", "THIRD_PARTY_NOTICES.md", "DEPENDENCY_NOTICES.md"]
    sources = []
    for relative in sorted(set(files)):
        live = ROOT / relative
        frozen = OUTPUT / "compiled-source" / relative
        frozen.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(live, frozen)
        sources.append({"live": inventory(live), "frozen": inventory(frozen)})
    environment = dict(os.environ)
    removed = sorted(name for name in environment if name.startswith(
        ("D2_TRACE_", "D2RS_", "D2_RS_", "D2_TALA_", "TALA_", "WEFTAN_"))
        or name in ("DEV_MODE", "TEST_MODE"))
    for name in removed:
        del environment[name]
    environment["CARGO_TERM_COLOR"] = "never"
    report = {"schema_version": 1,
              "scope": ("Frozen first-party Cargo source/assets and selected builds only; no test suites executed or claimed by this run."
                        if args.builds_only or args.example_only else
                        "Frozen first-party Cargo source/assets, affected rendering/export/orchestration and CLI tests, diagnostic and feature-off release.")
                       + " Not full D2 parity or full external dependency source archive.",
              "builds_only": args.builds_only,
              "example_only": args.example_only,
              "sources": sources, "steps": [], "binaries": {},
              "build_driver": inventory(tool_snapshot),
              "removed_environment_names": removed, "sources_unchanged": True,
              "passed": False}

    def save() -> None:
        REPORT.write_text(json.dumps(report, indent=2) + "\n")

    steps = [
        ("affected-suites", ["cargo", "test", "-p", "d2-rs-render", "-p", "d2-rs-export",
                             "-p", "d2-rs-lib", "--profile", "diagnostic", "--features",
                             "d2-rs-render/diagnostic-traces,d2-rs-export/diagnostic-traces,d2-tala-rs/diagnostic-traces"]),
        ("cli-suite", ["cargo", "test", "-p", "d2-rs-cli", "--bin", "d2-rs",
                       "--profile", "diagnostic", "--features", "diagnostic-traces"]),
        ("diagnostic-build", ["cargo", "build", "-p", "d2-rs-cli", "--bin", "d2-rs",
                              "--profile", "diagnostic", "--features", "diagnostic-traces"]),
        ("release-build", ["cargo", "build", "-p", "d2-rs-cli", "--bin", "d2-rs",
                           "--release", "--no-default-features"]),
    ]
    if args.with_tala_suite:
        steps.insert(0, ("tala-suite", ["cargo", "test", "-p", "d2-tala-rs",
                                     "--profile", "diagnostic", "--features",
                                     "diagnostic-traces"]))
    if args.builds_only:
        steps = [(name, argv) for name, argv in steps if name.endswith("build")]
    if args.example_only:
        steps = []
    if args.tala_example:
        steps.insert(1 if args.with_tala_suite else 0,
                     ("tala-example-build", ["cargo", "build", "-p", "d2-tala-rs",
                                             "--example", args.tala_example, "--profile",
                                             "diagnostic", "--features", "diagnostic-traces"]))
    for name, argv in steps:
        started = time.monotonic()
        log = OUTPUT / f"{name}.log"
        with log.open("wb") as handle:
            completed = subprocess.run(argv, cwd=ROOT, env=environment,
                                       stdout=handle, stderr=subprocess.STDOUT)
        unchanged = all(inventory(ROOT / entry["live"]["path"]) == entry["live"] for entry in sources)
        report["sources_unchanged"] &= unchanged
        report["steps"].append({"name": name, "argv": argv, "cwd": str(ROOT),
                                 "exit_code": completed.returncode,
                                 "elapsed_seconds": time.monotonic() - started,
                                 "sources_unchanged": unchanged, "log": inventory(log)})
        save()
        print(json.dumps(report["steps"][-1]), flush=True)
        if completed.returncode or not unchanged:
            return 1
        if name in ("diagnostic-build", "release-build"):
            profile = "diagnostic" if name == "diagnostic-build" else "release"
            label = "diagnostic" if profile == "diagnostic" else "feature-off"
            target = ROOT / "target" / profile / "d2-rs"
            frozen = ROOT / f".reference/bin/d2-rs-{args.label}-{label}-{args.suffix}"
            assert not frozen.exists()
            shutil.copy2(target, frozen)
            report["binaries"][profile] = inventory(frozen)
            save()
        elif name == "tala-example-build":
            target = ROOT / "target/diagnostic/examples" / args.tala_example
            frozen = ROOT / f".reference/bin/{args.tala_example}-{args.label}-diagnostic-{args.suffix}"
            assert not frozen.exists()
            shutil.copy2(target, frozen)
            report["binaries"]["tala-example"] = inventory(frozen)
            save()
    report["passed"] = True
    save()
    print(json.dumps({"passed": True, "source_count": len(sources), "binaries": report["binaries"]}), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
