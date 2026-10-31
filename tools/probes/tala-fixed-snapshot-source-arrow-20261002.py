#!/usr/bin/env python3
"""Freeze bounded single-seed stage and routing witnesses against pinned D2."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ["tala-fixed-descendant-direct", "tala-source-arrow-repeated-labels",
            "tala-source-arrow-repeated-labels-reversed-control"]
BINARIES = {"go": ROOT / ".reference/bin/d2-v0.9.0-search-occupied-route-trace",
            "rust": ROOT / "target/diagnostic/d2-rs"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suffix", default="after-20261002")
    parser.add_argument("--include-fixed-origin", action="store_true")
    args = parser.parse_args()
    destination = ROOT / f".reference/fixed-snapshot-source-arrow-{args.suffix}"
    destination.mkdir(parents=True, exist_ok=False)
    fixtures = FIXTURES + (["tala-fixed-origin-transpose"] if args.include_fixed_origin else [])
    runs = []
    hashes = {side: digest(path) for side, path in BINARIES.items()}
    for fixture in fixtures:
        folder = destination / fixture
        folder.mkdir()
        source = ROOT / "tests/d2" / f"{fixture}.d2"
        for mode in ["stages", "work"]:
            for side, binary in BINARIES.items():
                if mode == "stages":
                    prefix = "TALA" if side == "go" else "WEFTAN"
                    overrides = {f"{prefix}_TRACE_{name}": "1" for name in
                                 ["JSONL", "PLACEMENT", "TRANSPOSE_CANDIDATES"]}
                    overrides[f"{prefix}_TRACE_TRANSPOSE_NODE"] = "all"
                else:
                    overrides = {"D2_TRACE_GENERATION_WORK": "1",
                                 "D2_TRACE_PUBLICATION_WORK": "1",
                                 "D2_TRACE_SEARCH_HEAP": "1",
                                 "D2_TRACE_SEARCH_FLAVOR": "Default"}
                output = folder / f"{side}-{mode}.svg"
                argv = [str(binary), f"--layout={'tala' if side == 'go' else 'tala-rs'}",
                        "--tala-seeds=1", str(source), str(output)]
                completed = subprocess.run(argv, cwd=ROOT,
                                           env=os.environ | overrides,
                                           capture_output=True, timeout=120)
                (folder / f"{side}-{mode}.stdout").write_bytes(completed.stdout)
                (folder / f"{side}-{mode}.stderr").write_bytes(completed.stderr)
                runs.append({"fixture": str(source.relative_to(ROOT)),
                             "fixture_sha256": digest(source), "mode": mode,
                             "side": side, "argv": argv, "environment_overrides": overrides,
                             "exit_code": completed.returncode,
                             "binary_sha256_before": hashes[side],
                             "binary_sha256_after": digest(binary)})
    manifest = {"upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
                "cwd": str(ROOT), "seed": 1, "runs": runs,
                "files": [{"path": str(path.relative_to(ROOT)), "bytes": path.stat().st_size,
                           "sha256": digest(path)}
                          for path in sorted(destination.rglob("*")) if path.is_file()],
                "script_sha256": digest(Path(__file__)),
                "full_operation_trace_parity": False}
    (ROOT / f"docs/tala-fixed-snapshot-source-arrow-{args.suffix}-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n")
    assert all(run["exit_code"] == 0 and
               run["binary_sha256_before"] == run["binary_sha256_after"] for run in runs)
    print(json.dumps({"captured_runs": len(runs), "binaries": hashes}))


if __name__ == "__main__":
    main()
