#!/usr/bin/env python3
"""Build a Go 1.27.0 elk-go oracle with diagnostic layout traces."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess

from references import ROOT, REFERENCE, MANIFEST, verify_source

source = verify_source()
baseline = REFERENCE / "go-mod/github.com/d2lang/elk-go@v0.2.0"
trace_module = REFERENCE / "elk-go-v0.2.0-trace"
patch_file = ROOT / "tools/patches/elk-go-v0.2.0-bk-trace.patch"
p3_scope_patch_file = ROOT / "tools/patches/elk-go-v0.2.0-p3-scope-trace.patch"
p5_trace_patch_file = ROOT / "tools/patches/elk-go-v0.2.0-p5-trace.patch"
p5_segments_trace_patch_file = ROOT / "tools/patches/elk-go-v0.2.0-p5-segments.patch"
bk_compaction_trace_patch_file = ROOT / "tools/patches/elk-go-v0.2.0-bk-compaction-trace.patch"
port_sort_json_patch_file = ROOT / "tools/patches/elk-go-v0.2.0-port-sort-json.patch"
driver = ROOT / "tools/elk-oracle-trace/main.go"
module = ROOT / "tools/elk-oracle-trace"
output = REFERENCE / "bin/elk-wire-trace-oracle"


def make_tree_writable(directory: Path) -> None:
    for path in [directory, *directory.rglob("*")]:
        mode = path.stat().st_mode | stat.S_IWUSR
        if path.is_dir():
            mode |= stat.S_IXUSR
        path.chmod(mode)


if not baseline.is_dir():
    raise RuntimeError(f"Missing pinned elk-go source: {baseline}")
if trace_module.exists():
    make_tree_writable(trace_module)
    shutil.rmtree(trace_module)
shutil.copytree(baseline, trace_module)
make_tree_writable(trace_module)
patch_environment = {**os.environ, "TMPDIR": "/tmp"}
for source_patch in (
    patch_file,
    p3_scope_patch_file,
    p5_trace_patch_file,
    p5_segments_trace_patch_file,
    bk_compaction_trace_patch_file,
    port_sort_json_patch_file,
):
    subprocess.run(
        ["patch", "-p1", "-i", str(source_patch)],
        cwd=trace_module,
        env=patch_environment,
        check=True,
    )

output.parent.mkdir(parents=True, exist_ok=True)
environment = {**os.environ, "GOTOOLCHAIN": MANIFEST["d2"]["go_toolchain"]}
subprocess.run(
    ["go", "build", "-tags", "d2elkdiagnostic", "-trimpath", "-o", str(output), "."],
    cwd=module,
    env=environment,
    check=True,
)
verify_source()
info = subprocess.check_output(["go", "version", "-m", str(output)], env=environment, text=True)
if MANIFEST["d2"]["go_toolchain"] not in info.splitlines()[0]:
    raise RuntimeError("ELK trace oracle toolchain mismatch")
if "github.com/d2lang/elk-go\tv0.2.0" not in info:
    raise RuntimeError("ELK trace oracle dependency mismatch")

def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def digest_tree(directory: Path) -> str:
    tree_hash = hashlib.sha256()
    for path in sorted(item for item in directory.rglob("*") if item.is_file()):
        tree_hash.update(path.relative_to(directory).as_posix().encode())
        tree_hash.update(b"\0")
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                tree_hash.update(chunk)
    return tree_hash.hexdigest()


record = {
    "source_commit": MANIFEST["d2"]["commit"],
    "go_toolchain": MANIFEST["d2"]["go_toolchain"],
    "elk_go_module": "github.com/d2lang/elk-go v0.2.0",
    "elk_go_source_tree_sha256": digest_tree(baseline),
    "patch_sha256": digest(patch_file),
    "p3_scope_patch_sha256": digest(p3_scope_patch_file),
    "p5_trace_patch_sha256": digest(p5_trace_patch_file),
    "p5_segments_trace_patch_sha256": digest(p5_segments_trace_patch_file),
    "bk_compaction_trace_patch_sha256": digest(bk_compaction_trace_patch_file),
    "port_sort_json_patch_sha256": digest(port_sort_json_patch_file),
    "driver_sha256": digest(driver),
    "binary_sha256": digest(output),
    "build_info": info,
    "build_command": "uv run --no-project python scripts/build-elk-trace-oracle.py",
    "trace_environment": "Set D2_ELK_TRACE=1 to emit valid JSONL port-order, P3 scope/RNG, Brandes-Koepf candidates and spacing constraints, and per-layer P5 routing events on stderr.",
    "scope": "Development oracle only; the diagnostic build tag is not used by release artifacts.",
}
(REFERENCE / "elk-wire-trace-oracle-build.json").write_text(json.dumps(record, indent=2) + "\n")
print(output)
