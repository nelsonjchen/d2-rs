#!/usr/bin/env python3
"""Build the raw ELK oracle against pristine pinned D2's dependency graph."""
import hashlib
import json
import os
import subprocess

from references import ROOT, REFERENCE, MANIFEST, verify_source

source = verify_source()
driver = ROOT / "tools/elk-oracle/main.go"
output = REFERENCE / "bin/elk-wire-oracle"
output.parent.mkdir(parents=True, exist_ok=True)
environment = {**os.environ, "GOTOOLCHAIN": MANIFEST["d2"]["go_toolchain"]}
subprocess.run(["go", "build", "-mod=readonly", "-trimpath", "-o", str(output), str(driver)],
               cwd=source, env=environment, check=True)
verify_source()
info = subprocess.check_output(["go", "version", "-m", str(output)], env=environment, text=True)
if MANIFEST["d2"]["go_toolchain"] not in info.splitlines()[0]:
    raise RuntimeError("ELK oracle toolchain mismatch")
if "github.com/d2lang/elk-go\tv0.2.0\t" not in info:
    raise RuntimeError("ELK oracle dependency mismatch")
digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
record = {
    "source_commit": MANIFEST["d2"]["commit"],
    "go_toolchain": MANIFEST["d2"]["go_toolchain"],
    "driver_sha256": digest(driver), "binary_sha256": digest(output),
    "go_mod_sha256": digest(source / "go.mod"),
    "go_sum_sha256": digest(source / "go.sum"),
    "build_info": info,
    "build_command": "uv run --no-project python scripts/build-elk-oracle.py",
    "scope": "Raw elk-go dependency output; not the official D2 release binary",
}
(REFERENCE / "elk-wire-oracle-build.json").write_text(json.dumps(record, indent=2) + "\n")
print(output)
