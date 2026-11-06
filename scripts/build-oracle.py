#!/usr/bin/env python3
"""Build the development oracle against the verified pristine local checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from references import ROOT, REFERENCE, MANIFEST, verify_source

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--trace", action="store_true", help="use the separate instrumented worktree")
args = parser.parse_args()
verify_source()
module = ROOT / "tools/oracle"
output = REFERENCE / "bin" / ("d2-oracle-trace" if args.trace else "d2-oracle")
if args.trace:
    trace = (ROOT / MANIFEST["d2"]["trace_checkout"]).resolve()
    revision = subprocess.check_output(["git", "-C", str(trace), "rev-parse", "HEAD"], text=True).strip()
    if revision != MANIFEST["d2"]["commit"]:
        raise RuntimeError("Trace worktree revision mismatch")
    # Permit exactly the checked-in instrumentation patch, not arbitrary edits.
    expected = (ROOT / "tools/patches/d2-v090-stage-trace.patch").read_bytes()
    # Compare both tracked modifications and new files against the patch.  A
    # new trace helper is untracked in this worktree by design, so relying on
    # `git diff --name-only` alone would reject the exact patch.
    tracked_paths = subprocess.check_output(
        ["git", "-C", str(trace), "diff", "HEAD", "--name-only"], text=True
    ).splitlines()
    untracked_paths = subprocess.check_output(
        ["git", "-C", str(trace), "ls-files", "--others", "--exclude-standard"],
        text=True,
    ).splitlines()
    patch_paths = sorted(
        {
            line.split(" b/", 1)[1]
            for line in expected.decode().splitlines()
            if line.startswith("diff --git a/") and " b/" in line
        }
    )
    paths = sorted(set(tracked_paths) | set(untracked_paths))
    if paths != patch_paths:
        raise RuntimeError("Unexpected instrumented files")
    # Materialize the intended patch in a disposable index, never the working
    # index. Comparing complete bytes also catches unrelated edits in this file.
    with tempfile.TemporaryDirectory() as scratch:
        environment = {**os.environ, "GIT_INDEX_FILE": str(Path(scratch) / "index")}
        subprocess.run(["git", "-C", str(trace), "read-tree", "HEAD"], env=environment, check=True)
        subprocess.run(["git", "-C", str(trace), "apply", "--cached", "-"], input=expected, env=environment, check=True)
        for path in paths:
            intended = subprocess.check_output(["git", "-C", str(trace), "show", ":" + path], env=environment)
            if intended != (trace / path).read_bytes():
                raise RuntimeError("Trace source contains edits outside the pinned patch")
    module = REFERENCE / "trace-oracle-module"
    module.mkdir(parents=True, exist_ok=True)
    for name in ("main.go", "go.mod", "go.sum"):
        shutil.copy2(ROOT / "tools/oracle" / name, module / name)
    mod = module / "go.mod"
    mod.write_text(mod.read_text().replace("../../../d2", str(trace)))
output.parent.mkdir(parents=True, exist_ok=True)
subprocess.run(["go", "build", "-mod=readonly", "-o", str(output), "."],
               cwd=module, check=True,
               env={**os.environ, "GOTOOLCHAIN": MANIFEST["d2"]["go_toolchain"],
                    "GOMODCACHE": str(REFERENCE / "go-mod/pkg/mod")})
verify_source()
record = {"source_commit": MANIFEST["d2"]["commit"],
          "go_toolchain": MANIFEST["d2"]["go_toolchain"],
          "binary_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
          "trace_patch_sha256": hashlib.sha256(expected).hexdigest() if args.trace else None}
output.with_suffix(".provenance.json").write_text(json.dumps(record, indent=2) + "\n")
print(output)
