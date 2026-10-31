"""Build pinned Goldmark oracle harnesses (tools/markdown-*-oracle) for recipes.

Goldmark is a third-party module: harnesses build in a minimal module that
requires only github.com/yuin/goldmark at the version pinned in
reference-manifest.json `dependencies`, not against the D2 checkout. Builds
happen in the recipe workdir with the runner's absolute GOMODCACHE/GOCACHE.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess

from fixture_recipe import go_env, workdir
from references import MANIFEST, ROOT

MODULE = "github.com/yuin/goldmark"
VERSION = MANIFEST["dependencies"][MODULE]


def _go_directive() -> str:
    match = re.search(r"(?m)^go \S+$", (ROOT / "tools/oracle/go.mod").read_text())
    assert match, "tools/oracle/go.mod has no go directive"
    return match.group(0)


def goldmark_source() -> Path:
    """Pristine module directory in GOMODCACHE (downloaded if absent)."""
    result = subprocess.run(["go", "mod", "download", "-json", f"{MODULE}@{VERSION}"], cwd=workdir(),
                            env=go_env(), check=True, stdout=subprocess.PIPE)
    return Path(json.loads(result.stdout)["Dir"])


def prepare_module(name: str, harness: Path, *, copy_goldmark: bool = False) -> tuple[Path, Path | None]:
    """Module dir holding `harness` as main.go; optionally a writable Goldmark copy.

    The copy lets `-overlay` add or replace Goldmark files without touching
    the shared module cache; it is verified byte-identical to the pinned
    module before use.
    """
    module = workdir() / name
    module.mkdir(parents=True, exist_ok=True)
    shutil.copy2(harness, module / "main.go")
    shutil.copy2(ROOT / "tools/oracle/go.sum", module / "go.sum")
    go_mod = f"module d2rs.local/{name}\n\n{_go_directive()}\n\nrequire {MODULE} {VERSION}\n"
    copy = None
    if copy_goldmark:
        pristine = goldmark_source()
        copy = module / "goldmark"
        shutil.copytree(pristine, copy)
        for p in [copy, *copy.rglob("*")]:
            p.chmod(p.stat().st_mode | stat.S_IWUSR)
        listing = lambda root: {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*") if p.is_file()}
        assert listing(pristine) == listing(copy), "Goldmark copy differs from the pinned module"
        go_mod += f"\nreplace {MODULE} => {copy}\n"
    (module / "go.mod").write_text(go_mod)
    return module, copy


def build(module: Path, *, overlay: dict[str, str] | None = None) -> Path:
    """`go build` the module into the workdir and return the binary path."""
    binary = workdir() / "bin" / module.name
    argv = ["go", "build", "-mod=readonly"]
    if overlay:
        overlay_file = module / "overlay.json"
        overlay_file.write_text(json.dumps({"Replace": overlay}))
        argv += ["-overlay", str(overlay_file)]
    subprocess.run([*argv, "-o", str(binary), "."], cwd=module, env=go_env(), check=True)
    return binary


def run_json(binary: Path, requests, *args: str, timeout: float | None = None):
    """Feed JSON requests on stdin and decode the oracle's JSON stdout."""
    stdin = None if requests is None else json.dumps(requests).encode()
    result = subprocess.run([str(binary), *args], input=stdin, stdout=subprocess.PIPE, check=True,
                            timeout=timeout, env={**os.environ})
    return json.loads(result.stdout)
