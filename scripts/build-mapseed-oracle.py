#!/usr/bin/env python3
"""Build the pinned Go D2 CLI with seedable map iteration order (test only).

Go randomizes every map's hash seed and every `range` start offset, so a few
layouts (dagre's shiftReachableDown ranges over a map) have more than one Go
output. This builds the verified pristine checkout with the pinned toolchain
and one runtime change, tools/patches/go1.27-runtime-mapseed.patch, applied
through `go build -overlay` (the installed toolchain is never edited): with
D2RS_GO_MAPSEED=<n> set, maps draw their randomness from a counter mixed with
n, so a single-goroutine program is reproducible per seed. Unset, the binary
behaves like the stock oracle. The differential harnesses resample differing
cases with it through scripts/mapseed.py, which classifies them.

usage: build-mapseed-oracle.py [--output PATH]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

import mapseed
import references

PATCH = mapseed.RUNTIME_PATCH
DEFAULT_OUTPUT = references.GO_MAPSEED_ORACLE


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args(argv)
    source = references.verify_source()
    toolchain = references.MANIFEST["d2"]["go_toolchain"]
    env = {**os.environ, "GOTOOLCHAIN": toolchain,
           "GOMODCACHE": str(references.REFERENCE / "go-mod/pkg/mod"),
           "GOCACHE": str(references.REFERENCE / "go-build"), "GOFLAGS": "-mod=readonly"}
    version = subprocess.check_output(["go", "env", "GOVERSION"], env=env, text=True).strip()
    if version != toolchain:
        raise RuntimeError(f"Go toolchain {version} is not the pinned {toolchain}")
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="mapseed-") as scratch:
        overlay = mapseed.runtime_overlay(scratch, env)
        subprocess.run(["go", "build", "-overlay", str(overlay),
                        "-ldflags", "-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                        "-o", str(output), "."], cwd=source, env=env, check=True, timeout=900)
    references.verify_source()
    record = {"source_commit": references.SOURCE_COMMIT, "go_toolchain": toolchain,
              "binary_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
              "runtime_patch_sha256": hashlib.sha256(PATCH.read_bytes()).hexdigest()}
    # Not with_suffix: "go1.27-mapseed" would become the stock oracle's name.
    output.with_name(output.name + ".provenance.json").write_text(json.dumps(record, indent=2) + "\n")
    print(output)


if __name__ == "__main__":
    main()
