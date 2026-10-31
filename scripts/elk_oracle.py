"""Build and run pinned elk-go oracles for `elk-*` fixture recipes.

elk-go (github.com/d2lang/elk-go) is a third-party module with no
dependencies of its own. Harnesses build in a minimal workdir module that
requires only elk-go at the version pinned in reference-manifest.json
`dependencies`, so these recipes key on that version and the Go toolchain,
not on the D2 pin (docs/FIXTURES.md). Builds use the runner's absolute
GOMODCACHE/GOCACHE.

Oracle architecture: the runner's --goarch (default arm64). The Go compiler
fuses x*y+z into FMA instructions on arm64 but not on amd64, so elk-go float
output differs per architecture; the d2-rs-elk ports were qualified against
Go on arm64. On a host of another architecture the runner has already checked
that foreign binaries run transparently (binfmt/qemu or Rosetta); emulation is
bit-exact for these scalar instructions. Per-case deadlines are scaled by
EMULATION_SLOWDOWN there, so they stay comparable to native time
(docs/FIXTURES.md, "Host architecture").
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess

from fixture_recipe import go_env, workdir
from references import MANIFEST, ROOT

MODULE = "github.com/d2lang/elk-go"
VERSION = MANIFEST["dependencies"][MODULE]
WIRE_ORACLE = ROOT / "tools/elk-oracle/main.go"
GOARCH = os.environ.get("D2RS_GOARCH") or "arm64"
EMULATION_SLOWDOWN = 10
HOST_GOARCH = {"x86_64": "amd64", "aarch64": "arm64"}.get(platform.machine().lower(), platform.machine().lower())
NATIVE = HOST_GOARCH == GOARCH


def _go_directive() -> str:
    match = re.search(r"(?m)^go \S+$", (ROOT / "tools/oracle/go.mod").read_text())
    assert match, "tools/oracle/go.mod has no go directive"
    return match.group(0)


def build(harness: Path, name: str, *, module_path: str | None = None) -> list[str]:
    """`go build` `harness` as main.go of a module requiring only elk-go; returns the argv prefix to run it.

    `module_path` defaults to a local name; a path under
    github.com/d2lang/elk-go/ lets the harness import elk-go's `internal`
    packages (e.g. javacompat), exactly as the module's own code does.
    """
    module = workdir() / name
    module.mkdir(parents=True, exist_ok=True)
    shutil.copy2(harness, module / "main.go")
    shutil.copy2(ROOT / "tools/oracle/go.sum", module / "go.sum")
    (module / "go.mod").write_text(
        f"module {module_path or 'd2rs.local/' + name}\n\n{_go_directive()}\n\nrequire {MODULE} {VERSION}\n")
    binary = workdir() / "bin" / name
    env = {**go_env(), "GOARCH": GOARCH, "CGO_ENABLED": "0"}
    subprocess.run(["go", "build", "-mod=readonly", "-trimpath", "-o", str(binary), "."], cwd=module,
                   env=env, check=True)
    return [str(binary)]


def build_upstream(name: str, package: str, overlay: dict[str, Path], *, test: bool = False) -> list[str]:
    """Build an upstream D2 package with `overlay` (upstream-relative path -> file) added, for GOARCH.

    Runs in the runner's patched upstream tree ($D2RS_UPSTREAM) with its own
    go.mod. `test` builds the package's test binary (`go test -c`), so
    overlaid `_test.go` hooks can reach package internals; otherwise the
    package must be a `main` package (it may be a new overlay-only directory).
    """
    upstream = Path(os.environ["D2RS_UPSTREAM"])
    overlay_file = workdir() / f"{name}-overlay.json"
    overlay_file.write_text(json.dumps({"Replace": {str(upstream / k): str(v) for k, v in overlay.items()}}))
    binary = workdir() / "bin" / name
    env = {**go_env(), "GOARCH": GOARCH, "CGO_ENABLED": "0"}
    argv = ["go", "test", "-c"] if test else ["go", "build"]
    subprocess.run([*argv, "-mod=readonly", "-trimpath", "-overlay", str(overlay_file), "-o", str(binary),
                    "./" + package], cwd=upstream, env=env, check=True)
    return [str(binary)]


def wire_oracle() -> list[str]:
    """tools/elk-oracle: one ELK JSON graph on stdin -> elk.LayoutJSON output."""
    return build(WIRE_ORACLE, "elk-wire-oracle")


GO_RUNTIME_FATAL_EXIT = 2
STACK_ELIDED = "[goroutine stacks elided by scripts/elk_oracle.py]\n"


def stable_stderr(returncode: int, stderr: str) -> str:
    """`stderr` with Go runtime crash dumps reduced to their stable header.

    A Go runtime fatal error (exit 2: stack overflow, unrecovered panic) prints
    goroutine stacks whose addresses, goroutine numbers and frame arguments
    change on every run. Keep the header up to the first blank line, minus the
    lines that carry addresses (`runtime: sp=0x...`), e.g.
    "runtime: goroutine stack exceeds 1000000000-byte limit\nfatal error: stack overflow\n",
    then a fixed marker. Any other exit keeps its stderr verbatim.
    """
    if returncode != GO_RUNTIME_FATAL_EXIT:
        return stderr
    header = stderr.split("\n\n", 1)[0].split("\n")
    return "".join(line + "\n" for line in header if line and "0x" not in line) + STACK_ELIDED


def negative_zero_int(text: str):
    """`parse_int` keeping Go's `-0` as -0.0 (some captures recorded it this way)."""
    return -0.0 if text == "-0" else int(text)


def run(oracle: list[str], stdin: bytes, *, timeout: float | None = None,
        env: dict | None = None) -> subprocess.CompletedProcess:
    """Run an oracle; `timeout` is in native seconds (scaled under emulation)."""
    if timeout is not None and not NATIVE:
        timeout *= EMULATION_SLOWDOWN
    return subprocess.run(oracle, input=stdin, capture_output=True, timeout=timeout,
                          env=None if env is None else {**os.environ, **env})


def write_json(path: Path, value, *, indent: int | None = 2, ensure_ascii: bool = True):
    """Python `json.dumps(..., indent=2)` + newline: the captures' original layout."""
    if indent is None:
        text = json.dumps(value, separators=(",", ":"), ensure_ascii=ensure_ascii)
    else:
        text = json.dumps(value, indent=indent, ensure_ascii=ensure_ascii)
    path.write_text(text + "\n", encoding="utf-8")
