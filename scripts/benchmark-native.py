#!/usr/bin/env python3
"""Benchmark the pinned Go D2 release beside the native Rust stages.

The measurements are intentionally simple wall-clock samples.  They are a
qualification aid, not a claim of statistical significance: every command is
run with the same stdin, seed, warmups, and output sink, and the report records
the exact binaries and host information used.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import time


DEFAULT_FIXTURES = (
    "tests/d2/direction-right.d2",
    "tests/d2/features.d2",
    "tests/d2/benchmark-medium.d2",
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def command_version(command: list[str]) -> str | None:
    try:
        completed = subprocess.run(
            command,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return completed.stdout.strip() if completed.stdout.strip() else None


def binary_toolchain(path: Path) -> str | None:
    """Read the compiler recorded in a Go binary's build metadata.

    The host `go version` can differ from the toolchain that produced the
    release artifact (Go's toolchain selection downloads and invokes a pinned
    version automatically). Record both values so benchmarks identify the
    actual D2 runtime rather than the machine used to run this script.
    """
    metadata = command_version(["go", "version", "-m", str(path)])
    if not metadata:
        return None
    first_line = metadata.splitlines()[0]
    _, separator, version = first_line.partition(": ")
    return version if separator else first_line


def sample(command: list[str], source: bytes, warmups: int, runs: int, environment: dict[str, str]) -> list[float]:
    for _ in range(warmups):
        completed = subprocess.run(
            command,
            input=source,
            env=environment,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            check=False,
        )
        if completed.returncode:
            raise RuntimeError(
                f"{command!r} failed during warmup with {completed.returncode}: "
                f"{completed.stderr.decode(errors='replace')}"
            )
    samples: list[float] = []
    for _ in range(runs):
        started = time.perf_counter_ns()
        completed = subprocess.run(
            command,
            input=source,
            env=environment,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            check=False,
        )
        elapsed = (time.perf_counter_ns() - started) / 1_000_000_000
        if completed.returncode:
            raise RuntimeError(
                f"{command!r} failed with {completed.returncode}: "
                f"{completed.stderr.decode(errors='replace')}"
            )
        samples.append(elapsed)
    return samples


def peak_memory(command: list[str], source: bytes, environment: dict[str, str]) -> dict[str, object]:
    """One independent macOS maximum-RSS sample for a complete command."""
    if platform.system() != "Darwin":
        return {"assessed": False, "reason": "This probe uses macOS /usr/bin/time -l."}
    argv = ["/usr/bin/time", "-l", *command]
    completed = subprocess.run(argv, input=source, env=environment,
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=120)
    stderr = completed.stderr.decode(errors="replace")
    match = re.search(r"^\s*(\d+)\s+maximum resident set size\s*$", stderr, re.MULTILINE)
    if completed.returncode or match is None:
        raise RuntimeError(f"memory probe {argv!r} failed: {stderr}")
    return {"assessed": True, "maximum_rss_bytes": int(match.group(1)), "samples": 1,
            "argv": argv, "stderr_sha256": hashlib.sha256(completed.stderr).hexdigest()}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=Path("target/release/d2-rs"))
    parser.add_argument(
        "--go",
        type=Path,
        default=Path(".reference/bin/d2-v0.9.0-official"),
    )
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--threads", type=int, default=1)
    parser.add_argument("--output", type=Path)
    parser.add_argument("fixtures", nargs="*", default=list(DEFAULT_FIXTURES))
    args = parser.parse_args()

    if args.threads < 1:
        parser.error("--threads must be positive")
    if args.runs < 1 or args.warmups < 0:
        parser.error("--runs must be positive and --warmups must not be negative")
    for binary in (args.rust, args.go):
        if not binary.is_file():
            parser.error(f"binary does not exist: {binary}")

    reference = json.loads((Path(__file__).resolve().parents[1] / "reference-manifest.json").read_text())
    expected_release_hashes = {asset.get("binary_sha256") for asset in reference["releases"].values()}
    if sha256(args.go) not in expected_release_hashes:
        parser.error("--go must be a verified official release binary from reference-manifest.json")
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("D2_TRACE_", "D2RS_", "TALA_TRACE_", "WEFTAN_"))
                   and key not in ("DEV_MODE", "TEST_MODE")}
    environment.update({"GOMAXPROCS": str(args.threads), "RAYON_NUM_THREADS": str(args.threads)})
    rust_command = [str(args.rust), "render-svg", "--engine=tala", f"--seeds={args.seed}"]
    go_command = [str(args.go), "--layout=tala", f"--tala-seeds={args.seed}", "-", "-"]
    stages = {
        "rust_parse": [str(args.rust), "parse"],
        "rust_graph": [str(args.rust), "graph"],
        "rust_layout": [str(args.rust), "layout-source", "--engine=tala", f"--seeds={args.seed}"],
        "rust_startup_version": [str(args.rust), "--version"],
        "go_release_startup_version": [str(args.go), "--version"],
        "rust_render_svg": rust_command,
        "go_release_render_svg": go_command,
    }

    results = []
    for fixture_name in args.fixtures:
        fixture = Path(fixture_name)
        source = fixture.read_bytes()
        measurements = {
            stage: sample(command, source, args.warmups, args.runs, environment)
            for stage, command in stages.items()
        }
        results.append(
            {
                "fixture": fixture.as_posix(),
                "input_sha256": hashlib.sha256(source).hexdigest(),
                "samples_s": measurements,
                "peak_memory": {stage: peak_memory(stages[stage], source, environment)
                                for stage in ("rust_render_svg", "go_release_render_svg")},
                "median_s": {stage: statistics.median(values) for stage, values in measurements.items()},
                "min_s": {stage: min(values) for stage, values in measurements.items()},
                "max_s": {stage: max(values) for stage, values in measurements.items()},
            }
        )

    report = {
        "schema_version": 3,
        "kind": "native_stage_and_official_release_benchmark",
        "rust_binary": str(args.rust),
        "rust_binary_sha256": sha256(args.rust),
        "go_binary": str(args.go),
        "go_binary_sha256": sha256(args.go),
        "official_release_checksum_verified": True,
        "commands": stages,
        "environment_overrides": {"GOMAXPROCS": str(args.threads), "RAYON_NUM_THREADS": str(args.threads)},
        "diagnostic_environment_removed": True,
        "rustc": command_version(["rustc", "-Vv"]),
        "go": binary_toolchain(args.go),
        "go_host": command_version(["go", "version"]),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "logical_cpus": os.cpu_count(),
        "seed": args.seed,
        "runs": args.runs,
        "warmups": args.warmups,
        "results": results,
        "limitations": [
            "Wall-clock samples include process startup and stdin/stdout plumbing.",
            "Rust stage commands measure cumulative prefixes plus JSON serialization, not isolated algorithm time; Go exposes no equivalent public stage commands.",
            "Peak memory is one independent macOS sample per full render command; renderer internals, direct engine time and cross-host variance are not measured here.",
        ],
    }
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    else:
        print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
