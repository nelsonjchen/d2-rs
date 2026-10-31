#!/usr/bin/env python3
"""Build a development-only direct TALA oracle without changing pristine D2."""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
from pathlib import Path
from references import ROOT, REFERENCE, MANIFEST, verify_source

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--trace", action="store_true",
                    help="build against the separately instrumented v0.9.0 worktree")
args = parser.parse_args()

source = verify_source()
trace_patch_sha256 = None
if args.trace:
    source = (ROOT / MANIFEST["d2"]["trace_checkout"]).resolve()
    revision = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if revision != MANIFEST["d2"]["commit"]:
        raise RuntimeError(f"Trace reference commit mismatch: {revision}")
    trace_patch_sha256 = hashlib.sha256(
        (ROOT / "tools/patches/d2-v090-stage-trace.patch").read_bytes()
    ).hexdigest()
module = REFERENCE / "engine-oracle-module"
module.mkdir(parents=True, exist_ok=True)
# Go internal-package visibility is based on import paths. This development
# module uses a path inside TALA's import subtree but lives only in .reference.
module_path = "github.com/d2lang/d2/d2layouts/d2talalayout/tools/rust-parity-oracle"
base_mod = (ROOT / "tools/oracle/go.mod").read_text()
base_mod = base_mod.replace(base_mod.splitlines()[0], "module " + module_path, 1)
base_mod = base_mod.replace("../../../d2", str(source))
(module / "go.mod").write_text(base_mod)
shutil.copy2(ROOT / "tools/oracle/go.sum", module / "go.sum")
for source_file in sorted((ROOT / "tools/engine-oracle").glob("*.go")):
    shutil.copy2(source_file, module / source_file.name)
output = REFERENCE / "bin" / ("tala-engine-oracle-trace" if args.trace else "tala-engine-oracle")
output.parent.mkdir(parents=True, exist_ok=True)
# The trace checkout contains the in-tree oracle driver used for local
# experiments.  Its import path deliberately matches this temporary module,
# so Go reports an ambiguous import if both directories are visible.  Hide the
# disposable driver for the duration of this build and restore it even when
# compilation fails; the checkout itself remains untouched.
source_driver = source / "d2layouts/d2talalayout/tools/rust-parity-oracle"
hidden_driver = Path("/tmp") / "d2-rs-rust-parity-oracle-build"
driver_was_hidden = False
if source_driver.is_dir():
    if hidden_driver.exists():
        shutil.rmtree(hidden_driver)
    source_driver.rename(hidden_driver)
    driver_was_hidden = True
try:
    subprocess.run(["go", "build", "-mod=readonly", "-o", str(output), "."], cwd=module,
                   env={**os.environ, "GOTOOLCHAIN": MANIFEST["d2"]["go_toolchain"],
                        "GOMODCACHE": str(REFERENCE / "go-mod/pkg/mod")}, check=True)
finally:
    if driver_was_hidden:
        source_driver.parent.mkdir(parents=True, exist_ok=True)
        hidden_driver.rename(source_driver)
verify_source()
build_info = subprocess.check_output(["go", "version", "-m", str(output)], text=True)
if MANIFEST["d2"]["go_toolchain"] not in build_info.splitlines()[0]:
    raise RuntimeError("Direct engine oracle toolchain mismatch")
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
record = {"source_commit": MANIFEST["d2"]["commit"],
          "go_toolchain": MANIFEST["d2"]["go_toolchain"],
          "binary_sha256": digest(output),
          "driver_sha256": digest(ROOT / "tools/engine-oracle/main.go"),
          "go_mod_sha256": digest(module / "go.mod"),
          "go_sum_sha256": digest(module / "go.sum"),
          "build_command": ("uv run --no-project python scripts/build-engine-oracle.py"
                            + (" --trace" if args.trace else "")),
          "scope": ("instrumented direct engine oracle, diagnostic-only"
                    if args.trace else "pristine direct engine oracle, not the official D2 release binary"),
          "trace_patch_sha256": trace_patch_sha256}
output.with_suffix(".provenance.json").write_text(json.dumps(record, indent=2) + "\n")
print(output)
