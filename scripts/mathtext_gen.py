"""Shared helpers for the mathtext fixture input generators (scripts/fixture-gen-*.py)."""
from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path

import fixture_recipe

ROOT = Path(__file__).resolve().parent.parent


def required_version(go_mod: Path, module: str) -> str:
    """The version of `module` required by a repo go.mod."""
    match = re.search(rf"(?m)^\s*(?:require\s+)?{re.escape(module)} (\S+)", go_mod.read_text())
    if not match:
        raise SystemExit(f"{go_mod} does not require {module}")
    return match.group(1)


def module_dir(module: str, version: str) -> Path:
    """Source directory of module@version in the runner's shared GOMODCACHE (downloaded if needed)."""
    out = subprocess.run(["go", "mod", "download", "-json", f"{module}@{version}"],
                         cwd=fixture_recipe.workdir(), env=fixture_recipe.go_env(),
                         check=True, capture_output=True, text=True).stdout
    return Path(json.loads(out)["Dir"])


def write_output(value) -> None:
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(value, ensure_ascii=False) + "\n")
