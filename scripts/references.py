#!/usr/bin/env python3
"""Acquire and verify durable, pinned D2 development references."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = json.loads((ROOT / "reference-manifest.json").read_text())
REFERENCE = ROOT / ".reference"
# The compatibility target every scoreboard compares against.
SOURCE_COMMIT = MANIFEST["d2"]["commit"]
TRACE_CHECKOUT = (ROOT / MANIFEST["d2"]["trace_checkout"]).resolve()
GO_ORACLE = ROOT / MANIFEST["d2"]["oracle"]
# The same CLI with seedable map order (scripts/build-mapseed-oracle.py).
GO_MAPSEED_ORACLE = GO_ORACLE.with_name(GO_ORACLE.name + "-mapseed")
E2E_CASES = ROOT / MANIFEST["d2"]["e2e_cases"]
# Regenerated Go-vs-Rust reports and inventories: gitignored, never committed
# (docs/FIXTURES.md). Checkers write their default reports here.
REPORTS = ROOT / "oracle-cache/reports"


def read_report(name: str, producer: str):
    """Load a regenerated report another script reads as input or baseline."""
    path = REPORTS / name
    if not path.is_file():
        raise SystemExit(
            f"missing {path.relative_to(ROOT)}; regenerate it first with "
            f"`uv run --no-project python {producer}`"
        )
    return json.loads(path.read_text())


def command(*args: str, cwd: Path | None = None, env=None) -> str:
    return subprocess.check_output(args, cwd=cwd, env=env, text=True).strip()


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_source() -> Path:
    source = (ROOT / MANIFEST["d2"]["checkout"]).resolve()
    actual = command("git", "rev-parse", "HEAD", cwd=source)
    if actual != MANIFEST["d2"]["commit"]:
        raise RuntimeError(f"Reference commit mismatch: {actual}")
    if command("git", "status", "--porcelain", "--untracked-files=all", cwd=source):
        raise RuntimeError(f"Pristine reference is dirty: {source}")
    return source


def verify_oracle(binary: Path, *, trace: bool = False) -> dict:
    """Reject stale development oracles before accepting parity evidence."""
    provenance = binary.with_suffix(".provenance.json")
    record = json.loads(provenance.read_text())
    expected = {
        "source_commit": MANIFEST["d2"]["commit"],
        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
        "binary_sha256": digest(binary),
        "trace_patch_sha256": digest(ROOT / "tools/patches/d2-v090-stage-trace.patch") if trace else None,
    }
    for key, value in expected.items():
        if record.get(key) != value:
            raise RuntimeError(f"Stale oracle {binary}: {key} mismatch; rebuild with scripts/build-oracle.py" + (" --trace" if trace else ""))
    return record


def host_name() -> str:
    system = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}[platform.system()]
    arch = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "amd64", "AMD64": "amd64"}[platform.machine()]
    return f"{system}-{arch}"


def release(host: str, fetch: bool) -> Path:
    pin = MANIFEST["releases"][host]
    directory = REFERENCE / "releases" / host
    directory.mkdir(parents=True, exist_ok=True)
    archive = REFERENCE / "releases" / pin["archive"]
    if not archive.exists() and fetch:
        partial = archive.with_suffix(archive.suffix + ".partial")
        try:
            with urllib.request.urlopen(pin["url"], timeout=120) as response, partial.open("wb") as output:
                shutil.copyfileobj(response, output)
            if digest(partial) != pin["sha256"]:
                raise RuntimeError("Downloaded archive checksum mismatch")
            partial.replace(archive)
        finally:
            partial.unlink(missing_ok=True)
    if not archive.exists() or digest(archive) != pin["sha256"]:
        raise RuntimeError(f"Missing or incorrect release archive: {archive}")
    binary = directory / "d2-v0.9.0" / "bin" / ("d2.exe" if host.startswith("windows") else "d2")
    if fetch:
        with tarfile.open(archive) as contents:
            contents.extractall(directory, filter="data")
    if not binary.is_file():
        raise RuntimeError("Release not extracted; run references.py fetch")
    # Compare extracted bytes with the member of the checksum-verified archive,
    # even for platforms without a separate binary hash in the manifest.
    with tarfile.open(archive) as contents:
        member = contents.extractfile(binary.relative_to(directory).as_posix())
        if member is None or hashlib.file_digest(member, "sha256").hexdigest() != digest(binary):
            raise RuntimeError(f"Extracted binary differs from verified archive: {binary}")
    if pin.get("binary_sha256") and digest(binary) != pin["binary_sha256"]:
        raise RuntimeError("Release binary checksum mismatch")
    if host == host_name():
        if command(str(binary), "--version") != "v0.9.0":
            raise RuntimeError("Release reports an unexpected version")
        metadata = command("go", "version", "-m", str(binary))
        if MANIFEST["d2"]["go_toolchain"] not in metadata.splitlines()[0]:
            raise RuntimeError("Release Go toolchain mismatch")
    return binary


def source_builds() -> None:
    for name, pin in MANIFEST.get("source_builds", {}).items():
        binary = (ROOT / pin["binary"]).resolve()
        if not binary.is_file():
            raise RuntimeError(f"Missing source build {name}: {binary}")
        if digest(binary) != pin["binary_sha256"]:
            raise RuntimeError(f"Source build checksum mismatch: {binary}")
        metadata = command("go", "version", "-m", str(binary))
        if pin["go_toolchain"] not in metadata.splitlines()[0]:
            raise RuntimeError(f"Source build Go toolchain mismatch: {binary}")
        if command(str(binary), "--version") != pin["reported_version"]:
            raise RuntimeError(f"Source build version mismatch: {binary}")


def dependencies() -> None:
    source = verify_source()
    # A copied module manifest keeps any Go module bookkeeping away from the
    # pristine checkout. Downloaded sources/checksums live entirely under .reference.
    module = REFERENCE / "dependency-module"
    module.mkdir(parents=True, exist_ok=True)
    for name in ("go.mod", "go.sum"):
        shutil.copy2(source / name, module / name)
    environment = {**os.environ, "GOTOOLCHAIN": MANIFEST["d2"]["go_toolchain"],
                   "GOMODCACHE": str(REFERENCE / "go-mod/pkg/mod")}
    subprocess.run(["go", "mod", "download"], cwd=module, env=environment, check=True)
    subprocess.run(["go", "mod", "verify"], cwd=module, env=environment, check=True)
    verify_source()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("fetch", "verify", "dependencies"))
    parser.add_argument("--host", default=host_name(), choices=MANIFEST["releases"])
    args = parser.parse_args()
    source = verify_source()
    if args.action == "verify":
        source_builds()
    if args.action == "dependencies":
        dependencies()
    else:
        binary = release(args.host, args.action == "fetch")
        print(json.dumps({"source": str(source), "commit": MANIFEST["d2"]["commit"],
                          "binary": str(binary), "binary_sha256": digest(binary)}, indent=2))


if __name__ == "__main__":
    main()
